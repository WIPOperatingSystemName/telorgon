//! Capability-driven scalar camera controls. No V4L2 file descriptors or device paths.
use crate::integrations::pipewire::{
    self as transport, ConnectionHandle, MediaError, ObjectHandle, ObjectKind, ParameterChoice,
    ParameterValue, RegistryObject, Request,
};
use pipewire::spa::{pod::Property, sys::*};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraControlId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraValue {
    Bool(bool),
    Int(i32),
    Float(f32),
    Double(f64),
    Id(u32),
}
impl CameraValue {
    fn read(value: &ParameterValue) -> Option<Self> {
        Some(match value {
            ParameterValue::Bool(v) => Self::Bool(*v),
            ParameterValue::Int(v) => Self::Int(*v),
            ParameterValue::Float(v) if v.is_finite() => Self::Float(*v),
            ParameterValue::Double(v) if v.is_finite() => Self::Double(*v),
            ParameterValue::Id(v) => Self::Id(*v),
            _ => return None,
        })
    }
    fn parameter(self) -> ParameterValue {
        match self {
            Self::Bool(v) => ParameterValue::Bool(v),
            Self::Int(v) => ParameterValue::Int(v),
            Self::Float(v) => ParameterValue::Float(v),
            Self::Double(v) => ParameterValue::Double(v),
            Self::Id(v) => ParameterValue::Id(v),
        }
    }
    fn number(self) -> f64 {
        match self {
            Self::Bool(v) => u8::from(v) as f64,
            Self::Int(v) => v as f64,
            Self::Float(v) => v as f64,
            Self::Double(v) => v,
            Self::Id(v) => v as f64,
        }
    }
    fn same_type(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum CameraDomain {
    /// A scalar PropInfo declares type and default but no numeric bounds.
    Scalar(CameraValue),
    Range {
        default: CameraValue,
        min: CameraValue,
        max: CameraValue,
        step: Option<CameraValue>,
    },
    Enum {
        default: CameraValue,
        values: Vec<CameraValue>,
    },
}
impl CameraDomain {
    pub fn accepts(&self, value: CameraValue) -> bool {
        if !value.number().is_finite() {
            return false;
        }
        match self {
            Self::Scalar(default) => default.same_type(value),
            Self::Enum { values, .. } => values.contains(&value),
            Self::Range { min, max, step, .. } => {
                if !min.same_type(value)
                    || !max.same_type(value)
                    || value.number() < min.number()
                    || value.number() > max.number()
                {
                    return false;
                }
                if let Some(step) = step {
                    if !step.same_type(value) || step.number() <= 0.0 {
                        return false;
                    }
                    let steps = (value.number() - min.number()) / step.number();
                    // Int/Id are exactly represented in f64 and need exact integral steps.
                    let tolerance = if matches!(value, CameraValue::Int(_) | CameraValue::Id(_)) {
                        0.0
                    } else {
                        1e-5
                    };
                    if (steps - steps.round()).abs() > tolerance {
                        return false;
                    }
                }
                true
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct CameraControl {
    pub id: CameraControlId,
    pub name: String,
    pub description: String,
    pub domain: CameraDomain,
    pub labels: Vec<(CameraValue, String)>,
    pub current: Option<CameraValue>,
    /// Advisory: permissions may change and the server may reject a write.
    pub writable: bool,
}
/// A connection-scoped service; camera permission remains with its supplied connection.
/// Use ConnectionHandle subscriptions to refresh controls after object changes. A request
/// completes only when Props observes the requested value. Cancellation follows Request;
/// disconnect/removal fails outstanding operations and never rebinds recycled IDs.
pub struct CameraControls {
    connection: ConnectionHandle,
}
impl CameraControls {
    pub fn new(connection: ConnectionHandle) -> Self {
        Self { connection }
    }
    pub fn controls(&self, target: ObjectHandle) -> Result<Vec<CameraControl>, MediaError> {
        self.connection.ensure_ready()?;
        let snapshot = self.connection.snapshot();
        let object = snapshot.resolve(target)?;
        validate_source(object)?;
        Ok(controls(object))
    }
    pub fn set(
        &self,
        target: ObjectHandle,
        id: CameraControlId,
        value: CameraValue,
    ) -> Result<Request, MediaError> {
        self.connection.ensure_ready()?;
        let snapshot = self.connection.snapshot();
        let object = snapshot.resolve(target)?;
        let value = value.parameter();
        validate_write(object, id.0, &value)?;
        let bytes = transport::parameters::encode(
            SPA_TYPE_OBJECT_Props,
            SPA_PARAM_Props,
            vec![Property::new(
                id.0,
                transport::parameters::to_value(&value)?,
            )],
        )?;
        self.connection
            .mutate(transport::control::Mutation::Parameter {
                video_source: true,
                target,
                id: SPA_PARAM_Props,
                bytes,
                expected: vec![(id.0, value)],
            })
    }
}
fn validate_source(object: &RegistryObject) -> Result<(), MediaError> {
    if object.kind != ObjectKind::Node || object.media_class() != Some("Video/Source") {
        return Err(MediaError::Unsupported(
            "camera controls require a video source",
        ));
    }
    Ok(())
}
pub(crate) fn validate_write(
    object: &RegistryObject,
    id: u32,
    value: &ParameterValue,
) -> Result<(), MediaError> {
    validate_source(object)?;
    let control = controls(object)
        .into_iter()
        .find(|c| c.id.0 == id)
        .ok_or(MediaError::Unsupported("camera control not advertised"))?;
    if !object.can_write() {
        return Err(MediaError::PermissionDenied);
    }
    if !control.writable {
        return Err(MediaError::Unsupported("read-only camera control"));
    }
    if !CameraValue::read(value).is_some_and(|v| control.domain.accepts(v)) {
        return Err(MediaError::InvalidArgument(
            "camera control type, range or step",
        ));
    }
    Ok(())
}
fn controls(object: &RegistryObject) -> Vec<CameraControl> {
    let current = object.parameters.get(&(SPA_PARAM_Props, 0));
    object
        .parameters
        .iter()
        .filter(|((kind, _), _)| *kind == SPA_PARAM_PropInfo)
        .filter_map(|(_, info)| {
            let ParameterValue::Id(id) = info.property(SPA_PROP_INFO_id)? else {
                return None;
            };
            // Exclude device-path/FD/configuration properties even when a native source exposes
            // them in PropInfo. Only standardized video and driver-specific controls qualify.
            if !(*id > SPA_PROP_START_Video && *id < SPA_PROP_START_Other
                || *id >= SPA_PROP_START_CUSTOM)
            {
                return None;
            }
            if matches!(info.property(SPA_PROP_INFO_params),Some(ParameterValue::Bool(true)))
                || matches!(info.property(SPA_PROP_INFO_container),Some(ParameterValue::Id(kind)) if *kind!=SPA_TYPE_None) { return None; }
            let text = |key| match info.property(key) {
                Some(ParameterValue::String(v)) => v.clone(),
                _ => String::new(),
            };
            let mut domain = domain(info.property(SPA_PROP_INFO_type)?)?;
            let labels =
                if let Some(ParameterValue::Struct(values)) = info.property(SPA_PROP_INFO_labels) {
                    values
                        .chunks_exact(2)
                        .filter_map(|p| {
                            let value = CameraValue::read(&p[0])?;
                            let ParameterValue::String(label) = &p[1] else {
                                return None;
                            };
                            Some((value, label.clone()))
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
            // V4L2 menu controls advertise a scalar type and their actual choices in labels.
            if let CameraDomain::Scalar(default) = domain
                && !labels.is_empty()
            {
                domain = CameraDomain::Enum {
                    default,
                    values: labels.iter().map(|(v, _)| *v).collect(),
                };
            }
            Some(CameraControl {
                id: CameraControlId(*id),
                name: text(SPA_PROP_INFO_name),
                description: text(SPA_PROP_INFO_description),
                domain,
                labels,
                current: current
                    .and_then(|p| p.property(*id))
                    .and_then(CameraValue::read),
                writable: object.can_write()
                    && object.writable_parameters.contains(&SPA_PARAM_Props)
                    && !info.property_is_read_only(SPA_PROP_INFO_type)
                    && !current.is_some_and(|p| p.property_is_read_only(*id)),
            })
        })
        .take(64)
        .collect()
}
fn domain(value: &ParameterValue) -> Option<CameraDomain> {
    if let Some(default) = CameraValue::read(value) {
        return Some(CameraDomain::Scalar(default));
    }
    let ParameterValue::Choice { kind, values } = value else {
        return None;
    };
    let values: Vec<_> = values
        .iter()
        .map(CameraValue::read)
        .collect::<Option<_>>()?;
    let default = *values.first()?;
    if values.iter().any(|v| !default.same_type(*v)) {
        return None;
    }
    Some(match kind {
        ParameterChoice::Fixed => CameraDomain::Enum { default, values },
        ParameterChoice::Enum => CameraDomain::Enum {
            default,
            values: if values.len() > 1 {
                values[1..].to_vec()
            } else {
                values
            },
        },
        ParameterChoice::Range | ParameterChoice::Step => {
            let min = *values.get(1)?;
            let max = *values.get(2)?;
            if min.number() > max.number() {
                return None;
            }
            CameraDomain::Range {
                default,
                min,
                max,
                step: values.get(3).copied(),
            }
        }
        ParameterChoice::Flags => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pipewire::spa::{
        pod::{ChoiceValue, PropertyFlags, Value},
        utils::{Choice, ChoiceEnum, ChoiceFlags, Id},
    };
    use std::collections::BTreeMap;
    fn info(id: u32, value: Value, readonly: bool) -> ParameterValue {
        let mut domain = Property::new(SPA_PROP_INFO_type, value);
        if readonly {
            domain.flags = PropertyFlags::READONLY;
        }
        let bytes = transport::parameters::encode(
            SPA_TYPE_OBJECT_PropInfo,
            SPA_PARAM_PropInfo,
            vec![
                Property::new(SPA_PROP_INFO_id, Value::Id(Id(id))),
                domain,
                Property::new(
                    SPA_PROP_INFO_description,
                    Value::String("Camera control".into()),
                ),
            ],
        )
        .unwrap();
        transport::parameters::decode(&bytes).unwrap()
    }
    #[test]
    fn advertised_camera_ranges_preserve_steps_readonly_and_portal_authority() {
        let (handle, _receiver) = ConnectionHandle::test_channel(8);
        let target = {
            let mut registry = handle.shared.registry.lock().unwrap();
            registry.snapshot.restricted = true;
            let target = registry
                .insert(
                    21,
                    ObjectKind::Node,
                    u32::MAX,
                    BTreeMap::from([("media.class".into(), "Video/Source".into())]),
                )
                .unwrap();
            let object = registry.snapshot.objects.get_mut(&21).unwrap();
            object.writable_parameters.push(SPA_PARAM_Props);
            object.parameters.insert(
                (SPA_PARAM_PropInfo, 0),
                info(
                    SPA_PROP_brightness,
                    Value::Choice(ChoiceValue::Int(Choice(
                        ChoiceFlags::empty(),
                        ChoiceEnum::Step {
                            default: 0,
                            min: -10,
                            max: 10,
                            step: 2,
                        },
                    ))),
                    false,
                ),
            );
            object.parameters.insert(
                (SPA_PARAM_PropInfo, 1),
                info(SPA_PROP_exposure, Value::Int(4), true),
            );
            object.parameters.insert(
                (SPA_PARAM_PropInfo, 2),
                info(SPA_PROP_deviceFd, Value::Int(9), false),
            );
            object.parameters.insert(
                (SPA_PARAM_Props, 0),
                ParameterValue::Object(BTreeMap::from([(
                    SPA_PROP_brightness,
                    ParameterValue::Int(0),
                )])),
            );
            target
        };
        let controls = CameraControls::new(handle.clone());
        let discovered = controls.controls(target).unwrap();
        assert_eq!(discovered.len(), 2);
        assert_eq!(discovered[0].current, Some(CameraValue::Int(0)));
        assert!(
            controls
                .set(
                    target,
                    CameraControlId(SPA_PROP_brightness),
                    CameraValue::Int(8)
                )
                .is_ok()
        );
        assert!(matches!(
            controls.set(
                target,
                CameraControlId(SPA_PROP_brightness),
                CameraValue::Int(3)
            ),
            Err(MediaError::InvalidArgument(_))
        ));
        assert!(matches!(
            controls.set(
                target,
                CameraControlId(SPA_PROP_exposure),
                CameraValue::Int(4)
            ),
            Err(MediaError::Unsupported(_))
        ));
        assert!(matches!(
            controls.set(
                target,
                CameraControlId(SPA_PROP_deviceFd),
                CameraValue::Int(3)
            ),
            Err(MediaError::Unsupported(_))
        ));
        let mutation = transport::control::Mutation::Parameter {
            video_source: false,
            target,
            id: SPA_PARAM_Props,
            bytes: Vec::new(),
            expected: vec![],
        };
        assert!(matches!(
            handle.mutate(mutation),
            Err(MediaError::PermissionDenied)
        ));
        handle.shared.registry.lock().unwrap().remove(21);
        assert!(matches!(
            controls.controls(target),
            Err(MediaError::StaleHandle)
        ));
    }
    #[test]
    fn v4l2_menu_labels_constrain_scalar_controls() {
        let default = info(SPA_PROP_START_CUSTOM + 1, Value::Int(1), false);
        let ParameterValue::Object(mut properties) = default else {
            panic!()
        };
        properties.insert(
            SPA_PROP_INFO_labels,
            ParameterValue::Struct(vec![
                ParameterValue::Int(1),
                ParameterValue::String("Auto".into()),
                ParameterValue::Int(3),
                ParameterValue::String("Manual".into()),
            ]),
        );
        let object = RegistryObject {
            handle: ObjectHandle {
                epoch: 1,
                incarnation: 1,
                id: 1,
            },
            kind: ObjectKind::Node,
            permissions: u32::MAX,
            properties: BTreeMap::new(),
            parameters: BTreeMap::from([(
                (SPA_PARAM_PropInfo, 0),
                ParameterValue::Object(properties),
            )]),
            readable_parameters: vec![],
            writable_parameters: vec![SPA_PARAM_Props],
            metadata: BTreeMap::new(),
        };
        let controls = controls(&object);
        assert_eq!(controls[0].labels.len(), 2);
        assert!(controls[0].domain.accepts(CameraValue::Int(3)));
        assert!(!controls[0].domain.accepts(CameraValue::Int(2)));
    }
}
