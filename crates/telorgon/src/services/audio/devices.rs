//! Device capability and selection snapshots; read from a single registry incarnation.
use super::*;
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveAudioRoute {
    pub index: i32,
    pub device: i32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevice {
    pub handle: ObjectHandle,
    pub name: String,
    pub description: String,
    pub profiles: Vec<DeviceChoice>,
    pub routes: Vec<DeviceChoice>,
    pub active_profile: Option<i32>,
    pub active_routes: Vec<ActiveAudioRoute>,
    pub can_set_profile: bool,
    pub can_set_route: bool,
}
impl AudioControls {
    pub fn devices(&self) -> Vec<AudioDevice> {
        use spa::sys::*;
        let snapshot = self.connection.snapshot();
        let referenced: BTreeSet<u32> = snapshot
            .objects_of_kind(ObjectKind::Node)
            .filter(|node| {
                node.media_class()
                    .is_some_and(|class| class.contains("Audio"))
            })
            .filter_map(|node| node.properties.get("device.id")?.parse().ok())
            .collect();
        snapshot
            .objects_of_kind(ObjectKind::Device)
            .filter(|device| {
                device.media_class() == Some("Audio/Device")
                    || referenced.contains(&device.handle.id())
            })
            .map(|object| {
                let integer = |parameter: &ParameterValue, key| match parameter.property(key) {
                    Some(ParameterValue::Int(value)) => Some(*value),
                    _ => None,
                };
                let active_profile = object.parameters.iter().find_map(|((kind, _), value)| {
                    (*kind == SPA_PARAM_Profile)
                        .then(|| integer(value, SPA_PARAM_PROFILE_index))
                        .flatten()
                });
                let active_routes = object
                    .parameters
                    .iter()
                    .filter_map(|((kind, _), value)| {
                        if *kind != SPA_PARAM_Route {
                            return None;
                        }
                        Some(ActiveAudioRoute {
                            index: integer(value, SPA_PARAM_ROUTE_index)?,
                            device: integer(value, SPA_PARAM_ROUTE_device)?,
                        })
                    })
                    .collect();
                AudioDevice {
                    handle: object.handle,
                    name: object
                        .properties
                        .get("device.name")
                        .cloned()
                        .unwrap_or_default(),
                    description: object
                        .properties
                        .get("device.description")
                        .or(object.properties.get("device.nick"))
                        .cloned()
                        .unwrap_or_default(),
                    profiles: choices(object, false),
                    routes: choices(object, true),
                    active_profile,
                    active_routes,
                    can_set_profile: object.can_write()
                        && object.writable_parameters.contains(&SPA_PARAM_Profile),
                    can_set_route: object.can_write()
                        && object.writable_parameters.contains(&SPA_PARAM_Route),
                }
            })
            .collect()
    }
}
pub(super) fn choices(object: &RegistryObject, route: bool) -> Vec<DeviceChoice> {
    use spa::sys::*;
    let (id, index, name, description, available) = if route {
        (
            SPA_PARAM_EnumRoute,
            SPA_PARAM_ROUTE_index,
            SPA_PARAM_ROUTE_name,
            SPA_PARAM_ROUTE_description,
            SPA_PARAM_ROUTE_available,
        )
    } else {
        (
            SPA_PARAM_EnumProfile,
            SPA_PARAM_PROFILE_index,
            SPA_PARAM_PROFILE_name,
            SPA_PARAM_PROFILE_description,
            SPA_PARAM_PROFILE_available,
        )
    };
    object
        .parameters
        .iter()
        .filter(|((kind, _), _)| *kind == id)
        .filter_map(|(_, p)| {
            let Some(ParameterValue::Int(i)) = p.property(index) else {
                return None;
            };
            let string = |key| match p.property(key) {
                Some(ParameterValue::String(s)) => s.clone(),
                _ => String::new(),
            };
            Some(DeviceChoice {
                index: *i,
                name: string(name),
                description: string(description),
                available: match p.property(available) {
                    Some(ParameterValue::Id(v)) if *v == SPA_PARAM_AVAILABILITY_no => Some(false),
                    Some(ParameterValue::Id(v)) if *v == SPA_PARAM_AVAILABILITY_yes => Some(true),
                    _ => None,
                },
                devices: match p.property(SPA_PARAM_ROUTE_devices) {
                    Some(ParameterValue::Ints(values)) if route => values.clone(),
                    _ => Vec::new(),
                },
                profiles: match p.property(SPA_PARAM_ROUTE_profiles) {
                    Some(ParameterValue::Ints(values)) if route => values.clone(),
                    _ => Vec::new(),
                },
                device: match p.property(SPA_PARAM_ROUTE_device) {
                    Some(ParameterValue::Int(v)) if route => Some(*v),
                    _ => None,
                },
            })
        })
        .collect()
}
