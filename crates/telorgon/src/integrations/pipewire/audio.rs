//! Native stream ownership and control, isolated from realtime data processing.
use super::{
    connection::{Completion, native},
    *,
};
use crate::media::audio::*;
use pipewire as pw;
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
#[derive(Clone, Copy)]
pub(crate) enum AudioCommand {
    Active(bool),
    Drain,
    Flush,
}
pub(crate) struct NativeAudio {
    // Listener unregistration precedes stream destruction, after deactivate on control loop.
    _listener: pw::stream::StreamListener<()>,
    stream: pw::stream::StreamRc,
    shared: Arc<AudioShared>,
    config: AudioConfig,
    pending: Vec<(AudioCommand, Completion, Instant)>,
    flush_sent: bool,
}
impl NativeAudio {
    pub(crate) fn create(
        core: pw::core::CoreRc,
        config: AudioConfig,
        shared: Arc<AudioShared>,
        mut data: RealtimeData,
        snapshot: &RegistrySnapshot,
    ) -> Result<Self, MediaError> {
        let mut props = pw::properties::properties! {"media.type"=>"Audio","media.category"=>if config.direction==AudioDirection::Playback{"Playback"}else{"Capture"},
        "media.role"=>"Music","adapter.auto-port-config"=>"{ mode = dsp monitor = false position = preserve }","node.name"=>config.name.clone(),"node.description"=>config.name.clone(),"node.latency"=>format!("{}/{}",config.max_quantum.min(1024),config.format.rate)};
        match config.role {
            AudioRole::Application => {}
            AudioRole::VirtualSource => {
                props.insert("media.class", "Audio/Source");
                props.insert("node.virtual", "true");
            }
            AudioRole::VirtualSink => {
                props.insert("media.class", "Audio/Sink");
                props.insert("node.virtual", "true");
            }
        }
        if let Some(handle) = config.target.handle() {
            let object = snapshot.resolve(handle)?;
            let serial = object
                .serial()
                .ok_or(MediaError::Unsupported("target object serial"))?;
            props.insert("target.object", serial.to_string());
            props.insert("node.dont-reconnect", "true");
            match config.target {
                AudioTarget::SystemOutput(_) => {
                    if object.media_class() != Some("Audio/Sink") {
                        return Err(MediaError::InvalidArgument("system output target"));
                    }
                    props.insert("stream.capture.sink", "true");
                }
                AudioTarget::Application(_) => {
                    if object.media_class() != Some("Stream/Output/Audio") {
                        return Err(MediaError::InvalidArgument("application output stream"));
                    }
                    props.insert("stream.capture.sink", "true");
                    props.insert("stream.dont-remix", "true");
                }
                _ => {}
            }
        }
        let stream = pw::stream::StreamRc::new(core, &config.name, props).map_err(native)?;
        let state_shared = shared.clone();
        let param_shared = shared.clone();
        let drained_shared = shared.clone();
        let format = config.format;
        let listener = stream
            .add_local_listener_with_user_data(())
            .state_changed(move |_, _, _, state| {
                if matches!(
                    *state_shared.state.lock().unwrap_or_else(|e| e.into_inner()),
                    AudioState::Failed(_)
                ) {
                    return;
                }
                state_shared.state(match state {
                    pw::stream::StreamState::Paused => AudioState::Paused,
                    pw::stream::StreamState::Streaming => AudioState::Streaming,
                    pw::stream::StreamState::Error(error) => AudioState::Failed(native(error)),
                    pw::stream::StreamState::Unconnected => AudioState::Stopped,
                    _ => AudioState::Connecting,
                });
            })
            .param_changed(move |_, _, id, pod| {
                if id != pw::spa::sys::SPA_PARAM_Format {
                    return;
                }
                let valid = pod.is_some_and(|p| {
                    let mut actual = pw::spa::param::audio::AudioInfoRaw::new();
                    actual.parse(p).is_ok()
                        && actual.channels() == format.channels
                        && actual.rate() == format.rate
                        && actual.format() == native_format(format.sample_format)
                });
                param_shared.negotiated(valid);
                param_shared.discontinuities.fetch_add(1, Ordering::Relaxed);
                if pod.is_some() && !valid {
                    param_shared.state(AudioState::Failed(MediaError::Unsupported(
                        "negotiated audio format",
                    )));
                    param_shared.stop.store(true, Ordering::Release);
                }
            })
            .drained(move |_, _| {
                drained_shared.state(AudioState::Paused);
                drained_shared.empty.store(true, Ordering::Release);
            })
            .process(move |stream, _| data.process(stream))
            .register()
            .map_err(native)?;
        let mut info = pw::spa::param::audio::AudioInfoRaw::new();
        info.set_format(native_format(format.sample_format));
        info.set_rate(format.rate);
        info.set_channels(format.channels);
        let bytes = super::parameters::encode(
            pw::spa::sys::SPA_TYPE_OBJECT_Format,
            pw::spa::sys::SPA_PARAM_EnumFormat,
            info.into(),
        )?;
        let mut flags = pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS;
        if matches!(config.role, AudioRole::Application) {
            flags |= pw::stream::StreamFlags::AUTOCONNECT;
        }
        if config.start_paused {
            flags |= pw::stream::StreamFlags::INACTIVE;
        }
        stream
            .connect(
                if config.direction == AudioDirection::Playback {
                    pw::spa::utils::Direction::Output
                } else {
                    pw::spa::utils::Direction::Input
                },
                None,
                flags,
                &mut [pw::spa::pod::Pod::from_bytes(&bytes)
                    .ok_or(MediaError::InvalidArgument("audio format POD"))?],
            )
            .map_err(native)?;
        Ok(Self {
            _listener: listener,
            stream,
            shared,
            config,
            pending: Vec::with_capacity(8),
            flush_sent: false,
        })
    }
    pub(crate) fn command(&mut self, command: AudioCommand, completion: Completion) {
        if !completion.begin() {
            return;
        }
        let terminal = match self.state() {
            AudioState::Failed(error) => Some(error),
            AudioState::Stopped => Some(MediaError::NotReady),
            _ if self.shared.stop.load(Ordering::Acquire) => Some(MediaError::NotReady),
            _ => None,
        };
        if let Some(error) = terminal {
            completion.finish(Err(error));
            return;
        }
        if !self.pending.is_empty() {
            completion.finish(Err(MediaError::NotReady));
            return;
        }
        let result = match command {
            AudioCommand::Active(active) => {
                self.shared.drain.store(false, Ordering::Release);
                self.stream.set_active(active).map_err(native)
            }
            AudioCommand::Drain => {
                if self.config.direction != AudioDirection::Playback
                    || self.state() != AudioState::Streaming
                {
                    Err(MediaError::NotReady)
                } else {
                    self.shared.drain_epoch.fetch_add(1, Ordering::Release);
                    self.shared.drain.store(true, Ordering::Release);
                    self.shared.state(AudioState::Draining);
                    self.flush_sent = false;
                    Ok(())
                }
            }
            AudioCommand::Flush => {
                if self.state() != AudioState::Streaming {
                    Err(MediaError::NotReady)
                } else {
                    self.shared.flush.fetch_add(1, Ordering::Release);
                    self.stream.flush(false).map_err(native)
                }
            }
        };
        match result {
            Ok(()) => self.pending.push((
                command,
                completion,
                Instant::now() + Duration::from_secs(10),
            )),
            Err(error) => completion.finish(Err(error)),
        }
    }
    fn state(&self) -> AudioState {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub(crate) fn poll(&mut self, targets: &[ObjectHandle]) -> bool {
        if self.shared.processor_failed.load(Ordering::Acquire) {
            self.shared
                .state(AudioState::Failed(native("realtime processor panicked")));
            self.shared.stop.store(true, Ordering::Release);
        }
        if self
            .config
            .target
            .handle()
            .is_some_and(|h| !targets.contains(&h))
        {
            self.shared
                .state(AudioState::Failed(MediaError::StaleHandle));
            self.shared.stop.store(true, Ordering::Release);
        }
        if self.shared.stop.load(Ordering::Acquire)
            || matches!(self.state(), AudioState::Failed(_) | AudioState::Stopped)
        {
            return false;
        }
        if self.shared.drain.load(Ordering::Acquire)
            && self.shared.drained_epoch.load(Ordering::Acquire)
                == self.shared.drain_epoch.load(Ordering::Acquire)
            && !self.flush_sent
        {
            self.flush_sent = true;
            if let Err(error) = self.stream.flush(true) {
                self.shared.state(AudioState::Failed(native(error)));
                return false;
            }
        }
        let state = self.state();
        let mut timed_out = false;
        self.pending.retain(|(command, completion, deadline)| {
            let done = match command {
                AudioCommand::Active(true) => state == AudioState::Streaming,
                AudioCommand::Active(false) => state == AudioState::Paused,
                AudioCommand::Drain => state == AudioState::Paused,
                AudioCommand::Flush => {
                    self.shared.flush.load(Ordering::Acquire)
                        == self.shared.flushed.load(Ordering::Acquire)
                }
            };
            if done {
                completion.finish(Ok(()));
                false
            } else if Instant::now() >= *deadline {
                completion.finish(Err(MediaError::Timeout));
                timed_out = true;
                false
            } else {
                true
            }
        });
        if timed_out {
            self.shared.state(AudioState::Failed(MediaError::Timeout));
            self.shared.stop.store(true, Ordering::Release);
            return false;
        }
        true
    }
}
impl Drop for NativeAudio {
    fn drop(&mut self) {
        let error = match self.state() {
            AudioState::Failed(error) => error,
            _ => MediaError::Disconnected,
        };
        for (_, completion, _) in self.pending.drain(..) {
            completion.finish(Err(error.clone()));
        }
        let _ = self.stream.set_active(false);
        let _ = self.stream.disconnect();
        self.shared.format_epoch.fetch_and(!1, Ordering::Release);
        if !matches!(self.state(), AudioState::Failed(_)) {
            self.shared.state(AudioState::Stopped);
        }
    }
}
fn native_format(format: SampleFormat) -> pw::spa::param::audio::AudioFormat {
    match format {
        SampleFormat::F32 => pw::spa::param::audio::AudioFormat::F32LE,
        SampleFormat::S16 => pw::spa::param::audio::AudioFormat::S16LE,
    }
}
