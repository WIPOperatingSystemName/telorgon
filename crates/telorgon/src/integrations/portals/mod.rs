//! Portal clients and compositor backends have separate owners and feature gates.
#[cfg(feature="shell-screencast-linux")]
mod backend;
#[cfg(feature="shell-screencast-linux")]
pub(crate) use backend::*;
#[cfg(feature="portal-client-linux")]
mod request;
#[cfg(feature="portal-client-linux")]
pub mod client;
