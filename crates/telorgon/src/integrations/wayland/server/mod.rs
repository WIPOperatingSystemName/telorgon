//! Linux-only, Telorgon-owned bindings around the official `libwayland-server` ABI.
//!
//! This package owns no compositor policy, scene graph, renderer, input policy, or output backend.
//! It provides the narrow IPC/resource boundary used by `telorgon-compositor-wayland`.

mod error;
pub mod protocol;
mod schema;

#[cfg(target_os = "linux")]
pub mod ffi;
#[cfg(target_os = "linux")]
mod native_protocol;
#[cfg(target_os = "linux")]
mod request;
#[cfg(target_os = "linux")]
mod server;

pub use error::{WaylandServerError, WaylandServerErrorKind};
#[cfg(target_os = "linux")]
pub use native_protocol::NativeProtocol;
#[cfg(target_os = "linux")]
pub use request::{IncomingRequest, RequestDecodeError};
pub use schema::{
    ArgumentSchema, ArgumentType, InterfaceSchema, MessageKind, MessageSchema, ProtocolSchema,
};
#[cfg(target_os = "linux")]
pub use server::{
    ClientCredentials, ClientRef, Display, EventLoopRef, EventSource, Global, OwnedClient,
    ResourceRef,
};

/// Whether the official native server ABI can exist on this compilation target.
pub const NATIVE_SERVER_AVAILABLE: bool = cfg!(target_os = "linux");
