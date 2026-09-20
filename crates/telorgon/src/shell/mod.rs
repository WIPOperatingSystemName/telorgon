//! Protocol-neutral shell models, capabilities, and requests.
//!
//! This crate contains host-authored values only. It owns no display protocol object, native
//! handle, renderer resource, policy engine, event loop, application component, or background work.

pub mod capability;
pub mod capture;
pub mod diagnostics;
pub mod error;
pub mod host;
pub mod id;
pub mod model;
pub mod request;

pub use capability::*;
pub use diagnostics::*;
pub use error::*;
pub use host::*;
pub use id::*;
pub use model::*;
pub use request::{
    AcceptedRequestId, ClientInputRequest, ContactId, InputSource, OutputAppearanceActionId,
    OutputEdge, OutputModeActionId, OutputRequest, ReservedAreaExtent, ReservedAreaExtentError,
    ReservedAreaId, ResizeEdge, SeatId, ShellRequestResult, SurfaceInputContact, SurfaceInputError,
    SurfaceInputEvent, SurfaceInputKind, SurfaceRequest, SystemRequest, WorkspaceRequest,
};
