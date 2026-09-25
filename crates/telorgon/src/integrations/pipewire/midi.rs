//! MIDI filter ownership and the realtime SPA sequence boundary.
use super::{MediaError, RegistrySnapshot, connection::native};
use crate::media::midi::*;
use pipewire::{
    self as pw,
    spa::{self, sys as spa_sys},
    sys,
};
use std::{
    cell::UnsafeCell,
    ffi::{CStr, CString, c_void},
    ptr::NonNull,
    sync::{Arc, atomic::Ordering},
};

struct Realtime {
    port: Option<NonNull<c_void>>,
    endpoint: MidiEndpoint,
    representation: MidiRepresentation,
    pending: Option<MidiEvent>,
    previous: Option<(MidiTime, u64)>,
    work_limit: usize,
}
struct Callbacks {
    shared: Arc<MidiShared>,
    rt: UnsafeCell<Realtime>,
}
pub(crate) struct NativeMidi {
    filter: NonNull<sys::pw_filter>,
    _core: pw::core::CoreRc,
    callbacks: Box<Callbacks>,
    events: Box<sys::pw_filter_events>,
    hook: Box<spa_sys::spa_hook>,
}
impl NativeMidi {
    pub fn create(
        core: pw::core::CoreRc,
        config: MidiConfig,
        shared: Arc<MidiShared>,
        endpoint: MidiEndpoint,
    ) -> Result<Self, MediaError> {
        let name = CString::new(config.name.as_str())
            .map_err(|_| MediaError::InvalidArgument("MIDI name"))?;
        let output = config.direction == MidiDirection::Output;
        let mut props = pw::properties::properties! {"media.type"=>"Midi","media.category"=>if output {"Playback"}else{"Capture"},
        "media.class"=>if output {"Midi/Source"}else{"Midi/Sink"},"node.name"=>config.name.clone(),
        "node.description"=>config.name.clone(),"node.want-driver"=>"true","node.always-process"=>"true"};
        props.insert(super::owned_node::OWNER_KEY, shared.node.token());
        // SAFETY: owned core outlives filter; native call takes properties and copies name.
        let filter = NonNull::new(unsafe {
            sys::pw_filter_new(core.as_raw_ptr(), name.as_ptr(), props.into_raw())
        })
        .ok_or_else(|| native("create MIDI filter"))?;
        let mut result = Self {
            filter,
            _core: core,
            callbacks: Box::new(Callbacks {
                shared,
                rt: UnsafeCell::new(Realtime {
                    port: None,
                    endpoint,
                    representation: config.representation,
                    pending: None,
                    previous: None,
                    work_limit: config.queue_events,
                }),
            }),
            // SAFETY: add_listener initializes hook; absent event function pointers are null.
            events: Box::new(unsafe { std::mem::zeroed() }),
            hook: Box::new(unsafe { std::mem::zeroed() }),
        };
        result.events.version = 1;
        result.events.state_changed = Some(state_changed);
        result.events.process = Some(process);
        // SAFETY: callback, hook and event boxes remain at stable addresses until destroy.
        unsafe {
            sys::pw_filter_add_listener(
                filter.as_ptr(),
                &mut *result.hook,
                &*result.events,
                (&*result.callbacks as *const Callbacks).cast_mut().cast(),
            );
        }
        let props = pw::properties::properties! {"format.dsp"=>match config.representation {MidiRepresentation::Midi1=>"8 bit raw midi",MidiRepresentation::Ump=>"32 bit raw UMP"},"port.name"=>if output {"output"}else{"input"}};
        // SAFETY: filter is unconnected; properties transfer ownership. Port storage is an
        // opaque identifier owned by the filter, never dereferenced as Rust memory.
        let port = NonNull::new(unsafe {
            sys::pw_filter_add_port(
                filter.as_ptr(),
                if output {
                    spa_sys::SPA_DIRECTION_OUTPUT
                } else {
                    spa_sys::SPA_DIRECTION_INPUT
                },
                sys::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
                1,
                props.into_raw(),
                std::ptr::null_mut(),
                0,
            )
        })
        .ok_or_else(|| native("create MIDI port"))?;
        result.callbacks.rt.get_mut().port = Some(port);
        let bytes = super::parameters::encode(
            spa_sys::SPA_TYPE_OBJECT_ParamBuffers,
            spa_sys::SPA_PARAM_Buffers,
            vec![
                spa::pod::Property::new(
                    spa_sys::SPA_PARAM_BUFFERS_buffers,
                    spa::pod::Value::Int(2),
                ),
                spa::pod::Property::new(spa_sys::SPA_PARAM_BUFFERS_blocks, spa::pod::Value::Int(1)),
                spa::pod::Property::new(
                    spa_sys::SPA_PARAM_BUFFERS_size,
                    spa::pod::Value::Int(16 * 1024),
                ),
                spa::pod::Property::new(spa_sys::SPA_PARAM_BUFFERS_stride, spa::pod::Value::Int(1)),
            ],
        )?;
        let pod = spa::pod::Pod::from_bytes(&bytes)
            .ok_or(MediaError::InvalidArgument("MIDI buffer POD"))?;
        let mut params = [pod.as_raw_ptr().cast_const()];
        // SAFETY: port belongs to this filter, which copies the parameter synchronously.
        let status = unsafe {
            sys::pw_filter_update_params(filter.as_ptr(), port.as_ptr(), params.as_mut_ptr(), 1)
        };
        if status < 0 {
            return Err(native(std::io::Error::from_raw_os_error(-status)));
        }
        // SAFETY: no parameter pointers; all callback state is initialized before connect.
        let status = unsafe {
            sys::pw_filter_connect(
                filter.as_ptr(),
                sys::pw_filter_flags_PW_FILTER_FLAG_RT_PROCESS,
                std::ptr::null_mut(),
                0,
            )
        };
        if status < 0 {
            return Err(native(std::io::Error::from_raw_os_error(-status)));
        }
        Ok(result)
    }
    pub fn poll(&mut self, snapshot: &RegistrySnapshot) -> bool {
        let shared = &self.callbacks.shared;
        if shared.stop.load(Ordering::Acquire) {
            return false;
        }
        // SAFETY: query on native control loop while filter is owned.
        let native_id = unsafe { sys::pw_filter_get_node_id(self.filter.as_ptr()) };
        if let Err(error) = shared.node.update(snapshot, native_id) {
            shared.state(MidiState::Failed(error));
            shared.stop.store(true, Ordering::Release);
            return false;
        }
        !matches!(
            *shared.state.lock().unwrap_or_else(|e| e.into_inner()),
            MidiState::Failed(_) | MidiState::Stopped
        )
    }
}
impl Drop for NativeMidi {
    fn drop(&mut self) {
        // SAFETY: disconnect quiesces data-loop processing, then remove registration and
        // destroy before deallocating the boxes containing callbacks and event queues.
        unsafe {
            sys::pw_filter_disconnect(self.filter.as_ptr());
            spa::utils::hook::remove(*self.hook);
            sys::pw_filter_destroy(self.filter.as_ptr());
        }
        self.callbacks.shared.state(MidiState::Stopped);
    }
}
unsafe extern "C" fn state_changed(
    data: *mut c_void,
    _old: sys::pw_filter_state,
    state: sys::pw_filter_state,
    error: *const std::ffi::c_char,
) {
    // SAFETY: control-loop callback, data points at the stable registered box.
    let callbacks = unsafe { &*data.cast::<Callbacks>() };
    let state = match state {
        sys::pw_filter_state_PW_FILTER_STATE_PAUSED => MidiState::Paused,
        sys::pw_filter_state_PW_FILTER_STATE_STREAMING => MidiState::Streaming,
        sys::pw_filter_state_PW_FILTER_STATE_ERROR => MidiState::Failed(if error.is_null() {
            native("MIDI filter error")
        } else {
            native(unsafe { CStr::from_ptr(error) }.to_string_lossy())
        }),
        sys::pw_filter_state_PW_FILTER_STATE_UNCONNECTED => MidiState::Stopped,
        _ => MidiState::Connecting,
    };
    callbacks.shared.state(state);
}
/// One native buffer lease, returned even when validation fails. Never leaves the callback.
struct Buffer {
    port: NonNull<c_void>,
    buffer: NonNull<sys::pw_buffer>,
}
impl Drop for Buffer {
    fn drop(&mut self) {
        // SAFETY: the buffer was dequeued from this live port during this callback, exactly once.
        unsafe {
            sys::pw_filter_queue_buffer(self.port.as_ptr(), self.buffer.as_ptr());
        }
    }
}
unsafe extern "C" fn process(data: *mut c_void, position: *mut spa_sys::spa_io_position) {
    if data.is_null() || position.is_null() {
        return;
    }
    // SAFETY: PipeWire serializes the filter process callback. No other callback accesses
    // rt, and disconnect completes before rt is destroyed. Position is borrowed this cycle.
    let callbacks = unsafe { &*data.cast::<Callbacks>() };
    let rt = unsafe { &mut *callbacks.rt.get() };
    let clock = unsafe { &(*position).clock };
    let shared = &callbacks.shared;
    let mut time = MidiTime {
        generation: rt.previous.map_or(0, |(time, _)| time.generation),
        clock_id: clock.id,
        position: clock.position,
        monotonic_ns: clock.nsec,
        rate_num: clock.rate.num,
        rate_denom: clock.rate.denom,
    };
    if rt.previous.is_none_or(|(old, frames)| {
        old.clock_id != time.clock_id
            || old.position.saturating_add(frames) != time.position
            || time.monotonic_ns < old.monotonic_ns
            || old.rate_num != time.rate_num
            || old.rate_denom != time.rate_denom
    }) {
        let Some(generation) = time.generation.checked_add(1) else {
            shared.stop.store(true, Ordering::Release);
            return;
        };
        time.generation = generation;
        shared.discontinuities.fetch_add(1, Ordering::Relaxed);
    }
    rt.previous = Some((time, clock.duration));
    shared.publish_clock(time);
    let Some(port) = rt.port else {
        return;
    };
    // SAFETY: live MIDI port, RT-safe dequeue; returned buffer is exclusively leased.
    let Some(buffer) = NonNull::new(unsafe { sys::pw_filter_dequeue_buffer(port.as_ptr()) }) else {
        return;
    };
    let mut lease = Buffer { port, buffer };
    // SAFETY: PipeWire owns the buffer/plane structures for the full lease lifetime.
    let pw_buffer = unsafe { lease.buffer.as_mut() };
    let Some(buffer) = (unsafe { pw_buffer.buffer.as_mut() }) else {
        return;
    };
    if buffer.n_datas != 1 || buffer.datas.is_null() {
        shared.malformed.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let plane = unsafe { &mut *buffer.datas };
    let Some(chunk) = (unsafe { plane.chunk.as_mut() }) else {
        return;
    };
    if plane.data.is_null() || plane.maxsize > 1024 * 1024 {
        shared.malformed.fetch_add(1, Ordering::Relaxed);
        return;
    }
    match &mut rt.endpoint {
        MidiEndpoint::Input(producer) => {
            let offset = chunk.offset as usize;
            let size = chunk.size as usize;
            if offset
                .checked_add(size)
                .is_none_or(|end| end > plane.maxsize as usize)
            {
                shared.malformed.fetch_add(1, Ordering::Relaxed);
                return;
            }
            // SAFETY: mapped data and checked chunk range remain readable until requeue.
            let bytes =
                unsafe { std::slice::from_raw_parts(plane.data.cast::<u8>().add(offset), size) };
            if size == 0 {
                return;
            }
            let mut remaining = rt.work_limit;
            let result = read_sequence(bytes, time, clock.duration, rt.work_limit, |event| {
                if remaining == 0 || shared.stop.load(Ordering::Acquire) {
                    shared.overflows.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                remaining -= 1;
                if event.packet.representation() != rt.representation {
                    shared.malformed.fetch_add(1, Ordering::Relaxed);
                } else if producer.push(event).is_err() {
                    shared.overflows.fetch_add(1, Ordering::Relaxed);
                } else {
                    shared.delivered.fetch_add(1, Ordering::Relaxed);
                }
            });
            match result {
                Err(MediaError::ResourceLimit(_)) => {
                    shared.overflows.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    shared.malformed.fetch_add(1, Ordering::Relaxed);
                }
                Ok(()) => {}
            }
        }
        MidiEndpoint::Output(consumer) => {
            // SAFETY: output plane is exclusively writable until lease requeue. Bounded
            // slice never exceeds negotiated mapped capacity.
            let bytes = unsafe {
                std::slice::from_raw_parts_mut(plane.data.cast::<u8>(), plane.maxsize as usize)
            };
            let mut remaining = rt.work_limit;
            let length = write_sequence(bytes, |space| {
                while remaining > 0 && !shared.stop.load(Ordering::Acquire) {
                    remaining -= 1;
                    if rt.pending.is_none() {
                        rt.pending = consumer.pop().ok();
                    }
                    let event = rt.pending.as_ref()?;
                    if event.time.generation != time.generation
                        || event.time.clock_id != time.clock_id
                        || event.time.rate_num != time.rate_num
                        || event.time.rate_denom != time.rate_denom
                    {
                        rt.pending = None;
                        shared.stale.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    if event.time.position >= time.position.saturating_add(clock.duration) {
                        return None;
                    }
                    let size = 16 + event.packet.bytes().len().div_ceil(8) * 8;
                    if size > space {
                        // A packet that cannot fit even in an empty negotiated buffer must
                        // not permanently block the FIFO. Count and drop that event only.
                        if size + 16 > plane.maxsize as usize {
                            rt.pending = None;
                            shared.overflows.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        return None;
                    }
                    let event = rt.pending.take()?;
                    if event.time.position < time.position {
                        shared.late.fetch_add(1, Ordering::Relaxed);
                    }
                    shared.delivered.fetch_add(1, Ordering::Relaxed);
                    return Some((
                        event
                            .time
                            .position
                            .saturating_sub(time.position)
                            .min(u32::MAX as u64) as u32,
                        event.packet,
                    ));
                }
                None
            });
            chunk.offset = 0;
            chunk.size = length as u32;
            chunk.stride = 1;
            chunk.flags = 0;
            pw_buffer.size = clock.duration;
        }
    }
}
