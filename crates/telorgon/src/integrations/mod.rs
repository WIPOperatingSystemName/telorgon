
#[cfg(all(feature = "shell-xwayland", target_os = "linux"))]
pub mod x11;
#[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
pub(crate) mod portals;
#[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
pub(crate) mod pipewire;
pub mod wayland;
