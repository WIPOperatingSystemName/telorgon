//! Explicit video preview assembly. Construction never acquires permissions or opens devices.
#[path = "video_timing.rs"]
mod timing;
use timing::PreviewTiming;

use crate::{
    authoring::compose::{Signal, SignalWriter},
    graphics::{
        bridges::video_cpu::{VideoImageOptions, video_image},
        render::ImageResource,
    },
    integrations::pipewire::MediaError,
    media::video::*,
    ui::ImageId,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
#[derive(Clone, Debug)]
pub struct VideoPreviewSnapshot {
    pub image: Option<ImageResource>,
    pub state: VideoState,
    pub conversion_error: Option<MediaError>,
    /// Frames skipped by the preview to present the newest queued image.
    pub skipped: u64,
    /// Audio-master mode is waiting for a fresh, running audio clock.
    pub waiting_for_audio: bool,
}
/// Owns a capture stream and one conversion worker. Widgets watch `signal()` and bind the
/// snapshot image with `Image::resource` during view evaluation; existing host signal wakes
/// drive redraws. No UI APIs run on the PipeWire thread. Pixel conversion runs off that thread.
/// The signal retains one current image; older snapshots remain valid while callers own them.
/// Drop requests stop without joining. `shutdown` joins explicitly on a non-realtime thread.
/// Keep the portal session/connection owner alive until this preview has stopped.
pub struct VideoPreview {
    stream: Arc<VideoStream>,
    signal: Signal<VideoPreviewSnapshot>,
    stop: Arc<AtomicBool>,
    wake: SyncSender<()>,
    worker: Option<JoinHandle<()>>,
}
impl VideoPreview {
    pub fn start(
        stream: VideoStream,
        image: ImageId,
        options: VideoImageOptions,
    ) -> Result<Self, MediaError> {
        Self::start_inner(stream, image, options, None)
    }
    /// Present source-timestamped frames against CLOCK_MONOTONIC. Unlike the low-latency
    /// default preview, early frames wait and overly late frames are discarded.
    pub fn start_synchronized(
        stream: VideoStream,
        image: ImageId,
        options: VideoImageOptions,
        timing: VideoPlayoutConfig,
    ) -> Result<Self, MediaError> {
        timing.validate()?;
        Self::start_inner(
            stream,
            image,
            options,
            Some(PreviewTiming::monotonic(timing)),
        )
    }
    /// Align common-monotonic video timestamps with the audio stream's reported device
    /// delay. `timing.delay_ns` adds application latency not already in PipeWire's delay.
    /// Both streams must represent the same source timeline; this does not infer lip sync
    /// between unrelated devices or account for unknown external display latency.
    #[cfg(feature = "audio-linux")]
    pub fn start_audio_synchronized(
        stream: VideoStream,
        image: ImageId,
        options: VideoImageOptions,
        audio: crate::media::audio::AudioClock,
        timing: VideoPlayoutConfig,
    ) -> Result<Self, MediaError> {
        timing.validate()?;
        Self::start_inner(
            stream,
            image,
            options,
            Some(PreviewTiming::audio(timing, audio)),
        )
    }
    fn start_inner(
        stream: VideoStream,
        image: ImageId,
        options: VideoImageOptions,
        timing: Option<PreviewTiming>,
    ) -> Result<Self, MediaError> {
        if stream.direction() != VideoDirection::Capture {
            return Err(MediaError::InvalidArgument(
                "preview requires a capture stream",
            ));
        }
        if stream.gpu_enabled() {
            return Err(MediaError::Unsupported(
                "CPU preview requires a CPU capture stream; use the Vulkan video import bridge for GPU frames",
            ));
        }
        let stream = Arc::new(stream);
        let stop = Arc::new(AtomicBool::new(false));
        let (signal, publish) = Signal::new(VideoPreviewSnapshot {
            image: None,
            state: stream.state(),
            conversion_error: None,
            skipped: 0,
            waiting_for_audio: false,
        });
        let (wake, receiver) = sync_channel(1);
        let sender = wake.clone();
        let subscription = stream.subscribe(move || {
            let _ = sender.try_send(());
        })?;
        let work_stream = stream.clone();
        let work_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("telorgon-video-preview".into())
            .spawn(move || {
                run(
                    work_stream,
                    work_stop,
                    receiver,
                    subscription,
                    publish,
                    image,
                    options,
                    timing,
                );
            })
            .map_err(|e| MediaError::Native(e.to_string()))?;
        Ok(Self {
            stream,
            signal,
            stop,
            wake,
            worker: Some(worker),
        })
    }
    /// Borrow the canonical stream controls. Receiving directly competes with preview's
    /// consumer, so applications normally use this for state, diagnostics and requests only.
    pub fn stream(&self) -> &VideoStream {
        &self.stream
    }
    pub fn signal(&self) -> Signal<VideoPreviewSnapshot> {
        self.signal.clone()
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.stream.stop();
        let _ = self.wake.try_send(());
    }
    pub fn shutdown(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for VideoPreview {
    fn drop(&mut self) {
        self.stop();
    }
}
fn run(
    stream: Arc<VideoStream>,
    stop: Arc<AtomicBool>,
    receiver: std::sync::mpsc::Receiver<()>,
    subscription: VideoSubscription,
    publish: SignalWriter<VideoPreviewSnapshot>,
    image: ImageId,
    options: VideoImageOptions,
    timing: Option<PreviewTiming>,
) {
    let mut timing = timing;
    let mut playout = timing.as_ref().map(|config| {
        VideoPlayout::new(stream.clone(), config.video).expect("validated capture playout")
    });
    let mut wait = Duration::from_millis(250);
    let mut snapshot = VideoPreviewSnapshot {
        image: None,
        state: stream.state(),
        conversion_error: None,
        skipped: 0,
        waiting_for_audio: false,
    };
    let mut revision = 0u64;
    while !stop.load(Ordering::Acquire) {
        let _ = receiver.recv_timeout(wait);
        if stop.load(Ordering::Acquire) {
            break;
        }
        let update = subscription.take_update();
        let state = stream.state();
        let mut changed = state != snapshot.state;
        snapshot.state = state;
        if matches!(snapshot.state, VideoState::Stopped | VideoState::Failed(_)) {
            snapshot.image = None;
            publish.publish(snapshot.clone());
            return;
        }
        let mut latest = None;
        wait = Duration::from_millis(250);
        if update.as_ref().is_some_and(|update| update.format_changed) {
            snapshot.image = None;
        }
        if let Some(playout) = &mut playout {
            let dropped = playout.dropped_frames();
            let was_waiting = snapshot.waiting_for_audio;
            let result = monotonic_now().and_then(|now| {
                let timing = timing.as_mut().expect("configured playout timing");
                if timing.follows_audio() {
                    wait = wait.min(Duration::from_millis(10));
                }
                snapshot.waiting_for_audio = !timing.update(playout, now)?;
                if snapshot.waiting_for_audio {
                    snapshot.image = None;
                    Ok(VideoPlayoutPoll::Empty)
                } else {
                    playout.poll(now)
                }
            });
            changed |= was_waiting != snapshot.waiting_for_audio;
            match result {
                Ok(VideoPlayoutPoll::Ready {
                    frame: VideoFrame::Cpu(frame),
                    ..
                }) => latest = Some(frame),
                Ok(VideoPlayoutPoll::Pending { wait_ns }) => {
                    wait = Duration::from_nanos(wait_ns).min(wait)
                }
                Ok(_) => {}
                Err(error) => {
                    changed |= snapshot.conversion_error.as_ref() != Some(&error)
                        || snapshot.image.is_some();
                    snapshot.image = None;
                    snapshot.conversion_error = Some(error);
                    playout.reset();
                }
            }
            snapshot.skipped = snapshot
                .skipped
                .saturating_add(playout.dropped_frames().saturating_sub(dropped));
        } else {
            // A bounded drain also yields when the producer is continuously active.
            for _ in 0..16 {
                match stream.receive() {
                    Ok(Some(frame)) => {
                        if latest.replace(frame).is_some() {
                            snapshot.skipped = snapshot.skipped.saturating_add(1);
                        }
                    }
                    _ => break,
                }
            }
        }
        if let Some(frame) = latest {
            if playout.is_some() {
                wait = Duration::ZERO;
            }
            let Some(next) = revision.checked_add(1) else {
                break;
            };
            revision = next;
            match video_image(&frame, image, revision, options) {
                Ok(resource) => {
                    snapshot.image = Some(resource);
                    snapshot.conversion_error = None;
                }
                Err(error) => {
                    snapshot.image = None;
                    snapshot.conversion_error = Some(error);
                }
            }
            publish.publish(snapshot.clone());
        } else if changed || update.is_some_and(|u| u.format_changed) {
            publish.publish(snapshot.clone());
        }
    }
    snapshot.image = None;
    snapshot.state = VideoState::Stopped;
    publish.publish(snapshot);
}

fn monotonic_now() -> Result<i64, MediaError> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes one live timespec; no pointer escapes this call.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return Err(MediaError::Native(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    i64::try_from(i128::from(time.tv_sec) * 1_000_000_000 + i128::from(time.tv_nsec))
        .map_err(|_| MediaError::InvalidArgument("monotonic clock overflow"))
}
