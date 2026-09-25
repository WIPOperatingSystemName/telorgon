//! Telorgon-owned Wayland protocol state and Linux compositor runtime.

mod buffer;
mod capture;
mod foreign_toplevel;
mod core;
mod data_device;
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub(crate) mod diagnostics;
mod id;
mod object;
mod output;
mod region;
mod seat;
mod serial;
mod shell_export;
mod subsurface;
mod surface;
mod synchronization;
mod world;
mod xdg;

#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
mod native;
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub(crate) use native::{TimingEvent, TimingObserver};
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
mod capture_access;
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub use capture_access::CaptureAccess;

pub use buffer::{
    BufferDescriptor, BufferError, DmaBufDescriptor, DmaBufFlags, DmaBufPlane, ShmBuffer,
    ShmFormat, ShmPool,
};
pub use core::{CompositorAction, CompositorCore, CompositorCoreError};
pub use data_device::{
    DataAction, DataDeviceError, DataDeviceState, DataOffer, DataSource, MimeType,
};
pub use id::{ClientId, ProtocolObjectId, WaylandBufferId, WaylandSurfaceId};
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub use native::{
    DmaBufFormat, DmaBufImage, NativeCompositor, NativeCompositorError, PointerConstraintKind,
    PointerConstraintState, ShmBufferReader, ShmImage, ShmImageRegion, ToplevelIconImage,
    ToplevelIconSnapshot, ViewportSource, ViewportState, XwaylandAccess,
};
pub use object::{ObjectMetadata, ObjectRegistry, ObjectRegistryError, ProtocolObjectKind};
pub use output::{
    OutputDescription, OutputError, OutputLayoutSnapshot, OutputMode, OutputRootGeometry,
    OutputState, OutputTransform,
};
pub use region::{Region, RegionError};
pub use seat::{
    ButtonState, CursorImage, KeyboardFocus, PointerFocus, SeatCapabilities, SeatState,
};
pub use serial::{SerialKind, SerialLedger, SerialRecord, SerialValidationError};
pub use shell_export::{ShellSurfaceExport, SurfaceExportError};
pub use subsurface::{SubsurfaceError, SubsurfaceGraph, SubsurfacePosition};
pub use surface::{
    BufferAttachment, BufferTransform, CommitOutcome, SurfaceCommit, SurfaceError, SurfaceRole,
    SurfaceState, SurfaceStateSnapshot,
};
pub use synchronization::{
    BufferRelease, BufferUseError, BufferUseId, BufferUseTracker, SurfaceFrameCallback,
};
pub use world::{ClientLimits, WaylandWorld, WaylandWorldError};
pub use xdg::{
    DecorationMode, ResizeEdge, ToplevelState, XdgConfigure, XdgError, XdgPopupState,
    XdgPositioner, XdgSurfaceState, XdgToplevelState,
};

#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub(crate) use native::{DirectCaptureJob, DirectCaptureCompletion};
