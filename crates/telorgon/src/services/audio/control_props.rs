//! Resolve the active hardware mixer route; streams and virtual sinks use node Props.
use super::*;
use spa::{
    pod::{Object, Value},
    sys::*,
};
use std::collections::BTreeMap;

pub(super) struct ControlRoute<'a> {
    pub device: &'a RegistryObject,
    pub index: i32,
    pub profile_device: i32,
    pub props: &'a ParameterValue,
}

pub(super) fn control_route<'a>(
    snapshot: &'a RegistrySnapshot,
    node: &RegistryObject,
) -> Option<ControlRoute<'a>> {
    if !matches!(node.media_class(), Some("Audio/Sink" | "Audio/Source")) {
        return None;
    }
    let device_id: u32 = node.properties.get("device.id")?.parse().ok()?;
    let profile_device: i32 = node.properties.get("card.profile.device")?.parse().ok()?;
    let device = snapshot.objects.get(&device_id)?;
    if device.kind != ObjectKind::Device {
        return None;
    }
    device.parameters.iter().find_map(|((kind, _), route)| {
        if *kind != SPA_PARAM_Route
            || route.property(SPA_PARAM_ROUTE_device) != Some(&ParameterValue::Int(profile_device))
        {
            return None;
        }
        let ParameterValue::Int(index) = route.property(SPA_PARAM_ROUTE_index)? else {
            return None;
        };
        let props = route.property(SPA_PARAM_ROUTE_props)?;
        props.property(SPA_PROP_channelVolumes)?;
        Some(ControlRoute {
            device,
            index: *index,
            profile_device,
            props,
        })
    })
}

impl AudioControls {
    pub(super) fn volume_properties(
        &self,
        target: ObjectHandle,
        props: Vec<(u32, ParameterValue)>,
    ) -> Result<Request, MediaError> {
        let snapshot = self.connection.snapshot();
        let node = snapshot.resolve(target)?;
        if let Some(route) = control_route(&snapshot, node) {
            let (bytes, expected) = route_write(route.index, route.profile_device, props)?;
            self.connection
                .mutate(transport::control::Mutation::Parameter {
                    video_source: false,
                    target: route.device.handle,
                    id: SPA_PARAM_Route,
                    bytes,
                    expected,
                })
        } else {
            self.parameter(target, SPA_PARAM_Props, SPA_TYPE_OBJECT_Props, props)
        }
    }
}

fn route_write(
    index: i32,
    device: i32,
    props: Vec<(u32, ParameterValue)>,
) -> Result<(Vec<u8>, Vec<(u32, ParameterValue)>), MediaError> {
    let nested = props
        .iter()
        .map(|(key, value)| Ok(Property::new(*key, transport::parameters::to_value(value)?)))
        .collect::<Result<Vec<_>, MediaError>>()?;
    let bytes = transport::parameters::encode(
        SPA_TYPE_OBJECT_ParamRoute,
        SPA_PARAM_Route,
        vec![
            Property::new(SPA_PARAM_ROUTE_index, Value::Int(index)),
            Property::new(SPA_PARAM_ROUTE_device, Value::Int(device)),
            Property::new(
                SPA_PARAM_ROUTE_props,
                Value::Object(Object {
                    type_: SPA_TYPE_OBJECT_Props,
                    id: SPA_PARAM_Props,
                    properties: nested,
                }),
            ),
            Property::new(SPA_PARAM_ROUTE_save, Value::Bool(true)),
        ],
    )?;
    Ok((
        bytes,
        vec![
            (SPA_PARAM_ROUTE_index, ParameterValue::Int(index)),
            (SPA_PARAM_ROUTE_device, ParameterValue::Int(device)),
            (
                SPA_PARAM_ROUTE_props,
                ParameterValue::Object(props.into_iter().collect::<BTreeMap<_, _>>()),
            ),
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_route_write_preserves_selection_and_encodes_nested_props() {
        let props = vec![(
            SPA_PROP_channelVolumes,
            ParameterValue::Floats(vec![0.1, 0.2]),
        )];
        let (bytes, expected) = route_write(2, 3, props).unwrap();
        let decoded = transport::parameters::decode(&bytes).unwrap();
        assert_eq!(
            decoded.property(SPA_PARAM_ROUTE_index),
            Some(&ParameterValue::Int(2))
        );
        assert_eq!(
            decoded.property(SPA_PARAM_ROUTE_device),
            Some(&ParameterValue::Int(3))
        );
        assert_eq!(
            decoded.property(SPA_PARAM_ROUTE_save),
            Some(&ParameterValue::Bool(true))
        );
        assert_eq!(
            decoded.property(SPA_PARAM_ROUTE_props),
            Some(&expected[2].1)
        );
    }

    #[test]
    fn audio_route_resolves_output_and_input_independently() {
        let mut snapshot = RegistrySnapshot {
            epoch: 1, revision: 0, state: ConnectionState::Ready,
            objects: BTreeMap::new(), diagnostics: Default::default(),
            server_version: None, restricted: false,
        };
        let object = |id, kind, properties, parameters| RegistryObject {
            handle: ObjectHandle {
                epoch: 1,
                incarnation: u64::from(id),
                id,
            },
            kind,
            permissions: u32::MAX,
            properties,
            parameters,
            readable_parameters: vec![],
            writable_parameters: vec![SPA_PARAM_Route],
            metadata: BTreeMap::new(),
        };
        let route = |index, device, volume| {
            ParameterValue::Object(BTreeMap::from([
                (SPA_PARAM_ROUTE_index, ParameterValue::Int(index)),
                (SPA_PARAM_ROUTE_device, ParameterValue::Int(device)),
                (
                    SPA_PARAM_ROUTE_props,
                    ParameterValue::Object(BTreeMap::from([(
                        SPA_PROP_channelVolumes,
                        ParameterValue::Floats(vec![volume]),
                    )])),
                ),
            ]))
        };
        snapshot.objects.insert(
            37,
            object(
                37,
                ObjectKind::Device,
                BTreeMap::new(),
                BTreeMap::from([
                    ((SPA_PARAM_Route, 0), route(0, 0, 1.0)),
                    ((SPA_PARAM_Route, 1), route(2, 3, 0.2)),
                ]),
            ),
        );
        let mut node = object(
            83,
            ObjectKind::Node,
            BTreeMap::from([
                ("media.class".into(), "Audio/Sink".into()),
                ("device.id".into(), "37".into()),
                ("card.profile.device".into(), "3".into()),
            ]),
            BTreeMap::new(),
        );
        assert_eq!(control_route(&snapshot, &node).unwrap().index, 2);
        node.properties
            .insert("media.class".into(), "Audio/Source".into());
        node.properties
            .insert("card.profile.device".into(), "0".into());
        assert_eq!(control_route(&snapshot, &node).unwrap().index, 0);
        node.properties
            .insert("media.class".into(), "Stream/Output/Audio".into());
        assert!(
            control_route(&snapshot, &node).is_none(),
            "application volumes must stay on their own stream"
        );
    }
}
