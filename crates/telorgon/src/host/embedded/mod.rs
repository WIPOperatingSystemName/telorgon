//! Host-driven UI views with externally owned scheduling and presentation.

#[cfg(feature = "embedded-software")]
mod software;
#[cfg(feature = "embedded-software")]
pub use software::{SoftwareUiFrame, SoftwareUiHost, SoftwareUiHostError, SoftwareUiHostResult};

#[cfg(feature = "embedded-vulkan")]
mod vulkan;
#[cfg(feature = "embedded-vulkan")]
pub use vulkan::*;
