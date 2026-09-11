//! X11 compatibility components, including explicit private-helper supervision.
//! Managed integration and qualification are tracked in X11_QUALIFICATION.md.
//! Enabling this module does not advertise X11 support to applications.

pub mod acquisition;
pub mod association;
mod commands;
pub mod conversion;
pub mod discovery;
pub mod incr;
pub mod incr_wire;
pub mod inspection;
pub mod lifecycle;
pub mod manager;
pub mod normal_hints;
pub mod payload;
pub(crate) mod payload_format;
#[cfg(target_env = "gnu")]
pub mod process;
pub mod properties;
pub mod property_read;
pub mod readiness;
pub mod requests;
pub mod resize_sync;
pub mod resources;
pub mod selection;
pub mod transfer;
pub mod transport;
pub mod window;
pub mod xwm;

use std::fmt;

/// Content-free compatibility failure. Never attach property/clipboard bytes.
#[derive(Debug)]
pub struct Error(pub(crate) String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self(format!("X11 compatibility I/O failed: {error}"))
    }
}
pub type Result<T> = std::result::Result<T, Error>;

pub mod root_cursor;
