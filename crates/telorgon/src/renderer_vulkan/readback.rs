use std::sync::Arc;
use std::time::Duration;

use crate::core::{RectI, SizeI};
use crate::render::{
    ReadbackFormat, ReadbackImage, ReadbackRequest, RenderError, RenderErrorKind, RenderResult,
};
use ash::vk;
use gpu_allocator::MemoryLocation;

use crate::renderer_vulkan::buffer::AllocatedBuffer;
use crate::renderer_vulkan::error::internal;
use crate::renderer_vulkan::{SubmissionReceipt, VulkanDevice, VulkanFrameContext, VulkanTarget};

/// One reusable staging allocation. Pending work is pinned by the frame receipt even if dropped.
/// This is internal streaming plumbing, not a shell permission or a PipeWire buffer.
pub(crate) struct CaptureReadbackBuffer {
    device_id: u64,
    extent: SizeI,
    buffer: Arc<AllocatedBuffer>,
    pending_frame: Option<u64>,
}

impl CaptureReadbackBuffer {
    pub(crate) fn is_pending(&self) -> bool {
        self.pending_frame.is_some()
    }

    pub(crate) fn new(device: &VulkanDevice, extent: SizeI) -> RenderResult<Self> {
        let len = capture_byte_len(extent)?;
        Ok(Self {
            device_id: device.inner.id,
            extent,
            buffer: Arc::new(AllocatedBuffer::new(
                Arc::clone(&device.inner),
                len as u64,
                vk::BufferUsageFlags::TRANSFER_DST,
                MemoryLocation::GpuToCpu,
                "Telorgon capture staging",
            )?),
            pending_frame: None,
        })
    }

    /// Polls without waiting. On success, copies into a preallocated delivery buffer and enables
    /// staging reuse. The caller still owns the separate delivery-buffer consumer lifecycle.
    pub(crate) fn try_copy_into(
        &mut self,
        receipt: &mut SubmissionReceipt,
        bytes: &mut [u8],
    ) -> RenderResult<bool> {
        self.validate_receipt(receipt)?;
        if bytes.len() != capture_byte_len(self.extent)? {
            return Err(internal("capture delivery buffer has the wrong size"));
        }
        if !receipt.poll()? {
            return Ok(false);
        }
        self.buffer.read_into(bytes)?;
        self.pending_frame = None;
        Ok(true)
    }

    /// Retires a cancelled frame without exposing its pixels.
    pub(crate) fn try_discard(&mut self, receipt: &mut SubmissionReceipt) -> RenderResult<bool> {
        self.validate_receipt(receipt)?;
        if !receipt.poll()? {
            return Ok(false);
        }
        self.pending_frame = None;
        Ok(true)
    }

    fn validate_receipt(&self, receipt: &SubmissionReceipt) -> RenderResult<()> {
        let completion = receipt.completion();
        if completion.device_id() != self.device_id
            || self.pending_frame != Some(completion.frame_id())
        {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "capture readback and receipt belong to different frames",
            ));
        }
        Ok(())
    }
}

fn capture_byte_len(extent: SizeI) -> RenderResult<usize> {
    if extent.width <= 0 || extent.height <= 0 {
        return Err(internal("capture extent must be positive"));
    }
    let stride = (extent.width as u32)
        .checked_mul(4)
        .ok_or_else(|| internal("capture row size overflow"))?;
    (stride as usize)
        .checked_mul(extent.height as usize)
        .ok_or_else(|| internal("capture byte size overflow"))
}

pub struct PendingVulkanReadback {
    device_id: u64,
    frame_id: u64,
    buffer: Arc<AllocatedBuffer>,
    region: RectI,
    row_bytes: u32,
}

pub struct VulkanReadback {
    receipt: SubmissionReceipt,
    pending: PendingVulkanReadback,
}

impl VulkanFrameContext<'_> {
    /// Record after rendering a same-queue RGBA target that remains in color-attachment state.
    /// Restores that state, so readback does not break later rendering or target retirement.
    pub(crate) fn record_capture_readback(
        &mut self,
        target: &VulkanTarget<'_>,
        storage: &mut CaptureReadbackBuffer,
    ) -> RenderResult<()> {
        if storage.pending_frame.is_some()
            || storage.device_id != self.core.device.inner.id
            || target.device_id != storage.device_id
            || target.extent.width != storage.extent.width as u32
            || target.extent.height != storage.extent.height as u32
            || target.final_state != super::target::VulkanImageState::COLOR_ATTACHMENT
            || target.final_queue_family != vk::QUEUE_FAMILY_IGNORED
            || !matches!(
                target.format,
                vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB
            )
        {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "capture requires an idle staging buffer and a matching same-queue RGBA target",
            ));
        }
        let region = RectI {
            x: 0,
            y: 0,
            width: storage.extent.width,
            height: storage.extent.height,
        };
        self.record_readback_copy(target, region, Arc::clone(&storage.buffer), true)?;
        storage.pending_frame = Some(self.core.frame_id);
        Ok(())
    }

    pub fn record_readback(
        &mut self,
        target: &VulkanTarget<'_>,
        request: &ReadbackRequest,
    ) -> RenderResult<PendingVulkanReadback> {
        if request.format != ReadbackFormat::Rgba8 {
            return Err(RenderError::new(
                RenderErrorKind::Unsupported,
                "Vulkan readback supports only RGBA8",
            ));
        }
        let region = request.region;
        if region.x < 0
            || region.y < 0
            || region.width <= 0
            || region.height <= 0
            || region.x.saturating_add(region.width) > target.extent.width as i32
            || region.y.saturating_add(region.height) > target.extent.height as i32
        {
            return Err(RenderError::new(
                RenderErrorKind::InvalidTarget,
                "Vulkan readback region is outside the target",
            ));
        }
        let row_bytes = (region.width as u32)
            .checked_mul(4)
            .ok_or_else(|| internal("Vulkan readback row-byte overflow"))?;
        let byte_len = (row_bytes as u64)
            .checked_mul(region.height as u64)
            .ok_or_else(|| internal("Vulkan readback size overflow"))?;
        let buffer = Arc::new(AllocatedBuffer::new(
            self.core.device.inner.clone(),
            byte_len,
            vk::BufferUsageFlags::TRANSFER_DST,
            MemoryLocation::GpuToCpu,
            "Telorgon readback staging",
        )?);
        self.record_readback_copy(target, region, buffer, false)
    }

    fn record_readback_copy(
        &mut self,
        target: &VulkanTarget<'_>,
        region: RectI,
        buffer: Arc<AllocatedBuffer>,
        restore_color: bool,
    ) -> RenderResult<PendingVulkanReadback> {
        let row_bytes = region.width as u32 * 4;
        let byte_len = u64::from(row_bytes) * region.height as u64;
        let to_transfer = vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(target.image)
            .subresource_range(color_subresource());
        unsafe {
            self.core.device.inner.raw.cmd_pipeline_barrier2(
                self.core.command_buffer,
                &vk::DependencyInfo::default().image_memory_barriers(&[to_transfer]),
            );
            self.core.device.inner.raw.cmd_copy_image_to_buffer(
                self.core.command_buffer,
                target.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer.raw(),
                &[vk::BufferImageCopy::default()
                    .buffer_offset(0)
                    .buffer_row_length(0)
                    .buffer_image_height(0)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_offset(vk::Offset3D {
                        x: region.x,
                        y: region.y,
                        z: 0,
                    })
                    .image_extent(vk::Extent3D {
                        width: region.width as u32,
                        height: region.height as u32,
                        depth: 1,
                    })],
            );
            let host_barrier = vk::BufferMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::HOST)
                .dst_access_mask(vk::AccessFlags2::HOST_READ)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .buffer(buffer.raw())
                .offset(0)
                .size(byte_len);
            self.core.device.inner.raw.cmd_pipeline_barrier2(
                self.core.command_buffer,
                &vk::DependencyInfo::default().buffer_memory_barriers(&[host_barrier]),
            );
            if restore_color {
                let restore = vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::COPY)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_READ)
                    .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                    .dst_access_mask(
                        vk::AccessFlags2::COLOR_ATTACHMENT_READ
                            | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                    )
                    .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(target.image)
                    .subresource_range(color_subresource());
                self.core.device.inner.raw.cmd_pipeline_barrier2(
                    self.core.command_buffer,
                    &vk::DependencyInfo::default().image_memory_barriers(&[restore]),
                );
            }
        }
        self.core.buffers.push(Arc::clone(&buffer));
        Ok(PendingVulkanReadback {
            device_id: self.core.device.inner.id,
            frame_id: self.core.frame_id,
            buffer,
            region,
            row_bytes,
        })
    }
}

impl PendingVulkanReadback {
    pub fn bind_to_submission(self, receipt: SubmissionReceipt) -> RenderResult<VulkanReadback> {
        let completion = receipt.completion();
        if completion.device_id() != self.device_id || completion.frame_id() != self.frame_id {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "readback and submission belong to different Vulkan frames",
            ));
        }
        Ok(VulkanReadback {
            receipt,
            pending: self,
        })
    }
}

impl VulkanReadback {
    pub fn wait(mut self, timeout: Duration) -> RenderResult<ReadbackImage> {
        self.receipt.wait(timeout)?;
        let byte_len = self.pending.row_bytes as usize * self.pending.region.height as usize;
        let pixels = self.pending.buffer.read(byte_len)?;
        Ok(ReadbackImage {
            extent: SizeI {
                width: self.pending.region.width,
                height: self.pending.region.height,
            },
            row_bytes: self.pending.row_bytes as usize,
            pixels,
        })
    }
}

fn color_subresource() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    #[test]
    fn capture_sizes_reject_empty_negative_and_stride_overflow() {
        assert!(
            capture_byte_len(SizeI {
                width: 0,
                height: 1
            })
            .is_err()
        );
        assert!(
            capture_byte_len(SizeI {
                width: 1,
                height: -1
            })
            .is_err()
        );
        assert!(
            capture_byte_len(SizeI {
                width: i32::MAX,
                height: 1
            })
            .is_err()
        );
        assert_eq!(
            capture_byte_len(SizeI {
                width: 1920,
                height: 1080
            })
            .unwrap(),
            8_294_400
        );
    }

    #[test]
    #[ignore = "requires TELORGON_TEST_MODE=developer-hardware and Vulkan validation"]
    fn capture_staging_reuses_storage_after_exact_frame_completion() {
        use crate::core::ColorRgba8;
        use crate::render::{RenderBackend, RenderRequest, TargetLoad, TargetStore};
        use crate::renderer_vulkan::{
            DeviceSelection, OffscreenVulkanTarget, VulkanConfig, VulkanInstance,
        };
        assert_eq!(
            std::env::var("TELORGON_TEST_MODE").as_deref(),
            Ok("developer-hardware")
        );
        let config = VulkanConfig {
            enable_validation: true,
            ..Default::default()
        };
        let instance = VulkanInstance::load(&config, &[]).unwrap();
        let selection = DeviceSelection::best(&instance.adapters().unwrap()).unwrap();
        let device = VulkanDevice::create_owned(instance, &config, &selection, None).unwrap();
        let extent = SizeI {
            width: 16,
            height: 16,
        };
        let target = OffscreenVulkanTarget::new(&device, extent).unwrap();
        let mut scene = device.create_scene().unwrap();
        let mut source = crate::render::RenderScene::default();
        source.extent = crate::core::SizeF {
            width: 16.0,
            height: 16.0,
        };
        device
            .apply_scene_delta(&mut scene, &source.take_delta().unwrap())
            .unwrap();
        let mut storage = CaptureReadbackBuffer::new(&device, extent).unwrap();
        let original_buffer = storage.buffer.raw();
        let mut pixels = vec![0; capture_byte_len(extent).unwrap()];
        let mut previous_receipt = None;
        for color in [
            ColorRgba8::rgba(255, 0, 0, 255),
            ColorRgba8::rgba(0, 255, 0, 255),
        ] {
            let mut frame = device.begin_owned_frame().unwrap();
            {
                let mut context = frame.context_mut();
                device
                    .render(
                        &mut scene,
                        &mut context,
                        &target.target(),
                        &RenderRequest {
                            force: true,
                            load: TargetLoad::Clear(color),
                            store: TargetStore::Store,
                            region: None,
                        },
                    )
                    .unwrap();
                context
                    .record_capture_readback(&target.target(), &mut storage)
                    .unwrap();
                assert!(
                    context
                        .record_capture_readback(&target.target(), &mut storage)
                        .is_err()
                );
            }
            let mut receipt = frame.finish().unwrap().submit().unwrap();
            if let Some(previous) = &mut previous_receipt {
                assert!(storage.try_copy_into(previous, &mut pixels).is_err());
            }
            receipt.wait(Duration::from_secs(10)).unwrap();
            assert!(storage.try_copy_into(&mut receipt, &mut pixels).unwrap());
            assert_eq!(&pixels[..4], &[color.r, color.g, color.b, color.a]);
            assert_eq!(storage.buffer.raw(), original_buffer);
            assert!(storage.try_copy_into(&mut receipt, &mut pixels).is_err());
            previous_receipt = Some(receipt);
        }

        let mut frame = device.begin_owned_frame().unwrap();
        // The prior capture restored COLOR_ATTACHMENT_OPTIMAL; no additional render is needed.
        frame
            .context_mut()
            .record_capture_readback(&target.target(), &mut storage)
            .unwrap();
        let mut receipt = frame.finish().unwrap().submit().unwrap();
        receipt.wait(Duration::from_secs(10)).unwrap();
        assert!(storage.try_discard(&mut receipt).unwrap());
        assert!(storage.pending_frame.is_none());
    }
}
