
pub mod application;
#[cfg(feature = "embedded-vulkan")]
pub mod embedded;
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub(crate) mod linux_shell;
