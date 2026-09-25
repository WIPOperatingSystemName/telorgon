//! cargo run -p telorgon --no-default-features --features audio-linux,application-software --example audio_gui -- sound.wav
//! Decodes an explicit PCM/float WAVE file before opening the window. Playback starts
//! only on Play; controls affect this stream alone. No desktop-control service is created.
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
        app::*, components::application::AudioLevelMeter, integrations::pipewire::*,
        media::audio::*,
    };

    #[derive(Clone, Copy)]
    enum Command {
        Play,
        Pause,
        Resume,
        Stop,
        Quieter,
        Louder,
        Mute,
    }
    #[derive(Clone, Debug, PartialEq)]
    struct Snapshot {
        state: Option<AudioState>,
        pending: bool,
        exhausted: bool,
        format_generation: u64,
        format_ready: bool,
        gain: f32,
        mute: bool,
        latency_ms: f64,
        underruns: u64,
        levels: Option<AudioLevels>,
        error: Option<String>,
    }
    impl Default for Snapshot {
        fn default() -> Self {
            Self {
                state: None,
                pending: false,
                exhausted: false,
                format_generation: 0,
                format_ready: false,
                gain: 1.0,
                mute: false,
                latency_ms: 0.0,
                underruns: 0,
                levels: None,
                error: None,
            }
        }
    }
    #[derive(Clone)]
    struct Controls {
        commands: mpsc::SyncSender<Command>,
        snapshot: Signal<Snapshot>,
    }
    impl PartialEq for Controls {
        fn eq(&self, other: &Self) -> bool {
            self.snapshot == other.snapshot
        }
    }
    #[component(no_default)]
    struct Player {
        #[input]
        controls: Controls,
        #[state]
        admission_error: Option<String>,
    }
    impl Player {
        fn send(&mut self, command: Command) {
            self.admission_error = self
                .controls
                .commands
                .try_send(command)
                .err()
                .map(|error| error.to_string());
        }
    }
    impl Component for Player {
        fn view(&self) -> impl View {
            let status = self.watch(&self.controls.snapshot);
            let active = status
                .state
                .as_ref()
                .is_some_and(|state| !matches!(state, AudioState::Stopped | AudioState::Failed(_)));
            column()
                .padding(16.0)
                .gap(12.0)
                .child(text("Application sound playback"))
                .child(text(format!(
                    "{:?}{}{}",
                    status.state,
                    if status.pending {
                        " · request pending"
                    } else {
                        ""
                    },
                    if status.exhausted {
                        " · source submitted (device may still be playing)"
                    } else {
                        ""
                    }
                )))
                .child(
                    row()
                        .gap(8.0)
                        .height(36.0)
                        .child(
                            button("Play from start")
                                .enabled(!status.pending)
                                .on_press(|this: &mut Self| this.send(Command::Play)),
                        )
                        .child(
                            button("Pause")
                                .enabled(active && !status.pending)
                                .on_press(|this: &mut Self| this.send(Command::Pause)),
                        )
                        .child(
                            button("Resume")
                                .enabled(active && !status.pending && !status.exhausted)
                                .on_press(|this: &mut Self| this.send(Command::Resume)),
                        )
                        .child(
                            button("Stop")
                                .enabled(active)
                                .on_press(|this: &mut Self| this.send(Command::Stop)),
                        ),
                )
                .child(
                    row()
                        .gap(8.0)
                        .height(36.0)
                        .child(
                            button("Quieter")
                                .on_press(|this: &mut Self| this.send(Command::Quieter)),
                        )
                        .child(text(format!("{:.0}% local gain", status.gain * 100.0)))
                        .child(
                            button("Louder").on_press(|this: &mut Self| this.send(Command::Louder)),
                        )
                        .child(
                            button(if status.mute { "Unmute" } else { "Mute" })
                                .on_press(|this: &mut Self| this.send(Command::Mute)),
                        ),
                )
                .child(text(format!(
                    "Format generation {} · {}",
                    status.format_generation,
                    if status.format_ready {
                        "negotiated"
                    } else {
                        "unavailable"
                    }
                )))
                .child(text(format!(
                    "Graph latency {:.1} ms · underruns {}",
                    status.latency_ms, status.underruns
                )))
                .child(AudioLevelMeter::new(status.levels))
                .child(text(
                    self.admission_error
                        .clone()
                        .or_else(|| status.error.clone())
                        .unwrap_or_default(),
                ))
        }
    }
    pub fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let path = std::env::args_os()
            .nth(1)
            .ok_or("provide a PCM WAVE file")?;
        let file = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(file, 64 * 1024 * 1024 + 1),
            &mut bytes,
        )?;
        let asset = SoundAsset::decode_wav(&bytes)?;
        drop(bytes);
        let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
        let handle = connection.handle();
        let deadline = Instant::now() + Duration::from_secs(5);
        while handle.state() != ConnectionState::Ready {
            if Instant::now() >= deadline {
                return Err(format!("connection: {:?}", handle.state()).into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let (commands, receiver) = mpsc::sync_channel(16);
        let (snapshot, publish) = Signal::new(Snapshot::default());
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = stop.clone();
        let worker = std::thread::Builder::new()
            .name("sound-example".into())
            .spawn(move || {
                let mut playback: Option<SoundPlayback> = None;
                let mut meter: Option<AudioMeter> = None;
                let mut last_levels = Instant::now();
                let mut pending: Option<(Request, Instant)> = None;
                let mut status = Snapshot::default();
                while !stop_worker.load(Ordering::Acquire) {
                    if let Some((request, deadline)) = &pending {
                        match request.state() {
                            RequestState::Complete(result) => {
                                status.error = result.err().map(|error| error.to_string());
                                pending = None;
                            }
                            _ if Instant::now() >= *deadline => {
                                request.cancel();
                                if let Some(playback) = playback.take() {
                                    playback.stream().stop();
                                }
                                pending = None;
                                status.error =
                                    Some("Stream request timed out; playback stopped".into());
                            }
                            _ => {}
                        }
                    }
                    match receiver.recv_timeout(Duration::from_millis(50)) {
                        Ok(command) => {
                            // Stop always wins. Other native transitions are serialized; clicks
                            // made while a previous transition is pending report a visible error.
                            if pending.is_some() && !matches!(command, Command::Stop) {
                                status.error = Some("Wait for the current stream request".into());
                            } else {
                                let result: std::result::Result<(), MediaError> = (|| {
                                    status.error = None;
                                    match command {
                                        Command::Play => {
                                            meter = None;
                                            status.levels = None;
                                            if let Some(old) = playback.take() {
                                                old.stream().stop();
                                            }
                                            let next = asset.prepare(
                                                handle.clone(),
                                                AudioTarget::Default,
                                                false,
                                            )?;
                                            next.stream().set_gain(status.gain)?;
                                            next.stream().set_mute(status.mute);
                                            let next_meter = next.stream().meter();
                                            let request = next.stream().resume()?;
                                            pending = Some((
                                                request,
                                                Instant::now() + Duration::from_secs(5),
                                            ));
                                            playback = Some(next);
                                            meter = Some(next_meter);
                                        }
                                        Command::Stop => {
                                            if let Some((request, _)) = pending.take() {
                                                request.cancel();
                                            }
                                            if let Some(old) = playback.take() {
                                                old.stream().stop();
                                            }
                                        }
                                        Command::Pause | Command::Resume => {
                                            let stream = playback
                                                .as_ref()
                                                .ok_or(MediaError::NotReady)?
                                                .stream();
                                            let request = if matches!(command, Command::Pause) {
                                                stream.pause()?
                                            } else {
                                                stream.resume()?
                                            };
                                            pending = Some((
                                                request,
                                                Instant::now() + Duration::from_secs(5),
                                            ));
                                        }
                                        Command::Quieter | Command::Louder => {
                                            let delta = if matches!(command, Command::Quieter) {
                                                -0.1
                                            } else {
                                                0.1
                                            };
                                            let gain = (status.gain + delta).clamp(0.0, 1.0);
                                            if let Some(playback) = &playback {
                                                playback.stream().set_gain(gain)?;
                                            }
                                            status.gain = gain;
                                        }
                                        Command::Mute => {
                                            status.mute = !status.mute;
                                            if let Some(playback) = &playback {
                                                playback.stream().set_mute(status.mute);
                                            }
                                        }
                                    }
                                    Ok(())
                                })(
                                );
                                if let Err(error) = result {
                                    status.error = Some(error.to_string());
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    status.pending = pending.is_some();
                    status.state = playback.as_ref().map(|playback| playback.stream().state());
                    status.exhausted = playback
                        .as_ref()
                        .is_some_and(SoundPlayback::is_source_exhausted);
                    if let Some(playback) = &playback {
                        let negotiation = playback.stream().negotiation();
                        status.format_generation = negotiation.generation;
                        status.format_ready = negotiation.negotiated.is_some();
                        let diagnostics = playback.stream().diagnostics();
                        status.latency_ms =
                            diagnostics.latency_frames.max(0) as f64 * 1000.0 / asset.rate() as f64;
                        status.underruns = diagnostics.underruns;
                        if matches!(
                            status.state,
                            Some(AudioState::Streaming | AudioState::Draining)
                        ) && status.format_ready
                        {
                            if let Some(levels) = meter.as_mut().and_then(AudioMeter::read) {
                                status.levels = Some(levels);
                                last_levels = Instant::now();
                            }
                            let maximum_age = playback.stream().clock().snapshot().map_or(
                                Duration::from_millis(250),
                                |clock| {
                                    Duration::from_secs_f64(
                                        (3.0 * f64::from(clock.quantum_frames)
                                            / f64::from(clock.sample_rate.max(1)))
                                        .clamp(0.25, 10.0),
                                    )
                                },
                            );
                            if last_levels.elapsed() > maximum_age
                                || status.levels.is_some_and(|levels| {
                                    levels.format_generation != status.format_generation
                                })
                            {
                                status.levels = None;
                            }
                        } else {
                            status.levels = None;
                        }
                    } else {
                        meter = None;
                        status.levels = None;
                        status.format_ready = false;
                        status.latency_ms = 0.0;
                        status.underruns = 0;
                    }
                    publish.publish_if_changed(status.clone());
                }
                if let Some((request, _)) = pending {
                    request.cancel();
                }
                drop(playback);
            })?;
        let result = Application::gui("org.telorgon.examples.sound-playback", "Sound playback")
            .renderer(Renderer::Software)
            .window(
                Window::new("Sound playback")
                    .size(900, 520)
                    .content(Player {
                        controls: Controls { commands, snapshot },
                        admission_error: None,
                    }),
            )
            .run();
        stop.store(true, Ordering::Release);
        let joined = worker.join();
        connection.shutdown()?;
        joined.map_err(|_| "sound worker panicked")?;
        result?;
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    linux::run()
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This example requires Linux.");
}
