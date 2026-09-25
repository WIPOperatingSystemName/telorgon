//! Native proxies and listeners stay together; fields drop listeners before proxies.
use super::{
    connection::{Shared, native},
    parameters, *,
};
use pipewire as pw;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::Arc};

#[allow(dead_code)]
pub(super) enum Binding {
    Node(pw::node::NodeListener, Rc<pw::node::Node>),
    Device(pw::device::DeviceListener, Rc<pw::device::Device>),
    Port(pw::port::PortListener, pw::port::Port),
    Link(pw::link::LinkListener, pw::link::Link),
    Metadata(pw::metadata::MetadataListener, pw::metadata::Metadata),
}
fn change(shared: &Shared, handle: ObjectHandle, edit: impl FnOnce(&mut RegistryObject)) {
    {
        let mut r = shared.registry.lock().unwrap_or_else(|e| e.into_inner());
        let Some(object) = r
            .snapshot
            .objects
            .get_mut(&handle.id)
            .filter(|o| o.handle == handle)
        else {
            return;
        };
        edit(object);
        r.snapshot.revision += 1;
    }
    shared.emit(ConnectionEvent::ObjectChanged(handle));
}
fn info(
    shared: &Shared,
    handle: ObjectHandle,
    props: Option<&pw::spa::utils::dict::DictRef>,
    update: Option<(&[pw::spa::param::ParamInfo], &RefCell<BTreeMap<u32, u32>>)>,
) {
    // Info notifications are deltas. Only PARAMS updates replace capabilities. The
    // SERIAL flag invalidates an enumeration even if READ/WRITE remain unchanged.
    let parameters = update.map(|(params, previous)| {
        let next: BTreeMap<_, _> = params
            .iter()
            .take(64)
            .map(|parameter| (parameter.id().as_raw(), parameter.flags().bits()))
            .collect();
        let old = previous.replace(next.clone());
        let changed: Vec<_> = old
            .keys()
            .chain(next.keys())
            .copied()
            .filter(|id| old.get(id) != next.get(id))
            .collect();
        (next, changed)
    });
    change(shared, handle, |object| {
        if let Some(props) = props {
            for (key, value) in props.iter().take(128) {
                if object.properties.len() < 128 || object.properties.contains_key(key) {
                    object.properties.insert(
                        key.chars().take(256).collect(),
                        value.chars().take(1024).collect(),
                    );
                }
            }
        }
        if let Some((parameters, changed)) = &parameters {
            object.readable_parameters = parameters
                .iter()
                .filter(|(_, flags)| **flags & pw::spa::sys::SPA_PARAM_INFO_READ != 0)
                .map(|(id, _)| *id)
                .collect();
            object.writable_parameters = parameters
                .iter()
                .filter(|(_, flags)| **flags & pw::spa::sys::SPA_PARAM_INFO_WRITE != 0)
                .map(|(id, _)| *id)
                .collect();
            object.parameters.retain(|(id, _), _| {
                !changed.contains(id) && object.readable_parameters.contains(id)
            });
        }
    });
}
fn param(
    shared: &Shared,
    handle: ObjectHandle,
    kind: pw::spa::param::ParamType,
    index: u32,
    pod: Option<&pw::spa::pod::Pod>,
) {
    // Bounded stored enumeration; omitted unsupported forms never imply supported controls.
    // Props can enumerate separate audio-control and hardware blocks. Preserve their
    // native indexes; collapsing all of them to zero discards the volume/mute block.
    let index = if matches!(
        kind,
        pw::spa::param::ParamType::Format | pw::spa::param::ParamType::Profile
    ) {
        0
    } else {
        index
    };
    let value = pod.and_then(|p| parameters::decode(p.as_bytes()));
    change(shared, handle, |object| {
        store_parameter(object, kind.as_raw(), index, value)
    });
}
fn store_parameter(
    object: &mut RegistryObject,
    kind: u32,
    index: u32,
    value: Option<ParameterValue>,
) {
    if !object.readable_parameters.contains(&kind) {
        return; // Ignore queued parameter notifications after capability withdrawal.
    }
    if index == 0 {
        object.parameters.retain(|(id, _), _| *id != kind);
    }
    if let Some(value) = value {
        if object.parameters.len() < 128 || object.parameters.contains_key(&(kind, index)) {
            object.parameters.insert((kind, index), value);
        }
    } else {
        object.parameters.remove(&(kind, index));
    }
}
impl Binding {
    pub fn bind(
        registry: &pw::registry::Registry,
        global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
        handle: ObjectHandle,
        shared: &Arc<Shared>,
    ) -> Result<Self, MediaError> {
        use pw::{spa::param::ParamType, types::ObjectType};
        let a = shared.clone();
        let b = shared.clone();
        Ok(match global.type_ {
            ObjectType::Node => {
                let proxy = Rc::new(registry.bind::<pw::node::Node, _>(global).map_err(native)?);
                let weak = Rc::downgrade(&proxy);
                let subscribed = RefCell::new(Vec::new());
                let inventory = RefCell::new(BTreeMap::new());
                let listener = proxy
                    .add_listener_local()
                    .info(move |i| {
                        let params_changed =
                            i.change_mask().contains(pw::node::NodeChangeMask::PARAMS);
                        let refresh = if params_changed {
                            parameter_refreshes(i.params().iter().map(|p| (p.id(), p.flags())), &inventory.borrow())
                        } else { Vec::new() };
                        info(
                            &a,
                            handle,
                            i.props(),
                            params_changed.then(|| (i.params(), &inventory)),
                        );
                        if !params_changed {
                            return;
                        }
                        if let Some(proxy) = weak.upgrade() {
                            if let Some(ids) = subscriptions(
                                &subscribed,
                                i.params(),
                                &[
                                    ParamType::Props,
                                    ParamType::Format,
                                    ParamType::PropInfo,
                                    ParamType::EnumFormat,
                                ],
                            ) {
                                proxy.subscribe_params(&ids);
                            } else {
                                // A SERIAL-only update invalidates cached values without changing
                                // our subscriptions. Explicitly fetch the new enumeration.
                                for id in refresh {
                                    if subscribed.borrow().contains(&id) {
                                        proxy.enum_params(0, Some(id), 0, 128);
                                    }
                                }
                            }
                        }
                    })
                    .param(move |_, kind, index, _, p| param(&b, handle, kind, index, p))
                    .register();
                Self::Node(listener, proxy)
            }
            ObjectType::Device => {
                let proxy = Rc::new(
                    registry
                        .bind::<pw::device::Device, _>(global)
                        .map_err(native)?,
                );
                let weak = Rc::downgrade(&proxy);
                let subscribed = RefCell::new(Vec::new());
                let inventory = RefCell::new(BTreeMap::new());
                let listener = proxy
                    .add_listener_local()
                    .info(move |i| {
                        let params_changed = i
                            .change_mask()
                            .contains(pw::device::DeviceChangeMask::PARAMS);
                        let refresh = if params_changed {
                            parameter_refreshes(i.params().iter().map(|p| (p.id(), p.flags())), &inventory.borrow())
                        } else { Vec::new() };
                        info(
                            &a,
                            handle,
                            i.props(),
                            params_changed.then(|| (i.params(), &inventory)),
                        );
                        if !params_changed {
                            return;
                        }
                        if let Some(proxy) = weak.upgrade() {
                            if let Some(ids) = subscriptions(
                                &subscribed,
                                i.params(),
                                &[
                                    ParamType::EnumProfile,
                                    ParamType::Profile,
                                    ParamType::EnumRoute,
                                    ParamType::Route,
                                ],
                            ) {
                                proxy.subscribe_params(&ids);
                            } else {
                                // A SERIAL-only update invalidates cached values without changing
                                // our subscriptions. Explicitly fetch the new enumeration.
                                for id in refresh {
                                    if subscribed.borrow().contains(&id) {
                                        proxy.enum_params(0, Some(id), 0, 128);
                                    }
                                }
                            }
                        }
                    })
                    .param(move |_, kind, index, _, p| param(&b, handle, kind, index, p))
                    .register();
                Self::Device(listener, proxy)
            }
            ObjectType::Port => {
                let proxy = registry.bind::<pw::port::Port, _>(global).map_err(native)?;
                let inventory = RefCell::new(BTreeMap::new());
                let listener = proxy
                    .add_listener_local()
                    .info(move |i| {
                        info(
                            &a,
                            handle,
                            i.props(),
                            i.change_mask()
                                .contains(pw::port::PortChangeMask::PARAMS)
                                .then(|| (i.params(), &inventory)),
                        )
                    })
                    .register();
                Self::Port(listener, proxy)
            }
            ObjectType::Link => {
                let proxy = registry.bind::<pw::link::Link, _>(global).map_err(native)?;
                let listener = proxy
                    .add_listener_local()
                    .info(move |i| info(&a, handle, i.props(), None))
                    .register();
                Self::Link(listener, proxy)
            }
            ObjectType::Metadata => {
                let proxy = registry
                    .bind::<pw::metadata::Metadata, _>(global)
                    .map_err(native)?;
                let listener = proxy
                    .add_listener_local()
                    .property(move |subject, key, _, value| {
                        change(&a, handle, |object| match (key, value) {
                            (Some(key), Some(value)) if key.len() <= 256 && value.len() <= 4096 => {
                                let key = (subject, key.to_owned());
                                if object.metadata.len() < 256 || object.metadata.contains_key(&key)
                                {
                                    object.metadata.insert(key, value.to_owned());
                                }
                            }
                            (Some(key), None) => {
                                object.metadata.remove(&(subject, key.to_owned()));
                            }
                            (None, _) => object.metadata.retain(|(s, _), _| *s != subject),
                            _ => {}
                        });
                        0
                    })
                    .register();
                Self::Metadata(listener, proxy)
            }
            _ => return Err(MediaError::Unsupported("registry object type")),
        })
    }
}
pub(super) type Bindings = BTreeMap<u32, Binding>;

// Subscribe only after advertised READ capability. Format is often unavailable until
// ports negotiate; subscribing early makes the daemon enumerate an unset format.
fn parameter_refreshes(
    params: impl IntoIterator<Item = (pw::spa::param::ParamType, pw::spa::param::ParamInfoFlags)>,
    previous: &BTreeMap<u32, u32>,
) -> Vec<pw::spa::param::ParamType> {
    params.into_iter().filter_map(|(id, flags)| {
        (flags.contains(pw::spa::param::ParamInfoFlags::READ)
            && previous.get(&id.as_raw()) != Some(&flags.bits())).then_some(id)
    }).collect()
}

fn subscriptions(
    previous: &RefCell<Vec<pw::spa::param::ParamType>>,
    params: &[pw::spa::param::ParamInfo],
    wanted: &[pw::spa::param::ParamType],
) -> Option<Vec<pw::spa::param::ParamType>> {
    let ids: Vec<_> = params
        .iter()
        .filter(|p| {
            wanted.contains(&p.id()) && p.flags().contains(pw::spa::param::ParamInfoFlags::READ)
        })
        .map(|p| p.id())
        .collect();
    let mut old = previous.borrow_mut();
    if *old == ids {
        None
    } else {
        *old = ids.clone();
        Some(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serial_only_changes_refresh_subscribed_audio_parameters() {
        use pw::spa::param::{ParamInfoFlags as Flags, ParamType};
        let old = BTreeMap::from([
            (ParamType::Route.as_raw(), Flags::READWRITE.bits()),
            (ParamType::Props.as_raw(), (Flags::READWRITE | Flags::SERIAL).bits()),
        ]);
        assert_eq!(parameter_refreshes([
            (ParamType::Route, Flags::READWRITE | Flags::SERIAL),
            (ParamType::Props, Flags::READWRITE),
        ], &old), vec![ParamType::Route, ParamType::Props]);
        assert!(parameter_refreshes([
            (ParamType::Route, Flags::READWRITE),
            (ParamType::Props, Flags::READWRITE | Flags::SERIAL),
        ], &old).is_empty());
        assert!(parameter_refreshes([(ParamType::Route, Flags::WRITE)], &old).is_empty());
    }

    fn node() -> RegistryObject {
        RegistryObject {
            handle: ObjectHandle {
                epoch: 1,
                incarnation: 1,
                id: 49,
            },
            kind: ObjectKind::Node,
            permissions: 0,
            properties: BTreeMap::new(),
            parameters: BTreeMap::new(),
            readable_parameters: vec![pw::spa::sys::SPA_PARAM_Props],
            writable_parameters: vec![],
            metadata: BTreeMap::new(),
        }
    }
    #[test]
    fn audio_props_blocks_preserve_volume_and_refresh_without_stale_hardware() {
        use pw::spa::sys::*;
        let mut node = node();
        let props = SPA_PARAM_Props;
        let controls = ParameterValue::Object(BTreeMap::from([
            (SPA_PROP_mute, ParameterValue::Bool(false)),
            (
                SPA_PROP_channelVolumes,
                ParameterValue::Floats(vec![0.157739, 0.157739]),
            ),
        ]));
        let hardware = ParameterValue::Object(BTreeMap::from([(
            SPA_PROP_device,
            ParameterValue::String("front:2".into()),
        )]));
        store_parameter(&mut node, props, 0, Some(controls.clone()));
        store_parameter(&mut node, props, 1, Some(hardware));
        assert_eq!(node.parameters.get(&(props, 0)), Some(&controls));
        assert!(node.parameters.contains_key(&(props, 1)));
        let updated = ParameterValue::Object(BTreeMap::from([(
            SPA_PROP_mute,
            ParameterValue::Bool(true),
        )]));
        store_parameter(&mut node, props, 0, Some(updated.clone()));
        assert_eq!(node.parameters.get(&(props, 0)), Some(&updated));
        assert!(!node.parameters.contains_key(&(props, 1)));
        node.readable_parameters.clear();
        store_parameter(&mut node, props, 1, Some(controls));
        assert!(!node.parameters.contains_key(&(props, 1)));
    }
}
