
#[cfg(feature = "profiler")]
pub mod profiler;
#[cfg(not(target_os = "uefi"))]
pub mod session;
pub mod clipboard;
pub mod battery;

#[cfg(all(feature = "desktop-audio-linux", target_os = "linux"))]
pub mod audio;

#[cfg(all(feature = "tray-linux", target_os = "linux"))]
pub mod tray;

pub mod screen_brightness;

pub mod network;

pub mod display;
#[cfg(all(feature = "desktop-settings-linux", target_os = "linux"))]
pub mod desktop_settings;
