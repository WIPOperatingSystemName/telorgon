
pub mod application;
#[cfg(any(feature = "embedded-vulkan", feature = "embedded-software"))]
pub mod embedded;
#[cfg(feature = "boot-uefi")]
pub mod uefi;
#[cfg(all(target_os = "linux", feature = "boot-splash-linux"))]
pub mod linux_boot_splash;
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub(crate) mod linux_shell;
