//! Control requests complete on observed state, never merely on a successful send.
use super::{
    bindings::{Binding, Bindings},
    connection::{Completion, Shared},
    *,
};
use pipewire::{
    proxy::ProxyT,
    spa::{param::ParamType, pod::Pod},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) enum Mutation {
    Parameter {
        video_source: bool,
        target: ObjectHandle,
        id: u32,
        bytes: Vec<u8>,
        expected: Vec<(u32, ParameterValue)>,
    },
    #[cfg(feature = "desktop-audio-linux")]
    Metadata {
        target: ObjectHandle,
        subject: Option<ObjectHandle>,
        key: String,
        value: String,
        /// Defaults observe the effective policy selection, not just configured metadata.
        observed_key: String,
    },
}
pub(super) struct PendingMutation {
    mutation: Mutation,
    completion: Completion,
    deadline: Instant,
    proxy_id: u32,
}
impl Mutation {
    pub(crate) fn allows_restricted(&self) -> bool {
        matches!(
            self,
            Self::Parameter {
                video_source: true,
                ..
            }
        )
    }
    fn target(&self) -> ObjectHandle {
        match self {
            Self::Parameter { target, .. } => *target,
            #[cfg(feature = "desktop-audio-linux")]
            Self::Metadata { target, .. } => *target,
        }
    }
    fn validate(&self, snapshot: &RegistrySnapshot) -> Result<(), MediaError> {
        let object = snapshot.resolve(self.target())?;
        if snapshot.restricted && !self.allows_restricted() {
            return Err(MediaError::PermissionDenied);
        }
        #[cfg(feature = "video-linux")]
        if let Self::Parameter {
            video_source: true,
            id,
            expected,
            ..
        } = self
        {
            if *id != pipewire::spa::sys::SPA_PARAM_Props || expected.len() != 1 {
                return Err(MediaError::InvalidArgument("camera control mutation"));
            }
            crate::media::video::controls::validate_write(object, expected[0].0, &expected[0].1)?;
        }
        // W | X are required by these native methods.
        if !object.can_write() {
            return Err(MediaError::PermissionDenied);
        }
        #[cfg(feature = "desktop-audio-linux")]
        if let Self::Metadata {
            subject: Some(subject),
            ..
        } = self
        {
            snapshot.resolve(*subject)?;
        }
        match self {
            Self::Parameter { id, .. } => {
                if !object.writable_parameters.contains(id) {
                    return Err(MediaError::Unsupported("writable parameter"));
                }
            }
            #[cfg(feature = "desktop-audio-linux")]
            Self::Metadata { .. } => {}
        }
        Ok(())
    }
    fn observed(&self, snapshot: &RegistrySnapshot) -> bool {
        let Ok(object) = snapshot.resolve(self.target()) else {
            return false;
        };
        match self {
            Self::Parameter { id, expected, .. } => {
                object.parameters.iter().any(|((kind, _), value)| {
                    *kind == *id
                        && expected.iter().all(|(key, want)| {
                            value
                                .property(*key)
                                .is_some_and(|have| property_equivalent(*key, have, want))
                        })
                })
            }
            #[cfg(feature = "desktop-audio-linux")]
            Self::Metadata {
                subject,
                key: _,
                value,
                observed_key,
                ..
            } => {
                let current = object
                    .metadata
                    .get(&(subject.map_or(0, ObjectHandle::id), observed_key.clone()));
                current.is_some_and(|have| {
                    have == value
                        || serde_json::from_str::<serde_json::Value>(have)
                            .ok()
                            .zip(serde_json::from_str::<serde_json::Value>(value).ok())
                            .is_some_and(|(a, b)| a == b)
                })
            }
        }
    }
}
fn property_equivalent(key: u32, have: &ParameterValue, want: &ParameterValue) -> bool {
    use pipewire::spa::sys::{SPA_PROP_channelVolumes, SPA_PROP_volume};
    if key == SPA_PROP_volume || key == SPA_PROP_channelVolumes {
        // Account for fixed-point volume steps in either linear gain or cubic UI units.
        // Pulse-compatible controls use 65536 steps to unity; rounding is not a failed write.
        let close = |a: f32, b: f32| {
            let step = 1.0 / 65536.0 + f32::EPSILON;
            a.is_finite() && b.is_finite() && a >= 0.0 && b >= 0.0
                && ((a - b).abs() <= step || (a.cbrt() - b.cbrt()).abs() <= step)
        };
        return match (have, want) {
            (ParameterValue::Float(a), ParameterValue::Float(b)) => close(*a, *b),
            (ParameterValue::Floats(a), ParameterValue::Floats(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| close(*a, *b))
            }
            _ => equivalent(have, want),
        };
    }
    equivalent(have, want)
}
fn equivalent(a: &ParameterValue, b: &ParameterValue) -> bool {
    match (a, b) {
        (ParameterValue::Float(a), ParameterValue::Float(b)) => (a - b).abs() <= 0.00001,
        (ParameterValue::Floats(a), ParameterValue::Floats(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 0.00001)
        }
        (ParameterValue::Object(have), ParameterValue::Object(want)) => want.iter()
            .all(|(key, value)| have.get(key).is_some_and(|observed| property_equivalent(*key, observed, value))),
        _ => a == b,
    }
}
impl PendingMutation {
    pub fn start(
        mutation: Mutation,
        completion: Completion,
        bindings: &Bindings,
        shared: &Arc<Shared>,
        timeout: Duration,
    ) -> Result<Option<Self>, MediaError> {
        if !completion.begin() {
            return Ok(None);
        }
        let send = || {
            mutation.validate(
                &shared
                    .registry
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .snapshot,
            )?;
            let binding = bindings
                .get(&mutation.target().id())
                .ok_or(MediaError::StaleHandle)?;
            let proxy_id = match (&mutation, binding) {
                (Mutation::Parameter { id, bytes, .. }, Binding::Node(_, node)) => {
                    node.set_param(
                        ParamType::from_raw(*id),
                        0,
                        Pod::from_bytes(bytes)
                            .ok_or(MediaError::InvalidArgument("parameter POD"))?,
                    );
                    node.upcast_ref().id()
                }
                (Mutation::Parameter { id, bytes, .. }, Binding::Device(_, device)) => {
                    device.set_param(
                        ParamType::from_raw(*id),
                        0,
                        Pod::from_bytes(bytes)
                            .ok_or(MediaError::InvalidArgument("parameter POD"))?,
                    );
                    device.upcast_ref().id()
                }
                #[cfg(feature = "desktop-audio-linux")]
                (
                    Mutation::Metadata {
                        subject,
                        key,
                        value,
                        ..
                    },
                    Binding::Metadata(_, metadata),
                ) => {
                    metadata.set_property(
                        subject.map_or(0, ObjectHandle::id),
                        key,
                        Some("Spa:String:JSON"),
                        Some(value),
                    );
                    metadata.upcast_ref().id()
                }
                _ => return Err(MediaError::Unsupported("control object type")),
            };
            Ok(proxy_id)
        };
        match send() {
            Ok(proxy_id) => Ok(Some(Self {
                mutation,
                completion,
                proxy_id,
                deadline: Instant::now() + timeout,
            })),
            Err(error) => {
                completion.finish(Err(error.clone()));
                Err(error)
            }
        }
    }
    pub fn poll(&self, snapshot: &RegistrySnapshot) -> bool {
        if matches!(
            *self.completion.0.lock().unwrap_or_else(|e| e.into_inner()),
            RequestState::Complete(_)
        ) {
            return false;
        }
        let outcome = if let Err(error) = self.mutation.validate(snapshot) {
            Some(Err(error))
        } else if self.mutation.observed(snapshot) {
            Some(Ok(()))
        } else if Instant::now() >= self.deadline {
            Some(Err(MediaError::Timeout))
        } else {
            None
        };
        if let Some(outcome) = outcome {
            self.completion.finish(outcome);
            false
        } else {
            true
        }
    }
    pub fn error(&self, id: u32, error: MediaError) {
        if id == self.proxy_id {
            self.completion.finish(Err(error));
        }
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn audio_volume_confirmation_accepts_quantization_without_accepting_stale_values() {
        use pipewire::spa::sys::{SPA_PROP_channelVolumes, SPA_PARAM_ROUTE_props};
        for step in 0..=1000 {
            let wanted = (step as f32 / 1000.0).powi(3);
            let rounded = (wanted * 65536.0).floor() / 65536.0;
            let props = |gain| ParameterValue::Object(BTreeMap::from([
                (SPA_PROP_channelVolumes, ParameterValue::Floats(vec![gain, gain * 0.5])),
            ]));
            assert!(property_equivalent(SPA_PARAM_ROUTE_props, &props(rounded), &props(wanted)));
            let pulse_rounded = ((wanted.cbrt() * 65536.0).floor() / 65536.0).powi(3);
            assert!(property_equivalent(SPA_PARAM_ROUTE_props, &props(pulse_rounded), &props(wanted)));
            assert!(!property_equivalent(SPA_PARAM_ROUTE_props, &props(wanted + 0.01), &props(wanted)));
        }
        assert!(!property_equivalent(SPA_PROP_channelVolumes,
            &ParameterValue::Floats(vec![0.5]), &ParameterValue::Floats(vec![0.5, 0.5])));
        // The audio tolerance must not weaken unrelated parameter confirmations.
        assert!(!property_equivalent(999, &ParameterValue::Float(0.500014), &ParameterValue::Float(0.5)));
    }
    #[test]
    fn audio_route_completion_matches_requested_properties_not_whole_object() {
        let wanted = ParameterValue::Object(BTreeMap::from([(1, ParameterValue::Floats(vec![0.2]))]));
        let observed = ParameterValue::Object(BTreeMap::from([
            (1, ParameterValue::Floats(vec![0.200001])),
            (2, ParameterValue::Bool(false)),
        ]));
        assert!(equivalent(&observed, &wanted));
        assert!(!equivalent(&ParameterValue::Object(BTreeMap::from([(1, ParameterValue::Floats(vec![0.8]))])), &wanted));
    }
}
