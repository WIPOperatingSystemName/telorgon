use super::portal_wire as wire;
use crate::Signal;
use crate::authoring::compose::portal::{CaptureDecision, ScreenCastPortalContext};
use serde_json::json;
use std::{
    io,
    sync::{Arc, mpsc::sync_channel},
};

impl ScreenCastPortalContext {
    /// Connects a shell-authored picker helper to its parent's private stdin/stdout transport.
    /// Call only in the helper process launched by `PortalPickerWindow`. Stdout is reserved for
    /// consent replies. The context provides pending sources and consent, not sharing/settings control.
    pub fn from_picker_stdio() -> io::Result<Self> {
        if std::env::var_os("TELORGON_PORTAL_PICKER").is_none() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "not a portal picker helper",
            ));
        }
        if let Some(version) = std::env::var_os("TELORGON_PORTAL_PICKER_PROTOCOL") {
            if version != "1" { return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported picker protocol version")); }
            wire::validate_hello(wire::read(&mut io::stdin().lock())?)?;
            wire::write(&mut io::stdout().lock(), &wire::hello())?;
        }
        let initial = wire::parse_snapshot(wire::read(&mut io::stdin().lock())?)?;
        let (snapshot, writer) = Signal::new(initial.clone());
        let (decisions, replies) = sync_channel(16);
        std::thread::Builder::new()
            .name("portal-picker-state".into())
            .spawn(move || {
                let mut input = io::stdin().lock();
                let mut current = initial;
                while let Ok(value) = wire::read(&mut input).and_then(|v|wire::update(v, &current)) {
                    current = value;
                    writer.publish_if_changed(current.clone());
                }
                crate::request_exit();
            })?;
        std::thread::Builder::new().name("portal-picker-reply".into()).spawn(move || {
            while let Ok(decision) = replies.recv() {
                let value = match decision {
                    CaptureDecision::Approve(id, source, epoch) => json!({"request":id,"action":"share","sources":[[wire::source(source),epoch]]}),
                    CaptureDecision::ApproveMany(id, sources) => {
                        json!({"request":id,"action":"share","sources":sources.iter().map(|(s,e)|json!([wire::source(*s),e])).collect::<Vec<_>>()})
                    }
                    CaptureDecision::ApproveWithAudio(id, sources) => json!({"request":id,"action":"share","audio":true,"sources":sources.iter().map(|(s,e)|json!([wire::source(*s),e])).collect::<Vec<_>>()}),
                    CaptureDecision::ApproveRemembered(id, sources) => json!({"request":id,"action":"remember","sources":sources.iter().map(|(s,e)|json!([wire::source(*s),e])).collect::<Vec<_>>()}),
                    CaptureDecision::Deny(id) => json!({"request":id,"action":"cancel"}),
                    _ => continue,
                };
                let _ = wire::write(&mut io::stdout().lock(), &value);
                crate::request_exit();
                break;
            }
        })?;
        Ok(Self {
            snapshot,
            decisions,
            wake: Arc::new(|| {}),
        })
    }
}
