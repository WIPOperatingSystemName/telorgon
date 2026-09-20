//! Windows Vulkan-to-D3D11 image and synchronization transport for DXGI presentation.

#[cfg(target_os = "windows")]
mod bridge;
#[cfg(target_os = "windows")]
mod error;
#[cfg(target_os = "windows")]
mod frame;

#[cfg(target_os = "windows")]
pub use bridge::{AcquiredVulkanDxgiFrame, VulkanDxgiAcquireOutcome, VulkanDxgiBridge};
#[cfg(target_os = "windows")]
pub use error::{PresentError, PresentErrorKind, PresentResult};
#[cfg(target_os = "windows")]
pub use frame::DxgiPresentOutcome;
