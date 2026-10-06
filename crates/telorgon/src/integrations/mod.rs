
#[cfg(all(feature = "shell-xwayland", target_os = "linux"))]
pub mod x11;
#[cfg(all(any(feature = "shell-screencast-linux", feature="portal-client-linux"), target_os = "linux"))]
pub mod portals;
#[cfg(all(feature = "pipewire-linux", target_os = "linux"))]
pub mod pipewire;
pub mod wayland;

#[cfg(all(feature = "desktop-audio-linux", target_os = "linux"))]
pub mod wireplumber;

#[cfg(all(feature = "tray-linux", target_os = "linux"))]
pub(crate) mod status_notifier;

#[cfg(all(feature = "networkmanager-linux", target_os = "linux"))]
pub mod networkmanager;
