//! Screen-cast portal authoring context. Decisions are intentions, never grants.

mod virtual_display;
pub use virtual_display::{VirtualDisplayConfig, VirtualDisplaySnapshot};

use crate::authoring::compose::*;
use crate::shell::capture::CaptureSource;
use std::sync::{Arc, mpsc::SyncSender};

/// Read-only presentation values. IDs and source epochs must be returned unchanged when deciding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenCastPortalSnapshot {
    /// Current request ID and display label of the requesting application.
    pub pending: Option<(u64, String)>,
    /// Whether the pending request permits selecting multiple sources (at most eight).
    pub multiple: bool,
    /// True only when a configured client audio adapter has negotiated delivery.
    pub audio_available: bool,
    /// Source kinds requested by the application, even when a kind has no available sources.
    pub source_types: u32,
    /// Confirmed host-owned virtual displays; creation does not grant capture access.
    pub virtual_displays: Vec<VirtualDisplaySnapshot>,
    /// Whether the pending request accepts virtual displays.
    pub virtual_controls: bool,
    /// Mapped windows available for explicit virtual-display placement (not capture grants).
    pub virtual_windows: Vec<(crate::shell::WindowId, String)>,
    /// The application requested restorable permission; remembering still requires explicit consent.
    pub can_remember: bool,
    /// Remembering is limited to this application connection, rather than durable storage.
    pub remember_for_application: bool,
    /// Last successful explicit saved-permission refresh; contains no restoration tokens.
    pub saved_permissions: Vec<SavedCapturePermission>,
    /// A settings refresh or forget operation is awaiting worker completion.
    pub permissions_pending: bool,
    /// Whether the latest inventory operation completed successfully.
    pub permissions_loaded: bool,
    /// Failure of the latest saved-permission settings operation, separate from capture errors.
    pub permission_error: Option<String>,
    /// Active sharing IDs and display labels; pass an ID to `ScreenCastPortalContext::stop`.
    pub sharing: Vec<(u64, String)>,
    /// Sharing IDs still waiting for their PipeWire node to become ready.
    pub starting: Vec<u64>,
    /// Active sharing IDs with a saved grant, eligible for `stop_and_forget`.
    pub remembered: Vec<u64>,
    /// Eligible source identity, mapping epoch and display label for the pending request.
    pub sources: Vec<(CaptureSource, u64, String)>,
    /// Host-rendered thumbnails for this request only; never delivered to the requesting app.
    pub previews: Vec<PortalSourcePreview>,
    /// Last sharing failure, available to the shell until explicitly dismissed.
    pub failure: Option<String>,
}

/// A bounded settings summary. Use the exact app_id for forgetting saved permissions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedCapturePermission {
    pub app_id: String,
    pub grants: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CaptureDecision {
    Approve(u64, CaptureSource, u64),
    ApproveMany(u64, Vec<(CaptureSource, u64)>),
    ApproveWithAudio(u64, Vec<(CaptureSource, u64)>),
    ApproveRemembered(u64, Vec<(CaptureSource, u64)>),
    ApproveRestored(u64, Vec<(CaptureSource, u64)>),
    Deny(u64),
    Stop(u64),
    StopAndForget(u64),
    DismissFailure,
    RefreshPermissions,
    ForgetApplication(String),
    CreateVirtualDisplay(VirtualDisplayConfig),
    RemoveVirtualDisplay(crate::shell::OutputId),
    RouteVirtualDisplay(
        crate::shell::OutputId,
        Vec<(crate::shell::WindowId, crate::foundation::RectI)>,
    ),
}

/// Shared portal state and actions for shell-authored UI. Default handles are inert.
/// Obtain a live handle through a `ScreenCastPortal` component factory.
#[derive(Clone)]
pub struct ScreenCastPortalContext {
    pub(crate) snapshot: Signal<ScreenCastPortalSnapshot>,
    pub(crate) decisions: SyncSender<CaptureDecision>,
    pub(crate) wake: Arc<dyn Fn() + Send + Sync>,
}
impl Default for ScreenCastPortalContext {
    fn default() -> Self {
        let (snapshot, _) = Signal::new(ScreenCastPortalSnapshot::default());
        let (decisions, _) = std::sync::mpsc::sync_channel(1);
        Self {
            snapshot,
            decisions,
            wake: Arc::new(|| {}),
        }
    }
}
impl PartialEq for ScreenCastPortalContext {
    fn eq(&self, other: &Self) -> bool {
        self.snapshot == other.snapshot
    }
}
impl ScreenCastPortalContext {
    /// Watch this signal from a shell component to render pending consent and active shares.
    pub fn snapshot(&self) -> &Signal<ScreenCastPortalSnapshot> {
        &self.snapshot
    }

    /// Submit the exact request, source and mapping epoch from a snapshot. `true` means queued,
    /// not authorized: the host revalidates identity, source availability and requested types.
    pub fn approve(&self, request: u64, source: CaptureSource, epoch: u64) -> bool {
        self.send(CaptureDecision::Approve(request, source, epoch))
    }
    /// Approve one to eight exact snapshot source/epoch pairs as one sharing group.
    /// Returns queue acceptance only. The host rejects stale, duplicate, disallowed sources
    /// or multiple sources when the application requested only one. Stopping the group
    /// revokes every member; readiness is reported only after every stream is ready.
    pub fn approve_many(&self, request: u64, sources: &[(CaptureSource, u64)]) -> bool {
        if sources.is_empty() || sources.len() > 8 {
            return false;
        }
        self.send(CaptureDecision::ApproveMany(request, sources.to_vec()))
    }
    pub fn approve_with_audio(&self, request: u64, sources: &[(CaptureSource, u64)]) -> bool {
        let snapshot = self.snapshot.snapshot();
        if !snapshot.audio_available || snapshot.pending.as_ref().map(|r| r.0) != Some(request)
            || sources.is_empty() || sources.len() > 8 { return false; }
        self.send(CaptureDecision::ApproveWithAudio(request, sources.to_vec()))
    }
    /// Explicitly authorize remembering these exact sources for the requested lifetime.
    /// Inspect `remember_for_application`: temporary consent ends with the application connection.
    /// Queue acceptance is not durable completion; the host and backend revalidate the request.
    pub fn approve_and_remember(&self, request: u64, sources: &[(CaptureSource, u64)]) -> bool {
        if sources.is_empty() || sources.len() > 8 {
            return false;
        }
        self.send(CaptureDecision::ApproveRemembered(
            request,
            sources.to_vec(),
        ))
    }
    /// Reject a pending request from the snapshot.
    pub fn deny(&self, request: u64) -> bool {
        self.send(CaptureDecision::Deny(request))
    }
    /// Stop a sharing entry using its snapshot ID.
    pub fn stop(&self, sharing_id: u64) -> bool {
        self.send(CaptureDecision::Stop(sharing_id))
    }

    /// Stop the active sharing group and request durable removal of its saved permission.
    /// Queue acceptance is not disk completion; failures appear in the snapshot notification.
    pub fn stop_and_forget(&self, sharing_id: u64) -> bool {
        self.send(CaptureDecision::StopAndForget(sharing_id))
    }

    /// Refresh the saved-permission inventory for a shell settings view. Results arrive in
    /// the snapshot; queue acceptance is distinct from worker completion.
    pub fn refresh_saved_permissions(&self) -> bool {
        self.send(CaptureDecision::RefreshPermissions)
    }
    /// Stop this application's active/pending sharing and durably forget its saved grants.
    /// Other applications are unaffected. Completion refreshes the inventory; errors appear
    /// in snapshot.permission_error. Only one settings operation is admitted at a time by the host.
    pub fn forget_saved_permissions(&self, app_id: &str) -> bool {
        if app_id.is_empty() || app_id.len() > 512 || app_id.chars().any(char::is_control) {
            return false;
        }
        self.send(CaptureDecision::ForgetApplication(app_id.into()))
    }

    /// Dismiss the compositor's last sharing error notification.
    pub fn dismiss_failure(&self) -> bool {
        self.send(CaptureDecision::DismissFailure)
    }

    fn send(&self, decision: CaptureDecision) -> bool {
        if self.decisions.try_send(decision).is_err() {
            return false;
        }
        (self.wake)();
        true
    }
}

#[cfg(test)]
mod tests;

/// App identifiers are untrusted presentation text, even when supplied by the portal frontend.
pub(crate) fn application_label(app_id: &str) -> String {
    let label: String = app_id
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(80)
        .collect();
    if label.trim().is_empty() {
        "An application".into()
    } else {
        label
    }
}

/// A bounded RGBA thumbnail of an eligible source. Epochs prevent window-ID reuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortalSourcePreview {
    pub source: CaptureSource,
    pub epoch: u64,
    pub revision: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}
