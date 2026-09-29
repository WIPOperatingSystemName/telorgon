//! Opt-in desktop-wide audio control. Construct separately from playback; a restricted
//! portal connection cannot be promoted to desktop authority. Snapshots reflect observed
//! native state. Mutations return cancellable requests and complete after observation,
//! or fail/timeout. No request is replayed after reconnect. All methods are non-realtime.
mod volume;
mod control_props;
use crate::integrations::pipewire::{self as transport, *};
use pipewire::spa::{self, pod::Property};
pub use volume::*;

#[derive(Clone)]
pub struct AudioControls {
    connection: ConnectionHandle,
}
#[derive(Clone, Debug, PartialEq)]
pub struct AudioNode {
    pub handle: ObjectHandle,
    pub device: Option<ObjectHandle>,
    pub profile_device: Option<i32>,
    pub is_virtual: Option<bool>,
    pub media_role: Option<String>,
    /// Advertised node properties, not a claim about the negotiated hardware format.
    pub advertised_rate: Option<u32>,
    pub advertised_format: Option<String>,
    pub bluetooth_codec: Option<String>,
    pub name: String,
    pub description: String,
    pub media_class: String,
    pub application_id: Option<String>,
    pub process_id: Option<u32>,
    /// PipeWire application.process.binary; an identity hint, not an authority.
    pub process_binary: Option<String>,
    pub application_name: Option<String>,
    pub application_icon: Option<String>,
    pub volume: Option<Gain>,
    pub mute: Option<bool>,
    pub channel_volumes: Vec<Gain>,
    pub channel_map: Vec<u32>,
    pub can_set_volume: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceChoice {
    pub index: i32,
    pub name: String,
    pub description: String,
    pub available: Option<bool>,
    pub device: Option<i32>,
    /// Associated device/profile indexes advertised by EnumRoute; empty means unknown.
    pub devices: Vec<i32>,
    pub profiles: Vec<i32>,
    pub priority: Option<i32>,
    /// SPA direction: 0 input, 1 output. None for profiles or an unreported route direction.
    pub direction: Option<u32>,
}
impl AudioControls {
    pub fn new(connection: ConnectionHandle) -> Result<Self, MediaError> {
        if connection.snapshot().restricted {
            return Err(MediaError::PermissionDenied);
        }
        Ok(Self { connection })
    }
    pub fn nodes(&self) -> Vec<AudioNode> {
        use spa::sys::*;
        let snapshot = self.connection.snapshot();
        snapshot
            .objects_of_kind(ObjectKind::Node)
            .filter_map(|o| {
                let class = o.media_class()?;
                if !class.contains("Audio") {
                    return None;
                }
                let route = control_props::control_route(&snapshot, o);
                let props = route.as_ref().map(|route| route.props).or_else(|| {
                    o.parameters.iter().find_map(|((kind, _), props)| {
                        (*kind == SPA_PARAM_Props && (props.property(SPA_PROP_channelVolumes).is_some()
                            || props.property(SPA_PROP_volume).is_some())).then_some(props)
                    })
                });
                let prop = |key| props.and_then(|p| p.property(key));
                let channels = match prop(SPA_PROP_channelVolumes) {
                    Some(ParameterValue::Floats(v)) => {
                        v.iter().filter_map(|v| Gain::linear(*v).ok()).collect()
                    }
                    _ => Vec::new(),
                };
                Some(AudioNode {
                    handle: o.handle,
                    device: o.properties.get("device.id")
                        .and_then(|id| id.parse::<u32>().ok())
                        .and_then(|id| snapshot.objects.get(&id))
                        .filter(|device| device.kind == ObjectKind::Device)
                        .map(|device| device.handle),
                    profile_device: o.properties.get("card.profile.device").and_then(|value| value.parse().ok()),
                    is_virtual: o.properties.get("node.virtual").and_then(|value| match value.as_str() {
                        "true" | "1" => Some(true), "false" | "0" => Some(false), _ => None,
                    }),
                    media_role: o.properties.get("media.role").cloned(),
                    advertised_rate: o.properties.get("audio.rate").and_then(|value| value.parse().ok()),
                    advertised_format: o.properties.get("audio.format").cloned(),
                    bluetooth_codec: o.properties.get("api.bluez5.codec").cloned(),
                    name: o.properties.get("node.name").cloned().unwrap_or_default(),
                    description: o
                        .properties
                        .get("node.description")
                        .or(o.properties.get("node.nick"))
                        .cloned()
                        .unwrap_or_default(),
                    media_class: class.to_owned(),
                    application_id: o.properties.get("application.id").cloned(),
                    application_name: o.properties.get("application.name").cloned(),
                    process_binary: o.properties.get("application.process.binary").cloned(),
                    application_icon: o.properties.get("application.icon-name").cloned(),
                    process_id: o
                        .properties
                        .get("application.process.id")
                        .and_then(|s| s.parse().ok()),
                    volume: match prop(SPA_PROP_volume) {
                        Some(ParameterValue::Float(v)) => Gain::linear(*v).ok(),
                        _ => None,
                    },
                    mute: match prop(SPA_PROP_mute) {
                        Some(ParameterValue::Bool(v)) => Some(*v),
                        _ => None,
                    },
                    channel_volumes: channels,
                    channel_map: match prop(SPA_PROP_channelMap) {
                        Some(ParameterValue::Ids(v)) => v.clone(),
                        _ => Vec::new(),
                    },
                    can_set_volume: route.as_ref().map_or_else(
                        || o.writable_parameters.contains(&SPA_PARAM_Props) && o.can_write(),
                        |route| route.device.can_write() && route.device.writable_parameters.contains(&SPA_PARAM_Route),
                    ),
                })
            })
            .collect()
    }
    /// Application IDs are metadata, not security identities or window mappings. Returns
    /// every matching stream from this snapshot; call again on registry changes.
    pub fn application_streams(&self, application_id: &str) -> Vec<AudioNode> {
        self.nodes()
            .into_iter()
            .filter(|n| {
                n.application_id.as_deref() == Some(application_id)
                    && n.media_class.starts_with("Stream/")
            })
            .collect()
    }
    pub fn set_volume(
        &self,
        target: ObjectHandle,
        gain: Gain,
        policy: Amplification,
    ) -> Result<Request, MediaError> {
        policy.validate(gain)?;
        // Hardware/session policy normally exposes per-channel volumes. Preserve balance.
        let node = self
            .nodes()
            .into_iter()
            .find(|n| n.handle == target)
            .ok_or(MediaError::StaleHandle)?;
        if node.channel_volumes.is_empty() {
            self.volume_properties(
                target,
                vec![(
                    spa::sys::SPA_PROP_volume,
                    ParameterValue::Float(gain.value()),
                )],
            )
        } else {
            let old = node
                .channel_volumes
                .iter()
                .map(|g| g.value())
                .fold(0.0_f32, f32::max);
            let values = node
                .channel_volumes
                .iter()
                .map(|g| {
                    if old > 0.0 {
                        gain.value() * g.value() / old
                    } else {
                        gain.value()
                    }
                })
                .collect();
            self.volume_properties(
                target,
                vec![(
                    spa::sys::SPA_PROP_channelVolumes,
                    ParameterValue::Floats(values),
                )],
            )
        }
    }
    pub fn set_mute(&self, target: ObjectHandle, mute: bool) -> Result<Request, MediaError> {
        self.volume_properties(
            target,
            vec![(spa::sys::SPA_PROP_mute, ParameterValue::Bool(mute))],
        )
    }
    pub fn set_channels(
        &self,
        target: ObjectHandle,
        gains: &[Gain],
        policy: Amplification,
    ) -> Result<Request, MediaError> {
        let node = self
            .nodes()
            .into_iter()
            .find(|n| n.handle == target)
            .ok_or(MediaError::StaleHandle)?;
        if gains.is_empty() || gains.len() > 64 || gains.len() != node.channel_volumes.len() {
            return Err(MediaError::InvalidArgument("channel count"));
        }
        for gain in gains {
            policy.validate(*gain)?;
        }
        self.volume_properties(
            target,
            vec![(
                spa::sys::SPA_PROP_channelVolumes,
                ParameterValue::Floats(gains.iter().map(|g| g.value()).collect()),
            )],
        )
    }
    /// Stereo balance attenuates one side and preserves the louder side. Multichannel
    /// layouts require explicit set_channels; channel order must be FL, FR.
    pub fn set_balance(
        &self,
        target: ObjectHandle,
        balance: f32,
        policy: Amplification,
    ) -> Result<Request, MediaError> {
        if !balance.is_finite() || !(-1.0..=1.0).contains(&balance) {
            return Err(MediaError::InvalidArgument("balance"));
        }
        let node = self
            .nodes()
            .into_iter()
            .find(|n| n.handle == target)
            .ok_or(MediaError::StaleHandle)?;
        if node.channel_map
            != [
                spa::sys::SPA_AUDIO_CHANNEL_FL,
                spa::sys::SPA_AUDIO_CHANNEL_FR,
            ]
            || node.channel_volumes.len() != 2
        {
            return Err(MediaError::Unsupported("stereo balance"));
        }
        let peak = node
            .channel_volumes
            .iter()
            .map(|g| g.value())
            .fold(0.0_f32, f32::max);
        self.set_channels(
            target,
            &[
                Gain::linear(peak * (1.0 - balance.max(0.0)))?,
                Gain::linear(peak * (1.0 + balance.min(0.0)))?,
            ],
            policy,
        )
    }
    pub fn profiles(&self, target: ObjectHandle) -> Result<Vec<DeviceChoice>, MediaError> {
        self.choices(target, false)
    }
    pub fn routes(&self, target: ObjectHandle) -> Result<Vec<DeviceChoice>, MediaError> {
        self.choices(target, true)
    }
    fn choices(&self, target: ObjectHandle, route: bool) -> Result<Vec<DeviceChoice>, MediaError> {
        let snapshot = self.connection.snapshot();
        let object = snapshot.resolve(target)?;
        if object.kind != ObjectKind::Device { return Err(MediaError::InvalidArgument("audio device")); }
        Ok(devices::choices(object, route))
    }
    pub fn set_profile(&self, target: ObjectHandle, index: i32) -> Result<Request, MediaError> {
        use spa::sys::*;
        if !self
            .profiles(target)?
            .iter()
            .any(|p| p.index == index && p.available != Some(false))
        {
            return Err(MediaError::Unsupported("available profile"));
        }
        self.parameter(
            target,
            SPA_PARAM_Profile,
            SPA_TYPE_OBJECT_ParamProfile,
            vec![
                (SPA_PARAM_PROFILE_index, ParameterValue::Int(index)),
                (SPA_PARAM_PROFILE_save, ParameterValue::Bool(true)),
            ],
        )
    }
    pub fn set_route(
        &self,
        target: ObjectHandle,
        index: i32,
        device: i32,
    ) -> Result<Request, MediaError> {
        use spa::sys::*;
        if !self
            .routes(target)?
            .iter()
            .any(|p| p.index == index && p.available != Some(false)
                && (p.device == Some(device) || p.devices.contains(&device)
                    || (p.device.is_none() && p.devices.is_empty())))
        {
            return Err(MediaError::Unsupported("available route/device"));
        }
        self.parameter(
            target,
            SPA_PARAM_Route,
            SPA_TYPE_OBJECT_ParamRoute,
            vec![
                (SPA_PARAM_ROUTE_index, ParameterValue::Int(index)),
                (SPA_PARAM_ROUTE_device, ParameterValue::Int(device)),
                (SPA_PARAM_ROUTE_save, ParameterValue::Bool(true)),
            ],
        )
    }
    fn parameter(
        &self,
        target: ObjectHandle,
        id: u32,
        type_: u32,
        expected: Vec<(u32, ParameterValue)>,
    ) -> Result<Request, MediaError> {
        let properties = expected
            .iter()
            .map(|(k, v)| Ok(Property::new(*k, transport::parameters::to_value(v)?)))
            .collect::<Result<Vec<_>, MediaError>>()?;
        let bytes = transport::parameters::encode(type_, id, properties)?;
        // 'save' is an instruction, not guaranteed to be echoed in observed state.
        let expected = expected
            .into_iter()
            .filter(|(key, _)| {
                !((id == spa::sys::SPA_PARAM_Profile && *key == spa::sys::SPA_PARAM_PROFILE_save)
                    || (id == spa::sys::SPA_PARAM_Route && *key == spa::sys::SPA_PARAM_ROUTE_save))
            })
            .collect();
        self.connection
            .mutate(transport::control::Mutation::Parameter {
                video_source: false,
                target,
                id,
                bytes,
                expected,
            })
    }
    pub fn policy(&self) -> crate::integrations::wireplumber::SessionPolicy {
        crate::integrations::wireplumber::SessionPolicy::new(self.connection.clone())
    }
    /// Shared entry point for widgets and shortcuts; repeated calls apply to the latest
    /// observed state. Callers should await completion before repeating relative actions.
    pub fn execute(&self, action: AudioAction) -> Result<Request, MediaError> {
        match action {
            AudioAction::Volume {
                target,
                gain,
                amplification,
            } => self.set_volume(target, gain, amplification),
            AudioAction::Mute { target, mute } => self.set_mute(target, mute),
            AudioAction::Default { target, kind } => self.policy().set_default(kind, target),
            AudioAction::MoveStream { stream, target } => self.policy().move_stream(stream, target),
            AudioAction::Profile { target, index } => self.set_profile(target, index),
            AudioAction::Route { target, index, device } => self.set_route(target, index, device),
            AudioAction::Balance { target, balance, amplification } => self.set_balance(target, balance, amplification),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum AudioAction {
    /// Completes when session-manager routing metadata is accepted, not when links settle.
    MoveStream { stream: ObjectHandle, target: ObjectHandle },
    Profile { target: ObjectHandle, index: i32 },
    Route { target: ObjectHandle, index: i32, device: i32 },
    Balance { target: ObjectHandle, balance: f32, amplification: Amplification },
    Volume {
        target: ObjectHandle,
        gain: Gain,
        amplification: Amplification,
    },
    Mute {
        target: ObjectHandle,
        mute: bool,
    },
    Default {
        target: ObjectHandle,
        kind: crate::integrations::wireplumber::DefaultKind,
    },
}

mod actions;
pub use actions::{AudioActionQueue, AudioControlTarget, AudioSystemAction};

#[cfg(feature = "audio-linux")]
mod meter;
#[cfg(feature = "audio-linux")]
pub use meter::AudioNodeMeter;

mod devices;
pub use devices::{AudioDevice, ActiveAudioRoute};

pub mod mixer;
