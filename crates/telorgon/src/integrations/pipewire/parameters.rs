//! Bounded, pointer-free SPA parameter snapshots. This advanced representation intentionally
//! excludes native pointers, FDs and arbitrary opaque payloads.
use super::{MediaError, connection::native};
use pipewire::spa::{
    self,
    pod::{Object, Property, Value, ValueArray},
    utils::Id,
};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterChoice {
    Fixed,
    Range,
    Step,
    Enum,
    Flags,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ParameterValue {
    Bool(bool),
    Id(u32),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Rectangle(u32, u32),
    Fraction(u32, u32),
    Choice {
        kind: ParameterChoice,
        values: Vec<ParameterValue>,
    },
    Struct(Vec<ParameterValue>),
    ReadOnly(Box<ParameterValue>),
    String(String),
    Floats(Vec<f32>),
    Ids(Vec<u32>),
    Ints(Vec<i32>),
    Object(BTreeMap<u32, ParameterValue>),
}
impl ParameterValue {
    pub fn property_is_read_only(&self, key: u32) -> bool {
        matches!(self,Self::Object(p) if matches!(p.get(&key),Some(Self::ReadOnly(_))))
    }
    pub fn property(&self, key: u32) -> Option<&Self> {
        if let Self::Object(p) = self {
            p.get(&key).map(|value| match value {
                Self::ReadOnly(value) => value.as_ref(),
                value => value,
            })
        } else {
            None
        }
    }
}
/// Parses only value forms used by controls. All offsets and lengths are checked, with
/// depth/count/byte bounds. Unknown value forms are omitted, never treated as zero.
pub(crate) fn decode(bytes: &[u8]) -> Option<ParameterValue> {
    #[allow(non_upper_case_globals)]
    fn read(bytes: &[u8], depth: usize) -> Option<ParameterValue> {
        use spa::sys::*;
        if depth > 4 || bytes.len() > 16384 {
            return None;
        }
        let size = u32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
        let kind = u32::from_ne_bytes(bytes.get(4..8)?.try_into().ok()?);
        let body = bytes.get(8..8usize.checked_add(size)?)?;
        let u32_at = |offset| {
            Some(u32::from_ne_bytes(
                body.get(offset..offset + 4)?.try_into().ok()?,
            ))
        };
        Some(match kind {
            SPA_TYPE_Bool => ParameterValue::Bool(u32_at(0)? != 0),
            SPA_TYPE_Id => ParameterValue::Id(u32_at(0)?),
            SPA_TYPE_Int => ParameterValue::Int(u32_at(0)? as i32),
            SPA_TYPE_Long => {
                ParameterValue::Long(i64::from_ne_bytes(body.get(..8)?.try_into().ok()?))
            }
            SPA_TYPE_Float => ParameterValue::Float(f32::from_bits(u32_at(0)?)),
            SPA_TYPE_Double => {
                ParameterValue::Double(f64::from_ne_bytes(body.get(..8)?.try_into().ok()?))
            }
            SPA_TYPE_Rectangle => ParameterValue::Rectangle(u32_at(0)?, u32_at(4)?),
            SPA_TYPE_Fraction => ParameterValue::Fraction(u32_at(0)?, u32_at(4)?),
            SPA_TYPE_Choice => {
                let kind = match u32_at(0)? {
                    SPA_CHOICE_None => ParameterChoice::Fixed,
                    SPA_CHOICE_Range => ParameterChoice::Range,
                    SPA_CHOICE_Step => ParameterChoice::Step,
                    SPA_CHOICE_Enum => ParameterChoice::Enum,
                    SPA_CHOICE_Flags => ParameterChoice::Flags,
                    _ => return None,
                };
                let width = u32_at(8)? as usize;
                let child_type = u32_at(12)?;
                if !(1..=8).contains(&width)
                    || body.len() < 16
                    || (body.len() - 16) % width != 0
                    || (body.len() - 16) / width > 64
                    || !matches!(
                        child_type,
                        SPA_TYPE_Bool
                            | SPA_TYPE_Id
                            | SPA_TYPE_Int
                            | SPA_TYPE_Long
                            | SPA_TYPE_Float
                            | SPA_TYPE_Double
                            | SPA_TYPE_Rectangle
                            | SPA_TYPE_Fraction
                    )
                {
                    return None;
                }
                let mut values = Vec::new();
                for value in body[16..].chunks_exact(width) {
                    let mut bytes = Vec::with_capacity(width + 8);
                    bytes.extend_from_slice(&(width as u32).to_ne_bytes());
                    bytes.extend_from_slice(&child_type.to_ne_bytes());
                    bytes.extend_from_slice(value);
                    values.push(read(&bytes, depth + 1)?);
                }
                let valid = match kind {
                    // SPA fixation changes the choice kind in place, retaining the
                    // old alternatives in storage. Only the first value is current.
                    ParameterChoice::Fixed => {
                        values.truncate(1);
                        !values.is_empty()
                    }
                    ParameterChoice::Range => values.len() == 3,
                    ParameterChoice::Step => values.len() == 4,
                    ParameterChoice::Enum | ParameterChoice::Flags => !values.is_empty(),
                };
                if !valid {
                    return None;
                }
                ParameterValue::Choice { kind, values }
            }
            SPA_TYPE_Struct => {
                let mut offset = 0usize;
                let mut values = Vec::new();
                while offset < body.len() {
                    if values.len() >= 64 {
                        return None;
                    }
                    let size = u32_at(offset)? as usize;
                    let end = offset.checked_add(8)?.checked_add(size)?;
                    values.push(read(body.get(offset..end)?, depth + 1)?);
                    offset = offset.checked_add((size + 8).div_ceil(8) * 8)?;
                }
                ParameterValue::Struct(values)
            }
            SPA_TYPE_String => {
                let end = body.iter().position(|b| *b == 0)?;
                if end > 1024 {
                    return None;
                }
                ParameterValue::String(std::str::from_utf8(&body[..end]).ok()?.to_owned())
            }
            SPA_TYPE_Array => {
                if u32_at(0)? != 4
                    || body.len() < 8
                    || (body.len() - 8) % 4 != 0
                    || (body.len() - 8) / 4 > 64
                {
                    return None;
                }
                match u32_at(4)? {
                    SPA_TYPE_Float => ParameterValue::Floats(
                        body[8..]
                            .chunks_exact(4)
                            .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
                            .collect(),
                    ),
                    SPA_TYPE_Int => ParameterValue::Ints(
                        body[8..].chunks_exact(4)
                            .map(|b| i32::from_ne_bytes(b.try_into().unwrap())).collect(),
                    ),
                    SPA_TYPE_Id => ParameterValue::Ids(
                        body[8..]
                            .chunks_exact(4)
                            .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
                            .collect(),
                    ),
                    _ => return None,
                }
            }
            SPA_TYPE_Object => {
                let mut props = BTreeMap::new();
                let mut offset = 8;
                if body.len() < 8 {
                    return None;
                }
                while offset < body.len() {
                    if props.len() >= 64 {
                        return None;
                    }
                    let key = u32_at(offset)?;
                    let child_size = u32_at(offset + 8)? as usize;
                    let length = 8usize.checked_add(child_size)?;
                    let child = body.get(offset + 8..(offset + 8).checked_add(length)?)?;
                    if let Some(value) = read(child, depth + 1) {
                        let value = if u32_at(offset + 4)? & SPA_POD_PROP_FLAG_READONLY != 0 {
                            ParameterValue::ReadOnly(Box::new(value))
                        } else {
                            value
                        };
                        props.insert(key, value);
                    }
                    offset = offset.checked_add(8 + length.div_ceil(8) * 8)?;
                }
                ParameterValue::Object(props)
            }
            _ => return None,
        })
    }
    read(bytes, 0)
}
pub(crate) fn encode(
    type_: u32,
    id: u32,
    properties: Vec<Property>,
) -> Result<Vec<u8>, MediaError> {
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(Object {
            type_,
            id,
            properties,
        }),
    )
    .map(|r| r.0.into_inner())
    .map_err(|e| native(format!("parameter encoding: {e:?}")))
}
pub(crate) fn to_value(value: &ParameterValue) -> Result<Value, MediaError> {
    Ok(match value {
        ParameterValue::Bool(v) => Value::Bool(*v),
        ParameterValue::Id(v) => Value::Id(Id(*v)),
        ParameterValue::Int(v) => Value::Int(*v),
        ParameterValue::Long(v) => Value::Long(*v),
        ParameterValue::Float(v) => Value::Float(*v),
        ParameterValue::Double(v) => Value::Double(*v),
        ParameterValue::Rectangle(w, h) => Value::Rectangle(spa::utils::Rectangle {
            width: *w,
            height: *h,
        }),
        ParameterValue::Fraction(n, d) => {
            Value::Fraction(spa::utils::Fraction { num: *n, denom: *d })
        }
        ParameterValue::String(v) => Value::String(v.clone()),
        ParameterValue::Floats(v) => Value::ValueArray(ValueArray::Float(v.clone())),
        ParameterValue::Ints(v) => Value::ValueArray(ValueArray::Int(v.clone())),
        ParameterValue::Ids(v) => {
            Value::ValueArray(ValueArray::Id(v.iter().copied().map(Id).collect()))
        }
        ParameterValue::Object(_)
        | ParameterValue::Choice { .. }
        | ParameterValue::Struct(_)
        | ParameterValue::ReadOnly(_) => {
            return Err(MediaError::InvalidArgument("nested parameter write"));
        }
    })
}
