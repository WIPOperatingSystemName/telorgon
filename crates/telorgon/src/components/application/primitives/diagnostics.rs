//! Bounded, content-free diagnostics for application primitive boundaries.

use crate::components::application::primitives::{
    EnvironmentError, HudLayerError, RenderTargetViewError, VideoSurfaceError,
    ViewportOverlayPlacementError, WorldAnchorProjectionError,
};

/// Stable failure categories without host tokens, descriptions, coordinates, or media metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ApplicationPrimitiveDiagnosticKind {
    InvalidEnvironment,
    InvalidHudInput,
    InvalidViewportPlacement,
    InvalidWorldProjection,
    InvalidRenderTargetContent,
    InvalidVideoSurfaceContent,
    ProtectedVideoUnavailable,
    StaleHostContent,
}

impl ApplicationPrimitiveDiagnosticKind {
    pub const ALL: [Self; 8] = [
        Self::InvalidEnvironment,
        Self::InvalidHudInput,
        Self::InvalidViewportPlacement,
        Self::InvalidWorldProjection,
        Self::InvalidRenderTargetContent,
        Self::InvalidVideoSurfaceContent,
        Self::ProtectedVideoUnavailable,
        Self::StaleHostContent,
    ];

    const fn index(self) -> usize {
        self as usize
    }
}

/// Immutable counter snapshot suitable for redaction-safe inspection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplicationPrimitiveDiagnostics {
    counts: [u64; ApplicationPrimitiveDiagnosticKind::ALL.len()],
    total: u64,
}

impl ApplicationPrimitiveDiagnostics {
    pub const fn total(self) -> u64 {
        self.total
    }

    pub const fn is_empty(self) -> bool {
        self.total == 0
    }

    pub const fn count(self, kind: ApplicationPrimitiveDiagnosticKind) -> u64 {
        self.counts[kind.index()]
    }

    pub fn iter(self) -> impl ExactSizeIterator<Item = (ApplicationPrimitiveDiagnosticKind, u64)> {
        ApplicationPrimitiveDiagnosticKind::ALL
            .into_iter()
            .map(move |kind| (kind, self.count(kind)))
    }
}

/// Caller-owned bounded counter collector. No event payload or host content is retained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplicationPrimitiveDiagnosticCollector {
    diagnostics: ApplicationPrimitiveDiagnostics,
}

impl ApplicationPrimitiveDiagnosticCollector {
    pub const fn diagnostics(self) -> ApplicationPrimitiveDiagnostics {
        self.diagnostics
    }

    pub fn record(&mut self, kind: ApplicationPrimitiveDiagnosticKind) {
        let count = &mut self.diagnostics.counts[kind.index()];
        *count = count.saturating_add(1);
        self.diagnostics.total = self.diagnostics.total.saturating_add(1);
    }

    pub fn record_error(&mut self, error: impl Into<ApplicationPrimitiveDiagnosticKind>) {
        self.record(error.into());
    }

    pub fn clear(&mut self) -> ApplicationPrimitiveDiagnostics {
        let previous = self.diagnostics;
        self.diagnostics = ApplicationPrimitiveDiagnostics::default();
        previous
    }
}

impl From<EnvironmentError> for ApplicationPrimitiveDiagnosticKind {
    fn from(_: EnvironmentError) -> Self {
        Self::InvalidEnvironment
    }
}

impl From<HudLayerError> for ApplicationPrimitiveDiagnosticKind {
    fn from(_: HudLayerError) -> Self {
        Self::InvalidHudInput
    }
}

impl From<ViewportOverlayPlacementError> for ApplicationPrimitiveDiagnosticKind {
    fn from(_: ViewportOverlayPlacementError) -> Self {
        Self::InvalidViewportPlacement
    }
}

impl From<WorldAnchorProjectionError> for ApplicationPrimitiveDiagnosticKind {
    fn from(_: WorldAnchorProjectionError) -> Self {
        Self::InvalidWorldProjection
    }
}

impl From<RenderTargetViewError> for ApplicationPrimitiveDiagnosticKind {
    fn from(_: RenderTargetViewError) -> Self {
        Self::InvalidRenderTargetContent
    }
}

impl From<VideoSurfaceError> for ApplicationPrimitiveDiagnosticKind {
    fn from(_: VideoSurfaceError) -> Self {
        Self::InvalidVideoSurfaceContent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_is_bounded_typed_clearable_and_content_free() {
        let mut collector = ApplicationPrimitiveDiagnosticCollector::default();
        collector.record_error(HudLayerError::NonFinitePoint);
        collector.record_error(RenderTargetViewError::ZeroContentVersion);
        collector.record(ApplicationPrimitiveDiagnosticKind::StaleHostContent);
        collector.record(ApplicationPrimitiveDiagnosticKind::StaleHostContent);
        let diagnostics = collector.diagnostics();
        assert_eq!(diagnostics.total(), 4);
        assert_eq!(
            diagnostics.count(ApplicationPrimitiveDiagnosticKind::InvalidHudInput),
            1
        );
        assert_eq!(
            diagnostics.count(ApplicationPrimitiveDiagnosticKind::StaleHostContent),
            2
        );
        assert_eq!(diagnostics.iter().len(), 8);
        assert_eq!(collector.clear(), diagnostics);
        assert!(collector.diagnostics().is_empty());
    }

    #[test]
    fn counters_saturate_instead_of_wrapping_or_growing_storage() {
        let mut collector = ApplicationPrimitiveDiagnosticCollector {
            diagnostics: ApplicationPrimitiveDiagnostics {
                counts: [u64::MAX; ApplicationPrimitiveDiagnosticKind::ALL.len()],
                total: u64::MAX,
            },
        };
        collector.record(ApplicationPrimitiveDiagnosticKind::InvalidEnvironment);
        assert_eq!(collector.diagnostics().total(), u64::MAX);
        assert_eq!(
            collector
                .diagnostics()
                .count(ApplicationPrimitiveDiagnosticKind::InvalidEnvironment),
            u64::MAX
        );
    }
}
