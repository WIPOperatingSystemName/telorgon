use crate::render::RenderResult;
use ash::vk;
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};

use crate::renderer_vulkan::device::DeviceInner;
use crate::renderer_vulkan::error::{internal, vk_error};

pub(crate) struct AllocatedImage {
    device: std::sync::Arc<DeviceInner>,
    raw: vk::Image,
    view: vk::ImageView,
    allocation: Option<Allocation>,
    device_local_reserved_bytes: u64,
    pub(crate) format: vk::Format,
    pub(crate) extent: vk::Extent2D,
}

impl AllocatedImage {
    pub(crate) fn new_color_target(
        device: std::sync::Arc<DeviceInner>,
        extent: vk::Extent2D,
        format: vk::Format,
        name: &str,
    ) -> RenderResult<Self> {
        Self::new_with_usage(
            device,
            extent,
            format,
            vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC,
            name,
        )
    }

    pub(crate) fn new_color_target_traced(
        device: std::sync::Arc<DeviceInner>,
        extent: vk::Extent2D,
        format: vk::Format,
        name: &str,
        phase: &mut dyn FnMut(&'static str),
    ) -> RenderResult<Self> {
        Self::new_with_usage_traced(
            device,
            extent,
            format,
            vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC,
            name,
            phase,
        )
    }

    pub(crate) fn new_sampled(
        device: std::sync::Arc<DeviceInner>,
        extent: vk::Extent2D,
        format: vk::Format,
        name: &str,
    ) -> RenderResult<Self> {
        Self::new_with_usage(
            device,
            extent,
            format,
            vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST,
            name,
        )
    }

    fn new_with_usage(
        device: std::sync::Arc<DeviceInner>,
        extent: vk::Extent2D,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        name: &str,
    ) -> RenderResult<Self> {
        Self::new_with_usage_traced(device, extent, format, usage, name, &mut |_| {})
    }

    fn new_with_usage_traced(
        device: std::sync::Arc<DeviceInner>,
        extent: vk::Extent2D,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        name: &str,
        phase: &mut dyn FnMut(&'static str),
    ) -> RenderResult<Self> {
        phase("dmabuf_alloc_image");
        let raw = unsafe {
            device.raw.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
        }
        .map_err(|result| vk_error(format!("failed to create {name} image"), result))?;
        phase("dmabuf_alloc_requirements");
        let requirements = unsafe { device.raw.get_image_memory_requirements(raw) };
        let reservation = match device.reserve_device_local(requirements.size) {
            Ok(reservation) => reservation,
            Err(error) => {
                unsafe { device.raw.destroy_image(raw, None) };
                return Err(error);
            }
        };
        phase("dmabuf_alloc_lock_wait");
        let mut allocator = device.memory.allocator().lock().map_err(|_| {
            unsafe { device.raw.destroy_image(raw, None) };
            internal("Vulkan allocator lock poisoned")
        })?;
        phase("dmabuf_alloc_memory");
        let allocation_result = allocator.allocate(&AllocationCreateDesc {
            name,
            requirements,
            location: MemoryLocation::GpuOnly,
            linear: false,
            allocation_scheme: AllocationScheme::GpuAllocatorManaged,
        });
        drop(allocator);
        let allocation = match allocation_result {
            Ok(allocation) => allocation,
            Err(error) => {
                unsafe { device.raw.destroy_image(raw, None) };
                return Err(internal(format!(
                    "failed to allocate {name} image: {error}"
                )));
            }
        };
        phase("dmabuf_alloc_bind");
        if let Err(result) = unsafe {
            device
                .raw
                .bind_image_memory(raw, allocation.memory(), allocation.offset())
        } {
            unsafe { device.raw.destroy_image(raw, None) };
            if let Ok(mut allocator) = device.memory.allocator().lock() {
                let _ = allocator.free(allocation);
            }
            return Err(vk_error(format!("failed to bind {name} image"), result));
        }
        phase("dmabuf_alloc_view");
        let view_result = unsafe {
            device.raw.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(raw)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    }),
                None,
            )
        };
        let view = match view_result {
            Ok(view) => view,
            Err(result) => {
                unsafe { device.raw.destroy_image(raw, None) };
                if let Ok(mut allocator) = device.memory.allocator().lock() {
                    let _ = allocator.free(allocation);
                }
                return Err(vk_error(
                    format!("failed to create {name} image view"),
                    result,
                ));
            }
        };
        let device_local_reserved_bytes = reservation.commit();
        Ok(Self {
            device,
            raw,
            view,
            allocation: Some(allocation),
            device_local_reserved_bytes,
            format,
            extent,
        })
    }

    pub(crate) fn allocated_bytes(&self) -> u64 {
        self.device_local_reserved_bytes
    }

    pub(crate) fn raw(&self) -> vk::Image {
        self.raw
    }

    pub(crate) fn view(&self) -> vk::ImageView {
        self.view
    }

    pub(crate) fn device_id(&self) -> u64 {
        self.device.id
    }
}

impl Drop for AllocatedImage {
    fn drop(&mut self) {
        unsafe {
            self.device.raw.destroy_image_view(self.view, None);
            self.device.raw.destroy_image(self.raw, None);
        }
        if let Some(allocation) = self.allocation.take()
            && let Ok(mut allocator) = self.device.memory.allocator().lock()
        {
            let _ = allocator.free(allocation);
        }
        self.device
            .release_device_local(self.device_local_reserved_bytes);
    }
}
