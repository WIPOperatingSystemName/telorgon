//! Application capture tracks multiple/changing output nodes explicitly. Application
//! metadata is advisory, not a permission/security identity or a mapping from windows.
use super::*;
use crate::integrations::pipewire::*;
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct ApplicationSelector {
    pub application_id: String,
    pub process_id: Option<u32>,
}
pub struct ApplicationCapture {
    connection: ConnectionHandle,
    selector: ApplicationSelector,
    config: AudioConfig,
    streams: BTreeMap<ObjectHandle, AudioStream>,
    limit: usize,
}
impl ApplicationCapture {
    /// No microphone/default fallback: only matching application output streams are opened.
    /// Call refresh on registry events (or periodically from the host). This lets the caller
    /// decide when updated application membership should cause new recording streams.
    pub fn new(
        connection: ConnectionHandle,
        selector: ApplicationSelector,
        format: AudioFormat,
        max_streams: usize,
    ) -> Result<Self, MediaError> {
        if selector.application_id.is_empty()
            || selector.application_id.len() > 256
            || !(1..=32).contains(&max_streams)
        {
            return Err(MediaError::InvalidArgument("application capture selection"));
        }
        format.validate()?;
        Ok(Self {
            connection,
            selector,
            config: AudioConfig {
                name: "Telorgon application recording".into(),
                format,
                direction: AudioDirection::Capture,
                ..Default::default()
            },
            streams: BTreeMap::new(),
            limit: max_streams,
        })
    }
    /// Removes vanished streams before creating new ones. On capacity exhaustion existing
    /// recordings continue and ResourceLimit reports that membership is incomplete.
    pub fn refresh(&mut self) -> Result<(), MediaError> {
        let snapshot = self.connection.snapshot();
        if snapshot.state != ConnectionState::Ready {
            self.streams.clear();
            return Err(MediaError::Disconnected);
        }
        let targets = snapshot
            .objects
            .values()
            .filter(|o| {
                o.media_class() == Some("Stream/Output/Audio")
                    && o.properties.get("application.id") == Some(&self.selector.application_id)
                    && self.selector.process_id.is_none_or(|id| {
                        o.properties
                            .get("application.process.id")
                            .and_then(|s| s.parse::<u32>().ok())
                            == Some(id)
                    })
            })
            .map(|o| o.handle)
            .collect::<Vec<_>>();
        self.streams.retain(|handle, _| targets.contains(handle));
        for target in targets {
            if !self.streams.contains_key(&target) {
                if self.streams.len() >= self.limit {
                    return Err(MediaError::ResourceLimit("application capture streams"));
                }
                let mut config = self.config.clone();
                config.target = AudioTarget::Application(target);
                self.streams.insert(
                    target,
                    AudioStream::buffered(self.connection.clone(), config)?,
                );
            }
        }
        Ok(())
    }
    pub fn streams(&mut self) -> impl Iterator<Item = (ObjectHandle, &mut AudioStream)> {
        self.streams.iter_mut().map(|(h, s)| (*h, s))
    }
}
/// Two independent streams on one connection. Both initially pause; resume each explicitly
/// after preparation. PipeWire supplies clock observations for aligning input/output;
/// this owner does not promise same-callback sample alignment (use a multiport filter).
pub struct DuplexAudio {
    pub capture: AudioStream,
    pub playback: AudioStream,
}
impl DuplexAudio {
    pub fn buffered(
        connection: ConnectionHandle,
        mut capture: AudioConfig,
        mut playback: AudioConfig,
    ) -> Result<Self, MediaError> {
        capture.direction = AudioDirection::Capture;
        capture.start_paused = true;
        playback.direction = AudioDirection::Playback;
        playback.start_paused = true;
        let capture = AudioStream::buffered(connection.clone(), capture)?;
        let playback = AudioStream::buffered(connection, playback)?;
        Ok(Self { capture, playback })
    }
}
