//! Narrow unsafe pw_filter boundary. The control worker owns the native filter, callback
//! boxes, and core. Disconnect/destroy quiesce processing before callback memory is freed.
use super::{
    MediaError, RegistrySnapshot,
    connection::{Completion, native},
};
use crate::media::audio::{
    FilterClock, FilterConfig, FilterCycle, FilterInputs, FilterOutputs, FilterProcessor,
    FilterShared, FilterState,
};
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
    time::{Duration, Instant},
};
struct Realtime {
    ports: Vec<NonNull<c_void>>,
    config: FilterConfig,
    processor: Box<dyn FilterProcessor>,
    inputs: Vec<f32>,
    outputs: Vec<f32>,
    previous: Option<FilterClock>,
}
struct Callbacks {
    shared: Arc<FilterShared>,
    rt: UnsafeCell<Realtime>,
}
pub(crate) struct NativeFilter {
    filter: NonNull<sys::pw_filter>,
    _core: pw::core::CoreRc,
    callbacks: Box<Callbacks>,
    // Stable allocations for native callback registration. Removed only after disconnect.
    events: Box<sys::pw_filter_events>,
    hook: Box<spa_sys::spa_hook>,
    pending: Option<(bool, Completion, Instant)>,
}
impl NativeFilter {
    pub fn create(
        core: pw::core::CoreRc,
        config: FilterConfig,
        shared: Arc<FilterShared>,
        processor: Box<dyn FilterProcessor>,
    ) -> Result<Self, MediaError> {
        let name = CString::new(config.name.as_str())
            .map_err(|_| MediaError::InvalidArgument("filter name"))?;
        let mut props = pw::properties::properties! {"media.type"=>"Audio","media.category"=>"Filter","media.role"=>"DSP","node.name"=>config.name.clone(),"node.description"=>config.name.clone()};
        props.insert(super::owned_node::OWNER_KEY, shared.node.token());
        // SAFETY: core is owned for the entire filter lifetime, name is a valid C string,
        // and pw_filter_new takes ownership of the newly allocated properties.
        let filter = NonNull::new(unsafe {
            sys::pw_filter_new(core.as_raw_ptr(), name.as_ptr(), props.into_raw())
        })
        .ok_or_else(|| native("create filter"))?;
        let callbacks = Box::new(Callbacks {
            shared,
            rt: UnsafeCell::new(Realtime {
                ports: Vec::with_capacity(config.inputs.len() + config.outputs.len()),
                inputs: vec![0.0; config.inputs.len() * config.max_quantum],
                outputs: vec![0.0; config.outputs.len() * config.max_quantum],
                config: config.clone(),
                processor,
                previous: None,
            }),
        });
        let mut result = Self {
            filter,
            _core: core,
            callbacks,
            // SAFETY: SPA hook is initialized by add_listener; omitted event pointers are null.
            events: Box::new(unsafe { std::mem::zeroed() }),
            hook: Box::new(unsafe { std::mem::zeroed() }),
            pending: None,
        };
        result.events.version = 1;
        result.events.state_changed = Some(state_changed);
        result.events.process = Some(process);
        // SAFETY: all allocations stay at fixed addresses until disconnect/destroy below.
        unsafe {
            sys::pw_filter_add_listener(
                result.filter.as_ptr(),
                &mut *result.hook,
                &*result.events,
                (&*result.callbacks as *const Callbacks).cast_mut().cast(),
            );
        }
        for (index, port) in config.inputs.iter().chain(&config.outputs).enumerate() {
            let direction = if index < config.inputs.len() {
                spa_sys::SPA_DIRECTION_INPUT
            } else {
                spa_sys::SPA_DIRECTION_OUTPUT
            };
            let properties = pw::properties::properties! {"format.dsp"=>"32 bit float mono audio","port.name"=>port.clone()};
            // SAFETY: filter is unconnected, properties transfer ownership. One byte of
            // user storage gives each port a distinct stable opaque identifier.
            let ptr = NonNull::new(unsafe {
                sys::pw_filter_add_port(
                    result.filter.as_ptr(),
                    direction,
                    sys::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
                    1,
                    properties.into_raw(),
                    std::ptr::null_mut(),
                    0,
                )
            })
            .ok_or_else(|| native("create filter port"))?;
            result.callbacks.rt.get_mut().ports.push(ptr);
        }
        let latency = super::parameters::encode(
            spa_sys::SPA_TYPE_OBJECT_ParamProcessLatency,
            spa_sys::SPA_PARAM_ProcessLatency,
            vec![spa::pod::Property::new(
                spa_sys::SPA_PARAM_PROCESS_LATENCY_rate,
                spa::pod::Value::Int(config.latency_frames as i32),
            )],
        )?;
        let pod = spa::pod::Pod::from_bytes(&latency)
            .ok_or(MediaError::InvalidArgument("filter latency POD"))?;
        let mut params = [pod.as_raw_ptr().cast_const()];
        let flags = sys::pw_filter_flags_PW_FILTER_FLAG_RT_PROCESS
            | if config.start_paused {
                sys::pw_filter_flags_PW_FILTER_FLAG_INACTIVE
            } else {
                0
            };
        // SAFETY: valid owned filter; the native call copies params synchronously.
        check(unsafe {
            sys::pw_filter_connect(result.filter.as_ptr(), flags, params.as_mut_ptr(), 1)
        })?;
        Ok(result)
    }
    pub fn set_active(&mut self, active: bool, completion: Completion) {
        if !completion.begin() {
            return;
        }
        if self.pending.is_some() {
            completion.finish(Err(MediaError::NotReady));
            return;
        }
        // SAFETY: control-loop affinity, owned native object.
        match check(unsafe { sys::pw_filter_set_active(self.filter.as_ptr(), active) }) {
            Ok(()) => {
                self.pending = Some((active, completion, Instant::now() + Duration::from_secs(5)))
            }
            Err(e) => completion.finish(Err(e)),
        }
    }
    pub fn poll(&mut self, snapshot: &RegistrySnapshot) -> bool {
        let shared = &self.callbacks.shared;
        if shared.failed.load(Ordering::Acquire) {
            shared.state(FilterState::Failed(MediaError::Native(
                "processor failed or quantum exceeded configured limit".into(),
            )));
            return false;
        }
        if shared.stop.load(Ordering::Acquire) {
            return false;
        }
        // SAFETY: node ID query runs only on the native control worker.
        let native_id = unsafe { sys::pw_filter_get_node_id(self.filter.as_ptr()) };
        if let Err(error) = shared.node.update(snapshot, native_id) {
            shared.state(FilterState::Failed(error));
            shared.stop.store(true, Ordering::Release);
            return false;
        }
        let state = shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some((active, completion, deadline)) = &self.pending {
            let expected = if *active {
                FilterState::Processing
            } else {
                FilterState::Paused
            };
            if state == expected {
                completion.finish(Ok(()));
                self.pending = None;
            } else if Instant::now() >= *deadline {
                completion.finish(Err(MediaError::Timeout));
                self.pending = None;
            }
        }
        !matches!(state, FilterState::Failed(_) | FilterState::Stopped)
    }
}
fn check(result: i32) -> Result<(), MediaError> {
    if result < 0 {
        Err(native(std::io::Error::from_raw_os_error(-result)))
    } else {
        Ok(())
    }
}
impl Drop for NativeFilter {
    fn drop(&mut self) {
        // SAFETY: disconnect removes the graph node and synchronously quiesces its data-loop
        // processing. Listener removal and destruction happen before callback box deallocation.
        unsafe {
            sys::pw_filter_disconnect(self.filter.as_ptr());
            spa::utils::hook::remove(*self.hook);
            sys::pw_filter_destroy(self.filter.as_ptr());
        }
        let mut state = self
            .callbacks
            .shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !matches!(*state, FilterState::Failed(_)) {
            *state = FilterState::Stopped;
        }
    }
}
unsafe extern "C" fn state_changed(
    data: *mut c_void,
    _old: sys::pw_filter_state,
    state: sys::pw_filter_state,
    error: *const std::ffi::c_char,
) {
    // SAFETY: native registration uses our stable Callback allocation; only shared state
    // is accessed here. Mutable DSP state belongs exclusively to process's data thread.
    let callbacks = unsafe { &*data.cast::<Callbacks>() };
    let state = match state {
        sys::pw_filter_state_PW_FILTER_STATE_PAUSED => FilterState::Paused,
        sys::pw_filter_state_PW_FILTER_STATE_STREAMING => FilterState::Processing,
        sys::pw_filter_state_PW_FILTER_STATE_ERROR => FilterState::Failed(if error.is_null() {
            native("filter error")
        } else {
            native(unsafe { CStr::from_ptr(error) }.to_string_lossy())
        }),
        sys::pw_filter_state_PW_FILTER_STATE_UNCONNECTED => FilterState::Stopped,
        _ => FilterState::Connecting,
    };
    callbacks.shared.state(state);
}
unsafe extern "C" fn process(data: *mut c_void, position: *mut spa_sys::spa_io_position) {
    if data.is_null() || position.is_null() {
        return;
    }
    // SAFETY: PipeWire serializes process calls on the filter's data-loop. Other callbacks
    // never touch this UnsafeCell. Construction happens before connect; drop after disconnect.
    let callbacks = unsafe { &*data.cast::<Callbacks>() };
    let rt = unsafe { &mut *callbacks.rt.get() };
    let clock = unsafe { &(*position).clock };
    let frames = clock.duration as usize;
    if frames > rt.config.max_quantum {
        callbacks.shared.failed.store(true, Ordering::Release);
        return;
    }
    let mut now = FilterClock {
        id: clock.id,
        position: clock.position,
        monotonic_ns: clock.nsec,
        rate_num: clock.rate.num,
        rate_denom: clock.rate.denom,
        frames,
        discontinuity: false,
    };
    now.discontinuity = rt.previous.is_none_or(|old| {
        old.id != now.id
            || old.position.saturating_add(old.frames as u64) != now.position
            || old.rate_num != now.rate_num
            || old.rate_denom != now.rate_denom
    });
    if now.discontinuity {
        callbacks
            .shared
            .discontinuities
            .fetch_add(1, Ordering::Relaxed);
    }
    rt.previous = Some(now);
    let stride = rt.config.max_quantum;
    for (channel, port) in rt.ports[..rt.config.inputs.len()].iter().enumerate() {
        let dest = &mut rt.inputs[channel * stride..channel * stride + frames];
        dest.fill(0.0);
        // SAFETY: DSP ports negotiate mono f32. get_dsp_buffer returns at least frames
        // aligned samples valid only during this process call, or null for an absent buffer.
        let source =
            unsafe { sys::pw_filter_get_dsp_buffer(port.as_ptr(), frames as u32) }.cast::<f32>();
        if !source.is_null() {
            dest.copy_from_slice(unsafe { std::slice::from_raw_parts(source, frames) });
        }
    }
    rt.outputs.fill(0.0);
    if !callbacks.shared.failed.load(Ordering::Acquire)
        && !callbacks.shared.stop.load(Ordering::Acquire)
    {
        let cycle = FilterCycle {
            clock: now,
            inputs: FilterInputs {
                storage: &rt.inputs,
                stride,
                frames,
            },
            outputs: FilterOutputs {
                storage: &mut rt.outputs,
                stride,
                frames,
            },
        };
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.processor.process(cycle)))
            .is_err()
        {
            callbacks.shared.failed.store(true, Ordering::Release);
            rt.outputs.fill(0.0);
        }
    }
    for (channel, port) in rt.ports[rt.config.inputs.len()..].iter().enumerate() {
        // SAFETY: as above. Copying ensures processors never alias native input/output planes.
        let dest =
            unsafe { sys::pw_filter_get_dsp_buffer(port.as_ptr(), frames as u32) }.cast::<f32>();
        if !dest.is_null() {
            unsafe { std::slice::from_raw_parts_mut(dest, frames) }
                .copy_from_slice(&rt.outputs[channel * stride..channel * stride + frames]);
        }
    }
    callbacks
        .shared
        .frames
        .fetch_add(frames as u64, Ordering::Relaxed);
}
