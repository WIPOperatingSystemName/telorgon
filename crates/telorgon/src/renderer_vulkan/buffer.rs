use crate::render::RenderResult;
use ash::vk;
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};

use crate::renderer_vulkan::device::DeviceInner;
use crate::renderer_vulkan::error::{internal, vk_error};

pub(crate) struct AllocatedBuffer {
    device: std::sync::Arc<DeviceInner>,
    raw: vk::Buffer,
    allocation: Option<Allocation>,
    size: vk::DeviceSize,
    device_local_reserved_bytes: u64,
}

impl AllocatedBuffer {
    pub(crate) fn new(
        device: std::sync::Arc<DeviceInner>,
        size: vk::DeviceSize,
        usage: vk::BufferUsageFlags,
        location: MemoryLocation,
        name: &str,
    ) -> RenderResult<Self> {
        let size = size.max(4);
        let raw = unsafe {
            device.raw.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }
        .map_err(|result| vk_error(format!("failed to create {name} buffer"), result))?;
        let requirements = unsafe { device.raw.get_buffer_memory_requirements(raw) };
        let reservation = if matches!(location, MemoryLocation::GpuOnly) {
            match device.reserve_device_local(requirements.size) {
                Ok(reservation) => Some(reservation),
                Err(error) => {
                    unsafe { device.raw.destroy_buffer(raw, None) };
                    return Err(error);
                }
            }
        } else {
            None
        };
        let allocation_result = device
            .memory
            .allocator()
            .lock()
            .map_err(|_| {
                unsafe { device.raw.destroy_buffer(raw, None) };
                internal("Vulkan allocator lock poisoned")
            })?
            .allocate(&AllocationCreateDesc {
                name,
                requirements,
                location,
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            });
        let allocation = match allocation_result {
            Ok(allocation) => allocation,
            Err(error) => {
                unsafe { device.raw.destroy_buffer(raw, None) };
                return Err(internal(format!(
                    "failed to allocate {name} buffer: {error}"
                )));
            }
        };
        if let Err(result) = unsafe {
            device
                .raw
                .bind_buffer_memory(raw, allocation.memory(), allocation.offset())
        } {
            unsafe { device.raw.destroy_buffer(raw, None) };
            if let Ok(mut allocator) = device.memory.allocator().lock() {
                let _ = allocator.free(allocation);
            }
            return Err(vk_error(format!("failed to bind {name} buffer"), result));
        }
        let device_local_reserved_bytes = reservation
            .map(|reservation| reservation.commit())
            .unwrap_or(0);
        Ok(Self {
            device,
            raw,
            allocation: Some(allocation),
            size,
            device_local_reserved_bytes,
        })
    }

    pub(crate) fn raw(&self) -> vk::Buffer {
        self.raw
    }

    pub(crate) fn size(&self) -> vk::DeviceSize {
        self.size
    }

    pub(crate) fn write(&self, bytes: &[u8]) -> RenderResult<()> {
        self.write_at(0, bytes)
    }

    pub(crate) fn write_at(&self, offset: vk::DeviceSize, bytes: &[u8]) -> RenderResult<()> {
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| internal("mapped Vulkan buffer write range overflow"))?;
        if end > self.size {
            return Err(internal("mapped Vulkan buffer write exceeds allocation"));
        }
        let allocation = self
            .allocation
            .as_ref()
            .ok_or_else(|| internal("Vulkan buffer allocation was already released"))?;
        let pointer = allocation
            .mapped_ptr()
            .ok_or_else(|| internal("Vulkan upload buffer is not host mapped"))?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                pointer.as_ptr().cast::<u8>().add(offset as usize),
                bytes.len(),
            );
        }
        if !allocation
            .memory_properties()
            .contains(vk::MemoryPropertyFlags::HOST_COHERENT)
        {
            unsafe {
                self.device
                    .raw
                    .flush_mapped_memory_ranges(&[vk::MappedMemoryRange::default()
                        .memory(allocation.memory())
                        .offset(allocation.offset())
                        .size(vk::WHOLE_SIZE)])
            }
            .map_err(|result| vk_error("failed to flush Vulkan mapped buffer", result))?;
        }
        Ok(())
    }

    pub(crate) fn read(&self, len: usize) -> RenderResult<Vec<u8>> {
        if len as u64 > self.size {
            return Err(internal("mapped Vulkan buffer read exceeds allocation"));
        }
        let mut bytes = vec![0; len];
        self.read_into(&mut bytes)?;
        Ok(bytes)
    }

    /// Copies completed device writes into caller-owned storage without allocating pixel data.
    /// The caller must prove GPU completion before calling this method.
    pub(crate) fn read_into(&self, bytes: &mut [u8]) -> RenderResult<()> {
        let len = bytes.len();
        if len as u64 > self.size {
            return Err(internal("mapped Vulkan buffer read exceeds allocation"));
        }
        let allocation = self
            .allocation
            .as_ref()
            .ok_or_else(|| internal("Vulkan buffer allocation was already released"))?;
        if !allocation
            .memory_properties()
            .contains(vk::MemoryPropertyFlags::HOST_COHERENT)
        {
            unsafe {
                self.device
                    .raw
                    .invalidate_mapped_memory_ranges(&[vk::MappedMemoryRange::default()
                        .memory(allocation.memory())
                        .offset(allocation.offset())
                        .size(vk::WHOLE_SIZE)])
            }
            .map_err(|result| vk_error("failed to invalidate Vulkan readback buffer", result))?;
        }
        let pointer = allocation
            .mapped_ptr()
            .ok_or_else(|| internal("Vulkan readback buffer is not host mapped"))?;
        unsafe {
            std::ptr::copy_nonoverlapping(pointer.as_ptr().cast(), bytes.as_mut_ptr(), len);
        }
        Ok(())
    }
}

impl Drop for AllocatedBuffer {
    fn drop(&mut self) {
        unsafe { self.device.raw.destroy_buffer(self.raw, None) };
        if let Some(allocation) = self.allocation.take()
            && let Ok(mut allocator) = self.device.memory.allocator().lock()
        {
            let _ = allocator.free(allocation);
        }
        self.device
            .release_device_local(self.device_local_reserved_bytes);
    }
}
