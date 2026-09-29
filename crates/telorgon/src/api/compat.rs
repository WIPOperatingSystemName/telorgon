//! Existing module import paths retained during the ownership migration.

pub use crate::authoring::compose;
pub use crate::authoring::fill;
pub use crate::components::application as application_components;
pub use crate::components::application::primitives as application_primitives;
pub use crate::components::shell as shell_components;
pub use crate::components::shell::primitives as shell_primitives;
pub use crate::foundation as core;
#[cfg(all(feature = "application-vulkan", target_os = "windows"))]
pub use crate::graphics::bridges::vulkan_dxgi as bridge_vulkan_dxgi;
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub use crate::graphics::bridges::wayland as compositor_render;
#[cfg(any(
    feature = "application-vulkan",
    feature = "shell-wayland-linux",
    feature = "embedded-vulkan"
))]
pub use crate::graphics::gpu_abi;
pub use crate::graphics::material;
pub use crate::graphics::presentation;
#[cfg(all(feature = "application-vulkan", target_os = "windows"))]
pub use crate::graphics::presentation::dxgi as presenter_dxgi;
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub use crate::graphics::presentation::kms as presenter_vulkan_kms;
#[cfg(feature = "application-software")]
pub use crate::graphics::presentation::softbuffer as presenter_softbuffer;
#[cfg(feature = "application-vulkan")]
pub use crate::graphics::presentation::wsi as presenter_vulkan_wsi;
pub use crate::graphics::render;
#[cfg(any(feature = "application-software", feature = "shell-wayland-linux"))]
pub use crate::graphics::renderers::software as renderer_software;
#[cfg(any(
    feature = "application-vulkan",
    feature = "shell-wayland-linux",
    feature = "embedded-vulkan"
))]
pub use crate::graphics::renderers::vulkan as renderer_vulkan;
pub use crate::graphics::scene;
pub use crate::host::application as application_host;
#[cfg(feature = "embedded-vulkan")]
pub use crate::host::embedded as embed;
#[cfg(any(test, all(feature = "shell-wayland-linux", target_os = "linux")))]
pub use crate::integrations::wayland::compositor as compositor_wayland;
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub use crate::integrations::wayland::server as wayland_server;
#[cfg(all(feature = "shell-xwayland", target_os = "linux"))]
pub use crate::integrations::x11 as xwayland;
pub use crate::platform::conformance as platform_conformance;
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub use crate::platform::linux as platform_linux;
#[cfg(any(
    feature = "application-software",
    feature = "application-vulkan"
))]
pub use crate::platform::winit as platform_winit;
#[cfg(feature = "instrumentation")]
pub use crate::runtime::instrumentation as profiler;
#[cfg(feature = "profiler")]
pub use crate::services::profiler as profiler_server;
pub use crate::services::session;
pub use crate::shell::window_chrome;
pub use crate::ui::accessibility;
pub use crate::ui::layout;
pub use crate::ui::text;
