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
                priority: match p.property(if route { SPA_PARAM_ROUTE_priority } else { SPA_PARAM_PROFILE_priority }) {
                    Some(ParameterValue::Int(value)) => Some(*value), _ => None,
                },
                direction: if route {
                    match p.property(SPA_PARAM_ROUTE_direction) {
                        Some(ParameterValue::Id(value)) => Some(*value), _ => None,
                    }
                } else { None },
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use spa::sys::*;

    #[test]
    fn inventory_links_nodes_to_cards_and_keeps_reported_route_capabilities() {
        let (connection, _commands) = ConnectionHandle::test_channel(4);
        let handle = |id| ObjectHandle { epoch: 1, incarnation: id as u64, id };
        let device = RegistryObject {
            handle: handle(10), kind: ObjectKind::Device, permissions: u32::MAX,
            properties: BTreeMap::from([("media.class".into(), "Audio/Device".into())]),
            parameters: BTreeMap::from([
                ((SPA_PARAM_EnumProfile, 0), ParameterValue::Object(BTreeMap::from([
                    (SPA_PARAM_PROFILE_index, ParameterValue::Int(2)),
                    (SPA_PARAM_PROFILE_name, ParameterValue::String("stereo".into())),
                    (SPA_PARAM_PROFILE_priority, ParameterValue::Int(100)),
                ]))),
                ((SPA_PARAM_EnumRoute, 0), ParameterValue::Object(BTreeMap::from([
                    (SPA_PARAM_ROUTE_index, ParameterValue::Int(7)),
                    (SPA_PARAM_ROUTE_name, ParameterValue::String("headphones".into())),
                    (SPA_PARAM_ROUTE_direction, ParameterValue::Id(1)),
                    (SPA_PARAM_ROUTE_available, ParameterValue::Id(SPA_PARAM_AVAILABILITY_no)),
                    (SPA_PARAM_ROUTE_devices, ParameterValue::Ints(vec![0])),
                    (SPA_PARAM_ROUTE_profiles, ParameterValue::Ints(vec![2])),
                ]))),
                ((SPA_PARAM_Profile, 0), ParameterValue::Object(BTreeMap::from([
                    (SPA_PARAM_PROFILE_index, ParameterValue::Int(2)),
                ]))),
            ]),
            readable_parameters: vec![], writable_parameters: vec![SPA_PARAM_Profile], metadata: BTreeMap::new(),
        };
        let node = RegistryObject {
            handle: handle(11), kind: ObjectKind::Node, permissions: u32::MAX,
            properties: BTreeMap::from([
                ("media.class".into(), "Audio/Sink".into()),
                ("device.id".into(), "10".into()),
                ("card.profile.device".into(), "0".into()),
                ("node.virtual".into(), "false".into()),
                ("audio.rate".into(), "48000".into()),
            ]),
            parameters: BTreeMap::new(), readable_parameters: vec![], writable_parameters: vec![], metadata: BTreeMap::new(),
        };
        connection.shared.registry.lock().unwrap().snapshot.objects = BTreeMap::from([(10, device), (11, node)]);
        let controls = AudioControls::new(connection.clone()).unwrap();
        let nodes = controls.nodes();
        assert_eq!(nodes[0].device, Some(handle(10)));
        assert_eq!(nodes[0].profile_device, Some(0));
        assert_eq!(nodes[0].is_virtual, Some(false));
        assert_eq!(nodes[0].advertised_rate, Some(48000));
        let devices = controls.devices();
        assert_eq!(devices[0].profiles[0].priority, Some(100));
        assert_eq!(devices[0].profiles[0].available, None);
        assert_eq!(devices[0].routes[0].direction, Some(1));
        assert_eq!(devices[0].routes[0].available, Some(false));
        assert_eq!(devices[0].routes[0].profiles, vec![2]);
        assert_eq!(devices[0].active_profile, Some(2));
        assert!(devices[0].can_set_profile);
        assert!(!devices[0].can_set_route);
        connection.shared.registry.lock().unwrap().snapshot.objects.remove(&10);
        assert_eq!(controls.nodes()[0].device, None);
    }
}
