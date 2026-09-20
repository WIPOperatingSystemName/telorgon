//! Default capture consent and stop controls. Decisions are intentions, never grants.

use crate::compose::*;
use crate::shell::capture::CaptureSource;
use std::sync::{Arc, mpsc::SyncSender};

/// Read-only presentation values. IDs and source epochs must be returned unchanged when deciding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureUiSnapshot {
    /// Current request ID and display label of the requesting application.
    pub pending: Option<(u64, String)>,
    /// Active sharing IDs and display labels; pass an ID to `CaptureUi::stop`.
    pub sharing: Vec<(u64, String)>,
    /// Sharing IDs still waiting for their PipeWire node to become ready.
    pub starting: Vec<u64>,
    /// Eligible source identity, mapping epoch and display label for the pending request.
    pub sources: Vec<(CaptureSource, u64, String)>,
    /// Last sharing failure. The standard indicator displays it until dismissed.
    pub failure: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureDecision {
    Approve(u64, CaptureSource, u64),
    Deny(u64),
    Stop(u64),
    DismissFailure,
}

/// A host-issued consent handle for a custom chooser. Default handles are inert.
/// Obtain a live handle through `Compositor::capture_chooser`.
#[derive(Clone)]
pub struct CaptureUi {
    pub(crate) snapshot: Signal<CaptureUiSnapshot>,
    pub(crate) decisions: SyncSender<CaptureDecision>,
    pub(crate) wake: Arc<dyn Fn() + Send + Sync>,
}
impl Default for CaptureUi {
    fn default() -> Self {
        let (snapshot, _) = Signal::new(CaptureUiSnapshot::default());
        let (decisions, _) = std::sync::mpsc::sync_channel(1);
        Self {
            snapshot,
            decisions,
            wake: Arc::new(|| {}),
        }
    }
}
impl PartialEq for CaptureUi {
    fn eq(&self, other: &Self) -> bool {
        self.snapshot == other.snapshot
    }
}
impl CaptureUi {
    /// Watch this signal from a shell component to render pending consent and active shares.
    pub fn snapshot(&self) -> &Signal<CaptureUiSnapshot> {
        &self.snapshot
    }

    /// Submit the exact request, source and mapping epoch from a snapshot. `true` means queued,
    /// not authorized: the host revalidates identity, source availability and requested types.
    pub fn approve(&self, request: u64, source: CaptureSource, epoch: u64) -> bool {
        self.send(CaptureDecision::Approve(request, source, epoch))
    }
    /// Reject a pending request from the snapshot.
    pub fn deny(&self, request: u64) -> bool {
        self.send(CaptureDecision::Deny(request))
    }
    /// Stop a sharing entry using its snapshot ID.
    pub fn stop(&self, sharing_id: u64) -> bool {
        self.send(CaptureDecision::Stop(sharing_id))
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
    fn decide(&self, decision: CaptureDecision) {
        let _ = self.send(decision);
    }
}

pub use chooser::CapturePicker;
pub(crate) use indicator::CaptureIndicator;
mod chooser;
mod indicator;
mod layout;
mod selection;
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
