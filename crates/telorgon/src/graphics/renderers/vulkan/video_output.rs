//! Vulkan allocation and synchronous copies for reusable native PipeWire output buffers.
use super::{
    video_transfer::{ExportImage, pixel},
    *,
};
use crate::{
    integrations::pipewire::{MediaError, video_dma_buf},
    media::video::*,
};
use std::os::fd::AsFd;

// SAFETY: allocations match queried export-capable tuples, have fixed descriptors/layouts,
// and outlive native retirement. Copy implementation below finishes all GPU operations.
unsafe impl VideoGpuProducer for VulkanVideoTransfer {
    fn formats(&self) -> Vec<VideoDmaBufFormat> {
        let mut formats = Vec::new();
        for cap in &self.outputs {
            if let Some(pixel) = pixel(cap.drm_fourcc) {
                let format = VideoDmaBufFormat {
                    pixel,
                    modifier: cap.drm_modifier,
                    planes: 1,
                };
                if !formats.contains(&format) {
                    formats.push(format);
                }
            }
        }
        formats.truncate(256);
        formats
    }
    fn allocate(
        &mut self,
        format: VideoFormat,
        modifier: u64,
        byte_limit: usize,
    ) -> Result<Box<dyn VideoGpuOutputBuffer>, MediaError> {
        self.allocate_accounted(format, modifier, byte_limit, |_| Ok(()))
    }
}
impl VulkanVideoTransfer {
    pub(crate) fn allocate_accounted(
        &mut self,
        format: VideoFormat,
        modifier: u64,
        byte_limit: usize,
        mut reserve: impl FnMut(usize) -> Result<(), MediaError>,
    ) -> Result<Box<dyn VideoGpuOutputBuffer>, MediaError> {
        format.validate()?;
        let cap = self
            .outputs
            .iter()
            .find(|c| {
                pixel(c.drm_fourcc) == Some(format.pixel)
                    && c.drm_modifier == modifier
                    && c.max_extent.width >= format.width
                    && c.max_extent.height >= format.height
            })
            .ok_or(MediaError::Unsupported("Vulkan GPU producer tuple"))?;
        let allocation =
            ExportImage::new_accounted(&self.device, *cap, format, byte_limit, &mut reserve)?;
        let plane = allocation.export()?;
        if plane.allocation_size > byte_limit as u64 {
            return Err(MediaError::ResourceLimit("Vulkan output allocation budget"));
        }
        reserve(
            usize::try_from(plane.allocation_size)
                .map_err(|_| MediaError::ResourceLimit("Vulkan output allocation size"))?,
        )?;
        Ok(Box::new(Output {
            transfer: self.clone(),
            allocation,
            planes: vec![plane],
            format,
            initialized: false,
        }))
    }
}
struct Output {
    transfer: VulkanVideoTransfer,
    allocation: ExportImage,
    planes: Vec<VideoDmaBufPlane>,
    format: VideoFormat,
    initialized: bool,
}
// SAFETY: fixed allocation/descriptors and no asynchronous operations survive copy_from.
unsafe impl VideoGpuOutputBuffer for Output {
    fn planes(&self) -> &[VideoDmaBufPlane] {
        &self.planes
    }
    unsafe fn copy_from(&mut self, frame: &GpuVideoFrame) -> Result<(), MediaError> {
        if frame.format() != self.format {
            return Err(MediaError::InvalidArgument("GPU output format"));
        }
        let _source = frame.begin_import()?;
        if self.initialized {
            // Native ownership prevents new consumer submissions; these fences cover reads
            // still running after a consumer queued the buffer back to PipeWire.
            let fence = video_dma_buf::write_fence(self.planes[0].fd.as_fd())?;
            video_dma_buf::wait_fence(fence.as_fd())?;
        }
        let planes: Vec<_> = frame
            .data
            .planes
            .iter()
            .map(|p| BorrowedVideoDmaBufPlane {
                fd: p.fd.as_fd(),
                offset: p.offset,
                stride: p.stride,
                allocation_size: p.allocation_size,
            })
            .collect();
        self.transfer.copy_into(
            self.format,
            frame.modifier(),
            &planes,
            &self.allocation,
            self.initialized,
        )?;
        self.initialized = true;
        Ok(())
    }
}
