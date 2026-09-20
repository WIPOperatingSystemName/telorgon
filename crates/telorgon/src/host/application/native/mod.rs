mod resize;
mod winit_host;

use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub(crate) enum HostEvent {
    ExitRequested,
    RuntimeWake,
    #[cfg(all(feature = "application-vulkan-windows", target_os = "windows"))]
    PresentationWake,
    ResizeSignalChanged {
        signal: resize::ResizeSignalSnapshot,
        observed_at: Instant,
    },
}

#[cfg(all(
    feature = "application-software",
    feature = "application-vulkan-windows",
    target_os = "windows"
))]
mod auto;
#[cfg(feature = "application-software")]
mod software;
#[cfg(all(feature = "application-vulkan-windows", target_os = "windows"))]
mod vulkan;
#[cfg(all(feature = "application-vulkan-windows", target_os = "windows"))]
mod vulkan_pipeline;
#[cfg(all(feature = "application-vulkan-windows", target_os = "windows"))]
mod vulkan_worker;

#[cfg(all(
    feature = "application-software",
    feature = "application-vulkan-windows",
    target_os = "windows"
))]
pub use auto::run_gui_auto as run_gui;
#[cfg(all(
    feature = "application-vulkan-windows",
    target_os = "windows",
    not(feature = "application-software")
))]
pub use vulkan::run_gui_vulkan as run_gui;
#[cfg(all(
    not(all(feature = "application-vulkan-windows", target_os = "windows")),
    feature = "application-software"
))]
pub use winit_host::run_gui_software as run_gui;
