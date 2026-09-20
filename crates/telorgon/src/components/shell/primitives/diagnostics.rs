//! Fixed, saturating, payload-free diagnostics for shell primitive boundaries.

use crate::components::shell::primitives::{
    ClientSurfacePrimitiveError, DragRegionError, ExclusiveRegionError, OutputEdgeRegionError,
    OutputViewMappingError, ReservedAreaError, ResizeRegionError, ShellLayerError, ShellRootError,
    SurfaceInputRegionError, SurfacePlaceholderError, SurfaceSnapshotAuthorizationError,
    SurfaceSnapshotError, SurfaceTreeError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ShellPrimitiveDiagnosticKind {
    InvalidRoot,
    InvalidOutputMapping,
    InvalidLayer,
    InvalidClientSurface,
    InvalidSurfaceTree,
    InvalidPlaceholder,
    InvalidSnapshotAuthorization,
    InvalidSnapshot,
    InvalidReservation,
    InvalidExclusiveRegion,
    InvalidSurfaceInputMapping,
    InvalidDragRegion,
    InvalidResizeRegion,
    InvalidOutputEdge,
    StaleMount,
}

impl ShellPrimitiveDiagnosticKind {
    pub const ALL: [Self; 15] = [
        Self::InvalidRoot,
        Self::InvalidOutputMapping,
        Self::InvalidLayer,
        Self::InvalidClientSurface,
        Self::InvalidSurfaceTree,
        Self::InvalidPlaceholder,
        Self::InvalidSnapshotAuthorization,
        Self::InvalidSnapshot,
        Self::InvalidReservation,
        Self::InvalidExclusiveRegion,
        Self::InvalidSurfaceInputMapping,
        Self::InvalidDragRegion,
        Self::InvalidResizeRegion,
        Self::InvalidOutputEdge,
        Self::StaleMount,
    ];

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellPrimitiveDiagnostics {
    counts: [u64; ShellPrimitiveDiagnosticKind::ALL.len()],
    total: u64,
}

impl ShellPrimitiveDiagnostics {
    pub const fn total(self) -> u64 {
        self.total
    }

    pub const fn is_empty(self) -> bool {
        self.total == 0
    }

    pub const fn count(self, kind: ShellPrimitiveDiagnosticKind) -> u64 {
        self.counts[kind.index()]
    }

    pub fn iter(self) -> impl ExactSizeIterator<Item = (ShellPrimitiveDiagnosticKind, u64)> {
        ShellPrimitiveDiagnosticKind::ALL
            .into_iter()
            .map(move |kind| (kind, self.count(kind)))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellPrimitiveDiagnosticCollector {
    diagnostics: ShellPrimitiveDiagnostics,
}

impl ShellPrimitiveDiagnosticCollector {
    pub const fn diagnostics(self) -> ShellPrimitiveDiagnostics {
        self.diagnostics
    }

    pub fn record(&mut self, kind: ShellPrimitiveDiagnosticKind) {
        let count = &mut self.diagnostics.counts[kind.index()];
        *count = count.saturating_add(1);
        self.diagnostics.total = self.diagnostics.total.saturating_add(1);
    }

    pub fn record_error(&mut self, error: impl Into<ShellPrimitiveDiagnosticKind>) {
        self.record(error.into());
    }

    pub fn clear(&mut self) -> ShellPrimitiveDiagnostics {
        let previous = self.diagnostics;
        self.diagnostics = ShellPrimitiveDiagnostics::default();
        previous
    }
}

macro_rules! map_error {
    ($error:ty, $kind:ident) => {
        impl From<$error> for ShellPrimitiveDiagnosticKind {
            fn from(_: $error) -> Self {
                Self::$kind
            }
        }
    };
}

map_error!(ShellRootError, InvalidRoot);
map_error!(OutputViewMappingError, InvalidOutputMapping);
map_error!(ShellLayerError, InvalidLayer);
map_error!(ClientSurfacePrimitiveError, InvalidClientSurface);
map_error!(SurfaceTreeError, InvalidSurfaceTree);
map_error!(SurfacePlaceholderError, InvalidPlaceholder);
map_error!(
    SurfaceSnapshotAuthorizationError,
    InvalidSnapshotAuthorization
);
map_error!(SurfaceSnapshotError, InvalidSnapshot);
map_error!(ReservedAreaError, InvalidReservation);
map_error!(ExclusiveRegionError, InvalidExclusiveRegion);
map_error!(SurfaceInputRegionError, InvalidSurfaceInputMapping);
map_error!(DragRegionError, InvalidDragRegion);
map_error!(ResizeRegionError, InvalidResizeRegion);
map_error!(OutputEdgeRegionError, InvalidOutputEdge);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_is_fixed_typed_saturating_and_clearable() {
        let mut collector = ShellPrimitiveDiagnosticCollector::default();
        collector.record_error(DragRegionError::NonFinitePoint);
        collector.record_error(OutputEdgeRegionError::InvalidThickness);
        collector.record(ShellPrimitiveDiagnosticKind::StaleMount);
        collector.record(ShellPrimitiveDiagnosticKind::StaleMount);
        let diagnostics = collector.diagnostics();
        assert_eq!(diagnostics.total(), 4);
        assert_eq!(
            diagnostics.count(ShellPrimitiveDiagnosticKind::InvalidDragRegion),
            1
        );
        assert_eq!(
            diagnostics.count(ShellPrimitiveDiagnosticKind::StaleMount),
            2
        );
        assert_eq!(diagnostics.iter().len(), 15);
        assert_eq!(collector.clear(), diagnostics);
        assert!(collector.diagnostics().is_empty());
    }
}
