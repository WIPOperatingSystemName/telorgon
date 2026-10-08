
#[cfg(any(feature = "application-software", feature = "shell-wayland-linux", feature = "embedded-software"))]
pub mod software;
#[cfg(any(
    feature = "application-vulkan",
    feature = "shell-wayland-linux",
    feature = "embedded-vulkan"
))]
pub mod vulkan;
