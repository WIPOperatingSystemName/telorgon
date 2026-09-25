//! Saved buffered-stream preferences. PipeWire node names identify configured endpoints, not
//! physical hardware across arbitrary system reconfiguration. No runtime IDs are persisted.
use super::{AudioConfig, AudioDirection, AudioFormat, AudioRole, AudioStream, AudioTarget};
use crate::{
    data::{RestoreState, SaveState},
    integrations::pipewire::{
        ConnectionHandle, MediaError, ObjectHandle, ObjectKind, RegistryObject, RegistrySnapshot,
    },
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SavedDeviceId", into = "SavedDeviceId")]
pub struct AudioDeviceId {
    node_name: String,
    media_class: String,
}
#[derive(Serialize, Deserialize)]
struct SavedDeviceId {
    node_name: String,
    media_class: String,
}
impl TryFrom<SavedDeviceId> for AudioDeviceId {
    type Error = MediaError;
    fn try_from(saved: SavedDeviceId) -> Result<Self, Self::Error> {
        if saved.node_name.is_empty()
            || saved.node_name.len() > 1024
            || saved.node_name.contains('\0')
            || !matches!(saved.media_class.as_str(), "Audio/Sink" | "Audio/Source")
        {
            return Err(MediaError::InvalidArgument("persistent audio endpoint"));
        }
        Ok(Self {
            node_name: saved.node_name,
            media_class: saved.media_class,
        })
    }
}
impl From<AudioDeviceId> for SavedDeviceId {
    fn from(id: AudioDeviceId) -> Self {
        Self {
            node_name: id.node_name,
            media_class: id.media_class,
        }
    }
}
impl AudioDeviceId {
    /// Only configured sinks/sources qualify. Application stream identities are intentionally excluded.
    pub fn from_node(node: &RegistryObject) -> Result<Self, MediaError> {
        if node.kind != ObjectKind::Node {
            return Err(MediaError::InvalidArgument("audio endpoint kind"));
        }
        SavedDeviceId {
            node_name: node
                .properties
                .get("node.name")
                .cloned()
                .unwrap_or_default(),
            media_class: node.media_class().unwrap_or_default().into(),
        }
        .try_into()
    }
    pub fn node_name(&self) -> &str {
        &self.node_name
    }
    fn resolve(&self, snapshot: &RegistrySnapshot) -> Result<Option<ObjectHandle>, MediaError> {
        let mut matches = snapshot.objects_of_kind(ObjectKind::Node).filter(|node| {
            node.properties.get("node.name") == Some(&self.node_name)
                && node.media_class() == Some(self.media_class.as_str())
        });
        let first = matches.next().map(|node| node.handle);
        if matches.next().is_some() {
            return Err(MediaError::InvalidArgument(
                "ambiguous saved audio endpoint",
            ));
        }
        Ok(first)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioDeviceFallback {
    SystemDefault,
    #[default]
    Fail,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "selection", rename_all = "kebab-case")]
pub enum AudioDevicePreference {
    #[default]
    SystemDefault,
    Specific {
        device: AudioDeviceId,
        fallback: AudioDeviceFallback,
    },
}

/// Saved preferences for ordinary buffered playback/capture, not virtual nodes or application capture.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "SavedStream", into = "SavedStream")]
pub struct AudioStreamPreference {
    pub name: String,
    pub direction: AudioDirection,
    pub format: AudioFormat,
    pub device: AudioDevicePreference,
    pub buffer_frames: usize,
    pub max_quantum: usize,
    pub start_paused: bool,
}
#[derive(Serialize, Deserialize)]
struct SavedStream {
    name: String,
    direction: AudioDirection,
    format: AudioFormat,
    device: AudioDevicePreference,
    buffer_frames: usize,
    max_quantum: usize,
    start_paused: bool,
}
impl From<AudioStreamPreference> for SavedStream {
    fn from(value: AudioStreamPreference) -> Self {
        Self {
            name: value.name,
            direction: value.direction,
            format: value.format,
            device: value.device,
            buffer_frames: value.buffer_frames,
            max_quantum: value.max_quantum,
            start_paused: value.start_paused,
        }
    }
}
impl TryFrom<SavedStream> for AudioStreamPreference {
    type Error = MediaError;
    fn try_from(value: SavedStream) -> Result<Self, Self::Error> {
        let value = Self {
            name: value.name,
            direction: value.direction,
            format: value.format,
            device: value.device,
            buffer_frames: value.buffer_frames,
            max_quantum: value.max_quantum,
            start_paused: value.start_paused,
        };
        value.validate()?;
        Ok(value)
    }
}
impl Default for AudioStreamPreference {
    fn default() -> Self {
        let config = AudioConfig::default();
        Self {
            name: config.name,
            direction: config.direction,
            format: config.format,
            device: AudioDevicePreference::default(),
            buffer_frames: config.buffer_frames,
            max_quantum: config.max_quantum,
            start_paused: config.start_paused,
        }
    }
}
impl AudioStreamPreference {
    fn config(&self, target: AudioTarget) -> AudioConfig {
        AudioConfig {
            name: self.name.clone(),
            role: AudioRole::Application,
            format: self.format,
            direction: self.direction,
            target,
            buffer_frames: self.buffer_frames,
            max_quantum: self.max_quantum,
            start_paused: self.start_paused,
        }
    }
    pub fn validate(&self) -> Result<(), MediaError> {
        self.config(AudioTarget::Default).validate()?;
        if let AudioDevicePreference::Specific { device, .. } = &self.device {
            let class = match self.direction {
                AudioDirection::Playback => "Audio/Sink",
                AudioDirection::Capture => "Audio/Source",
            };
            if device.media_class != class {
                return Err(MediaError::InvalidArgument("audio device direction"));
            }
        }
        Ok(())
    }
    /// Resolve against current discovery. A stale connection-scoped ID is never reconstructed.
    pub fn resolve(&self, snapshot: &RegistrySnapshot) -> Result<(AudioConfig, bool), MediaError> {
        self.validate()?;
        let (target, fallback) = match &self.device {
            AudioDevicePreference::SystemDefault => (AudioTarget::Default, false),
            AudioDevicePreference::Specific { device, fallback } => {
                match device.resolve(snapshot)? {
                    Some(handle) => (AudioTarget::Node(handle), false),
                    None if *fallback == AudioDeviceFallback::SystemDefault => {
                        (AudioTarget::Default, true)
                    }
                    None => {
                        return Err(MediaError::Unsupported(
                            "preferred audio endpoint is unavailable",
                        ));
                    }
                }
            }
        };
        Ok((self.config(target), fallback))
    }
}

/// Reconnection requests are submitted asynchronously; inspect stream.state()/negotiation() for
/// actual readiness and format. This does not stop or replace a previously opened stream.
pub struct RestoredAudioStream {
    pub stream: AudioStream,
    pub used_fallback: bool,
    pub preference: AudioStreamPreference,
}
impl SaveState for RestoredAudioStream {
    type Saved = AudioStreamPreference;
    type Error = MediaError;
    fn save_state(&self) -> Result<Self::Saved, Self::Error> {
        // Keep the desired endpoint even when a temporary fallback is active.
        Ok(self.preference.clone())
    }
}
impl RestoreState<AudioStreamPreference> for ConnectionHandle {
    type Resource = RestoredAudioStream;
    type Error = MediaError;
    fn restore_state(&self, saved: &AudioStreamPreference) -> Result<Self::Resource, Self::Error> {
        let (config, used_fallback) = saved.resolve(&self.snapshot())?;
        let stream = AudioStream::buffered(self.clone(), config)?;
        Ok(RestoredAudioStream {
            stream,
            used_fallback,
            preference: saved.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::pipewire::{ConnectionDiagnostics, ConnectionState};
    use std::collections::BTreeMap;
    fn snapshot() -> RegistrySnapshot {
        RegistrySnapshot {
            epoch: 5,
            revision: 1,
            state: ConnectionState::Ready,
            objects: BTreeMap::new(),
            diagnostics: ConnectionDiagnostics::default(),
            server_version: None,
            restricted: false,
        }
    }
    fn node(id: u32) -> RegistryObject {
        RegistryObject {
            handle: ObjectHandle {
                epoch: 5,
                incarnation: 2,
                id,
            },
            kind: ObjectKind::Node,
            permissions: 0,
            properties: BTreeMap::from([
                ("node.name".into(), "configured-speakers".into()),
                ("media.class".into(), "Audio/Sink".into()),
            ]),
            parameters: BTreeMap::new(),
            readable_parameters: vec![],
            writable_parameters: vec![],
            metadata: BTreeMap::new(),
        }
    }
    #[test]
    fn saved_selector_resolves_fresh_handles_and_retains_preference_on_fallback() {
        let endpoint = node(17);
        let mut preference = AudioStreamPreference {
            device: AudioDevicePreference::Specific {
                device: AudioDeviceId::from_node(&endpoint).unwrap(),
                fallback: AudioDeviceFallback::SystemDefault,
            },
            ..Default::default()
        };
        let registry = crate::data::Registry::new("audio-test", 1);
        registry.create("audio", preference.clone()).unwrap();
        let saved = registry.to_toml().unwrap();
        assert!(!saved.contains("incarnation"));
        let mut snapshot = snapshot();
        assert!(preference.resolve(&snapshot).unwrap().1);
        snapshot.objects.insert(99, node(99));
        let (config, fallback) = preference.resolve(&snapshot).unwrap();
        assert!(!fallback);
        assert!(matches!(config.target, AudioTarget::Node(handle) if handle.id() == 99));
        snapshot.objects.insert(100, node(100));
        assert!(preference.resolve(&snapshot).is_err());
        preference.direction = AudioDirection::Capture;
        assert!(preference.validate().is_err());
    }
}

impl<'de> Deserialize<'de> for AudioFormat {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Saved {
            sample_format: super::SampleFormat,
            rate: u32,
            channels: u32,
        }
        let saved = Saved::deserialize(deserializer)?;
        let format = Self {
            sample_format: saved.sample_format,
            rate: saved.rate,
            channels: saved.channels,
        };
        format.validate().map_err(serde::de::Error::custom)?;
        Ok(format)
    }
}

impl SaveState for AudioStream {
    type Saved = AudioStreamPreference;
    type Error = MediaError;
    fn save_state(&self) -> Result<Self::Saved, Self::Error> {
        if !matches!(self.config.role, AudioRole::Application) {
            return Err(MediaError::Unsupported("saving virtual audio streams"));
        }
        let device = match self.config.target {
            AudioTarget::Default => AudioDevicePreference::SystemDefault,
            AudioTarget::Node(handle) => AudioDevicePreference::Specific {
                device: AudioDeviceId::from_node(self.connection.snapshot().resolve(handle)?)?,
                fallback: AudioDeviceFallback::Fail,
            },
            _ => {
                return Err(MediaError::Unsupported(
                    "saving application or monitor capture targets",
                ));
            }
        };
        let preference = AudioStreamPreference {
            name: self.config.name.clone(),
            direction: self.config.direction,
            format: self.config.format,
            device,
            buffer_frames: self.config.buffer_frames,
            max_quantum: self.config.max_quantum,
            start_paused: self.config.start_paused,
        };
        preference.validate()?;
        Ok(preference)
    }
}
