pub use crate::graphics::presentation::PresentDisposition;
use crate::graphics::presentation::{CompletionProof, CompletionStage};
use crate::graphics::renderers::vulkan::{CompletionPoint, VulkanDevice, VulkanRecordedFrame, VulkanTarget};
use ash::vk;

use crate::graphics::presentation::wsi::error::PresentResult;
use crate::graphics::presentation::wsi::presenter::VulkanWinitPresenter;

pub enum AcquireOutcome<'presenter> {
    Ready(AcquiredVulkanFrame<'presenter>),
    Suspended,
    NotReady,
    NeedsReconfigure,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PresentOutcome {
    pub completion: CompletionPoint,
    /// Present-engine completion proof. Exact presentation IDs are preferred, with swapchain
    /// maintenance fences retained as the fallback.
    pub presentation_completion: Option<PresentCompletion>,
    pub disposition: PresentDisposition,
    pub reconfigure_pending: bool,
    pub maintenance_pending: bool,
}

impl PresentOutcome {
    pub const fn render_completion(&self) -> CompletionProof<CompletionPoint> {
        CompletionProof::new(CompletionStage::Render, self.completion)
    }
}

/// Identifies one presentation fence without exposing native synchronization handles.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PresentCompletion {
    pub(crate) swapchain_generation: u64,
    pub(crate) proof: PresentCompletionProof,
}

impl PresentCompletion {
    pub const fn stage(&self) -> CompletionStage {
        CompletionStage::Present
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum PresentCompletionProof {
    PresentId(u64),
    PresentFence { acquire_slot: usize },
}

pub struct AcquiredVulkanFrame<'presenter> {
    pub(crate) presenter: &'presenter mut VulkanWinitPresenter,
    pub(crate) state: AcquiredFrameState,
    pub(crate) consumed: bool,
}

#[derive(Copy, Clone)]
pub(crate) struct AcquiredFrameState {
    pub(crate) device_id: u64,
    pub(crate) frame_id: u64,
    pub(crate) image_index: usize,
    pub(crate) acquire_slot: usize,
    pub(crate) acquired_suboptimal: bool,
}

impl AcquiredVulkanFrame<'_> {
    pub fn target(&self) -> VulkanTarget<'_> {
        let state = self
            .presenter
            .swapchain
            .as_ref()
            .expect("acquired frame must retain its swapchain");
        unsafe {
            crate::graphics::renderers::vulkan::interop::swapchain_target(
                &state.device,
                state.images[self.state.image_index],
                state.views[self.state.image_index],
                state.format,
                state.extent,
                state.initialized[self.state.image_index],
                state.alpha_mode,
            )
        }
    }

    pub fn submit_and_present(
        mut self,
        device: &VulkanDevice,
        frame: VulkanRecordedFrame,
    ) -> PresentResult<PresentOutcome> {
        self.consumed = true;
        self.presenter.submit_acquired(device, frame, self.state)
    }

    pub fn discard(mut self, device: &VulkanDevice) -> PresentResult<()> {
        self.consumed = true;
        self.presenter.discard_acquired(
            device,
            self.state.device_id,
            self.state.acquire_slot,
            self.state.image_index,
        )
    }
}

impl Drop for AcquiredVulkanFrame<'_> {
    fn drop(&mut self) {
        if !self.consumed {
            self.presenter.abandon_acquired(self.state.image_index);
        }
    }
}

pub(crate) fn present_info<'a>(
    semaphore: &'a [vk::Semaphore; 1],
    swapchain: &'a [vk::SwapchainKHR; 1],
    index: &'a [u32; 1],
) -> vk::PresentInfoKHR<'a> {
    vk::PresentInfoKHR::default()
        .wait_semaphores(semaphore)
        .swapchains(swapchain)
        .image_indices(index)
}
