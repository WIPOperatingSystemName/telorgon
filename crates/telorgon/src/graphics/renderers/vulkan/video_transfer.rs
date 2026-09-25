//! A completed GPU copy separates PipeWire buffer reuse from application image ownership.
use super::{device::DeviceInner, external_image::ExternalImageInner, *};
use crate::{
    graphics::render::{ImageAlphaMode, ImageColorEncoding},
    integrations::pipewire::{MediaError, video_dma_buf},
    media::video::*,
};
use ash::vk;
use ash::vk::Handle;
use std::{os::fd::FromRawFd, sync::Arc};
pub(super) fn error(e: impl std::fmt::Display) -> MediaError {
    MediaError::Native(e.to_string().chars().take(512).collect())
}
const COPY_USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::from_raw(
    vk::ImageUsageFlags::SAMPLED.as_raw()
        | vk::ImageUsageFlags::TRANSFER_SRC.as_raw()
        | vk::ImageUsageFlags::TRANSFER_DST.as_raw(),
);
/// GPU-copy backend for RGBA/BGRA/RGBx/BGRx and RGBA float16 DMA-BUF capture/production. It queries exact device
/// format/modifier capabilities, waits producer fences, copies without CPU pixel mapping,
/// and exports a completed independent allocation. Shared-memory fallback remains available.
/// This backend copies one memory plane without interpreting pixel color or alpha values.
/// As a VideoGpuProducer it owns reusable output allocations and waits consumer fences before
/// each overwrite; application submissions never replace the descriptors of a native buffer.
/// Each call waits GPU completion on the native control worker; never call it on UI/RT threads.
/// A stalled GPU driver can delay connection shutdown. No input buffer is released early.
#[derive(Clone)]
pub struct VulkanVideoTransfer {
    pub(super) device: VulkanDevice,
    inputs: Vec<VulkanDmaBufFormatCapability>,
    pub(super) outputs: Vec<VulkanDmaBufFormatCapability>,
}
impl VulkanVideoTransfer {
    pub fn new(device: VulkanDevice) -> Result<Self, MediaError> {
        if device.hosted.is_some() {
            return Err(MediaError::Unsupported(
                "video transfer requires an owned Vulkan device",
            ));
        }
        let inputs = device
            .dma_buf_import_capabilities(
                vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC,
            )
            .map_err(error)?;
        let mut outputs = device
            .dma_buf_import_capabilities(COPY_USAGE)
            .map_err(error)?;
        outputs.retain(|c| c.exportable() && c.plane_count == 1);
        outputs.sort_by_key(|c| (c.drm_modifier != 0, c.drm_modifier));
        if outputs.is_empty() {
            return Err(MediaError::Unsupported("Vulkan DMA-BUF export allocation"));
        }
        Ok(Self {
            device,
            inputs,
            outputs,
        })
    }
}
pub(super) fn pixel(fourcc: u32) -> Option<PixelFormat> {
    match fourcc {
        DRM_FORMAT_ABGR8888 => Some(PixelFormat::Rgba8),
        DRM_FORMAT_ABGR16161616F => Some(PixelFormat::RgbaF16),
        DRM_FORMAT_ARGB8888 => Some(PixelFormat::Bgra8),
        DRM_FORMAT_XBGR8888 => Some(PixelFormat::Rgbx8),
        DRM_FORMAT_XRGB8888 => Some(PixelFormat::Bgrx8),
        _ => None,
    }
}
pub(super) fn encoding(format: VideoFormat) -> Result<ImageColorEncoding, MediaError> {
    if format.color.range != ColorRange::Full
        || format.color.matrix != ColorMatrix::Rgb
        || format.color.primaries != ColorPrimaries::Bt709
    {
        return Err(MediaError::Unsupported(
            "Vulkan video requires known full-range BT.709 RGB",
        ));
    }
    match format.color.transfer {
        TransferFunction::Srgb => Ok(ImageColorEncoding::Srgb),
        TransferFunction::Linear => Ok(ImageColorEncoding::Linear),
        _ => Err(MediaError::Unsupported("Vulkan video transfer function")),
    }
}
// SAFETY: each copy waits the producer's exported implicit write fence and its own command
// fence before returning, including error paths. Outputs are dedicated, independently owned
// allocations; no native FD or raw pointer is retained after this method.
unsafe impl VideoGpuTransfer for VulkanVideoTransfer {
    fn formats(&self) -> Vec<VideoDmaBufFormat> {
        let mut result = Vec::new();
        for c in &self.inputs {
            if c.plane_count != 1
                || !self
                    .outputs
                    .iter()
                    .any(|o| o.format == c.format && o.drm_fourcc == c.drm_fourcc)
            {
                continue;
            }
            if let Some(pixel) = pixel(c.drm_fourcc) {
                let format = VideoDmaBufFormat {
                    pixel,
                    modifier: c.drm_modifier,
                    planes: 1,
                };
                if !result.contains(&format) {
                    result.push(format);
                }
            }
        }
        result.truncate(256);
        result
    }
    fn copy_to_owned(
        &mut self,
        input: BorrowedGpuVideoFrame<'_>,
        byte_limit: usize,
    ) -> Result<GpuVideoFrame, MediaError> {
        crate::media::video::gpu::validate_planes(input.format, input.modifier, input.planes)?;
        input.metadata.validate(input.format)?;
        let byte_limit = byte_limit
            .checked_sub(input.metadata.allocation_bytes())
            .ok_or(MediaError::ResourceLimit("GPU metadata budget"))?;
        if input.planes.len() != 1 {
            return Err(MediaError::Unsupported("Vulkan video memory plane count"));
        }
        let source = self
            .inputs
            .iter()
            .find(|c| {
                pixel(c.drm_fourcc) == Some(input.format.pixel)
                    && c.drm_modifier == input.modifier
                    && self
                        .outputs
                        .iter()
                        .any(|o| o.format == c.format && o.drm_fourcc == c.drm_fourcc)
            })
            .ok_or(MediaError::Unsupported(
                "video DMA-BUF tuple on this Vulkan device",
            ))?;
        let target = self
            .outputs
            .iter()
            .find(|c| {
                c.format == source.format
                    && c.drm_fourcc == source.drm_fourcc
                    && input.format.width <= c.max_extent.width
                    && input.format.height <= c.max_extent.height
            })
            .ok_or(MediaError::Unsupported("video GPU export extent"))?;
        let allocation = ExportImage::new(&self.device, *target, input.format, byte_limit)?;
        self.copy_into(
            input.format,
            input.modifier,
            input.planes,
            &allocation,
            false,
        )?;
        let plane = allocation.export()?;
        if plane.allocation_size > byte_limit as u64 {
            return Err(MediaError::ResourceLimit("GPU exported allocation budget"));
        }
        // SAFETY: completed copy, sole exported allocation owner, no future writer, exact
        // modifier/layout queried from the allocation. Destroying Vulkan handles below does
        // not free the underlying allocation retained by the exported DMA-BUF FD.
        unsafe {
            GpuVideoFrame::from_completed_dma_buf(
                input.format,
                allocation.modifier,
                vec![plane],
                input.metadata.clone(),
            )
        }
    }
}

impl VulkanVideoTransfer {
    pub(super) fn copy_into(
        &self,
        format: VideoFormat,
        modifier: u64,
        planes: &[BorrowedVideoDmaBufPlane<'_>],
        allocation: &ExportImage,
        initialized: bool,
    ) -> Result<(), MediaError> {
        crate::media::video::gpu::validate_planes(format, modifier, planes)?;
        if planes.len() != 1
            || format.width != allocation.extent.width
            || format.height != allocation.extent.height
        {
            return Err(MediaError::InvalidArgument(
                "GPU copy extent or plane count",
            ));
        }
        let source = self
            .inputs
            .iter()
            .find(|c| pixel(c.drm_fourcc) == Some(format.pixel) && c.drm_modifier == modifier)
            .ok_or(MediaError::Unsupported("GPU source tuple"))?;
        let p = planes[0];
        let acquire = video_dma_buf::read_fence(p.fd)?;
        // SAFETY: the transfer input contract and checked native layout establish allocation
        // identity/lifetime. This method owns all imported resources until copy completion.
        let mut imported = unsafe {
            self.device.import_dma_buf(VulkanDmaBufImport {
                planes: vec![VulkanDmaBufPlane {
                    memory: p.fd.try_clone_to_owned().map_err(error)?,
                    memory_index: 0,
                    offset: p.offset,
                    size: p.allocation_size - p.offset,
                    row_pitch: p.stride,
                    allocation_size: p.allocation_size,
                }],
                drm_fourcc: source.drm_fourcc,
                drm_modifier: source.drm_modifier,
                format: source.format,
                extent: vk::Extent2D {
                    width: format.width,
                    height: format.height,
                },
                usage: source.usage,
                content_version: 1,
                lease_generation: 1,
                color_encoding: source.color_encoding,
                alpha_mode: source.alpha_mode,
                origin: VulkanExternalImageOrigin::TopLeft,
                initial_use: HostedImageUse::General,
                final_use: HostedImageUse::General,
                acquire: Some(acquire),
                damage: Vec::new(),
                protected: false,
            })
        }
        .map_err(error)?;
        let imported = imported.take_inner().map_err(error)?;
        Commands::new(self.device.inner.clone())?.copy(&imported, allocation, initialized)
    }
}
pub(super) struct ExportImage {
    device: Arc<DeviceInner>,
    pub(super) image: vk::Image,
    memory: vk::DeviceMemory,
    extent: vk::Extent3D,
    modifier: u64,
    layout: vk::SubresourceLayout,
    bytes: u64,
}
impl ExportImage {
    pub(super) fn new(
        device: &VulkanDevice,
        cap: VulkanDmaBufFormatCapability,
        format: VideoFormat,
        limit: usize,
    ) -> Result<Self, MediaError> {
        Self::new_accounted(device, cap, format, limit, |_| Ok(()))
    }
    pub(super) fn new_accounted(
        device: &VulkanDevice,
        cap: VulkanDmaBufFormatCapability,
        format: VideoFormat,
        limit: usize,
        reserve: impl FnOnce(usize) -> Result<(), MediaError>,
    ) -> Result<Self, MediaError> {
        let extent = vk::Extent3D {
            width: format.width,
            height: format.height,
            depth: 1,
        };
        let mut result = Self {
            device: device.inner.clone(),
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
            extent,
            modifier: cap.drm_modifier,
            layout: Default::default(),
            bytes: 0,
        };
        let modifiers = [cap.drm_modifier];
        let mut drm =
            vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&modifiers);
        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(cap.format)
            .extent(extent)
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(cap.usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut drm)
            .push_next(&mut external);
        // SAFETY: exact queried export-capable tuple; all outputs are guarded immediately.
        result.image = unsafe { device.inner.raw.create_image(&info, None) }.map_err(error)?;
        let requirements = unsafe { device.inner.raw.get_image_memory_requirements(result.image) };
        if requirements.size > limit as u64 {
            return Err(MediaError::ResourceLimit("GPU video copy allocation"));
        }
        // The caller owns the charge through allocation failure and native retirement.
        // Charge the driver's aligned requirement before allocating device memory.
        reserve(
            usize::try_from(requirements.size)
                .map_err(|_| MediaError::ResourceLimit("GPU allocation size"))?,
        )?;
        let properties = unsafe {
            device
                .inner
                .instance
                .inner
                .raw
                .get_physical_device_memory_properties(device.inner.physical_device)
        };
        let index = (0..properties.memory_type_count)
            .filter(|i| requirements.memory_type_bits & (1u32 << i) != 0)
            .min_by_key(|i| {
                !properties.memory_types[*i as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .ok_or(MediaError::Unsupported("GPU export memory type"))?;
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(result.image);
        let mut export = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let allocate = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(index)
            .push_next(&mut dedicated)
            .push_next(&mut export);
        result.memory =
            unsafe { device.inner.raw.allocate_memory(&allocate, None) }.map_err(error)?;
        unsafe {
            device
                .inner
                .raw
                .bind_image_memory(result.image, result.memory, 0)
        }
        .map_err(error)?;
        result.bytes = requirements.size;
        let mut selected = vk::ImageDrmFormatModifierPropertiesEXT::default();
        unsafe {
            ash::ext::image_drm_format_modifier::Device::new(
                &device.inner.instance.inner.raw,
                &device.inner.raw,
            )
            .get_image_drm_format_modifier_properties(result.image, &mut selected)
        }
        .map_err(error)?;
        if selected.drm_format_modifier != cap.drm_modifier {
            return Err(MediaError::InvalidArgument(
                "GPU allocator selected an unadvertised modifier",
            ));
        }
        result.layout = unsafe {
            device.inner.raw.get_image_subresource_layout(
                result.image,
                vk::ImageSubresource {
                    aspect_mask: vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
                    mip_level: 0,
                    array_layer: 0,
                },
            )
        };
        Ok(result)
    }
    pub(super) fn export(&self) -> Result<VideoDmaBufPlane, MediaError> {
        let stride: u32 = self
            .layout
            .row_pitch
            .try_into()
            .map_err(|_| MediaError::InvalidArgument("GPU row pitch"))?;
        let loader = ash::khr::external_memory_fd::Device::new(
            &self.device.instance.inner.raw,
            &self.device.raw,
        );
        let info = vk::MemoryGetFdInfoKHR::default()
            .memory(self.memory)
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let fd = unsafe { loader.get_memory_fd(&info) }.map_err(error)?;
        if fd < 0 {
            return Err(MediaError::InvalidArgument("GPU exported no DMA-BUF"));
        }
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
        let bytes = video_dma_buf::allocation_size(std::os::fd::AsFd::as_fd(&fd))?;
        Ok(VideoDmaBufPlane {
            fd,
            offset: self.layout.offset,
            stride,
            allocation_size: bytes,
        })
    }
}
impl Drop for ExportImage {
    fn drop(&mut self) {
        unsafe {
            if !self.image.is_null() {
                self.device.raw.destroy_image(self.image, None);
            }
            if !self.memory.is_null() {
                self.device.raw.free_memory(self.memory, None);
            }
        }
    }
}
struct Commands {
    device: Arc<DeviceInner>,
    pool: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
}
impl Commands {
    fn new(device: Arc<DeviceInner>) -> Result<Self, MediaError> {
        let mut result = Self {
            device,
            pool: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
        };
        result.pool = unsafe {
            result.device.raw.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(result.device.queue_family)
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                None,
            )
        }
        .map_err(error)?;
        result.command = unsafe {
            result.device.raw.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(result.pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .map_err(error)?[0];
        result.fence = unsafe {
            result
                .device
                .raw
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .map_err(error)?;
        Ok(result)
    }
    fn copy(
        &self,
        source: &ExternalImageInner,
        destination: &ExportImage,
        initialized: bool,
    ) -> Result<(), MediaError> {
        let raw = &self.device.raw;
        let range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        let barrier = |image, old, new, src, dst, from, to| {
            vk::ImageMemoryBarrier::default()
                .image(image)
                .subresource_range(range)
                .old_layout(old)
                .new_layout(new)
                .src_access_mask(src)
                .dst_access_mask(dst)
                .src_queue_family_index(from)
                .dst_queue_family_index(to)
        };
        // SAFETY: private command pool, freshly imported source and fresh export target.
        unsafe {
            raw.begin_command_buffer(
                self.command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
        }
        .map_err(error)?;
        unsafe {
            raw.cmd_pipeline_barrier(
                self.command,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[
                    barrier(
                        source.image,
                        vk::ImageLayout::GENERAL,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        vk::AccessFlags::empty(),
                        vk::AccessFlags::TRANSFER_READ,
                        vk::QUEUE_FAMILY_FOREIGN_EXT,
                        self.device.queue_family,
                    ),
                    barrier(
                        destination.image,
                        if initialized {
                            vk::ImageLayout::GENERAL
                        } else {
                            vk::ImageLayout::UNDEFINED
                        },
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        vk::AccessFlags::empty(),
                        vk::AccessFlags::TRANSFER_WRITE,
                        if initialized {
                            vk::QUEUE_FAMILY_FOREIGN_EXT
                        } else {
                            vk::QUEUE_FAMILY_IGNORED
                        },
                        if initialized {
                            self.device.queue_family
                        } else {
                            vk::QUEUE_FAMILY_IGNORED
                        },
                    ),
                ],
            );
            let layers = vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            };
            raw.cmd_copy_image(
                self.command,
                source.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                destination.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[vk::ImageCopy::default()
                    .src_subresource(layers)
                    .dst_subresource(layers)
                    .extent(destination.extent)],
            );
            raw.cmd_pipeline_barrier(
                self.command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[
                    barrier(
                        source.image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        vk::ImageLayout::GENERAL,
                        vk::AccessFlags::TRANSFER_READ,
                        vk::AccessFlags::empty(),
                        self.device.queue_family,
                        vk::QUEUE_FAMILY_FOREIGN_EXT,
                    ),
                    barrier(
                        destination.image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        vk::ImageLayout::GENERAL,
                        vk::AccessFlags::TRANSFER_WRITE,
                        vk::AccessFlags::empty(),
                        self.device.queue_family,
                        vk::QUEUE_FAMILY_FOREIGN_EXT,
                    ),
                ],
            );
            raw.end_command_buffer(self.command)
        }
        .map_err(error)?;
        let waits = match source.acquire {
            VulkanExternalAcquire::BinarySemaphore(s) => vec![s],
            _ => Vec::new(),
        };
        let stages = vec![vk::PipelineStageFlags::TRANSFER; waits.len()];
        let signals = match source.release {
            VulkanExternalRelease::BinarySemaphore(s) => vec![s],
            _ => Vec::new(),
        };
        let commands = [self.command];
        let submit = vk::SubmitInfo::default()
            .command_buffers(&commands)
            .wait_semaphores(&waits)
            .wait_dst_stage_mask(&stages)
            .signal_semaphores(&signals);
        {
            let _queue = self
                .device
                .queue_lock
                .lock()
                .map_err(|_| MediaError::Native("Vulkan queue lock poisoned".into()))?;
            unsafe { raw.queue_submit(self.device.queue, &[submit], self.fence) }.map_err(error)?;
        }
        // Never release input storage on a host timeout while a GPU read is still in flight.
        // Device loss terminates GPU use; other transient host errors keep resources pinned.
        loop {
            match unsafe { raw.wait_for_fences(&[self.fence], true, u64::MAX) } {
                Ok(()) => return Ok(()),
                Err(vk::Result::ERROR_DEVICE_LOST) => {
                    return Err(error("Vulkan device lost during video copy"));
                }
                Err(_) => std::thread::yield_now(),
            }
        }
    }
}
impl Drop for Commands {
    fn drop(&mut self) {
        unsafe {
            if !self.fence.is_null() {
                self.device.raw.destroy_fence(self.fence, None);
            }
            if !self.pool.is_null() {
                self.device.raw.destroy_command_pool(self.pool, None);
            }
        }
    }
}

impl VulkanDevice {
    /// Import an immutable completed media image for one renderer lease. The image/accounting
    /// owner remains pinned through GPU completion. `revision` must increase for each binding
    /// to the same ImageId. Crop/transform/cursor stay in the frame metadata for host placement;
    /// this imports the complete coded extent. Alpha association is an explicit host assertion.
    /// One frame may have only one live renderer lease (including on other devices); drop its
    /// scene binding and complete GPU work before importing that same allocation again.
    pub fn import_video_frame(
        &self,
        frame: GpuVideoFrame,
        revision: u64,
        alpha: ImageAlphaMode,
    ) -> Result<VulkanExternalImageLease, MediaError> {
        if revision == 0 {
            return Err(MediaError::InvalidArgument("GPU video revision"));
        }
        let owner = frame.begin_import()?;
        let color = encoding(frame.format())?;
        let capabilities = self
            .dma_buf_import_capabilities(vk::ImageUsageFlags::SAMPLED)
            .map_err(error)?;
        let cap = capabilities
            .iter()
            .find(|c| {
                pixel(c.drm_fourcc) == Some(frame.format().pixel)
                    && c.drm_modifier == frame.modifier()
                    && c.color_encoding == color
                    && c.plane_count == frame.data.planes.len() as u32
            })
            .ok_or(MediaError::Unsupported("renderer GPU video tuple"))?;
        let planes = frame
            .data
            .planes
            .iter()
            .map(|p| {
                Ok(VulkanDmaBufPlane {
                    memory: p.fd.try_clone().map_err(error)?,
                    memory_index: 0,
                    offset: p.offset,
                    size: p.allocation_size - p.offset,
                    row_pitch: p.stride,
                    allocation_size: p.allocation_size,
                })
            })
            .collect::<Result<Vec<_>, MediaError>>()?;
        let damage = frame
            .metadata()
            .damage
            .iter()
            .map(|r| crate::foundation::RectI {
                x: r.x as i32,
                y: r.y as i32,
                width: r.width as i32,
                height: r.height as i32,
            })
            .collect();
        // SAFETY: only trusted completed exporters construct GpuVideoFrame. The retained owner
        // below keeps immutable allocations/accounting alive through renderer receipt completion.
        let mut lease = unsafe {
            self.import_dma_buf(VulkanDmaBufImport {
                planes,
                drm_fourcc: cap.drm_fourcc,
                drm_modifier: cap.drm_modifier,
                format: cap.format,
                extent: vk::Extent2D {
                    width: frame.format().width,
                    height: frame.format().height,
                },
                usage: cap.usage,
                content_version: revision,
                lease_generation: revision,
                color_encoding: color,
                alpha_mode: alpha,
                origin: VulkanExternalImageOrigin::TopLeft,
                initial_use: HostedImageUse::General,
                final_use: HostedImageUse::General,
                acquire: None,
                damage,
                protected: false,
            })
        }
        .map_err(error)?;
        let inner = Arc::get_mut(lease.inner.as_mut().unwrap()).ok_or(
            MediaError::InvalidArgument("shared video import before binding"),
        )?;
        inner.retained_owner = Some(Arc::new(owner));
        Ok(lease)
    }
}
