//! Linux PipeWire transport and registry, independent of compositor and desktop authority.
//!
//! Construct [`Connection`] explicitly and poll its snapshot or subscribe with a host wake
//! callback. Native objects live on a private control worker. None of these APIs are realtime
//! safe. Drop requests shutdown; [`Connection::shutdown`] joins the worker off the UI thread.
//! Recovery is explicit and invalidates every previous object handle. A portal remote must be
//! reauthorized by its portal owner; it is never replaced by a normal connection implicitly.
mod bindings;
pub(crate) mod connection;
pub(crate) mod parameters;
pub use parameters::{ParameterChoice, ParameterValue};
mod registry;
mod worker;
pub use connection::*;
pub use registry::*;

#[cfg(feature = "shell-screencast-linux")]
mod screencast;
#[cfg(feature = "shell-screencast-linux")]
pub(crate) use screencast::{StreamStatus, VideoStream};

#[cfg(any(feature = "desktop-audio-linux", feature = "video-linux"))]
pub(crate) mod control;

#[cfg(test)]
mod tests;

#[cfg(feature = "audio-linux")]
pub(crate) mod audio;

pub mod graph;

#[cfg(feature = "audio-linux")]
mod filter;

#[cfg(feature = "midi-linux")]
mod midi;

#[cfg(feature = "video-linux")]
pub(crate) mod video;
#[cfg(feature = "video-linux")]
mod video_buffer;
#[cfg(feature = "video-linux")]
mod video_format;
#[cfg(feature = "video-linux")]
mod video_metadata;
#[cfg(feature = "video-linux")]
mod video_timer;

#[cfg(feature = "video-linux")]
pub(crate) mod video_dma_buf;

#[cfg(feature = "video-linux")]
mod video_output;

#[cfg(any(feature = "audio-linux", feature = "midi-linux"))]
pub(crate) mod owned_node;
