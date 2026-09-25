mod resize;
mod winit_host;

use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub(crate) enum HostEvent {
    ExitRequested,
    RuntimeWake,
    #[cfg(any(all(feature = "application-vulkan-windows", target_os = "windows"), all(feature = "application-vulkan-linux", target_os = "linux")))]
    PresentationWake,
    ResizeSignalChanged {
        signal: resize::ResizeSignalSnapshot,
        observed_at: Instant,
    },
}

#[cfg(all(
    feature = "application-software",
    any(all(feature = "application-vulkan-windows", target_os = "windows"), all(feature = "application-vulkan-linux", target_os = "linux"))
))]
mod auto;
#[cfg(feature = "application-software")]
mod software;
#[cfg(any(all(feature = "application-vulkan-windows", target_os = "windows"), all(feature = "application-vulkan-linux", target_os = "linux")))]
mod vulkan;
#[cfg(any(all(feature = "application-vulkan-windows", target_os = "windows"), all(feature = "application-vulkan-linux", target_os = "linux")))]
mod vulkan_pipeline;
#[cfg(any(all(feature = "application-vulkan-windows", target_os = "windows"), all(feature = "application-vulkan-linux", target_os = "linux")))]
mod vulkan_worker;

#[cfg(all(
    feature = "application-software",
    any(all(feature = "application-vulkan-windows", target_os = "windows"), all(feature = "application-vulkan-linux", target_os = "linux"))
))]
pub use auto::run_gui_auto as run_gui;
#[cfg(all(
    any(all(feature = "application-vulkan-windows", target_os = "windows"), all(feature = "application-vulkan-linux", target_os = "linux")),
    not(feature = "application-software")
))]
pub use vulkan::run_gui_vulkan as run_gui;
#[cfg(all(
    not(any(all(feature = "application-vulkan-windows", target_os = "windows"), all(feature = "application-vulkan-linux", target_os = "linux"))),
    feature = "application-software"
))]
pub use winit_host::run_gui_software as run_gui;
