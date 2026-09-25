//! Optional screen video transport assembled from the active renderer's capabilities.
use super::*;
use crate::{
    graphics::{
        bridges::wayland::capture::VulkanCaptureSlot,
        renderers::vulkan::{VulkanDevice, VulkanVideoRenderer, VulkanVideoTransfer},
    },
    integrations::pipewire::MediaError,
    media::video::{
        PixelFormat, VideoDmaBufFormat, VideoFormat, VideoGpuOutputBuffer, VideoGpuProducer,
    },
    shell::capture::CaptureLayout,
};

impl VulkanShellRenderer {
    pub(in crate::host::linux_shell::renderer) fn capture_memory_budget(
        &self,
    ) -> std::sync::Arc<crate::media::video::MemoryBudget> {
        self.capture_gpu_budget.clone()
    }

    pub(in crate::host::linux_shell::renderer) fn capture_damage(
        &self,
        after: Option<u64>,
        layout: CaptureLayout,
    ) -> (u64, Vec<crate::media::video::VideoRect>) {
        (
            self.content_version,
            super::super::capture_damage::collect(
                &self.damage_history,
                self.content_version,
                after,
                layout,
            ),
        )
    }

    pub(in crate::host::linux_shell::renderer) fn screen_gpu_producer(
        &self,
        layout: CaptureLayout,
        fps: u32,
    ) -> Option<Box<dyn VideoGpuProducer>> {
        // Advertise GPU transport only when both approved-scene rendering and transport
        // allocation are supported. These probes do not capture pixels or submit GPU work.
        let transfer = VulkanVideoTransfer::new(self.device.clone()).ok()?;
        let pixels = [PixelFormat::Rgba8, PixelFormat::Bgra8, PixelFormat::RgbaF16]
            .into_iter()
            .filter(|pixel| {
                let format = VideoFormat {
                    pixel: *pixel,
                    color: if *pixel == PixelFormat::RgbaF16 {
                        crate::media::video::Colorimetry::LINEAR_BT709
                    } else {
                        crate::media::video::Colorimetry::SRGB
                    },
                    ..VideoFormat::rgba(layout.width(), layout.height(), fps)
                };
                VulkanVideoRenderer::new(self.device.clone(), format, 512 * 1024 * 1024).is_ok()
                    && VideoGpuProducer::formats(&transfer)
                        .iter()
                        .any(|f| f.pixel == *pixel)
            })
            .collect::<Vec<_>>();
        if pixels.is_empty() {
            return None;
        }
        Some(Box::new(ScreenGpuProducer {
            transfer,
            pixels,
            device: self.device.clone(),
            budget: self.capture_gpu_budget.clone(),
        }))
    }

    pub(in crate::host::linux_shell::renderer) fn allocate_video_capture(
        &self,
        layout: CaptureLayout,
        format: VideoFormat,
        gpu: bool,
    ) -> AppResult<CaptureBuffer> {
        if format.width != layout.width()
            || format.height != layout.height()
            || layout.stride() != layout.width() * 4
        {
            return Err(AppError::new(
                "capture layout differs from negotiated video",
            ));
        }
        let slot = if gpu {
            VulkanCaptureSlot::new_gpu(&self.device, format, self.capture_gpu_budget.clone())
        } else {
            VulkanCaptureSlot::new_cpu_video(&self.device, format, self.capture_gpu_budget.clone())
        }
        .map_err(app_error)?;
        Ok(CaptureBuffer {
            layout,
            slot,
            pixels: if gpu {
                Default::default()
            } else {
                crate::media::video::CapturePixels::new(
                    layout.byte_len(),
                    Some(self.capture_gpu_budget.clone()),
                )
                .map_err(app_error)?
            },
        })
    }
}

struct ScreenGpuProducer {
    device: VulkanDevice,
    transfer: VulkanVideoTransfer,
    pixels: Vec<PixelFormat>,
    budget: std::sync::Arc<crate::media::video::MemoryBudget>,
}
// SAFETY: only narrows the concrete producer's capabilities; allocation and ownership stay
// with its existing implementation. This prevents advertising formats the capture renderer
// cannot supply even when the transport device can allocate them.
unsafe impl VideoGpuProducer for ScreenGpuProducer {
    fn formats(&self) -> Vec<VideoDmaBufFormat> {
        VideoGpuProducer::formats(&self.transfer)
            .into_iter()
            .filter(|f| self.pixels.contains(&f.pixel))
            .collect()
    }
    fn allocate(
        &mut self,
        format: VideoFormat,
        modifier: u64,
        byte_limit: usize,
    ) -> Result<Box<dyn VideoGpuOutputBuffer>, MediaError> {
        if !self.pixels.contains(&format.pixel) {
            return Err(MediaError::Unsupported("screen GPU pixel format"));
        }
        // Resize can exceed a render target's extent even when transfer-only images fit.
        // Reject that fixation so native negotiation can fall back to shared memory.
        VulkanVideoRenderer::new(self.device.clone(), format, 512 * 1024 * 1024)?;
        let limit = byte_limit.min(self.budget.available());
        let mut charge = crate::media::video::MemoryReservation::new(self.budget.clone(), 0)?;
        let buffer = self
            .transfer
            .allocate_accounted(format, modifier, limit, |bytes| charge.resize(bytes))?;
        Ok(Box::new(BudgetedCaptureBuffer {
            buffer,
            _charge: charge,
        }))
    }
}

struct BudgetedCaptureBuffer {
    // Fields drop in order: native resources before their accounting charge.
    buffer: Box<dyn VideoGpuOutputBuffer>,
    _charge: crate::media::video::MemoryReservation,
}
// SAFETY: forwards the unchanged allocation and exclusive copy contract to the concrete
// Vulkan producer; the only additional ownership is its shared memory charge.
unsafe impl VideoGpuOutputBuffer for BudgetedCaptureBuffer {
    fn planes(&self) -> &[crate::media::video::VideoDmaBufPlane] {
        self.buffer.planes()
    }
    unsafe fn copy_from(
        &mut self,
        frame: &crate::media::video::GpuVideoFrame,
    ) -> Result<(), MediaError> {
        // SAFETY: caller supplies the underlying trait's exclusive native-buffer access.
        unsafe { self.buffer.copy_from(frame) }
    }
}
