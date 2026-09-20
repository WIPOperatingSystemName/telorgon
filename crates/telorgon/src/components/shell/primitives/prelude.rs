//! Curated shell primitive authoring surface.

pub use crate::shell::{
    ClientSurfaceSnapshot, ContactId, InputSource, LayerAuthority, OutputEdge, OutputId,
    OutputRequest, OutputSnapshot, ReservedAreaExtent, ReservedAreaId, ResizeEdge,
    ShellCapabilityGrant, ShellLayerKind, SurfaceId, SurfaceInputContact, SurfaceRegion,
    SurfaceRequest, SurfaceRevision,
};
pub use crate::components::shell::primitives::{
    ClientSurface, ClientSurfaceMountError, ClientSurfacePrimitiveError, ClientSurfaceRef,
    ClientSurfaceStyle, DragRegion, DragRegionError, DragRegionIntent, ExclusiveHitDecision,
    ExclusiveRegion, ExclusiveRegionError, ExclusiveRegionGeometry, ExclusiveRegionMountError,
    ExclusiveRegionRef, ExclusiveRegionStyle, OutputEdgeActivation, OutputEdgeIntent,
    OutputEdgeKind, OutputEdgeRegion, OutputEdgeRegionError, OutputEdgeThickness, OutputView,
    OutputViewMappingError, OutputViewRef, OutputViewStyle, ReservedArea, ReservedAreaError,
    ReservedAreaRef, ResizeRegion, ResizeRegionError, ResizeRegionIntent, ShellLayer,
    ShellLayerError, ShellLayerMountError, ShellLayerOrder, ShellLayerRef, ShellLayerStyle,
    ShellPrimitiveDiagnosticCollector, ShellPrimitiveDiagnosticKind, ShellPrimitiveDiagnostics,
    ShellRoot, ShellRootError, ShellRootRef, ShellRootStyle, ShellUiExt, SurfaceInputMapping,
    SurfaceInputRegion, SurfaceInputRegionError, SurfacePlaceholder, SurfacePlaceholderError,
    SurfacePlaceholderMountError, SurfacePlaceholderReason, SurfacePlaceholderRef,
    SurfacePlaceholderStyle, SurfaceSnapshot, SurfaceSnapshotAuthorization,
    SurfaceSnapshotAuthorizationError, SurfaceSnapshotError, SurfaceSnapshotMountError,
    SurfaceSnapshotPolicy, SurfaceSnapshotRef, SurfaceSnapshotRevision, SurfaceSnapshotStyle,
    SurfaceSnapshotToken, SurfaceTree, SurfaceTreeError, SurfaceTreeMountError, SurfaceTreeRef,
};
