//! Bounded private transport between a portal backend and its shell-authored picker process.
use crate::{
    ScreenCastPortalSnapshot,
    shell::{OutputId, WindowId, capture::CaptureSource},
};
use serde_json::{Value, json};
use std::io::{self, BufRead, Read, Write};
use std::num::NonZeroU32;

const LIMIT: u64 = 1024 * 1024;
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid portal picker message")
}
pub(crate) fn read(reader: &mut impl BufRead) -> io::Result<Value> {
    let mut bytes = Vec::new();
    reader.take(LIMIT + 1).read_until(b'\n', &mut bytes)?;
    if bytes.len() as u64 > LIMIT || bytes.last() != Some(&b'\n') {
        return Err(invalid());
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}
pub(crate) fn write(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() as u64 > LIMIT {
        return Err(invalid());
    }
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()
}
pub(crate) fn source(value: CaptureSource) -> Value {
    match value {
        CaptureSource::Output(id) => json!([1, id.get(), 0]),
        CaptureSource::Window(id) => json!([2, id.slot(), id.generation()]),
        CaptureSource::VirtualOutput(id) => json!([4, id.get(), 0]),
    }
}
pub(crate) fn parse_source(value: &Value) -> io::Result<CaptureSource> {
    let id = value[1].as_u64().ok_or_else(invalid)?;
    Ok(match value[0].as_u64() {
        Some(1) => CaptureSource::Output(OutputId::from_raw(id).ok_or_else(invalid)?),
        Some(4) => CaptureSource::VirtualOutput(OutputId::from_raw(id).ok_or_else(invalid)?),
        Some(2) => CaptureSource::Window(WindowId::new(
            NonZeroU32::new(u32::try_from(id).map_err(|_| invalid())?).ok_or_else(invalid)?,
            NonZeroU32::new(
                u32::try_from(value[2].as_u64().ok_or_else(invalid)?).map_err(|_| invalid())?,
            )
            .ok_or_else(invalid)?,
        )),
        _ => return Err(invalid()),
    })
}
pub(crate) fn snapshot(value: &ScreenCastPortalSnapshot) -> Value {
    json!({"audio":value.audio_available,"request": value.pending, "types": value.source_types, "multiple": value.multiple,
        "remember": value.can_remember, "temporary": value.remember_for_application,
        "sources": value.sources.iter().map(|(id, epoch, label)| json!([source(*id), epoch, label])).collect::<Vec<_>>()})
}
pub(crate) fn parse_snapshot(value: Value) -> io::Result<ScreenCastPortalSnapshot> {
    let request = &value["request"];
    let pending = if request.is_null() {
        None
    } else {
        Some((
            request[0].as_u64().ok_or_else(invalid)?,
            request[1].as_str().ok_or_else(invalid)?.to_owned(),
        ))
    };
    let sources = value["sources"].as_array().ok_or_else(invalid)?;
    if sources.len() > 1024 {
        return Err(invalid());
    }
    Ok(ScreenCastPortalSnapshot {
        pending,
        audio_available: value["audio"].as_bool().unwrap_or(false),
        source_types: value["types"]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(invalid)?,
        multiple: value["multiple"].as_bool().ok_or_else(invalid)?,
        can_remember: value["remember"].as_bool().ok_or_else(invalid)?,
        remember_for_application: value["temporary"].as_bool().ok_or_else(invalid)?,
        sources: sources
            .iter()
            .map(|entry| {
                Ok((
                    parse_source(&entry[0])?,
                    entry[1].as_u64().ok_or_else(invalid)?,
                    entry[2].as_str().ok_or_else(invalid)?.to_owned(),
                ))
            })
            .collect::<io::Result<_>>()?,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_preserves_source_epochs_request_types_and_consent_options() {
        let initial = ScreenCastPortalSnapshot {
            pending: Some((7, "Recorder".into())),
            source_types: 3,
            multiple: true,
            can_remember: true,
            remember_for_application: true,
            sources: vec![(
                CaptureSource::Window(WindowId::new(
                    NonZeroU32::new(2).unwrap(),
                    NonZeroU32::new(9).unwrap(),
                )),
                42,
                "Editor".into(),
            )],
            ..Default::default()
        };
        assert_eq!(parse_snapshot(snapshot(&initial)).unwrap(), initial);
        assert!(parse_source(&json!([2, 1, 0])).is_err());
        assert!(read(&mut std::io::Cursor::new(b"{}".as_slice())).is_err());
    }
}

pub(crate) fn preview(request: u64, frame: &crate::PortalSourcePreview) -> Value {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut pixels = String::with_capacity(frame.pixels.len() * 2);
    for byte in frame.pixels.iter() {
        pixels.push(HEX[(byte >> 4) as usize] as char);
        pixels.push(HEX[(byte & 15) as usize] as char);
    }
    json!({"preview":request,"source":source(frame.source),"epoch":frame.epoch,
        "revision":frame.revision,"width":frame.width,"height":frame.height,"pixels":pixels})
}

pub(crate) fn update(
    value: Value,
    previous: &ScreenCastPortalSnapshot,
) -> io::Result<ScreenCastPortalSnapshot> {
    let Some(request) = value.get("preview") else {
        let mut next = parse_snapshot(value)?;
        if next.pending == previous.pending {
            next.previews = previous
                .previews
                .iter()
                .filter(|p| {
                    next.sources
                        .iter()
                        .any(|(s, e, _)| (*s, *e) == (p.source, p.epoch))
                })
                .cloned()
                .collect();
        }
        return Ok(next);
    };
    let request = request.as_u64().ok_or_else(invalid)?;
    let source = parse_source(&value["source"])?;
    let epoch = value["epoch"].as_u64().ok_or_else(invalid)?;
    let width = value["width"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 384)
        .ok_or_else(invalid)? as u32;
    let height = value["height"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 216)
        .ok_or_else(invalid)? as u32;
    let revision = value["revision"].as_u64().ok_or_else(invalid)?;
    let hex = value["pixels"]
        .as_str()
        .filter(|s| s.len() == (width * height * 8) as usize)
        .ok_or_else(invalid)?;
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        _ => Err(invalid()),
    };
    let pixels = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|p| Ok(nibble(p[0])? * 16 + nibble(p[1])?))
        .collect::<io::Result<Vec<u8>>>()?;
    let mut next = previous.clone();
    if previous.pending.as_ref().map(|p| p.0) != Some(request)
        || !previous
            .sources
            .iter()
            .any(|(s, e, _)| (*s, *e) == (source, epoch))
    {
        return Ok(next);
    }
    let frame = crate::PortalSourcePreview {
        source,
        epoch,
        revision,
        width,
        height,
        pixels: pixels.into(),
    };
    if let Some(old) = next.previews.iter_mut().find(|p| p.source == source) {
        if revision > old.revision {
            *old = frame;
        }
    } else {
        next.previews.push(frame);
    }
    Ok(next)
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    fn state() -> ScreenCastPortalSnapshot {
        ScreenCastPortalSnapshot {
            pending: Some((9, "Recorder".into())),
            sources: vec![(CaptureSource::Output(OutputId::MIN), 4, "Monitor".into())],
            ..Default::default()
        }
    }
    fn frame() -> crate::PortalSourcePreview {
        crate::PortalSourcePreview {
            source: CaptureSource::Output(OutputId::MIN),
            epoch: 4,
            revision: 2,
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 255, 0, 255].into(),
        }
    }
    #[test]
    fn thumbnail_transport_preserves_pixels_and_discards_stale_requests_and_epochs() {
        let initial = state();
        let next = update(preview(9, &frame()), &initial).unwrap();
        assert_eq!(next.previews, vec![frame()]);
        assert_eq!(
            update(snapshot(&initial), &next).unwrap().previews,
            next.previews
        );
        assert!(
            update(preview(10, &frame()), &initial)
                .unwrap()
                .previews
                .is_empty()
        );
        let mut replaced = initial.clone();
        replaced.sources[0].1 = 5;
        assert!(
            update(preview(9, &frame()), &replaced)
                .unwrap()
                .previews
                .is_empty()
        );
        assert!(
            update(snapshot(&replaced), &next)
                .unwrap()
                .previews
                .is_empty()
        );
        replaced.pending = None;
        assert!(
            update(snapshot(&replaced), &next)
                .unwrap()
                .previews
                .is_empty()
        );
    }
    #[test]
    fn hidpi_thumbnail_fits_bounded_transport_without_losing_pixels() {
        let mut frame = frame();
        frame.width = 384;
        frame.height = 216;
        frame.pixels = (0..384 * 216 * 4).map(|n| (n % 251) as u8).collect::<Vec<_>>().into();
        let value = preview(9, &frame);
        assert!(serde_json::to_vec(&value).unwrap().len() + 1 < LIMIT as usize);
        assert_eq!(update(value, &state()).unwrap().previews, vec![frame]);
    }
    #[test]
    fn thumbnail_transport_rejects_oversized_or_malformed_pixels() {
        let mut value = preview(9, &frame());
        value["width"] = json!(385);
        assert!(update(value, &state()).is_err());
        let mut value = preview(9, &frame());
        value["pixels"] = json!("00");
        assert!(update(value, &state()).is_err());
        let mut value = preview(9, &frame());
        value["pixels"] = json!("xxxxxxxxxxxxxxxx");
        assert!(update(value, &state()).is_err());
    }
}

pub(crate) fn hello() -> Value { json!({"protocol":"telorgon.portal-picker", "version":1}) }
pub(crate) fn validate_hello(value: Value) -> io::Result<()> {
    if value == hello() { Ok(()) } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, "incompatible Telorgon portal picker protocol"))
    }
}

#[cfg(test)]
mod handshake_tests {
    use super::*;
    #[test]
    fn version_and_protocol_must_match_before_consent() {
        assert!(validate_hello(hello()).is_ok());
        assert!(validate_hello(json!({"protocol":"telorgon.portal-picker","version":2})).is_err());
        assert!(validate_hello(json!({"request":7,"action":"share"})).is_err());
    }
}
