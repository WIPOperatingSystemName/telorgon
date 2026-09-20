use crate::foundation::SizeI;
use crate::graphics::renderers::vulkan::{VulkanDevice, VulkanRecordedFrame, VulkanRecordingFrame};
use ash::vk;
#[cfg(any(feature = "instrumentation", test))]
use std::time::Duration;
#[cfg(feature = "instrumentation")]
use std::time::Instant;

use crate::graphics::presentation::wsi::error::{PresentError, PresentErrorKind, PresentResult};
use crate::graphics::presentation::wsi::frame::{
    AcquireOutcome, AcquiredFrameState, AcquiredVulkanFrame, PresentCompletion,
    PresentCompletionProof, PresentDisposition, PresentOutcome, present_info,
};
use crate::graphics::presentation::wsi::recovery::{PresenterRecovery, PresenterState, is_zero};
use crate::graphics::presentation::wsi::surface::VulkanWinitSurface;
use crate::graphics::presentation::wsi::swapchain::{SwapchainState, VulkanPresentModePreference};

#[cfg(any(feature = "instrumentation", test))]
const ZERO_TIMEOUT_ACQUIRE_STALL_THRESHOLD: Duration = Duration::from_millis(100);

#[cfg(any(feature = "instrumentation", test))]
fn zero_timeout_acquire_stalled(duration: Duration) -> bool {
    duration >= ZERO_TIMEOUT_ACQUIRE_STALL_THRESHOLD
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
struct RetirementAnchor {
    first_present_image: Option<usize>,
    acquire_slot: Option<usize>,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PresenterReconfigurePolicy {
    #[default]
    Eager,
    DeferSuboptimal,
}

impl RetirementAnchor {
    fn note_present(&mut self, image: usize) {
        self.first_present_image.get_or_insert(image);
    }

    fn note_acquire(&mut self, image: usize, slot: usize) {
        if self.acquire_slot.is_none() && self.first_present_image == Some(image) {
            self.acquire_slot = Some(slot);
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

pub struct VulkanWinitPresenter {
    pub(crate) surface: VulkanWinitSurface,
    pub(crate) swapchain: Option<SwapchainState>,
    retired_swapchains: Vec<SwapchainState>,
    retirement_anchor: RetirementAnchor,
    recovery: PresenterRecovery,
    reconfigure_policy: PresenterReconfigurePolicy,
    frames_in_flight: usize,
    device_id: u64,
    swapchain_maintenance1: bool,
    present_mode_preference: VulkanPresentModePreference,
}

impl VulkanWinitPresenter {
    pub fn new(
        surface: VulkanWinitSurface,
        device: &VulkanDevice,
        extent: SizeI,
        frames_in_flight: usize,
    ) -> PresentResult<Self> {
        Self::new_with_present_mode(
            surface,
            device,
            extent,
            frames_in_flight,
            VulkanPresentModePreference::default(),
        )
    }

    pub fn new_with_present_mode(
        surface: VulkanWinitSurface,
        device: &VulkanDevice,
        extent: SizeI,
        frames_in_flight: usize,
        present_mode_preference: VulkanPresentModePreference,
    ) -> PresentResult<Self> {
        let mut presenter = Self {
            surface,
            swapchain: None,
            retired_swapchains: Vec::new(),
            retirement_anchor: RetirementAnchor::default(),
            recovery: PresenterRecovery::new(extent),
            reconfigure_policy: PresenterReconfigurePolicy::Eager,
            frames_in_flight: frames_in_flight.max(1),
            device_id: device_identity(device),
            swapchain_maintenance1: device.capabilities().swapchain_maintenance1,
            present_mode_preference,
        };
        if !is_zero(extent) {
            presenter.reconfigure(device)?;
        }
        Ok(presenter)
    }

    pub fn recovery(&self) -> PresenterRecovery {
        self.recovery
    }

    pub fn set_reconfigure_policy(&mut self, policy: PresenterReconfigurePolicy) {
        self.reconfigure_policy = policy;
    }

    /// Polls whether the presentation engine has finished consuming a queued image.
    ///
    /// A completion from an older generation is complete once a successor generation is active.
    /// Current-generation completions prefer `VK_KHR_present_wait` presentation IDs and fall back
    /// to `VK_EXT_swapchain_maintenance1` present fences. Both are stronger than successful return
    /// from `vkQueuePresentKHR`, while presentation IDs identify the exact queued frame.
    pub fn poll_present_completion(
        &mut self,
        completion: PresentCompletion,
    ) -> PresentResult<bool> {
        if completion.swapchain_generation != self.recovery.generation {
            return Ok(completion.swapchain_generation < self.recovery.generation);
        }
        let state = self.swapchain.as_mut().ok_or_else(|| {
            PresentError::new(
                PresentErrorKind::InvalidState,
                "presentation completion has no active swapchain",
            )
        })?;
        match completion.proof {
            PresentCompletionProof::PresentId(present_id) => state.present_id_complete(present_id),
            PresentCompletionProof::PresentFence { acquire_slot } => {
                state.present_slot_complete(acquire_slot)
            }
        }
    }

    /// Returns whether retired swapchain generations still need presentation progress before they
    /// can be destroyed without blocking a queue.
    pub fn has_pending_retirement(&self) -> bool {
        !self.retired_swapchains.is_empty()
    }

    /// Bounds retained generations by waiting only when asynchronous retirement falls behind.
    ///
    /// Hosts should call this from their presentation worker, never from a window event callback.
    pub fn enforce_retirement_limit(
        &mut self,
        device: &VulkanDevice,
        maximum: usize,
    ) -> PresentResult<()> {
        self.validate_device(device)?;
        if self.retired_swapchains.len() <= maximum {
            return Ok(());
        }
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("presenter.retirement_wait");
        unsafe { crate::graphics::renderers::vulkan::interop::wait_presentation_queues_idle(device) }.map_err(
            |result| PresentError::from_vk("bounded swapchain retirement failed", result),
        )?;
        self.retired_swapchains.clear();
        self.retirement_anchor.reset();
        Ok(())
    }

    pub fn resize(&mut self, extent: SizeI) -> bool {
        if self.recovery.state != PresenterState::Shutdown {
            self.recovery.resize(extent)
        } else {
            false
        }
    }

    pub fn suspend(&mut self) -> PresentResult<()> {
        self.retire_all_blocking()?;
        self.recovery.state = PresenterState::Suspended;
        Ok(())
    }

    pub fn resume(&mut self, device: &VulkanDevice, extent: SizeI) -> PresentResult<()> {
        self.validate_device(device)?;
        self.recovery.resize(extent);
        if !is_zero(extent) {
            self.reconfigure(device)?;
        }
        Ok(())
    }

    pub fn replace_surface(
        &mut self,
        device: &VulkanDevice,
        surface: VulkanWinitSurface,
        extent: SizeI,
    ) -> PresentResult<()> {
        self.validate_device(device)?;
        self.retire_all_blocking()?;
        self.surface = surface;
        self.recovery.resize(extent);
        self.reconfigure(device)
    }

    pub fn acquire<'a>(
        &'a mut self,
        device: &VulkanDevice,
        frame: &VulkanRecordingFrame<'_>,
    ) -> PresentResult<AcquireOutcome<'a>> {
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("presenter.acquire");
        self.validate_device(device)?;
        self.collect_retired()?;
        #[cfg(feature = "instrumentation")]
        crate::runtime::instrumentation::counter!(
            "presenter.retired_swapchains",
            self.retired_swapchains.len()
        );
        if frame.device_id() != self.device_id {
            return Err(PresentError::new(
                PresentErrorKind::InvalidState,
                "recording frame belongs to another Vulkan device",
            ));
        }
        match self.recovery.state {
            PresenterState::Suspended => return Ok(AcquireOutcome::Suspended),
            PresenterState::NeedsReconfigure | PresenterState::Unconfigured => {
                self.reconfigure(device)?;
            }
            PresenterState::SurfaceLost => return Ok(AcquireOutcome::NeedsReconfigure),
            PresenterState::DeviceLost | PresenterState::Shutdown => {
                return Err(PresentError::new(
                    PresentErrorKind::InvalidState,
                    format!("presenter is {:?}", self.recovery.state),
                ));
            }
            PresenterState::Ready => {}
        }
        let state = self.swapchain.as_mut().ok_or_else(|| {
            PresentError::new(
                PresentErrorKind::InvalidState,
                "swapchain is not configured",
            )
        })?;
        let Some(acquire_slot) = state.acquire_slot()? else {
            return Ok(AcquireOutcome::NotReady);
        };
        let acquire_semaphore = state.acquire_semaphores[acquire_slot];
        let acquire_fence = state.acquire_fence(acquire_slot);
        #[cfg(feature = "instrumentation")]
        let raw_acquire_started = Instant::now();
        #[cfg(feature = "instrumentation")]
        let raw_acquire_span = crate::runtime::instrumentation::span!("presenter.acquire.raw_dispatch");
        let acquired = unsafe {
            state
                .loader
                .acquire_next_image(state.raw, 0, acquire_semaphore, acquire_fence)
        };
        #[cfg(feature = "instrumentation")]
        {
            drop(raw_acquire_span);
            let raw_acquire_duration = raw_acquire_started.elapsed();
            crate::runtime::instrumentation::counter!(
                "presentation.acquire.raw_dispatch_duration_ns",
                raw_acquire_duration.as_nanos()
            );
            if zero_timeout_acquire_stalled(raw_acquire_duration) {
                crate::runtime::instrumentation::record_diagnostic(
                    "presentation.vulkan_wsi.zero_timeout_acquire_stall",
                    crate::runtime::instrumentation::DiagnosticSeverity::Warning,
                    1,
                );
            }
        }
        let (image_index, suboptimal) = match acquired {
            Ok(value) => value,
            Err(vk::Result::NOT_READY | vk::Result::TIMEOUT) => {
                return Ok(AcquireOutcome::NotReady);
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.recovery.state = PresenterState::NeedsReconfigure;
                return Ok(AcquireOutcome::NeedsReconfigure);
            }
            Err(vk::Result::ERROR_SURFACE_LOST_KHR) => {
                self.recovery.state = PresenterState::SurfaceLost;
                return Ok(AcquireOutcome::NeedsReconfigure);
            }
            Err(vk::Result::ERROR_DEVICE_LOST) => {
                self.recovery.state = PresenterState::DeviceLost;
                return Err(PresentError::from_vk(
                    "failed to acquire swapchain image",
                    vk::Result::ERROR_DEVICE_LOST,
                ));
            }
            Err(result) => {
                return Err(PresentError::from_vk(
                    "failed to acquire swapchain image",
                    result,
                ));
            }
        };
        state.mark_acquired(acquire_slot);
        let image_index = image_index as usize;
        if !self.swapchain_maintenance1 && !self.retired_swapchains.is_empty() {
            self.retirement_anchor
                .note_acquire(image_index, acquire_slot);
        }
        self.recovery.state = PresenterState::Ready;
        Ok(AcquireOutcome::Ready(AcquiredVulkanFrame {
            presenter: self,
            state: AcquiredFrameState {
                device_id: device_identity(device),
                frame_id: frame.frame_id(),
                image_index,
                acquire_slot,
                acquired_suboptimal: suboptimal,
            },
            consumed: false,
        }))
    }

    pub fn shutdown(&mut self, device: &VulkanDevice) -> PresentResult<()> {
        self.validate_device(device)?;
        self.retire_all_blocking()?;
        self.recovery.state = PresenterState::Shutdown;
        Ok(())
    }

    pub(crate) fn submit_acquired(
        &mut self,
        device: &VulkanDevice,
        frame: VulkanRecordedFrame,
        acquired: AcquiredFrameState,
    ) -> PresentResult<PresentOutcome> {
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("presenter.submit_present");
        if acquired.device_id != device_identity(device)
            || frame.device_id() != acquired.device_id
            || frame.frame_id() != acquired.frame_id
        {
            self.recovery.state = PresenterState::NeedsReconfigure;
            return Err(PresentError::new(
                PresentErrorKind::InvalidState,
                "acquired image and recorded frame identity do not match",
            ));
        }
        let swapchain_generation = self.recovery.generation;
        let state = self.swapchain.as_mut().ok_or_else(|| {
            PresentError::new(
                PresentErrorKind::InvalidState,
                "swapchain is not configured",
            )
        })?;
        let acquire = state.acquire_semaphores[acquired.acquire_slot];
        let finished = state.present_semaphore(acquired.image_index, acquired.acquire_slot);
        let receipt = unsafe {
            crate::graphics::renderers::vulkan::interop::submit_present_frame(device, frame, acquire, finished)
        }
        .map_err(|error| {
            self.recovery.state = PresenterState::DeviceLost;
            PresentError::new(PresentErrorKind::DeviceLost, error.to_string())
        })?;
        let completion = receipt.completion();
        let waits = [finished];
        let swapchains = [state.raw];
        let indices = [acquired.image_index as u32];
        let present_fence = state.prepare_present_fence(acquired.acquire_slot)?;
        let present_fences = present_fence.map(|fence| [fence]);
        let present_id = state.prepare_present_id()?;
        let present_ids = present_id.map(|present_id| [present_id]);
        let mut present_fence_info = vk::SwapchainPresentFenceInfoEXT::default();
        let mut present_id_info = vk::PresentIdKHR::default();
        let mut info = present_info(&waits, &swapchains, &indices);
        if let Some(fences) = present_fences.as_ref() {
            present_fence_info = present_fence_info.fences(fences);
            info = info.push_next(&mut present_fence_info);
        }
        if let Some(present_ids) = present_ids.as_ref() {
            present_id_info = present_id_info.present_ids(present_ids);
            info = info.push_next(&mut present_id_info);
        }
        let presented =
            unsafe { crate::graphics::renderers::vulkan::interop::queue_present(device, &state.loader, &info) };
        if present_fence.is_some() {
            state.mark_present_pending(acquired.acquire_slot);
        }
        state.acquire_receipts[acquired.acquire_slot] = Some(receipt);
        state.initialized[acquired.image_index] = true;
        state.advance_acquire_slot();
        let disposition = match presented {
            Ok(suboptimal) if suboptimal || acquired.acquired_suboptimal => {
                self.recovery.state = state_after_suboptimal(self.reconfigure_policy);
                PresentDisposition::PresentedSuboptimal
            }
            Ok(_) => {
                self.recovery.state = PresenterState::Ready;
                PresentDisposition::Presented
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.recovery.state = PresenterState::NeedsReconfigure;
                PresentDisposition::NeedsReconfigure
            }
            Err(vk::Result::ERROR_SURFACE_LOST_KHR) => {
                self.recovery.state = PresenterState::SurfaceLost;
                PresentDisposition::SurfaceLost
            }
            Err(vk::Result::ERROR_DEVICE_LOST) => {
                self.recovery.state = PresenterState::DeviceLost;
                return Err(PresentError::from_vk(
                    "failed to present swapchain image",
                    vk::Result::ERROR_DEVICE_LOST,
                ));
            }
            Err(result) => {
                return Err(PresentError::from_vk(
                    "failed to present swapchain image",
                    result,
                ));
            }
        };
        if !self.swapchain_maintenance1
            && !self.retired_swapchains.is_empty()
            && matches!(
                disposition,
                PresentDisposition::Presented | PresentDisposition::PresentedSuboptimal
            )
        {
            self.retirement_anchor.note_present(acquired.image_index);
        }
        Ok(PresentOutcome {
            completion,
            presentation_completion: present_completion(
                swapchain_generation,
                acquired.acquire_slot,
                present_id,
                present_fence.is_some(),
                disposition,
            ),
            disposition,
            reconfigure_pending: self.recovery.state == PresenterState::NeedsReconfigure,
            maintenance_pending: self.has_pending_retirement(),
        })
    }

    pub(crate) fn discard_acquired(
        &mut self,
        device: &VulkanDevice,
        acquired_device_id: u64,
        _acquire_slot: usize,
        image_index: usize,
    ) -> PresentResult<()> {
        self.validate_device(device)?;
        if acquired_device_id != self.device_id {
            return Err(PresentError::new(
                PresentErrorKind::InvalidState,
                "acquired image belongs to another device",
            ));
        }
        if let Some(state) = self.swapchain.as_ref() {
            state.release_image(image_index)?;
        }
        self.recovery.state = PresenterState::NeedsReconfigure;
        Ok(())
    }

    pub(crate) fn abandon_acquired(&mut self, image_index: usize) {
        if let Some(state) = self.swapchain.as_ref() {
            let _ = state.release_image(image_index);
        }
        self.recovery.state = PresenterState::NeedsReconfigure;
    }

    fn reconfigure(&mut self, device: &VulkanDevice) -> PresentResult<()> {
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("presenter.reconfigure");
        self.validate_device(device)?;
        if is_zero(self.recovery.requested_extent) {
            return self.suspend();
        }
        let old = self.swapchain.take();
        let old_handle = old
            .as_ref()
            .map_or(vk::SwapchainKHR::null(), |state| state.raw);
        let next = SwapchainState::create(
            &self.surface,
            device,
            self.recovery.requested_extent,
            self.frames_in_flight,
            old_handle,
            self.present_mode_preference,
        );
        if let Some(old) = old {
            self.retired_swapchains.push(old);
            self.recovery.mark_retired();
        }
        self.retirement_anchor.reset();
        self.swapchain = Some(next?);
        self.recovery.mark_reconfigured()?;
        #[cfg(feature = "instrumentation")]
        if let Some(state) = self.swapchain.as_ref() {
            crate::runtime::instrumentation::counter!("presenter.swapchain.generation", self.recovery.generation);
            crate::runtime::instrumentation::counter!(
                "presenter.swapchain.present_mode",
                state.present_mode.as_raw()
            );
            crate::runtime::instrumentation::counter!("presenter.swapchain.image_count", state.images.len());
            crate::runtime::instrumentation::counter!("presenter.swapchain.extent_width", state.extent.width);
            crate::runtime::instrumentation::counter!("presenter.swapchain.extent_height", state.extent.height);
            crate::runtime::instrumentation::counter!(
                "presenter.swapchain.maintenance1",
                u8::from(self.swapchain_maintenance1)
            );
            crate::runtime::instrumentation::counter!(
                "presenter.swapchain.present_wait",
                u8::from(state.present_wait.is_some())
            );
            crate::runtime::instrumentation::counter!(
                "presenter.swapchain.one_to_one_scaling",
                u8::from(state.one_to_one_present_scaling)
            );
        }
        Ok(())
    }

    fn collect_retired(&mut self) -> PresentResult<()> {
        if self.retired_swapchains.is_empty() {
            self.retirement_anchor.reset();
            return Ok(());
        }
        if self.swapchain_maintenance1 {
            let mut pending = Vec::with_capacity(self.retired_swapchains.len());
            for mut retired in self.retired_swapchains.drain(..) {
                if !retired.presentation_complete()? {
                    pending.push(retired);
                }
            }
            self.retired_swapchains = pending;
            self.retirement_anchor.reset();
            return Ok(());
        }
        let Some(slot) = self.retirement_anchor.acquire_slot else {
            return Ok(());
        };
        let complete = self
            .swapchain
            .as_ref()
            .ok_or_else(|| {
                PresentError::new(
                    PresentErrorKind::InvalidState,
                    "swapchain retirement has no active successor",
                )
            })?
            .acquisition_complete(slot)?;
        if complete {
            self.retired_swapchains.clear();
            self.retirement_anchor.reset();
        }
        Ok(())
    }

    fn retire_all_blocking(&mut self) -> PresentResult<()> {
        let device = self
            .swapchain
            .as_ref()
            .or_else(|| self.retired_swapchains.first())
            .map(|swapchain| swapchain.device.clone());
        if let Some(device) = device {
            unsafe { crate::graphics::renderers::vulkan::interop::wait_presentation_queues_idle(&device) }
                .map_err(|result| {
                    PresentError::from_vk("swapchain shutdown retirement failed", result)
                })?;
        }
        self.swapchain = None;
        self.retired_swapchains.clear();
        self.retirement_anchor.reset();
        Ok(())
    }

    fn validate_device(&self, device: &VulkanDevice) -> PresentResult<()> {
        if device_identity(device) != self.device_id {
            return Err(PresentError::new(
                PresentErrorKind::InvalidState,
                "presenter belongs to another Vulkan device",
            ));
        }
        Ok(())
    }
}

const fn state_after_suboptimal(policy: PresenterReconfigurePolicy) -> PresenterState {
    match policy {
        PresenterReconfigurePolicy::Eager => PresenterState::NeedsReconfigure,
        PresenterReconfigurePolicy::DeferSuboptimal => PresenterState::Ready,
    }
}

impl Drop for VulkanWinitPresenter {
    fn drop(&mut self) {
        let _ = self.retire_all_blocking();
        self.recovery.state = PresenterState::Shutdown;
    }
}

fn device_identity(device: &VulkanDevice) -> u64 {
    crate::graphics::renderers::vulkan::interop::device_id(device)
}

fn present_completion(
    swapchain_generation: u64,
    acquire_slot: usize,
    present_id: Option<u64>,
    present_fence: bool,
    disposition: PresentDisposition,
) -> Option<PresentCompletion> {
    if !matches!(
        disposition,
        PresentDisposition::Presented | PresentDisposition::PresentedSuboptimal
    ) {
        return None;
    }
    let proof = present_id
        .map(PresentCompletionProof::PresentId)
        .or_else(|| {
            present_fence.then_some(PresentCompletionProof::PresentFence { acquire_slot })
        })?;
    Some(PresentCompletion {
        swapchain_generation,
        proof,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        PresentCompletionProof, PresentDisposition, PresenterReconfigurePolicy, PresenterState,
        RetirementAnchor, present_completion, state_after_suboptimal, zero_timeout_acquire_stalled,
    };
    use std::time::Duration;

    #[test]
    fn retirement_waits_for_the_first_presented_image_to_be_reacquired() {
        let mut anchor = RetirementAnchor::default();
        anchor.note_acquire(2, 0);
        assert_eq!(anchor.acquire_slot, None);

        anchor.note_present(1);
        anchor.note_present(2);
        anchor.note_acquire(0, 1);
        assert_eq!(anchor.acquire_slot, None);

        anchor.note_acquire(1, 2);
        anchor.note_acquire(1, 0);
        assert_eq!(anchor.acquire_slot, Some(2));

        anchor.reset();
        assert_eq!(anchor, RetirementAnchor::default());
    }

    #[test]
    fn deferred_suboptimal_policy_keeps_the_current_generation_ready() {
        assert_eq!(
            state_after_suboptimal(PresenterReconfigurePolicy::Eager),
            PresenterState::NeedsReconfigure
        );
        assert_eq!(
            state_after_suboptimal(PresenterReconfigurePolicy::DeferSuboptimal),
            PresenterState::Ready
        );
    }

    #[test]
    fn zero_timeout_acquire_stall_threshold_rejects_normal_scheduler_jitter() {
        assert!(!zero_timeout_acquire_stalled(Duration::from_millis(99)));
        assert!(zero_timeout_acquire_stalled(Duration::from_millis(100)));
        assert!(zero_timeout_acquire_stalled(Duration::from_secs(2)));
    }

    #[test]
    fn exact_present_id_is_preferred_over_a_maintenance_fence() {
        let completion = present_completion(7, 2, Some(41), true, PresentDisposition::Presented)
            .expect("successful present should expose completion proof");
        assert_eq!(completion.swapchain_generation, 7);
        assert_eq!(completion.proof, PresentCompletionProof::PresentId(41));

        let fallback =
            present_completion(7, 2, None, true, PresentDisposition::PresentedSuboptimal)
                .expect("maintenance fence should remain a fallback");
        assert_eq!(
            fallback.proof,
            PresentCompletionProof::PresentFence { acquire_slot: 2 }
        );

        assert!(
            present_completion(7, 2, Some(42), true, PresentDisposition::NeedsReconfigure)
                .is_none()
        );
    }
}
