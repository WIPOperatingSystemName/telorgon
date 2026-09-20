use std::mem::ManuallyDrop;
use std::sync::Mutex;

use crate::graphics::render::RenderResult;
use gpu_allocator::vulkan::{Allocator, AllocatorCreateDesc};
use gpu_allocator::{AllocationSizes, AllocatorDebugSettings};

use crate::graphics::renderers::vulkan::error::internal;

pub(crate) struct VulkanMemory {
    allocator: ManuallyDrop<Mutex<Allocator>>,
}

impl VulkanMemory {
    pub(crate) fn new(desc: &AllocatorCreateDesc) -> RenderResult<Self> {
        let allocator = Allocator::new(desc)
            .map_err(|error| internal(format!("failed to create Vulkan allocator: {error}")))?;
        Ok(Self {
            allocator: ManuallyDrop::new(Mutex::new(allocator)),
        })
    }

    pub(crate) fn allocator(&self) -> &Mutex<Allocator> {
        &self.allocator
    }

    pub(crate) unsafe fn destroy(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.allocator) };
    }
}

pub(crate) fn allocator_desc(
    instance: ash::Instance,
    device: ash::Device,
    physical_device: ash::vk::PhysicalDevice,
) -> AllocatorCreateDesc {
    AllocatorCreateDesc {
        instance,
        device,
        physical_device,
        debug_settings: AllocatorDebugSettings::default(),
        buffer_device_address: false,
        allocation_sizes: AllocationSizes::default(),
    }
}
