//! Fixed, saturating, payload-free diagnostics for shell component boundaries.

use crate::components::shell::{
    ApplicationActionSourceError, LauncherError, LockCompositionError,
    NotificationActionSourceError, NotificationHostError, PanelError, StatusActionSourceError,
    StatusAreaError, SystemModalHostError, WindowFrameError, WorkspaceViewError,
};

/// Stable categories that never retain labels, notification content, IDs, grants, or host data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ShellComponentDiagnosticKind {
    InvalidChrome,
    InvalidWorkspace,
    InvalidPanel,
    InvalidLauncher,
    InvalidStatus,
    InvalidNotification,
    InvalidSecureComposition,
    InvalidActionSource,
    UnauthorizedAction,
    LifecycleSuppressed,
    PrivacyRedacted,
    StaleMount,
}

impl ShellComponentDiagnosticKind {
    pub const ALL: [Self; 12] = [
        Self::InvalidChrome,
        Self::InvalidWorkspace,
        Self::InvalidPanel,
        Self::InvalidLauncher,
        Self::InvalidStatus,
        Self::InvalidNotification,
        Self::InvalidSecureComposition,
        Self::InvalidActionSource,
        Self::UnauthorizedAction,
        Self::LifecycleSuppressed,
        Self::PrivacyRedacted,
        Self::StaleMount,
    ];

    const fn index(self) -> usize {
        self as usize
    }
}

/// Immutable counter snapshot with fixed storage and no component or host payloads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellComponentDiagnostics {
    counts: [u64; ShellComponentDiagnosticKind::ALL.len()],
    total: u64,
}

impl ShellComponentDiagnostics {
    pub const fn total(self) -> u64 {
        self.total
    }

    pub const fn is_empty(self) -> bool {
        self.total == 0
    }

    pub const fn count(self, kind: ShellComponentDiagnosticKind) -> u64 {
        self.counts[kind.index()]
    }

    pub fn iter(self) -> impl ExactSizeIterator<Item = (ShellComponentDiagnosticKind, u64)> {
        ShellComponentDiagnosticKind::ALL
            .into_iter()
            .map(move |kind| (kind, self.count(kind)))
    }
}

/// Caller-owned collector. Recording never logs, invokes a host, or retains an error payload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellComponentDiagnosticCollector {
    diagnostics: ShellComponentDiagnostics,
}

impl ShellComponentDiagnosticCollector {
    pub const fn diagnostics(self) -> ShellComponentDiagnostics {
        self.diagnostics
    }

    pub fn record(&mut self, kind: ShellComponentDiagnosticKind) {
        let count = &mut self.diagnostics.counts[kind.index()];
        *count = count.saturating_add(1);
        self.diagnostics.total = self.diagnostics.total.saturating_add(1);
    }

    pub fn record_error(&mut self, error: impl Into<ShellComponentDiagnosticKind>) {
        self.record(error.into());
    }

    pub fn clear(&mut self) -> ShellComponentDiagnostics {
        let previous = self.diagnostics;
        self.diagnostics = ShellComponentDiagnostics::default();
        previous
    }
}

macro_rules! map_error {
    ($error:ty, $kind:ident) => {
        impl From<$error> for ShellComponentDiagnosticKind {
            fn from(_: $error) -> Self {
                Self::$kind
            }
        }
    };
}

map_error!(WindowFrameError, InvalidChrome);
map_error!(WorkspaceViewError, InvalidWorkspace);
map_error!(PanelError, InvalidPanel);
map_error!(LauncherError, InvalidLauncher);
map_error!(StatusAreaError, InvalidStatus);
map_error!(NotificationHostError, InvalidNotification);
map_error!(LockCompositionError, InvalidSecureComposition);
map_error!(SystemModalHostError, InvalidSecureComposition);
map_error!(ApplicationActionSourceError, InvalidActionSource);
map_error!(StatusActionSourceError, InvalidActionSource);
map_error!(NotificationActionSourceError, InvalidActionSource);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_is_fixed_typed_saturating_clearable_and_payload_free() {
        let mut collector = ShellComponentDiagnosticCollector::default();
        collector.record_error(NotificationHostError::MissingAccessibleName);
        collector.record_error(LockCompositionError::RequiresLockLayer);
        collector.record(ShellComponentDiagnosticKind::PrivacyRedacted);
        collector.record(ShellComponentDiagnosticKind::PrivacyRedacted);
        let diagnostics = collector.diagnostics();
        assert_eq!(diagnostics.total(), 4);
        assert_eq!(
            diagnostics.count(ShellComponentDiagnosticKind::InvalidNotification),
            1
        );
        assert_eq!(
            diagnostics.count(ShellComponentDiagnosticKind::PrivacyRedacted),
            2
        );
        assert_eq!(diagnostics.iter().len(), 12);
        assert_eq!(collector.clear(), diagnostics);
        assert!(collector.diagnostics().is_empty());
    }

    #[test]
    fn counters_saturate_without_growing_storage() {
        let mut collector = ShellComponentDiagnosticCollector {
            diagnostics: ShellComponentDiagnostics {
                counts: [u64::MAX; ShellComponentDiagnosticKind::ALL.len()],
                total: u64::MAX,
            },
        };
        collector.record(ShellComponentDiagnosticKind::StaleMount);
        assert_eq!(collector.diagnostics().total(), u64::MAX);
        assert_eq!(
            collector
                .diagnostics()
                .count(ShellComponentDiagnosticKind::StaleMount),
            u64::MAX
        );
    }
}
