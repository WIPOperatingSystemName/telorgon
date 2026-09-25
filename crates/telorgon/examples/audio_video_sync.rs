//! cargo run -p telorgon --no-default-features --features audio-linux,video-linux,application-software --example audio_video_sync
//! Start plays a quiet synthetic beep each second and publishes matching synthetic video.
//! Captures only this example's own video node, never a camera, microphone or desktop.
//! Visual scheduling follows the audio clock/device-delay estimate; physical display and
//! external speaker latency are not calibrated. Closing releases every owned resource.
#[cfg(target_os = "linux")]
mod linux {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };
    use telorgon::{
        app::*,
        graphics::bridges::video_cpu::VideoImageOptions,
        host::application::video::{VideoPreview, VideoPreviewSnapshot},
        integrations::pipewire::*,
        media::{audio::*, video::*},
        ui::ImageId,
    };
    #[derive(Clone, Debug, PartialEq)]
    struct AudioStatus {
        state: AudioState,
        pending: bool,
        error: Option<String>,
    }
    #[derive(Clone)]
    struct Controls {
        sender: mpsc::SyncSender<bool>,
        status: Signal<AudioStatus>,
    }
    impl PartialEq for Controls {
        fn eq(&self, other: &Self) -> bool {
            self.status == other.status
        }
    }
    #[component(no_default)]
    struct Preview {
        #[input]
        controls: Controls,
        #[input]
        frames: Signal<VideoPreviewSnapshot>,
        #[state]
        error: Option<String>,
    }
    impl Component for Preview {
        fn view(&self) -> impl View {
            let audio = self.watch(&self.controls.status);
            let video = self.watch(&self.frames);
            let mut content = column()
                .gap(12.0)
                .padding(16.0)
                .child(text("Synthetic audio/video synchronization"))
                .child(text(
                    "A quiet beep and white flash share the audio sample timeline.",
                ))
                .child(
                    row()
                        .height(36.0)
                        .gap(10.0)
                        .child(
                            button("Start / resume")
                                .enabled(!audio.pending && audio.state != AudioState::Streaming)
                                .on_press(|this: &mut Self| {
                                    this.error = this
                                        .controls
                                        .sender
                                        .try_send(true)
                                        .err()
                                        .map(|e| e.to_string());
                                }),
                        )
                        .child(
                            button("Pause")
                                .enabled(!audio.pending && audio.state == AudioState::Streaming)
                                .on_press(|this: &mut Self| {
                                    this.error = this
                                        .controls
                                        .sender
                                        .try_send(false)
                                        .err()
                                        .map(|e| e.to_string());
                                }),
                        ),
                )
                .child(text(format!(
                    "Audio {:?}{} · video {:?} · {} skipped",
                    audio.state,
                    if audio.pending { " (pending)" } else { "" },
                    video.state,
                    video.skipped
                )));
            if let Some(image) = &video.image {
                content = content.child(
                    Image::resource(image.clone())
                        .width(640.0)
                        .height(360.0)
                        .accessible_label("White during each beep; blue between beeps"),
                );
            } else {
                content = content.child(text(if video.waiting_for_audio {
                    "Waiting for the audio clock"
                } else {
                    "Waiting for synthetic video"
                }));
            }
            content.child(text(
                self.error
                    .clone()
                    .or_else(|| audio.error.clone())
                    .or_else(|| video.conversion_error.as_ref().map(ToString::to_string))
                    .unwrap_or_default(),
            ))
        }
    }
    pub fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
        let handle = connection.handle();
        let deadline = Instant::now() + Duration::from_secs(5);
        while handle.state() != ConnectionState::Ready {
            if Instant::now() >= deadline {
                return Err("PipeWire connection timed out".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let audio = AudioStream::realtime(
            handle.clone(),
            AudioConfig {
                name: "Telorgon A/V sync tone".into(),
                start_paused: true,
                ..Default::default()
            },
            |cycle: AudioCycle<'_>| {
                for (offset, frame) in cycle
                    .samples
                    .chunks_exact_mut(cycle.channels as usize)
                    .enumerate()
                {
                    let phase =
                        (cycle.timing.frame_position + offset as u64) % u64::from(cycle.rate);
                    let time = phase as f32 / cycle.rate as f32;
                    // Ten-millisecond fades avoid clicks at the 100-ms pulse boundaries.
                    let envelope = (time / 0.01).min((0.1 - time) / 0.01).clamp(0.0, 1.0);
                    let sample = (time * 440.0 * std::f32::consts::TAU).sin() * 0.08 * envelope;
                    frame.fill(sample);
                }
            },
        )?;
        let clock = audio.clock();
        let format = VideoFormat::rgba(320, 180, 30);
        let mut config = VideoConfig::producer(format);
        config.name = "Telorgon A/V sync synthetic video".into();
        config.virtual_camera = true;
        config.buffer_frames = 3;
        let producer = VideoStream::open(handle.clone(), config)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let node = loop {
            if let Some(node) = producer.node() {
                break node;
            }
            if let VideoState::Failed(error) = producer.state() {
                return Err(error.into());
            }
            if Instant::now() >= deadline {
                return Err("synthetic video node timed out".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut capture = VideoConfig::capture(node, vec![format]);
        capture.buffer_frames = 3;
        let mut preview = VideoPreview::start_audio_synchronized(
            VideoStream::open(handle, capture)?,
            ImageId(0x70000002),
            VideoImageOptions::default(),
            clock.clone(),
            VideoPlayoutConfig::default(),
        )?;
        let (sender, receiver) = mpsc::sync_channel(8);
        let (status, publish) = Signal::new(AudioStatus {
            state: audio.state(),
            pending: false,
            error: None,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = std::thread::Builder::new()
            .name("av-sync-source".into())
            .spawn(move || {
                let mut pending: Option<(Request, Instant)> = None;
                let mut error = None;
                let mut last_clock = None;
                let mut last_discontinuities = None;
                let mut last_video = Instant::now();
                while !worker_stop.load(Ordering::Acquire) {
                    if let Some((request, deadline)) = &pending {
                        match request.state() {
                            RequestState::Complete(result) => {
                                error = result.err().map(|e| e.to_string());
                                pending = None;
                            }
                            _ if Instant::now() >= *deadline => {
                                request.cancel();
                                audio.stop();
                                pending = None;
                                error = Some("Audio transition timed out; stream stopped".into());
                            }
                            _ => {}
                        }
                    }
                    match receiver.recv_timeout(Duration::from_millis(5)) {
                        Ok(active) if pending.is_none() => {
                            match if active {
                                audio.resume()
                            } else {
                                audio.pause()
                            } {
                                Ok(request) => {
                                    pending =
                                        Some((request, Instant::now() + Duration::from_secs(5)));
                                    error = None;
                                }
                                Err(e) => error = Some(e.to_string()),
                            }
                        }
                        Ok(_) => error = Some("An audio transition is already pending".into()),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    if producer.state() == VideoState::Streaming
                        && last_video.elapsed() >= Duration::from_millis(33)
                    {
                        if let Some(sample) = clock.snapshot() {
                            let key = (
                                sample.timing.format_generation,
                                sample.timing.frame_position,
                            );
                            if last_clock != Some(key) {
                                let on = sample.timing.frame_position
                                    % u64::from(sample.sample_rate)
                                    < u64::from(sample.sample_rate / 10);
                                let color = if on {
                                    [245, 245, 245, 255]
                                } else {
                                    [24, 42, 86, 255]
                                };
                                let mut pixels = vec![0; 320 * 180 * 4];
                                for pixel in pixels.chunks_exact_mut(4) {
                                    pixel.copy_from_slice(&color);
                                }
                                let metadata = FrameMetadata {
                                    timestamp_ns: Some(sample.timing.monotonic_ns),
                                    discontinuity: last_discontinuities
                                        != Some(sample.discontinuities),
                                    ..Default::default()
                                };
                                let result = CpuVideoFrame::packed(format, pixels, metadata)
                                    .and_then(|frame| {
                                        producer.submit(frame).map_err(|(error, _)| error)
                                    });
                                match result {
                                    Ok(()) => {
                                        last_clock = Some(key);
                                        last_discontinuities = Some(sample.discontinuities);
                                    }
                                    Err(MediaError::QueueFull | MediaError::NotReady) => {}
                                    Err(e) => error = Some(e.to_string()),
                                }
                                last_video = Instant::now();
                            }
                        } else {
                            last_clock = None;
                            last_discontinuities = None;
                        }
                    }
                    if let VideoState::Failed(e) = producer.state() {
                        error = Some(e.to_string());
                    }
                    publish.publish_if_changed(AudioStatus {
                        state: audio.state(),
                        pending: pending.is_some(),
                        error: error.clone(),
                    });
                }
                if let Some((request, _)) = pending {
                    request.cancel();
                }
                audio.stop();
                producer.stop();
            })?;
        let result = Application::gui("org.telorgon.examples.audio-video-sync", "Audio/video sync")
            .renderer(Renderer::Software)
            .window(
                Window::new("Audio/video sync")
                    .size(720, 600)
                    .content(Preview {
                        controls: Controls { sender, status },
                        frames: preview.signal(),
                        error: None,
                    }),
            )
            .run();
        stop.store(true, Ordering::Release);
        preview.shutdown();
        let joined = worker.join();
        connection.shutdown()?;
        joined.map_err(|_| "audio/video worker panicked")?;
        result?;
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    linux::run()
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This example requires Linux.");
}
