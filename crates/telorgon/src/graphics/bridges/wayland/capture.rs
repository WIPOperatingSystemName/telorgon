//! Capture rendering bridge. Source eligibility and stream ownership belong to the host.

use crate::foundation::{ColorRgba8, SizeI};
use crate::graphics::render::{
    RenderError, RenderErrorKind, RenderRequest, RenderResult, TargetLoad, TargetStore,
};
use crate::graphics::renderers::vulkan::{
    CaptureReadbackBuffer, SubmissionReceipt, VulkanCaptureTarget, VulkanCompositePlacement,
    VulkanCompositeScene, VulkanDevice, VulkanFrameContext,
};

/// CPU fallback storage: a reusable sRGB attachment and GPU-to-CPU buffer.
struct CpuCaptureSlot {
    target: VulkanCaptureTarget,
    staging: CaptureReadbackBuffer,
    #[cfg(feature = "video-linux")]
    format: Option<crate::media::video::VideoFormat>,
}

/// One active capture job, with CPU readback or an asynchronous immutable GPU export.
pub(crate) struct VulkanCaptureSlot {
    #[cfg(feature = "video-linux")]
    metadata: crate::media::video::FrameMetadata,
    cpu: Option<CpuCaptureSlot>,
    #[cfg(feature = "video-linux")]
    gpu: Option<GpuCaptureSlot>,
}
#[cfg(feature = "video-linux")]
struct GpuCaptureSlot {
    renderer: crate::graphics::renderers::vulkan::VulkanVideoRenderer,
    pending: Option<crate::graphics::renderers::vulkan::VulkanPendingVideoFrame>,
    ready: Option<crate::media::video::GpuVideoFrame>,
}
#[cfg(feature = "video-linux")]
fn media_error(error: crate::integrations::pipewire::MediaError) -> RenderError {
    RenderError::new(RenderErrorKind::HostContract, error.to_string())
}

impl VulkanCaptureSlot {
    #[cfg(feature = "video-linux")]
    pub(crate) fn new_budgeted(
        device: &VulkanDevice,
        extent: SizeI,
        budget: std::sync::Arc<crate::media::video::MemoryBudget>,
    ) -> RenderResult<Self> {
        Ok(Self {
            metadata: Default::default(),
            cpu: Some(CpuCaptureSlot {
                target: VulkanCaptureTarget::new_budgeted(device, extent, budget.clone())?,
                staging: CaptureReadbackBuffer::new_budgeted(device, extent, budget)?,
                format: None,
            }),
            gpu: None,
        })
    }
    pub(crate) fn new(device: &VulkanDevice, extent: SizeI) -> RenderResult<Self> {
        Ok(Self {
            #[cfg(feature = "video-linux")]
            metadata: Default::default(),
            cpu: Some(CpuCaptureSlot {
                target: VulkanCaptureTarget::new(device, extent)?,
                staging: CaptureReadbackBuffer::new(device, extent)?,
                #[cfg(feature = "video-linux")]
                format: None,
            }),
            #[cfg(feature = "video-linux")]
            gpu: None,
        })
    }

    #[cfg(feature = "video-linux")]
    pub(crate) fn set_metadata(&mut self, metadata: crate::media::video::FrameMetadata) {
        self.metadata = metadata;
    }
    #[cfg(feature = "video-linux")]
    pub(crate) fn metadata(&self) -> crate::media::video::FrameMetadata {
        self.metadata.clone()
    }

    #[cfg(feature = "video-linux")]
    pub(crate) fn new_cpu_video(
        device: &VulkanDevice,
        format: crate::media::video::VideoFormat,
        budget: std::sync::Arc<crate::media::video::MemoryBudget>,
    ) -> RenderResult<Self> {
        crate::graphics::bridges::video_cpu::rgba_to_packed(&mut [], format.pixel)
            .map_err(media_error)?;
        let mut slot = Self::new_budgeted(
            device,
            SizeI {
                width: format.width as i32,
                height: format.height as i32,
            },
            budget,
        )?;
        slot.cpu.as_mut().unwrap().format = Some(format);
        Ok(slot)
    }
    #[cfg(feature = "video-linux")]
    pub(crate) fn video_format(&self) -> Option<crate::media::video::VideoFormat> {
        self.gpu
            .as_ref()
            .map(|g| g.renderer.format())
            .or_else(|| self.cpu.as_ref().and_then(|c| c.format))
    }
    #[cfg(feature = "video-linux")]
    pub(crate) fn new_gpu(
        device: &VulkanDevice,
        format: crate::media::video::VideoFormat,
        budget: std::sync::Arc<crate::media::video::MemoryBudget>,
    ) -> RenderResult<Self> {
        Ok(Self {
            #[cfg(feature = "video-linux")]
            metadata: Default::default(),
            cpu: None,
            gpu: Some(GpuCaptureSlot {
                renderer: crate::graphics::renderers::vulkan::VulkanVideoRenderer::new(
                    device.clone(),
                    format,
                    512 * 1024 * 1024,
                )
                .map_err(media_error)?
                .with_parent_budget(budget),
                pending: None,
                ready: None,
            }),
        })
    }
    #[cfg(feature = "video-linux")]
    pub(crate) fn is_gpu(&self) -> bool {
        self.gpu.is_some()
    }
    #[cfg(feature = "video-linux")]
    pub(crate) fn take_gpu_frame(&mut self) -> Option<crate::media::video::GpuVideoFrame> {
        self.gpu.as_mut()?.ready.take()
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
        #[cfg(feature = "video-linux")]
        if let Some(gpu) = &mut self.gpu {
            if gpu.pending.is_some() || gpu.ready.is_some() {
                return Err(RenderError::new(
                    RenderErrorKind::HostContract,
                    "GPU capture delivery is still pending",
                ));
            }
            gpu.pending = Some(
                gpu.renderer
                    .record_composite(scenes, placements, frame, self.metadata.clone())
                    .map_err(media_error)?,
            );
            return Ok(());
        }
        let cpu = self.cpu.as_mut().expect("capture storage kind");
        if cpu.staging.is_pending() {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "capture slot is still awaiting frame completion",
            ));
        }
        // Pin the destination even if the session disappears before submission completion.
        frame.core.images.push(cpu.target.image());
        let target = cpu.target.target();
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
        frame.record_capture_readback(&target, &mut cpu.staging)
    }

    pub(crate) fn try_finish(
        &mut self,
        receipt: &mut SubmissionReceipt,
        destination: &mut [u8],
    ) -> RenderResult<bool> {
        #[cfg(feature = "video-linux")]
        if let Some(gpu) = &mut self.gpu {
            let pending = gpu.pending.as_mut().ok_or_else(|| {
                RenderError::new(RenderErrorKind::HostContract, "no recorded GPU capture")
            })?;
            if let Some(frame) = pending.try_complete(receipt).map_err(media_error)? {
                gpu.ready = Some(frame);
                gpu.pending = None;
                return Ok(true);
            }
            return Ok(false);
        }
        let cpu = self.cpu.as_mut().expect("capture storage kind");
        if !cpu.staging.try_copy_into(receipt, destination)? {
            return Ok(false);
        }
        #[cfg(feature = "video-linux")]
        if let Some(format) = cpu.format {
            crate::graphics::bridges::video_cpu::rgba_to_packed(destination, format.pixel)
                .map_err(media_error)?;
        }
        Ok(true)
    }

    pub(crate) fn try_discard(&mut self, receipt: &mut SubmissionReceipt) -> RenderResult<bool> {
        #[cfg(feature = "video-linux")]
        if self.gpu.is_some() {
            if !self.try_finish(receipt, &mut [])? {
                return Ok(false);
            }
            self.take_gpu_frame();
            return Ok(true);
        }
        self.cpu
            .as_mut()
            .expect("capture storage kind")
            .staging
            .try_discard(receipt)
    }
}
