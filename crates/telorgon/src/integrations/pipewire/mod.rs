//! PipeWire delivery. This module receives approved pixels, never compositor/window objects.
mod buffer;
mod generation;

use generation::{Generation, Generations};
use std::{cell::RefCell, rc::Rc};

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use pipewire as pw;
use pw::properties::properties;
use pw::spa::{
    self,
    pod::{Object, Pod, Property, Value},
    utils::{Fraction, Id, Rectangle},
};

use crate::shell::capture::CaptureLayout;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StreamStatus {
    Connecting,
    Ready { node_id: u32 },
    Failed(String),
    Stopped,
}

/// At most one queued frame and one recyclable frame. The PipeWire thread retains one current
/// frame for stationary desktops. Replaced queued frames return to the producer for reuse.
struct Mailbox {
    pending: Option<Vec<u8>>,
    spare: Option<Vec<u8>>,
    stopped: bool,
    status: StreamStatus,
    requested: Option<Generation>,
    generation: u64,
    ready_generation: u64,
}

pub(crate) struct VideoStream {
    layout: CaptureLayout,
    generation: u64,
    mailbox: Arc<Mutex<Mailbox>>,
    stop: pw::channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

/// Publish terminal state even when the Rust worker unwinds. The owner must be woken to
/// revoke the session and poll thread retirement; a dead worker cannot report readiness.
struct WorkerExit {
    mailbox: Arc<Mutex<Mailbox>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Drop for WorkerExit {
    fn drop(&mut self) {
        {
            // A panic while editing the mailbox must not prevent revocation. Recover only
            // to clear storage and publish failure, never to resume pixel delivery.
            let mut state = self
                .mailbox
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.pending = None;
            state.spare = None;
            state.stopped = true;
            if std::thread::panicking() {
                state.status = StreamStatus::Failed("video worker panicked".into());
            } else if !matches!(state.status, StreamStatus::Failed(_)) {
                state.status = StreamStatus::Stopped;
            }
        }
        (self.wake)();
    }
}

impl Mailbox {
    fn publish(
        &mut self,
        pixels: Vec<u8>,
        expected_bytes: usize,
    ) -> Result<Option<Vec<u8>>, Vec<u8>> {
        if self.stopped || pixels.len() != expected_bytes {
            return Err(pixels);
        }
        let previous = self.pending.replace(pixels);
        Ok(previous.or_else(|| self.spare.take()))
    }
}

impl VideoStream {
    /// Returns immediately; readiness is reported through `status` and the host wake callback.
    pub(crate) fn start(
        stream_id: u64,
        layout: CaptureLayout,
        fps: u32,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        validate_layout(layout, fps)?;
        let mailbox = Arc::new(Mutex::new(Mailbox {
            pending: None,
            spare: None,
            stopped: false,
            status: StreamStatus::Connecting,
            requested: None,
            generation: 1,
            ready_generation: 0,
        }));
        let (stop, receiver) = pw::channel::channel();
        let shared = Arc::clone(&mailbox);
        let thread = std::thread::Builder::new()
            .name(format!("telorgon-video-{stream_id}"))
            .spawn(move || {
                let _exit = WorkerExit {
                    mailbox: Arc::clone(&shared),
                    wake: Arc::clone(&wake),
                };
                let result = run(stream_id, layout, fps, &shared, &wake, receiver);
                if let Ok(mut state) = shared.lock() {
                    if let Err(error) = result {
                        state.status = StreamStatus::Failed(error.chars().take(512).collect());
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            layout,
            generation: 1,
            mailbox,
            stop,
            thread: Some(thread),
        })
    }

    /// Coalescing is owned by the host. Only one unacknowledged replacement may be queued.
    pub(crate) fn resize(&mut self, layout: CaptureLayout, fps: u32) -> Result<(), String> {
        validate_layout(layout, fps)?;
        let generation = self
            .generation
            .checked_add(1)
            .ok_or("video generation exhausted")?;
        let mut shared = self.mailbox.lock().map_err(|_| "video mailbox poisoned")?;
        if shared.stopped
            || shared.requested.is_some()
            || shared.ready_generation != self.generation
        {
            return Err("video stream is not ready for resize".into());
        }
        shared.pending = None;
        shared.spare = None;
        shared.generation = generation;
        shared.requested = Some(Generation {
            serial: generation,
            layout,
        });
        shared.status = StreamStatus::Connecting;
        self.layout = layout;
        self.generation = generation;
        Ok(())
    }

    pub(crate) fn status(&self) -> StreamStatus {
        self.mailbox
            .lock()
            .map(|s| s.status.clone())
            .unwrap_or_else(|_| StreamStatus::Failed("video mailbox poisoned".into()))
    }

    /// Returns storage available for reuse; errors return ownership of the submitted pixels.
    pub(crate) fn publish(&self, pixels: Vec<u8>) -> Result<Option<Vec<u8>>, Vec<u8>> {
        if pixels.len() != self.layout.byte_len() {
            return Err(pixels);
        }
        let Ok(mut state) = self.mailbox.lock() else {
            return Err(pixels);
        };
        if state.stopped || state.ready_generation != self.generation {
            return Err(pixels);
        }
        state.publish(pixels, self.layout.byte_len())
    }

    pub(crate) fn take_spare(&self) -> Option<Vec<u8>> {
        self.mailbox.lock().ok()?.spare.take()
    }

    pub(crate) fn stop(&self) {
        if let Ok(mut state) = self.mailbox.lock() {
            if state.stopped {
                return;
            }
            state.stopped = true;
            state.pending = None;
            state.spare = None;
        }
        let _ = self.stop.send(());
    }

    /// Nonblocking owner-loop retirement; join only after the thread has ended.
    pub(crate) fn retired(&mut self) -> bool {
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            return false;
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        true
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        self.stop();
    }
}

fn validate_layout(layout: CaptureLayout, fps: u32) -> Result<(), String> {
    if fps == 0
        || fps > 60
        || layout.width() > 8192
        || layout.height() > 8192
        || layout.stride()
            != layout
                .width()
                .checked_mul(4)
                .ok_or("video stride overflow")?
        || layout.byte_len() > i32::MAX as usize
    {
        return Err("unsupported video stream layout or frame rate".into());
    }
    Ok(())
}

fn encode(type_: u32, id: u32, properties: Vec<Property>) -> Result<Vec<u8>, String> {
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(Object {
            type_,
            id,
            properties,
        }),
    )
    .map(|value| value.0.into_inner())
    .map_err(|error| format!("video POD: {error:?}"))
}

fn format_pod(layout: CaptureLayout, fps: u32) -> Result<Vec<u8>, String> {
    use spa::sys::*;
    encode(
        SPA_TYPE_OBJECT_Format,
        SPA_PARAM_EnumFormat,
        vec![
            Property::new(SPA_FORMAT_mediaType, Value::Id(Id(SPA_MEDIA_TYPE_video))),
            Property::new(
                SPA_FORMAT_mediaSubtype,
                Value::Id(Id(SPA_MEDIA_SUBTYPE_raw)),
            ),
            Property::new(
                SPA_FORMAT_VIDEO_format,
                Value::Id(Id(SPA_VIDEO_FORMAT_RGBA)),
            ),
            Property::new(
                SPA_FORMAT_VIDEO_colorRange,
                Value::Id(Id(SPA_VIDEO_COLOR_RANGE_0_255)),
            ),
            Property::new(
                SPA_FORMAT_VIDEO_colorMatrix,
                Value::Id(Id(SPA_VIDEO_COLOR_MATRIX_RGB)),
            ),
            Property::new(
                SPA_FORMAT_VIDEO_transferFunction,
                Value::Id(Id(SPA_VIDEO_TRANSFER_SRGB)),
            ),
            Property::new(
                SPA_FORMAT_VIDEO_colorPrimaries,
                Value::Id(Id(SPA_VIDEO_COLOR_PRIMARIES_BT709)),
            ),
            Property::new(
                SPA_FORMAT_VIDEO_size,
                Value::Rectangle(Rectangle {
                    width: layout.width(),
                    height: layout.height(),
                }),
            ),
            Property::new(
                SPA_FORMAT_VIDEO_framerate,
                Value::Fraction(Fraction { num: fps, denom: 1 }),
            ),
        ],
    )
}

fn buffers_pod(layout: CaptureLayout) -> Result<Vec<u8>, String> {
    use spa::sys::*;
    encode(
        SPA_TYPE_OBJECT_ParamBuffers,
        SPA_PARAM_Buffers,
        vec![
            Property::new(SPA_PARAM_BUFFERS_buffers, Value::Int(3)),
            Property::new(SPA_PARAM_BUFFERS_blocks, Value::Int(1)),
            Property::new(SPA_PARAM_BUFFERS_size, Value::Int(layout.byte_len() as i32)),
            Property::new(SPA_PARAM_BUFFERS_stride, Value::Int(layout.stride() as i32)),
            Property::new(SPA_PARAM_BUFFERS_dataType, Value::Int(1 << SPA_DATA_MemFd)),
        ],
    )
}

fn matching_format(pod: &Pod, layout: CaptureLayout, fps: u32) -> bool {
    let mut info = spa::param::video::VideoInfoRaw::new();
    if info.parse(pod).is_err() {
        return false;
    }
    let rate = info.framerate();
    info.format() == spa::param::video::VideoFormat::RGBA
        && info.size().width == layout.width()
        && info.size().height == layout.height()
        && rate.num > 0
        && rate.denom > 0
        && u64::from(rate.num) <= u64::from(fps) * u64::from(rate.denom)
}

fn header_pod() -> Result<Vec<u8>, String> {
    use spa::sys::*;
    encode(
        SPA_TYPE_OBJECT_ParamMeta,
        SPA_PARAM_Meta,
        vec![
            Property::new(SPA_PARAM_META_type, Value::Id(Id(SPA_META_Header))),
            Property::new(
                SPA_PARAM_META_size,
                Value::Int(std::mem::size_of::<spa_meta_header>() as i32),
            ),
        ],
    )
}

struct WorkerFrames {
    buffers: Generations,
    current: Option<Vec<u8>>,
}

fn run(
    stream_id: u64,
    layout: CaptureLayout,
    fps: u32,
    mailbox: &Arc<Mutex<Mailbox>>,
    wake: &Arc<dyn Fn() + Send + Sync>,
    stop: pw::channel::Receiver<()>,
) -> Result<(), String> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(|e| e.to_string())?;
    let core = context.connect_rc(None).map_err(|e| e.to_string())?;
    let stream = pw::stream::StreamRc::new(
        core,
        "Telorgon screen sharing",
        properties! {
            "media.type" => "Video", "media.category" => "Capture", "media.role" => "Screen",
            "media.class" => "Video/Source", "node.virtual" => "true",
            "node.name" => format!("telorgon.capture.{stream_id}"),
        },
    )
    .map_err(|e| e.to_string())?;
    let quit_loop = mainloop.clone();
    let _stop = stop.attach(mainloop.loop_(), move |_| quit_loop.quit());
    let state_mailbox = Arc::clone(mailbox);
    let state_wake = Arc::clone(wake);
    let error_loop = mainloop.clone();
    let process_mailbox = Arc::clone(mailbox);
    let frames = Rc::new(RefCell::new(WorkerFrames {
        buffers: Generations::new(layout),
        current: None,
    }));
    let param_frames = Rc::clone(&frames);
    let process_frames = Rc::clone(&frames);
    let add_frames = Rc::clone(&frames);
    let remove_frames = Rc::clone(&frames);
    let add_loop = mainloop.clone();
    let remove_loop = mainloop.clone();
    let mut sequence = 0u64;
    let param_loop = mainloop.clone();
    let _listener = stream
        .add_local_listener_with_user_data(())
        .state_changed(move |stream, _, _, state| {
            if let Ok(mut shared) = state_mailbox.lock() {
                if shared.stopped {
                    return;
                }
                match state {
                    pw::stream::StreamState::Paused | pw::stream::StreamState::Streaming => {
                        let node_id = stream.node_id();
                        if node_id != u32::MAX && shared.generation == 1 {
                            shared.ready_generation = 1;
                            shared.status = StreamStatus::Ready { node_id };
                        }
                    }
                    pw::stream::StreamState::Error(error) => {
                        shared.status = StreamStatus::Failed(error.chars().take(512).collect());
                        shared.stopped = true;
                        error_loop.quit();
                    }
                    pw::stream::StreamState::Unconnected => {
                        error_loop.quit();
                    }
                    _ => {}
                }
            }
            state_wake();
        })
        .param_changed(move |stream, _, id, param| {
            if id != spa::sys::SPA_PARAM_Format {
                return;
            }
            let target = param_frames.borrow().buffers.target;
            let Some(param) = param else {
                let mut frames = param_frames.borrow_mut();
                frames.buffers.format(false);
                frames.current = None;
                return;
            };
            if !matching_format(param, target.layout, fps) {
                // A previous format event may still be queued while EnumFormat is changing.
                // Never interpret its buffers using the replacement layout.
                param_frames.borrow_mut().buffers.format(false);
                if target.serial == 1 {
                    param_loop.quit();
                }
                return;
            }
            param_frames.borrow_mut().buffers.format(true);
            let result = (|| {
                let buffers = buffers_pod(target.layout)?;
                let header = header_pod()?;
                stream
                    .update_params(&mut [
                        Pod::from_bytes(&buffers).ok_or("invalid buffer POD")?,
                        Pod::from_bytes(&header).ok_or("invalid header POD")?,
                    ])
                    .map_err(|error| error.to_string())
            })();
            if result.is_err() {
                param_loop.quit();
            }
        })
        .add_buffer(move |_, _, buffer| {
            if !add_frames.borrow_mut().buffers.add(buffer as usize) {
                add_loop.quit();
            }
        })
        .remove_buffer(move |_, _, buffer| {
            if !remove_frames.borrow_mut().buffers.remove(buffer as usize) {
                remove_loop.quit();
            }
        })
        .process(move |stream, _| {
            let mut frames = process_frames.borrow_mut();
            if let Ok(mut shared) = process_mailbox.lock() {
                if shared.stopped
                    || shared.generation != frames.buffers.target.serial
                    || shared.ready_generation != shared.generation
                {
                    frames.current = None;
                    return;
                }
                if let Some(next) = shared.pending.take() {
                    shared.spare = frames.current.replace(next);
                }
            } else {
                return;
            }
            let target = frames.buffers.target;
            let current = frames.current.take();
            drop(frames);
            buffer::deliver(stream, target.layout, current.as_deref(), sequence, |key| {
                process_frames.borrow().buffers.accepts(key)
            });
            // Queueing is a native call; keep callback state unborrowed across that boundary.
            let mut frames = process_frames.borrow_mut();
            if frames.buffers.target == target {
                frames.current = current;
            }
            sequence = sequence.wrapping_add(1);
        })
        .register()
        .map_err(|e| e.to_string())?;
    let format = format_pod(layout, fps)?;
    stream
        .connect(
            spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::DRIVER | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut [Pod::from_bytes(&format).ok_or("invalid format POD")?],
        )
        .map_err(|e| e.to_string())?;
    let timer_stream = stream.clone();
    let timer_loop = mainloop.clone();
    let timer_mailbox = Arc::clone(mailbox);
    let timer_wake = Arc::clone(wake);
    let timer = mainloop.loop_().add_timer(move |_| {
        let request = match timer_mailbox.lock() {
            Ok(mut shared) if !shared.stopped => shared.requested.take(),
            _ => {
                timer_loop.quit();
                return;
            }
        };
        if let Some(target) = request {
            {
                let mut frames = frames.borrow_mut();
                if !frames.buffers.resize(target) {
                    timer_loop.quit();
                    return;
                }
                frames.current = None;
            }
            // No RefCell/mutex guard crosses update_params: it may invoke callbacks.
            let result = (|| {
                let format = format_pod(target.layout, fps)?;
                timer_stream
                    .update_params(&mut [Pod::from_bytes(&format).ok_or("invalid format POD")?])
                    .map_err(|error| error.to_string())
            })();
            if result.is_err() {
                timer_loop.quit();
                return;
            }
        }
        let ready = {
            let frames = frames.borrow();
            frames
                .buffers
                .ready()
                .then_some(frames.buffers.target.serial)
        };
        let mut changed = false;
        if let Some(generation) = ready {
            let node_id = timer_stream.node_id();
            if node_id != u32::MAX
                && let Ok(mut shared) = timer_mailbox.lock()
            {
                if !shared.stopped
                    && shared.generation == generation
                    && shared.ready_generation != generation
                {
                    shared.ready_generation = generation;
                    shared.status = StreamStatus::Ready { node_id };
                    changed = true;
                }
            }
        }
        if changed {
            timer_wake();
        }
        let _ = timer_stream.trigger_process();
    });
    timer
        .update_timer(
            Some(Duration::from_nanos(1)),
            Some(Duration::from_nanos(
                1_000_000_000_u64.div_ceil(u64::from(fps)),
            )),
        )
        .into_result()
        .map_err(|e| e.to_string())?;
    mainloop.run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;

    #[test]
    fn resize_clears_old_pixels_and_rejects_delivery_until_generation_acknowledgement() {
        let layout =
            CaptureLayout::rgba8(NonZeroU32::new(4).unwrap(), NonZeroU32::new(2).unwrap(), 16)
                .unwrap();
        let replacement =
            CaptureLayout::rgba8(NonZeroU32::new(2).unwrap(), NonZeroU32::new(4).unwrap(), 8)
                .unwrap();
        let mailbox = Arc::new(Mutex::new(Mailbox {
            pending: Some(vec![1; 32]),
            spare: Some(vec![2; 32]),
            stopped: false,
            status: StreamStatus::Ready { node_id: 42 },
            requested: None,
            generation: 1,
            ready_generation: 1,
        }));
        // A local channel only: no worker, daemon, stream or main loop is started by this test.
        let (stop, _receiver) = pw::channel::channel();
        let mut video = VideoStream {
            layout,
            generation: 1,
            mailbox: mailbox.clone(),
            stop,
            thread: None,
        };
        video.resize(replacement, 30).unwrap();
        assert_eq!(video.status(), StreamStatus::Connecting);
        assert_eq!(video.publish(vec![3; 32]), Err(vec![3; 32]));
        assert!(video.resize(layout, 30).is_err());
        {
            let mut shared = mailbox.lock().unwrap();
            assert!(shared.pending.is_none());
            assert!(shared.spare.is_none());
            assert_eq!(
                shared.requested.take(),
                Some(Generation {
                    serial: 2,
                    layout: replacement
                })
            );
        }
        // Consuming the command is not an acknowledgement of old-buffer retirement.
        assert!(video.resize(layout, 30).is_err());
        assert_eq!(video.publish(vec![4; 32]), Err(vec![4; 32]));
        {
            let mut shared = mailbox.lock().unwrap();
            shared.ready_generation = 2;
            shared.status = StreamStatus::Ready { node_id: 42 };
        }
        assert_eq!(video.publish(vec![5; 32]), Ok(None));
        assert_eq!(video.status(), StreamStatus::Ready { node_id: 42 });
        video.stop();
        assert!(video.resize(layout, 30).is_err());
    }

    #[test]
    fn unwinding_worker_revokes_even_a_poisoned_mailbox_and_wakes_owner() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mailbox = Arc::new(Mutex::new(Mailbox {
            pending: Some(vec![1; 4]),
            spare: Some(vec![2; 4]),
            stopped: false,
            status: StreamStatus::Ready { node_id: 42 },
            requested: None,
            generation: 1,
            ready_generation: 1,
        }));
        let wakes = Arc::new(AtomicUsize::new(0));
        let notified = Arc::clone(&wakes);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _exit = WorkerExit {
                mailbox: Arc::clone(&mailbox),
                wake: Arc::new(move || {
                    notified.fetch_add(1, Ordering::SeqCst);
                }),
            };
            let _locked = mailbox.lock().unwrap();
            panic!("injected delivery worker failure");
        }));
        assert!(result.is_err());
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        let mut state = mailbox.lock().unwrap_or_else(|error| error.into_inner());
        assert!(state.stopped);
        assert!(matches!(state.status, StreamStatus::Failed(_)));
        assert!(state.pending.is_none());
        assert!(state.spare.is_none());
        assert_eq!(state.publish(vec![3; 4], 4), Err(vec![3; 4]));
    }

    #[test]
    fn normal_worker_exit_preserves_failure_and_releases_mailbox_before_wake() {
        let mailbox = Arc::new(Mutex::new(Mailbox {
            pending: Some(vec![1; 4]),
            spare: None,
            stopped: false,
            status: StreamStatus::Failed("negotiation failed".into()),
            requested: None,
            generation: 1,
            ready_generation: 1,
        }));
        let observed = Arc::clone(&mailbox);
        drop(WorkerExit {
            mailbox: Arc::clone(&mailbox),
            wake: Arc::new(move || {
                let state = observed
                    .try_lock()
                    .expect("wake must run outside mailbox lock");
                assert!(state.stopped);
                assert!(state.pending.is_none());
            }),
        });
        assert_eq!(
            mailbox.lock().unwrap().status,
            StreamStatus::Failed("negotiation failed".into())
        );
    }

    #[test]
    fn negotiated_video_pods_have_valid_wire_encoding() {
        let layout = CaptureLayout::rgba8(
            NonZeroU32::new(1920).unwrap(),
            NonZeroU32::new(1080).unwrap(),
            7680,
        )
        .unwrap();
        let bytes = format_pod(layout, 30).unwrap();
        let pod = Pod::from_bytes(&bytes).unwrap();
        assert!(matching_format(pod, layout, 30));
        assert!(!matching_format(pod, layout, 15));
        let other = CaptureLayout::rgba8(
            NonZeroU32::new(1280).unwrap(),
            NonZeroU32::new(720).unwrap(),
            5120,
        )
        .unwrap();
        assert!(!matching_format(pod, other, 30));
        assert!(Pod::from_bytes(&buffers_pod(layout).unwrap()).is_some());
    }

    #[test]
    fn slow_consumer_keeps_only_latest_frame_and_returns_replaced_storage() {
        let mut mailbox = Mailbox {
            pending: None,
            spare: None,
            stopped: false,
            status: StreamStatus::Connecting,
            requested: None,
            generation: 1,
            ready_generation: 0,
        };
        assert_eq!(mailbox.publish(vec![1; 4], 4), Ok(None));
        assert_eq!(mailbox.publish(vec![2; 4], 4), Ok(Some(vec![1; 4])));
        assert_eq!(mailbox.pending.as_deref(), Some([2; 4].as_slice()));
        assert_eq!(mailbox.publish(vec![3; 3], 4), Err(vec![3; 3]));
        assert_eq!(mailbox.pending.as_deref(), Some([2; 4].as_slice()));
        mailbox.stopped = true;
        assert_eq!(mailbox.publish(vec![4; 4], 4), Err(vec![4; 4]));
    }
}
