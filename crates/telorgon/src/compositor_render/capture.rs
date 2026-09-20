//! Capture rendering bridge. Source eligibility and stream ownership belong to the host.

use crate::core::{ColorRgba8, SizeI};
use crate::render::{
    RenderError, RenderErrorKind, RenderRequest, RenderResult, TargetLoad, TargetStore,
};
use crate::renderer_vulkan::{
    CaptureReadbackBuffer, SubmissionReceipt, VulkanCaptureTarget, VulkanCompositePlacement,
    VulkanCompositeScene, VulkanDevice, VulkanFrameContext,
};

/// One bounded capture slot: a reusable sRGB attachment and a reusable GPU-to-CPU buffer.
/// Delivery buffers are owned by the stream adapter, not by this GPU slot.
pub(crate) struct VulkanCaptureSlot {
    target: VulkanCaptureTarget,
    staging: CaptureReadbackBuffer,
}

impl VulkanCaptureSlot {
    pub(crate) fn new(device: &VulkanDevice, extent: SizeI) -> RenderResult<Self> {
        Ok(Self {
            target: VulkanCaptureTarget::new(device, extent)?,
            staging: CaptureReadbackBuffer::new(device, extent)?,
        })
    }

    /// Records in the caller's existing frame; never submits or waits internally.
    /// Placements must describe the approved source, including the selected cursor policy.
    pub(crate) fn record<'frame>(
        &'frame mut self,
        device: &VulkanDevice,
        scenes: &mut [VulkanCompositeScene<'_>],
        placements: &[VulkanCompositePlacement],
        frame: &mut VulkanFrameContext<'frame>,
    ) -> RenderResult<()> {
        if self.staging.is_pending() {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "capture slot is still awaiting frame completion",
            ));
        }
        // Pin the destination even if the session disappears before submission completion.
        frame.core.images.push(self.target.image());
        let target = self.target.target();
        device.render_composite(
            scenes,
            placements,
            frame,
            &target,
            &RenderRequest {
                force: true,
                load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                store: TargetStore::Store,
                region: None,
            },
        )?;
        frame.record_capture_readback(&target, &mut self.staging)
    }

    pub(crate) fn try_copy_into(
        &mut self,
        receipt: &mut SubmissionReceipt,
        destination: &mut [u8],
    ) -> RenderResult<bool> {
        self.staging.try_copy_into(receipt, destination)
    }

    pub(crate) fn try_discard(&mut self, receipt: &mut SubmissionReceipt) -> RenderResult<bool> {
        self.staging.try_discard(receipt)
    }
}
