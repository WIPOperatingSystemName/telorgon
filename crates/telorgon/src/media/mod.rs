//! Media owners independent of GUI and compositor entry points.
#[cfg(all(feature = "audio-linux", target_os = "linux"))]
pub mod audio;

#[cfg(all(feature = "midi-linux", target_os = "linux"))]
pub mod midi;

#[cfg(all(feature="video-linux",target_os="linux"))]
pub mod video;

pub mod timing;
