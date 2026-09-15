use std::ops::Range;
use std::sync::Arc;

use crate::gpu_abi::GpuView;
use crate::render::{RenderError, RenderErrorKind, RenderResult};
use ash::vk;
use bytemuck::Pod;
use gpu_allocator::MemoryLocation;

use crate::renderer_vulkan::buffer::AllocatedBuffer;
use crate::renderer_vulkan::device::DeviceInner;
use crate::renderer_vulkan::image::AllocatedImage;

const MIN_SCENE_BUFFER_BYTES: u64 = 256;
const COPY_ALIGNMENT: usize = 4;

#[derive(Default)]
pub(crate) struct RetainedGpuBuffer {
    buffer: Option<Arc<AllocatedBuffer>>,
    capacity: u64,
    generation: u64,
    initialized: bool,
}

impl RetainedGpuBuffer {
    pub(crate) fn ensure(
        &mut self,
        device: &Arc<DeviceInner>,
        required: u64,
        name: &str,
    ) -> RenderResult<bool> {
        let required = required.max(4);
        if self.capacity >= required && self.buffer.is_some() {
            return Ok(false);
        }
        let capacity = geometric_capacity(self.capacity, required);
        self.buffer = Some(Arc::new(AllocatedBuffer::new(
            Arc::clone(device),
            capacity,
            vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryLocation::GpuOnly,
            name,
        )?));
        self.capacity = capacity;
        self.generation = self.generation.saturating_add(1);
        self.initialized = false;
        Ok(true)
    }

    pub(crate) fn buffer(&self) -> &Arc<AllocatedBuffer> {
        self.buffer
            .as_ref()
            .expect("retained Vulkan buffer must be allocated before use")
    }

    pub(crate) fn is_allocated(&self) -> bool {
        self.buffer.is_some()
    }

    pub(crate) fn capacity(&self) -> u64 {
        self.capacity
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn initialized(&self) -> bool {
        self.initialized
    }

    pub(crate) fn mark_initialized(&mut self) {
        self.initialized = true;
    }
}

pub(crate) struct UploadChunk {
    pub(crate) destination_offset: u64,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) struct BufferUploadGroup {
    pub(crate) buffer: Arc<AllocatedBuffer>,
    pub(crate) previously_initialized: bool,
    pub(crate) chunks: Vec<UploadChunk>,
}

#[derive(Default)]
pub(crate) struct SceneUploadPlan {
    pub(crate) groups: Vec<BufferUploadGroup>,
    pub(crate) image_groups: Vec<ImageUploadGroup>,
    pub(crate) byte_count: u64,
    pub(crate) buffer_allocations: u32,
    pub(crate) buffer_growths: u32,
}

pub(crate) struct ImageUploadGroup {
    pub(crate) image: Arc<AllocatedImage>,
    pub(crate) previously_initialized: bool,
    pub(crate) preserve_from: Option<Arc<AllocatedImage>>,
    pub(crate) bytes_per_pixel: u32,
    pub(crate) chunks: Vec<ImageUploadChunk>,
}

pub(crate) struct ImageUploadChunk {
    pub(crate) offset: vk::Offset3D,
    pub(crate) extent: vk::Extent3D,
    pub(crate) row_bytes: usize,
    pub(crate) bytes: Vec<u8>,
}

impl SceneUploadPlan {
    pub(crate) fn push_pod_ranges<T: Pod>(
        &mut self,
        retained: &RetainedGpuBuffer,
        values: &[T],
        ranges: &[Range<usize>],
    ) {
        let stride = size_of::<T>();
        let mut chunks = Vec::new();
        for range in ranges {
            let start = range.start.min(values.len());
            let end = range.end.min(values.len()).max(start);
            if start == end {
                continue;
            }
            let bytes = bytemuck::cast_slice(&values[start..end]).to_vec();
            self.byte_count += bytes.len() as u64;
            chunks.push(UploadChunk {
                destination_offset: (start * stride) as u64,
                bytes,
            });
        }
        if !chunks.is_empty() {
            self.groups.push(BufferUploadGroup {
                buffer: Arc::clone(retained.buffer()),
                previously_initialized: retained.initialized(),
                chunks,
            });
        }
    }

    pub(crate) fn push_image_uploads(
        &mut self,
        image: Arc<AllocatedImage>,
        previously_initialized: bool,
        preserve_from: Option<Arc<AllocatedImage>>,
        bytes_per_pixel: u32,
        chunks: Vec<ImageUploadChunk>,
    ) {
        if chunks.is_empty() {
            return;
        }
        self.byte_count += chunks
            .iter()
            .map(|chunk| chunk.bytes.len() as u64)
            .sum::<u64>();
        self.image_groups.push(ImageUploadGroup {
            image,
            previously_initialized,
            preserve_from,
            bytes_per_pixel,
            chunks,
        });
    }
}

pub(crate) struct StagedDestination {
    pub(crate) buffer: Arc<AllocatedBuffer>,
    pub(crate) previously_initialized: bool,
    pub(crate) regions: Vec<vk::BufferCopy2<'static>>,
}

pub(crate) struct StagedUploads {
    pub(crate) bytes: Vec<u8>,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) view_offset: u64,
    pub(crate) destinations: Vec<StagedDestination>,
    pub(crate) image_destinations: Vec<StagedImageDestination>,
    pub(crate) byte_count: u64,
}

pub(crate) struct StagedImageDestination {
    pub(crate) image: Arc<AllocatedImage>,
    pub(crate) previously_initialized: bool,
    pub(crate) preserve_from: Option<Arc<AllocatedImage>>,
    pub(crate) regions: Vec<vk::BufferImageCopy2<'static>>,
}

impl StagedUploads {
    pub(crate) fn build(
        view: &GpuView,
        plan: SceneUploadPlan,
        staging_capacity: u64,
    ) -> RenderResult<Self> {
        let mut bytes = Vec::new();
        let mut staged = Self::append(
            view,
            plan,
            staging_capacity,
            &mut bytes,
            align_of::<GpuView>(),
        )?;
        staged.bytes = bytes;
        Ok(staged)
    }

    /// Appends one scene's uniform and uploads to a shared frame staging stream.
    pub(crate) fn append(
        view: &GpuView,
        plan: SceneUploadPlan,
        staging_capacity: u64,
        bytes: &mut Vec<u8>,
        view_alignment: usize,
    ) -> RenderResult<Self> {
        Self::append_at(view, plan, staging_capacity, bytes, view_alignment, 0)
    }

    /// Append only this pass's suffix, while GPU offsets remain absolute in the frame stream.
    pub(crate) fn append_at(
        view: &GpuView,
        plan: SceneUploadPlan,
        staging_capacity: u64,
        bytes: &mut Vec<u8>,
        view_alignment: usize,
        base: usize,
    ) -> RenderResult<Self> {
        align_vec_at(bytes, view_alignment.max(align_of::<GpuView>()), base);
        let view_offset = (base + bytes.len()) as u64;
        bytes.extend_from_slice(bytemuck::bytes_of(view));
        let mut destinations = Vec::with_capacity(plan.groups.len());
        for group in plan.groups {
            let mut regions = Vec::with_capacity(group.chunks.len());
            for chunk in group.chunks {
                align_vec_at(bytes, COPY_ALIGNMENT, base);
                let source_offset = (base + bytes.len()) as u64;
                let size = chunk.bytes.len() as u64;
                bytes.extend_from_slice(&chunk.bytes);
                regions.push(
                    vk::BufferCopy2::default()
                        .src_offset(source_offset)
                        .dst_offset(chunk.destination_offset)
                        .size(size),
                );
            }
            destinations.push(StagedDestination {
                buffer: group.buffer,
                previously_initialized: group.previously_initialized,
                regions,
            });
        }
        let mut image_destinations = Vec::with_capacity(plan.image_groups.len());
        for group in plan.image_groups {
            let mut regions = Vec::with_capacity(group.chunks.len());
            for chunk in group.chunks {
                align_vec_at(bytes, COPY_ALIGNMENT, base);
                let source_offset = (base + bytes.len()) as u64;
                bytes.extend_from_slice(&chunk.bytes);
                regions.push(
                    vk::BufferImageCopy2::default()
                        .buffer_offset(source_offset)
                        .buffer_row_length(chunk.row_bytes as u32 / group.bytes_per_pixel)
                        .buffer_image_height(chunk.extent.height)
                        .image_subresource(vk::ImageSubresourceLayers {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            mip_level: 0,
                            base_array_layer: 0,
                            layer_count: 1,
                        })
                        .image_offset(chunk.offset)
                        .image_extent(chunk.extent),
                );
            }
            image_destinations.push(StagedImageDestination {
                image: group.image,
                previously_initialized: group.previously_initialized,
                preserve_from: group.preserve_from,
                regions,
            });
        }
        if (base + bytes.len()) as u64 > staging_capacity {
            return Err(RenderError::new(
                RenderErrorKind::OutOfMemory,
                format!(
                    "Vulkan frame staging requires {} bytes but its reusable slot provides {staging_capacity}",
                    base + bytes.len()
                ),
            ));
        }
        Ok(Self {
            bytes: Vec::new(),
            view_offset,
            destinations,
            image_destinations,
            byte_count: plan.byte_count,
        })
    }

    pub(crate) fn copy_count(&self) -> u32 {
        (self.destinations.len()
            + self.image_destinations.len()
            + self
                .image_destinations
                .iter()
                .filter(|destination| destination.preserve_from.is_some())
                .count()) as u32
    }

    pub(crate) fn retained_images(&self) -> impl Iterator<Item = Arc<AllocatedImage>> + '_ {
        self.image_destinations.iter().flat_map(|destination| {
            std::iter::once(Arc::clone(&destination.image))
                .chain(destination.preserve_from.iter().map(Arc::clone))
        })
    }
}

pub(crate) fn geometric_capacity(current: u64, required: u64) -> u64 {
    let mut capacity = current.max(MIN_SCENE_BUFFER_BYTES);
    while capacity < required {
        let doubled = capacity.saturating_mul(2);
        if doubled == capacity {
            return required;
        }
        capacity = doubled;
    }
    capacity
}

fn align_vec(bytes: &mut Vec<u8>, alignment: usize) {
    let remainder = bytes.len() % alignment;
    if remainder != 0 {
        bytes.resize(bytes.len() + alignment - remainder, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::RenderErrorKind;
    use bytemuck::Zeroable;

    #[test]
    fn suffix_staging_uses_absolute_offsets_without_initializing_prefix() {
        let view = GpuView::zeroed();
        let base = 1024 * 1024 + 3;
        let mut suffix = Vec::new();
        let staged = StagedUploads::append_at(
            &view,
            SceneUploadPlan::default(),
            2 * 1024 * 1024,
            &mut suffix,
            256,
            base,
        )
        .unwrap();
        assert_eq!(staged.view_offset, 1024 * 1024 + 256);
        assert_eq!(suffix.len(), 253 + size_of::<GpuView>());
        assert!(
            StagedUploads::append_at(
                &view,
                SceneUploadPlan::default(),
                base as u64,
                &mut Vec::new(),
                256,
                base
            )
            .is_err()
        );
    }

    #[test]
    fn geometric_growth_is_stable_until_capacity_is_exceeded() {
        assert_eq!(geometric_capacity(0, 1), 256);
        assert_eq!(geometric_capacity(256, 128), 256);
        assert_eq!(geometric_capacity(256, 257), 512);
        assert_eq!(geometric_capacity(512, 2_000), 2_048);
    }

    #[test]
    fn fixed_staging_capacity_returns_typed_exhaustion() {
        let error = StagedUploads::build(
            &GpuView::zeroed(),
            SceneUploadPlan::default(),
            size_of::<GpuView>() as u64 - 1,
        )
        .err()
        .expect("undersized staging must fail");
        assert_eq!(error.kind(), RenderErrorKind::OutOfMemory);
    }

    #[test]
    fn appended_views_receive_distinct_aligned_offsets() {
        let mut bytes = Vec::new();
        let first = StagedUploads::append(
            &GpuView::zeroed(),
            SceneUploadPlan::default(),
            4_096,
            &mut bytes,
            256,
        )
        .unwrap();
        let second = StagedUploads::append(
            &GpuView::zeroed(),
            SceneUploadPlan::default(),
            4_096,
            &mut bytes,
            256,
        )
        .unwrap();
        assert_eq!(first.view_offset, 0);
        assert_eq!(second.view_offset, 256);
        assert_eq!(bytes.len(), 256 + size_of::<GpuView>());
    }
}

fn align_vec_at(bytes: &mut Vec<u8>, alignment: usize, base: usize) {
    if base == 0 {
        align_vec(bytes, alignment);
        return;
    }
    let remainder = (base + bytes.len()) % alignment;
    if remainder != 0 {
        bytes.resize(bytes.len() + alignment - remainder, 0);
    }
}
