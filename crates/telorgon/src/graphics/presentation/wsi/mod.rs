//! Vulkan WSI surface, swapchain, acquisition, presentation, and recovery ownership.

mod error;
mod frame;
mod presenter;
mod recovery;
mod surface;
mod swapchain;

pub use error::{PresentError, PresentErrorKind, PresentResult};
pub use frame::{
    AcquireOutcome, AcquiredVulkanFrame, PresentCompletion, PresentDisposition, PresentOutcome,
};
pub use presenter::{PresenterReconfigurePolicy, VulkanWinitPresenter};
pub use recovery::{PresenterRecovery, PresenterState};
pub use surface::{VulkanWinitSurface, required_instance_extensions};
pub use swapchain::VulkanPresentModePreference;
