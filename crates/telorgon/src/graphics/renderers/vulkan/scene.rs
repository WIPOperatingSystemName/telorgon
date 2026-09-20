use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::foundation::{ColorRgba8, SizeF, SizeI};
use crate::graphics::gpu_abi::{
    CLIP_ANALYTIC_ROUNDED_RECT, CLIP_NONE, CLIP_SCISSOR, GpuBoxInstance, GpuClip, GpuGlyphInstance,
    GpuImageInstance, GpuMaterialInstance, GpuSpatial, NO_GPU_SLOT, pack_srgba8,
};
use crate::graphics::render::{
    BatchKey, BoxInstance, DirtyRanges, DrawItem, GlyphInstance, ImageAlphaMode,
    ImageColorEncoding, ImageId, ImageInstance, ImagePixelFormat, ImageResourceDelta,
    MaterialInstance, MaterialKind, MaterialResource, MaterialResourceDelta, PipelineKind,
    PrimitiveKind, RenderClip, RenderError, RenderErrorKind, RenderResult, RenderSceneDelta,
    RenderSpatialNode, apply_patches,
};
use ash::vk;
use bytemuck::Zeroable;

use crate::graphics::renderers::vulkan::VulkanMaterializationTarget;
use crate::graphics::renderers::vulkan::descriptor::MAX_TEXTURE_SETS;
use crate::graphics::renderers::vulkan::device::DeviceInner;
use crate::graphics::renderers::vulkan::external_image::{ExternalImageInner, VulkanExternalImageLease};
use crate::graphics::renderers::vulkan::image::AllocatedImage;
use crate::graphics::renderers::vulkan::upload::{ImageUploadChunk, RetainedGpuBuffer, SceneUploadPlan};

static NEXT_SCENE_ID: AtomicU64 = AtomicU64::new(1);
const MAX_RETIRED_IMAGES_PER_TEXTURE: usize = 2;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct VulkanSceneMetrics {
    pub buffer_allocations: u64,
    pub buffer_growths: u64,
    pub uploaded_bytes: u64,
    pub buffer_copies: u64,
    pub descriptor_writes: u64,
    pub resident_capacity_bytes: u64,
    pub pending_upload_bytes: u64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct DrawBatch {
    pub(crate) key: BatchKey,
    pub(crate) kind: PrimitiveKind,
    pub(crate) first_instance: u32,
    pub(crate) instance_count: u32,
}

#[derive(Default)]
struct RetainedTexture {
    image: Option<Arc<AllocatedImage>>,
    retired: Vec<Arc<AllocatedImage>>,
    extent: SizeI,
    format: vk::Format,
    generation: u64,
    initialized: bool,
}

impl RetainedTexture {
    fn ensure(
        &mut self,
        device: &Arc<DeviceInner>,
        extent: SizeI,
        format: vk::Format,
        name: &str,
    ) -> crate::graphics::render::RenderResult<bool> {
        if self.image.is_some() && self.extent == extent && self.format == format {
            return Ok(false);
        }
        self.retired.clear();
        self.image = Some(Arc::new(AllocatedImage::new_sampled(
            Arc::clone(device),
            vk::Extent2D {
                width: extent.width.max(1) as u32,
                height: extent.height.max(1) as u32,
            },
            format,
            name,
        )?));
        self.extent = extent;
        self.format = format;
        self.generation = self.generation.saturating_add(1);
        self.initialized = false;
        Ok(true)
    }

    fn prepare_write(
        &mut self,
        device: &Arc<DeviceInner>,
        name: &str,
        preserve_contents: bool,
    ) -> crate::graphics::render::RenderResult<Option<Arc<AllocatedImage>>> {
        let Some(current) = self.image.as_ref() else {
            return Ok(None);
        };
        if !self.initialized || Arc::strong_count(current) == 1 {
            return Ok(None);
        }

        let previous = self
            .image
            .take()
            .expect("retained texture must exist while preparing a write");
        let reusable = self
            .retired
            .iter()
            .position(|image| Arc::strong_count(image) == 1)
            .map(|index| self.retired.swap_remove(index));
        let replacement_was_initialized = reusable.is_some();
        let replacement = match reusable {
            Some(image) => image,
            None => Arc::new(AllocatedImage::new_sampled(
                Arc::clone(device),
                vk::Extent2D {
                    width: self.extent.width.max(1) as u32,
                    height: self.extent.height.max(1) as u32,
                },
                self.format,
                name,
            )?),
        };
        self.retired.push(Arc::clone(&previous));
        while self.retired.len() > MAX_RETIRED_IMAGES_PER_TEXTURE {
            let removable = self
                .retired
                .iter()
                .position(|image| Arc::strong_count(image) == 1)
                .unwrap_or(0);
            self.retired.swap_remove(removable);
        }
        self.image = Some(replacement);
        self.generation = self.generation.saturating_add(1);
        self.initialized = replacement_was_initialized;
        Ok(preserve_contents.then_some(previous))
    }

    fn image(&self) -> &Arc<AllocatedImage> {
        self.image
            .as_ref()
            .expect("sampled image must be allocated before descriptor binding")
    }
}

struct VulkanImageResource {
    extent: SizeI,
    color_encoding: ImageColorEncoding,
    alpha_mode: ImageAlphaMode,
    pixel_format: ImagePixelFormat,
    pixels: Vec<u8>,
    pending: Vec<ImageUploadChunk>,
    texture: RetainedTexture,
}

struct ExternalSceneImage {
    image: Arc<ExternalImageInner>,
}

pub struct VulkanScene {
    pub(crate) id: u64,
    pub(crate) device_id: u64,
    pub(crate) epoch: u64,
    pub(crate) extent: SizeF,
    pub(crate) background: ColorRgba8,
    pub(crate) boxes: Vec<BoxInstance>,
    pub(crate) glyphs: Vec<GlyphInstance>,
    pub(crate) images: Vec<ImageInstance>,
    pub(crate) materials: Vec<MaterialInstance>,
    pub(crate) clips: Vec<RenderClip>,
    pub(crate) spatial: Vec<RenderSpatialNode>,
    pub(crate) draw_order: Vec<DrawItem>,
    pub(crate) gpu_boxes: Vec<GpuBoxInstance>,
    pub(crate) gpu_glyphs: Vec<GpuGlyphInstance>,
    pub(crate) gpu_images: Vec<GpuImageInstance>,
    pub(crate) gpu_materials: Vec<GpuMaterialInstance>,
    pub(crate) gpu_clips: Vec<GpuClip>,
    pub(crate) gpu_spatial: Vec<GpuSpatial>,
    pub(crate) material_parameters: Vec<u32>,
    pub(crate) draw_indices: Vec<u32>,
    pub(crate) batches: Vec<DrawBatch>,
    box_dirty: DirtyRanges,
    glyph_dirty: DirtyRanges,
    image_dirty: DirtyRanges,
    material_dirty: DirtyRanges,
    clip_dirty: DirtyRanges,
    spatial_dirty: DirtyRanges,
    material_parameter_dirty: DirtyRanges,
    draw_dirty: DirtyRanges,
    box_buffer: RetainedGpuBuffer,
    glyph_buffer: RetainedGpuBuffer,
    image_buffer: RetainedGpuBuffer,
    material_buffer: RetainedGpuBuffer,
    clip_buffer: RetainedGpuBuffer,
    spatial_buffer: RetainedGpuBuffer,
    material_parameter_buffer: RetainedGpuBuffer,
    draw_buffer: RetainedGpuBuffer,
    atlas_extent: SizeI,
    atlas_pixels: Vec<u8>,
    atlas_pending: Vec<ImageUploadChunk>,
    atlas_texture: RetainedTexture,
    image_resources: BTreeMap<ImageId, VulkanImageResource>,
    external_images: BTreeMap<ImageId, ExternalSceneImage>,
    pub(crate) material_resources: BTreeMap<u32, MaterialResource>,
    texture_slots: BTreeMap<(u32, Option<u32>), usize>,
    metrics: VulkanSceneMetrics,
}

impl Default for VulkanScene {
    fn default() -> Self {
        Self::new(0)
    }
}

impl VulkanScene {
    pub(crate) fn new(device_id: u64) -> Self {
        Self {
            id: NEXT_SCENE_ID.fetch_add(1, Ordering::Relaxed),
            device_id,
            epoch: 0,
            extent: SizeF::default(),
            background: ColorRgba8::default(),
            boxes: Vec::new(),
            glyphs: Vec::new(),
            images: Vec::new(),
            materials: Vec::new(),
            clips: Vec::new(),
            spatial: Vec::new(),
            draw_order: Vec::new(),
            gpu_boxes: Vec::new(),
            gpu_glyphs: Vec::new(),
            gpu_images: Vec::new(),
            gpu_materials: Vec::new(),
            gpu_clips: vec![none_clip()],
            gpu_spatial: vec![identity_spatial()],
            material_parameters: Vec::new(),
            draw_indices: Vec::new(),
            batches: Vec::new(),
            box_dirty: DirtyRanges::default(),
            glyph_dirty: DirtyRanges::default(),
            image_dirty: DirtyRanges::default(),
            material_dirty: DirtyRanges::default(),
            clip_dirty: DirtyRanges::default(),
            spatial_dirty: DirtyRanges::default(),
            material_parameter_dirty: DirtyRanges::default(),
            draw_dirty: DirtyRanges::default(),
            box_buffer: RetainedGpuBuffer::default(),
            glyph_buffer: RetainedGpuBuffer::default(),
            image_buffer: RetainedGpuBuffer::default(),
            material_buffer: RetainedGpuBuffer::default(),
            clip_buffer: RetainedGpuBuffer::default(),
            spatial_buffer: RetainedGpuBuffer::default(),
            material_parameter_buffer: RetainedGpuBuffer::default(),
            draw_buffer: RetainedGpuBuffer::default(),
            atlas_extent: SizeI {
                width: 1,
                height: 1,
            },
            atlas_pixels: vec![0],
            atlas_pending: Vec::new(),
            atlas_texture: RetainedTexture::default(),
            image_resources: BTreeMap::new(),
            external_images: BTreeMap::new(),
            material_resources: BTreeMap::new(),
            texture_slots: BTreeMap::new(),
            metrics: VulkanSceneMetrics::default(),
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn background(&self) -> ColorRgba8 {
        self.background
    }

    pub fn metrics(&self) -> VulkanSceneMetrics {
        let mut metrics = self.metrics;
        metrics.resident_capacity_bytes = [
            &self.box_buffer,
            &self.glyph_buffer,
            &self.image_buffer,
            &self.material_buffer,
            &self.clip_buffer,
            &self.spatial_buffer,
            &self.material_parameter_buffer,
            &self.draw_buffer,
        ]
        .into_iter()
        .map(RetainedGpuBuffer::capacity)
        .sum();
        metrics.pending_upload_bytes = self.pending_upload_bytes();
        metrics
    }

    pub(crate) fn apply(&mut self, delta: &RenderSceneDelta) {
        self.epoch = delta.epoch;
        self.extent = delta.extent;
        self.background = delta.background;
        self.apply_image_resources(&delta.image_resources);
        self.apply_material_resources(&delta.material_resources);
        self.apply_atlas(delta.atlas_extent, &delta.atlas_pages);

        apply_patches(&mut self.boxes, &delta.boxes, delta.box_len);
        patch_gpu_values(
            &mut self.gpu_boxes,
            &delta.boxes,
            delta.box_len,
            convert_box,
            &mut self.box_dirty,
        );
        apply_patches(&mut self.glyphs, &delta.glyphs, delta.glyph_len);
        patch_gpu_values(
            &mut self.gpu_glyphs,
            &delta.glyphs,
            delta.glyph_len,
            convert_glyph,
            &mut self.glyph_dirty,
        );
        let rebuild_images = !delta.images.is_empty() || !delta.image_resources.is_empty();
        apply_patches(&mut self.images, &delta.images, delta.image_len);
        if rebuild_images {
            let image_resources = &self.image_resources;
            let external_images = &self.external_images;
            self.gpu_images = self
                .images
                .iter()
                .map(|instance| {
                    let alpha = image_resources
                        .get(&instance.image)
                        .map(|resource| resource.alpha_mode)
                        .or_else(|| {
                            external_images
                                .get(&instance.image)
                                .map(|resource| resource.image.alpha_mode)
                        });
                    convert_image(instance, alpha)
                })
                .collect();
            self.image_dirty.add(0..self.gpu_images.len());
        } else {
            self.gpu_images.truncate(delta.image_len);
        }
        let rebuild_materials = !delta.materials.is_empty() || !delta.material_resources.is_empty();
        apply_patches(&mut self.materials, &delta.materials, delta.material_len);
        if rebuild_materials {
            self.rebuild_materials();
        } else {
            self.gpu_materials.truncate(delta.material_len);
        }

        let rebuild_clips = !delta.clips.is_empty() || delta.clip_len != self.clips.len();
        apply_patches(&mut self.clips, &delta.clips, delta.clip_len);
        if rebuild_clips {
            self.rebuild_clips();
        }
        let rebuild_spatial =
            !delta.spatial_nodes.is_empty() || delta.spatial_len != self.spatial.len();
        apply_patches(&mut self.spatial, &delta.spatial_nodes, delta.spatial_len);
        if rebuild_spatial {
            self.rebuild_spatial();
        }

        if let Some(order) = &delta.draw_order {
            self.draw_order = order.to_vec();
            self.draw_indices = self.draw_order.iter().map(|draw| draw.index).collect();
            self.batches = build_batches(&self.draw_order);
            self.draw_dirty.add(0..self.draw_indices.len());
            self.rebuild_texture_slots();
        } else if !delta.image_resources.is_empty() || !delta.material_resources.is_empty() {
            self.rebuild_texture_slots();
        }
    }

    fn apply_atlas(&mut self, extent: SizeI, pages: &[crate::ui::text::AtlasPageUpdate]) {
        if extent.width <= 0 || extent.height <= 0 {
            return;
        }
        if self.atlas_extent != extent {
            self.atlas_extent = extent;
            self.atlas_pixels
                .resize(extent.width as usize * extent.height as usize, 0);
            self.atlas_texture.image = None;
            self.atlas_pending.clear();
        }
        for page in pages {
            for row in 0..page.height as usize {
                let source = row * page.width as usize;
                let target =
                    (page.y as usize + row) * self.atlas_extent.width as usize + page.x as usize;
                self.atlas_pixels[target..target + page.width as usize]
                    .copy_from_slice(&page.pixels_a8[source..source + page.width as usize]);
            }
            self.atlas_pending.push(ImageUploadChunk {
                offset: vk::Offset3D {
                    x: page.x,
                    y: page.y,
                    z: 0,
                },
                extent: vk::Extent3D {
                    width: page.width as u32,
                    height: page.height as u32,
                    depth: 1,
                },
                row_bytes: page.width as usize,
                bytes: page.pixels_a8.to_vec(),
            });
        }
    }

    fn apply_image_resources(&mut self, updates: &[ImageResourceDelta]) {
        for update in updates {
            match update {
                ImageResourceDelta::Remove(id) => {
                    self.image_resources.remove(id);
                }
                ImageResourceDelta::Write(update) => {
                    let resource = self.image_resources.entry(update.image).or_insert_with(|| {
                        VulkanImageResource {
                            extent: update.extent,
                            color_encoding: update.color_encoding,
                            alpha_mode: update.alpha_mode,
                            pixel_format: update.pixel_format,
                            pixels: vec![
                                0;
                                update.extent.width as usize
                                    * update.extent.height as usize
                                    * 4
                            ],
                            pending: Vec::new(),
                            texture: RetainedTexture::default(),
                        }
                    });
                    if resource.extent != update.extent
                        || resource.color_encoding != update.color_encoding
                        || resource.pixel_format != update.pixel_format
                    {
                        resource.extent = update.extent;
                        resource.color_encoding = update.color_encoding;
                        resource.pixel_format = update.pixel_format;
                        resource.pixels.resize(
                            update.extent.width as usize * update.extent.height as usize * 4,
                            0,
                        );
                        resource.texture.image = None;
                        resource.pending.clear();
                    }
                    resource.alpha_mode = update.alpha_mode;
                    let destination_stride = update.extent.width as usize * 4;
                    let copy_bytes = update.rect.width as usize * 4;
                    for row in 0..update.rect.height as usize {
                        let source = row * update.row_bytes;
                        let target = (update.rect.y as usize + row) * destination_stride
                            + update.rect.x as usize * 4;
                        resource.pixels[target..target + copy_bytes]
                            .copy_from_slice(&update.pixels[source..source + copy_bytes]);
                    }
                    resource.pending.push(ImageUploadChunk {
                        offset: vk::Offset3D {
                            x: update.rect.x,
                            y: update.rect.y,
                            z: 0,
                        },
                        extent: vk::Extent3D {
                            width: update.rect.width as u32,
                            height: update.rect.height as u32,
                            depth: 1,
                        },
                        row_bytes: update.row_bytes,
                        bytes: update.pixels.to_vec(),
                    });
                }
            }
        }
    }

    /// Resolves a renderer-neutral image ID to one linear external Vulkan image lease.
    pub fn bind_external_image(
        &mut self,
        image: ImageId,
        mut lease: VulkanExternalImageLease,
    ) -> RenderResult<()> {
        let external = lease.take_inner()?;
        if external.device_id != self.device_id {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "external image lease belongs to another Vulkan device",
            ));
        }
        if self.image_resources.contains_key(&image) {
            return Err(RenderError::new(
                RenderErrorKind::InvalidScene,
                "one image ID cannot name both uploaded and external Vulkan content",
            ));
        }
        if self
            .external_images
            .get(&image)
            .is_some_and(|current| external.lease_generation <= current.image.lease_generation)
        {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "external image lease generations must increase when rebound",
            ));
        }
        self.external_images
            .insert(image, ExternalSceneImage { image: external });
        self.rebuild_external_instances(image);
        self.rebuild_texture_slots();
        Ok(())
    }

    /// Removes the logical binding. Submitted frames retain their own pins until host completion.
    pub fn remove_external_image(&mut self, image: ImageId) -> bool {
        let removed = self.external_images.remove(&image).is_some();
        if removed {
            self.rebuild_texture_slots();
        }
        removed
    }

    /// Binds a compositor-owned image that is populated earlier in the same command buffer.
    /// Submitted frames retain their own `Arc`, so rebinding cannot retire an in-flight texture.
    pub(crate) fn bind_materialized_image(
        &mut self,
        image: ImageId,
        target: &VulkanMaterializationTarget,
        alpha_mode: ImageAlphaMode,
    ) -> RenderResult<()> {
        let retained = target.image();
        if retained.device_id() != self.device_id {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "materialized image belongs to another Vulkan device",
            ));
        }
        let extent = target.extent();
        if self.image_resources.get(&image).is_some_and(|resource| {
            resource.alpha_mode == alpha_mode
                && resource
                    .texture
                    .image
                    .as_ref()
                    .is_some_and(|old| Arc::ptr_eq(old, &retained))
        }) {
            // Storage and native view did not change. Contents are updated by an ordered GPU pass;
            // retain descriptor identity and avoid rebuilding the binding on every commit.
            return Ok(());
        }
        let generation = self
            .image_resources
            .get(&image)
            .map_or(1, |resource| resource.texture.generation.saturating_add(1));
        self.external_images.remove(&image);
        self.image_resources.insert(
            image,
            VulkanImageResource {
                extent,
                color_encoding: VulkanMaterializationTarget::COLOR_ENCODING,
                alpha_mode,
                pixel_format: ImagePixelFormat::Rgba8,
                pixels: Vec::new(),
                pending: Vec::new(),
                texture: RetainedTexture {
                    image: Some(retained),
                    retired: Vec::new(),
                    extent,
                    format: VulkanMaterializationTarget::FORMAT,
                    generation,
                    initialized: true,
                },
            },
        );
        self.rebuild_external_instances(image);
        self.rebuild_texture_slots();
        Ok(())
    }

    /// Removes a compositor-owned materialization after a scene returns to uploaded pixels.
    /// In-flight submissions keep their own image pin.
    pub(crate) fn remove_materialized_image(&mut self, image: ImageId) -> bool {
        let removed = self.image_resources.remove(&image).is_some();
        if removed {
            self.rebuild_texture_slots();
        }
        removed
    }

    /// Apply after binding and publishing compositor-owned snapshot instances. The image
    /// shader clamps to these texel bounds, so upscaling cannot sample capacity padding.
    pub(crate) fn crop_materialized_image(
        &mut self,
        image: ImageId,
        active: crate::SizeI,
        capacity: crate::SizeI,
    ) {
        assert!(
            active.width > 0
                && active.height > 0
                && active.width <= capacity.width
                && active.height <= capacity.height
        );
        let uv = [
            0.0,
            0.0,
            active.width as f32 / capacity.width as f32,
            active.height as f32 / capacity.height as f32,
        ];
        for (index, instance) in self.images.iter().enumerate() {
            if instance.image == image && self.gpu_images[index].uv_normalized != uv {
                self.gpu_images[index].uv_normalized = uv;
                self.image_dirty.add(index..index + 1);
            }
        }
    }

    fn rebuild_external_instances(&mut self, image: ImageId) {
        let alpha = self
            .external_images
            .get(&image)
            .map(|resource| resource.image.alpha_mode);
        for (index, instance) in self.images.iter().enumerate() {
            if instance.image == image {
                if index >= self.gpu_images.len() {
                    self.gpu_images
                        .resize(index + 1, GpuImageInstance::zeroed());
                }
                self.gpu_images[index] = convert_image(instance, alpha);
                self.image_dirty.add(index..index + 1);
            }
        }
    }

    fn apply_material_resources(&mut self, updates: &[MaterialResourceDelta]) {
        for update in updates {
            match update {
                MaterialResourceDelta::Upsert(resource) => {
                    self.material_resources
                        .insert(resource.material.0, *resource);
                }
                MaterialResourceDelta::Remove(id) => {
                    self.material_resources.remove(&id.0);
                }
            }
        }
    }

    fn rebuild_materials(&mut self) {
        let previous_parameters = std::mem::take(&mut self.material_parameters);
        let mut offsets = BTreeMap::new();
        for (id, resource) in &self.material_resources {
            let start = self.material_parameters.len() as u32;
            match resource.kind {
                MaterialKind::LiquidGlass(parameters) => {
                    self.material_parameters.extend(parameters.words());
                }
                MaterialKind::GaussianBlur(parameters) => {
                    self.material_parameters.extend(parameters.words())
                }
                _ => self.material_parameters.extend(resource.colors.map(pack)),
            }
            offsets.insert(*id, (start, self.material_parameters.len() as u32 - start));
        }
        let next_materials: Vec<GpuMaterialInstance> = self
            .materials
            .iter()
            .map(|instance| {
                let resource = self.material_resources.get(&instance.material.0);
                GpuMaterialInstance {
                    rect: rect(instance.rect),
                    params_spatial_clip: [
                        offsets.get(&instance.material.0).map_or(0, |range| range.0),
                        offsets.get(&instance.material.0).map_or(2, |range| range.1),
                        instance.spatial.0,
                        clip_slot(instance.clip.0),
                    ],
                    opacity: instance.opacity,
                    material_variant: resource.map_or(0, |value| match value.kind {
                        MaterialKind::Solid => 0,
                        MaterialKind::LinearGradientHorizontal => 1,
                        MaterialKind::LinearGradientVertical => 2,
                        MaterialKind::LiquidGlass(_) => 3,
                        MaterialKind::GaussianBlur(_) => 4,
                    }),
                    flags: 0,
                    reserved: 0,
                    resource_range_reserved: [0; 4],
                }
            })
            .collect();
        if self.gpu_materials != next_materials {
            self.material_dirty.add(0..next_materials.len());
            self.gpu_materials = next_materials;
        }
        if self.material_parameters != previous_parameters {
            self.material_parameter_dirty
                .add(0..self.material_parameters.len());
        }
    }

    fn rebuild_clips(&mut self) {
        let maximum = self.clips.iter().map(|clip| clip.id.0).max().unwrap_or(0) as usize;
        self.gpu_clips = vec![none_clip(); maximum.saturating_add(1).max(1)];
        for clip in &self.clips {
            if clip.id.0 != 0 {
                self.gpu_clips[clip.id.0 as usize] = convert_clip(clip);
            }
        }
        self.clip_dirty.add(0..self.gpu_clips.len());
    }

    fn rebuild_spatial(&mut self) {
        let maximum = self
            .spatial
            .iter()
            .map(|spatial| spatial.id.0)
            .max()
            .unwrap_or(0) as usize;
        self.gpu_spatial = vec![identity_spatial(); maximum.saturating_add(1).max(1)];
        for spatial in &self.spatial {
            self.gpu_spatial[spatial.id.0 as usize] = convert_spatial(spatial);
        }
        self.spatial_dirty.add(0..self.gpu_spatial.len());
    }

    fn rebuild_texture_slots(&mut self) {
        self.texture_slots.clear();
        let resources = self
            .draw_order
            .iter()
            .filter_map(|draw| match draw.kind {
                PrimitiveKind::Image => Some((draw.batch.resource, None)),
                PrimitiveKind::Material => self
                    .material_resources
                    .get(&draw.batch.resource)
                    .and_then(|r| match r.kind {
                        MaterialKind::LiquidGlass(p) => {
                            Some((p.backdrop.0, Some(p.sharp_backdrop.0)))
                        }
                        MaterialKind::GaussianBlur(p) => Some((p.source.0, None)),
                        _ => None,
                    }),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        for (slot, resource) in resources.into_iter().enumerate() {
            self.texture_slots.insert(resource, slot + 1);
        }
    }

    pub(crate) fn texture_slot(&self, batch: &DrawBatch) -> Option<usize> {
        match batch.kind {
            PrimitiveKind::Glyph => Some(0),
            PrimitiveKind::Image => self.texture_slots.get(&(batch.key.resource, None)).copied(),
            PrimitiveKind::Material => {
                self.material_resources
                    .get(&batch.key.resource)
                    .and_then(|r| match r.kind {
                        MaterialKind::LiquidGlass(p) => self
                            .texture_slots
                            .get(&(p.backdrop.0, Some(p.sharp_backdrop.0)))
                            .copied(),
                        MaterialKind::GaussianBlur(p) => {
                            self.texture_slots.get(&(p.source.0, None)).copied()
                        }
                        _ => None,
                    })
            }
            PrimitiveKind::Box => None,
        }
    }

    pub(crate) fn texture_count(&self) -> usize {
        usize::from(
            !self.texture_slots.is_empty()
                || self
                    .draw_order
                    .iter()
                    .any(|draw| draw.kind == PrimitiveKind::Glyph),
        ) + self.texture_slots.len()
    }

    pub(crate) fn image_resource_ids(&self) -> BTreeSet<u32> {
        self.image_resources
            .keys()
            .chain(self.external_images.keys())
            .map(|id| id.0)
            .collect()
    }

    /// Metadata only, for correlating desktop resize traces with retained bindings.
    pub(crate) fn retained_image_trace_metadata(&self, image: ImageId) -> Option<(SizeI, u64)> {
        self.image_resources
            .get(&image)
            .map(|resource| (resource.extent, resource.texture.generation))
    }

    pub(crate) fn has_external_image(&self, image: ImageId) -> bool {
        self.external_images.contains_key(&image)
    }

    pub(crate) fn external_image_content_version(&self, image: ImageId) -> Option<u64> {
        self.external_images
            .get(&image)
            .map(|resource| resource.image.content_version)
    }

    pub(crate) fn pending_upload_bytes(&self) -> u64 {
        range_bytes(&self.box_dirty, size_of::<GpuBoxInstance>())
            + range_bytes(&self.glyph_dirty, size_of::<GpuGlyphInstance>())
            + range_bytes(&self.image_dirty, size_of::<GpuImageInstance>())
            + range_bytes(&self.material_dirty, size_of::<GpuMaterialInstance>())
            + range_bytes(&self.clip_dirty, size_of::<GpuClip>())
            + range_bytes(&self.spatial_dirty, size_of::<GpuSpatial>())
            + range_bytes(&self.material_parameter_dirty, size_of::<u32>())
            + range_bytes(&self.draw_dirty, size_of::<u32>())
            + self
                .atlas_pending
                .iter()
                .map(|value| value.bytes.len() as u64)
                .sum::<u64>()
            + self
                .image_resources
                .values()
                .flat_map(|resource| &resource.pending)
                .map(|value| value.bytes.len() as u64)
                .sum::<u64>()
    }

    pub(crate) fn queued_descriptor_writes(&self) -> u32 {
        let required = |buffer: &RetainedGpuBuffer, bytes: u64| {
            u32::from(buffer.capacity() < bytes.max(4) || buffer.capacity() == 0)
        };
        required(&self.box_buffer, byte_len(&self.gpu_boxes))
            + required(&self.spatial_buffer, byte_len(&self.gpu_spatial))
            + required(&self.draw_buffer, byte_len(&self.draw_indices))
            + u32::from(self.gpu_clips.len() > 1 && !self.clip_buffer.is_allocated())
            + u32::from(!self.gpu_glyphs.is_empty() && !self.glyph_buffer.is_allocated())
            + u32::from(!self.gpu_images.is_empty() && !self.image_buffer.is_allocated())
            + u32::from(!self.gpu_materials.is_empty() && !self.material_buffer.is_allocated())
            + self.texture_count() as u32
            + self
                .texture_slots
                .keys()
                .filter(|(_, second)| second.is_some())
                .count() as u32
    }

    pub(crate) fn prepare_uploads(
        &mut self,
        device: &Arc<DeviceInner>,
    ) -> crate::graphics::render::RenderResult<SceneUploadPlan> {
        let mut plan = SceneUploadPlan::default();
        ensure_buffer(
            device,
            &mut self.box_buffer,
            &mut self.box_dirty,
            self.gpu_boxes.len(),
            byte_len(&self.gpu_boxes),
            "Telorgon retained box records",
            &mut plan,
        )?;
        ensure_buffer(
            device,
            &mut self.spatial_buffer,
            &mut self.spatial_dirty,
            self.gpu_spatial.len(),
            byte_len(&self.gpu_spatial),
            "Telorgon retained spatial records",
            &mut plan,
        )?;
        ensure_buffer(
            device,
            &mut self.draw_buffer,
            &mut self.draw_dirty,
            self.draw_indices.len(),
            byte_len(&self.draw_indices),
            "Telorgon retained draw indices",
            &mut plan,
        )?;
        ensure_optional_buffer(
            device,
            &mut self.glyph_buffer,
            &mut self.glyph_dirty,
            self.gpu_glyphs.len(),
            byte_len(&self.gpu_glyphs),
            "Telorgon retained glyph records",
            &mut plan,
        )?;
        ensure_optional_buffer(
            device,
            &mut self.image_buffer,
            &mut self.image_dirty,
            self.gpu_images.len(),
            byte_len(&self.gpu_images),
            "Telorgon retained image records",
            &mut plan,
        )?;
        ensure_optional_buffer(
            device,
            &mut self.material_buffer,
            &mut self.material_dirty,
            self.gpu_materials.len(),
            byte_len(&self.gpu_materials),
            "Telorgon retained material records",
            &mut plan,
        )?;
        let has_real_clips = self.gpu_clips.len() > 1;
        if has_real_clips {
            ensure_optional_buffer(
                device,
                &mut self.clip_buffer,
                &mut self.clip_dirty,
                self.gpu_clips.len(),
                byte_len(&self.gpu_clips),
                "Telorgon retained clip records",
                &mut plan,
            )?;
        }
        ensure_optional_buffer(
            device,
            &mut self.material_parameter_buffer,
            &mut self.material_parameter_dirty,
            self.material_parameters.len(),
            byte_len(&self.material_parameters),
            "Telorgon retained material parameters",
            &mut plan,
        )?;

        plan.push_pod_ranges(&self.box_buffer, &self.gpu_boxes, &self.box_dirty.ranges);
        plan.push_pod_ranges(
            &self.spatial_buffer,
            &self.gpu_spatial,
            &self.spatial_dirty.ranges,
        );
        plan.push_pod_ranges(
            &self.draw_buffer,
            &self.draw_indices,
            &self.draw_dirty.ranges,
        );
        if self.glyph_buffer.is_allocated() {
            plan.push_pod_ranges(
                &self.glyph_buffer,
                &self.gpu_glyphs,
                &self.glyph_dirty.ranges,
            );
        }
        if self.image_buffer.is_allocated() {
            plan.push_pod_ranges(
                &self.image_buffer,
                &self.gpu_images,
                &self.image_dirty.ranges,
            );
        }
        if self.material_buffer.is_allocated() {
            plan.push_pod_ranges(
                &self.material_buffer,
                &self.gpu_materials,
                &self.material_dirty.ranges,
            );
        }
        if has_real_clips {
            plan.push_pod_ranges(&self.clip_buffer, &self.gpu_clips, &self.clip_dirty.ranges);
        }
        if self.material_parameter_buffer.is_allocated() {
            plan.push_pod_ranges(
                &self.material_parameter_buffer,
                &self.material_parameters,
                &self.material_parameter_dirty.ranges,
            );
        }

        if !self.gpu_glyphs.is_empty() {
            let allocated = self.atlas_texture.ensure(
                device,
                self.atlas_extent,
                vk::Format::R8_UNORM,
                "Telorgon glyph atlas",
            )?;
            let preserve_from = if allocated || self.atlas_pending.is_empty() {
                None
            } else {
                self.atlas_texture.prepare_write(
                    device,
                    "Telorgon glyph atlas",
                    !image_upload_is_full(&self.atlas_pending, self.atlas_extent, 1),
                )?
            };
            let chunks = if allocated {
                vec![ImageUploadChunk {
                    offset: vk::Offset3D::default(),
                    extent: vk::Extent3D {
                        width: self.atlas_extent.width as u32,
                        height: self.atlas_extent.height as u32,
                        depth: 1,
                    },
                    row_bytes: self.atlas_extent.width as usize,
                    bytes: self.atlas_pixels.clone(),
                }]
            } else {
                std::mem::take(&mut self.atlas_pending)
            };
            plan.push_image_uploads(
                Arc::clone(self.atlas_texture.image()),
                self.atlas_texture.initialized,
                preserve_from,
                1,
                chunks,
            );
        }
        for resource in self.image_resources.values_mut() {
            let format = match (resource.pixel_format, resource.color_encoding) {
                (ImagePixelFormat::Rgba8, ImageColorEncoding::Linear) => vk::Format::R8G8B8A8_UNORM,
                (ImagePixelFormat::Rgba8, ImageColorEncoding::Srgb) => vk::Format::R8G8B8A8_SRGB,
                (ImagePixelFormat::Bgra8, ImageColorEncoding::Linear) => vk::Format::B8G8R8A8_UNORM,
                (ImagePixelFormat::Bgra8, ImageColorEncoding::Srgb) => vk::Format::B8G8R8A8_SRGB,
            };
            let allocated = resource.texture.ensure(
                device,
                resource.extent,
                format,
                "Telorgon retained four-channel image",
            )?;
            let preserve_from = if allocated || resource.pending.is_empty() {
                None
            } else {
                resource.texture.prepare_write(
                    device,
                    "Telorgon retained four-channel image",
                    !image_upload_is_full(&resource.pending, resource.extent, 4),
                )?
            };
            let chunks = if allocated {
                vec![ImageUploadChunk {
                    offset: vk::Offset3D::default(),
                    extent: vk::Extent3D {
                        width: resource.extent.width as u32,
                        height: resource.extent.height as u32,
                        depth: 1,
                    },
                    row_bytes: resource.extent.width as usize * 4,
                    bytes: resource.pixels.clone(),
                }]
            } else {
                std::mem::take(&mut resource.pending)
            };
            plan.push_image_uploads(
                Arc::clone(resource.texture.image()),
                resource.texture.initialized,
                preserve_from,
                4,
                chunks,
            );
        }
        Ok(plan)
    }

    pub(crate) fn common_buffers(
        &self,
    ) -> (
        &Arc<crate::graphics::renderers::vulkan::buffer::AllocatedBuffer>,
        &Arc<crate::graphics::renderers::vulkan::buffer::AllocatedBuffer>,
    ) {
        (self.spatial_buffer.buffer(), self.draw_buffer.buffer())
    }
    pub(crate) fn clip_buffer(
        &self,
    ) -> Option<&Arc<crate::graphics::renderers::vulkan::buffer::AllocatedBuffer>> {
        self.clip_buffer
            .is_allocated()
            .then(|| self.clip_buffer.buffer())
    }
    pub(crate) fn primitive_buffer(
        &self,
        kind: PrimitiveKind,
    ) -> Option<&Arc<crate::graphics::renderers::vulkan::buffer::AllocatedBuffer>> {
        match kind {
            PrimitiveKind::Box => Some(self.box_buffer.buffer()),
            PrimitiveKind::Glyph => self
                .glyph_buffer
                .is_allocated()
                .then(|| self.glyph_buffer.buffer()),
            PrimitiveKind::Image => self
                .image_buffer
                .is_allocated()
                .then(|| self.image_buffer.buffer()),
            PrimitiveKind::Material => self
                .material_buffer
                .is_allocated()
                .then(|| self.material_buffer.buffer()),
        }
    }
    pub(crate) fn material_parameter_buffer(
        &self,
    ) -> Option<&Arc<crate::graphics::renderers::vulkan::buffer::AllocatedBuffer>> {
        self.material_parameter_buffer
            .is_allocated()
            .then(|| self.material_parameter_buffer.buffer())
    }
    pub(crate) fn buffer_generations(&self) -> (u64, u64, u64, u64) {
        (
            self.spatial_buffer.generation(),
            self.draw_buffer.generation(),
            self.box_buffer.generation(),
            self.clip_buffer.generation(),
        )
    }
    pub(crate) fn primitive_generation(&self, kind: PrimitiveKind) -> u64 {
        match kind {
            PrimitiveKind::Box => self.box_buffer.generation(),
            PrimitiveKind::Glyph => self.glyph_buffer.generation(),
            PrimitiveKind::Image => self.image_buffer.generation(),
            PrimitiveKind::Material => self.material_buffer.generation(),
        }
    }
    pub(crate) fn material_parameter_generation(&self) -> u64 {
        self.material_parameter_buffer.generation()
    }
    pub(crate) fn texture(&self, slot: usize, binding: u32) -> Option<(vk::ImageView, u64)> {
        if slot == 0 {
            if binding != 0 {
                return None;
            }
            return self
                .atlas_texture
                .image
                .as_ref()
                .map(|image| (image.view(), self.atlas_texture.generation));
        }
        let resource_id = self
            .texture_slots
            .iter()
            .find_map(|(id, value)| (*value == slot).then_some(*id))?;
        let image = ImageId(if binding == 0 {
            resource_id.0
        } else {
            resource_id.1?
        });
        self.image_resources
            .get(&image)
            .and_then(|resource| {
                resource
                    .texture
                    .image
                    .as_ref()
                    .map(|image| (image.view(), resource.texture.generation))
            })
            .or_else(|| {
                self.external_images
                    .get(&image)
                    .map(|resource| (resource.image.view, resource.image.lease_generation))
            })
    }
    pub(crate) fn retained_buffers(
        &self,
    ) -> Vec<Arc<crate::graphics::renderers::vulkan::buffer::AllocatedBuffer>> {
        [
            &self.box_buffer,
            &self.glyph_buffer,
            &self.image_buffer,
            &self.material_buffer,
            &self.clip_buffer,
            &self.spatial_buffer,
            &self.material_parameter_buffer,
            &self.draw_buffer,
        ]
        .into_iter()
        .filter(|buffer| buffer.is_allocated())
        .map(|buffer| Arc::clone(buffer.buffer()))
        .collect()
    }
    pub(crate) fn retained_images(&self) -> Vec<Arc<AllocatedImage>> {
        let mut images = Vec::new();
        if let Some(atlas) = &self.atlas_texture.image {
            images.push(Arc::clone(atlas));
        }
        images.extend(
            self.image_resources
                .values()
                .filter_map(|resource| resource.texture.image.as_ref().map(Arc::clone)),
        );
        images
    }

    pub(crate) fn retained_external_images(&self) -> Vec<Arc<ExternalImageInner>> {
        let used = self
            .draw_order
            .iter()
            .filter(|draw| draw.kind == PrimitiveKind::Image)
            .map(|draw| ImageId(draw.batch.resource))
            .collect::<BTreeSet<_>>();
        used.into_iter()
            .filter_map(|image| {
                self.external_images
                    .get(&image)
                    .map(|resource| Arc::clone(&resource.image))
            })
            .collect()
    }

    pub(crate) fn commit_uploads(
        &mut self,
        uploaded_bytes: u64,
        buffer_copies: u32,
        buffer_allocations: u32,
        buffer_growths: u32,
        descriptor_writes: u32,
    ) {
        self.box_dirty.ranges.clear();
        self.glyph_dirty.ranges.clear();
        self.image_dirty.ranges.clear();
        self.material_dirty.ranges.clear();
        self.clip_dirty.ranges.clear();
        self.spatial_dirty.ranges.clear();
        self.material_parameter_dirty.ranges.clear();
        self.draw_dirty.ranges.clear();
        for buffer in [
            &mut self.box_buffer,
            &mut self.glyph_buffer,
            &mut self.image_buffer,
            &mut self.material_buffer,
            &mut self.clip_buffer,
            &mut self.spatial_buffer,
            &mut self.material_parameter_buffer,
            &mut self.draw_buffer,
        ] {
            if buffer.is_allocated() {
                buffer.mark_initialized();
            }
        }
        if self.atlas_texture.image.is_some() {
            self.atlas_texture.initialized = true;
        }
        self.atlas_pending.clear();
        for resource in self.image_resources.values_mut() {
            if resource.texture.image.is_some() {
                resource.texture.initialized = true;
            }
            resource.pending.clear();
        }
        self.metrics.buffer_allocations += buffer_allocations as u64;
        self.metrics.buffer_growths += buffer_growths as u64;
        self.metrics.uploaded_bytes += uploaded_bytes;
        self.metrics.buffer_copies += buffer_copies as u64;
        self.metrics.descriptor_writes += descriptor_writes as u64;
    }
}

fn ensure_buffer(
    device: &Arc<DeviceInner>,
    buffer: &mut RetainedGpuBuffer,
    dirty: &mut DirtyRanges,
    len: usize,
    bytes: u64,
    name: &str,
    plan: &mut SceneUploadPlan,
) -> crate::graphics::render::RenderResult<()> {
    let was_allocated = buffer.is_allocated();
    if buffer.ensure(device, bytes, name)? {
        dirty.ranges.clear();
        dirty.add(0..len);
        plan.buffer_allocations += 1;
        plan.buffer_growths += u32::from(was_allocated);
    }
    Ok(())
}
fn ensure_optional_buffer(
    device: &Arc<DeviceInner>,
    buffer: &mut RetainedGpuBuffer,
    dirty: &mut DirtyRanges,
    len: usize,
    bytes: u64,
    name: &str,
    plan: &mut SceneUploadPlan,
) -> crate::graphics::render::RenderResult<()> {
    if len == 0 {
        Ok(())
    } else {
        ensure_buffer(device, buffer, dirty, len, bytes, name, plan)
    }
}

fn image_upload_is_full(
    chunks: &[ImageUploadChunk],
    extent: SizeI,
    bytes_per_pixel: usize,
) -> bool {
    let [chunk] = chunks else {
        return false;
    };
    chunk.offset == vk::Offset3D::default()
        && chunk.extent
            == vk::Extent3D {
                width: extent.width.max(0) as u32,
                height: extent.height.max(0) as u32,
                depth: 1,
            }
        && chunk.row_bytes == extent.width.max(0) as usize * bytes_per_pixel
        && chunk.bytes.len()
            == extent.width.max(0) as usize * extent.height.max(0) as usize * bytes_per_pixel
}

fn patch_gpu_values<T, U: Clone + Zeroable>(
    target: &mut Vec<U>,
    patches: &[crate::graphics::render::RangePatch<T>],
    final_len: usize,
    convert: impl Fn(&T) -> U,
    dirty: &mut DirtyRanges,
) {
    for patch in patches {
        let required = patch.start + patch.values.len();
        if target.len() < required {
            target.resize(required, U::zeroed());
        }
        for (offset, value) in patch.values.iter().enumerate() {
            target[patch.start + offset] = convert(value);
        }
        dirty.add(patch.start..required);
    }
    target.truncate(final_len);
}
fn build_batches(order: &[DrawItem]) -> Vec<DrawBatch> {
    let mut batches = Vec::<DrawBatch>::new();
    for (index, draw) in order.iter().enumerate() {
        if let Some(batch) = batches.last_mut()
            && batch.key == draw.batch
            && batch.kind == draw.kind
            && batch.first_instance + batch.instance_count == index as u32
        {
            batch.instance_count += 1;
        } else {
            batches.push(DrawBatch {
                key: draw.batch,
                kind: draw.kind,
                first_instance: index as u32,
                instance_count: 1,
            });
        }
    }
    batches
}
fn convert_box(instance: &BoxInstance) -> GpuBoxInstance {
    let fill = instance.background.unwrap_or_default();
    let border = instance.border;
    let border_present = border.top.width > 0.0
        || border.right.width > 0.0
        || border.bottom.width > 0.0
        || border.left.width > 0.0;
    let flags = u32::from(instance.background.is_some()) | (u32::from(border_present) << 1);
    let shadows = instance.shadows.as_slice();
    let shadow = |index: usize| shadows.get(index).copied().unwrap_or_default();
    let shadow_0 = shadow(0);
    let shadow_1 = shadow(1);
    GpuBoxInstance {
        rect: rect(instance.rect),
        radii: [
            instance.corner_radii.top_left,
            instance.corner_radii.top_right,
            instance.corner_radii.bottom_right,
            instance.corner_radii.bottom_left,
        ],
        border_widths: [
            border.top.width,
            border.right.width,
            border.bottom.width,
            border.left.width,
        ],
        fill_border_t_r_b: [
            pack(fill),
            pack(border.top.color),
            pack(border.right.color),
            pack(border.bottom.color),
        ],
        border_l_spatial_clip_flags: [
            pack(border.left.color),
            instance.spatial.0,
            clip_slot(instance.clip.0),
            flags,
        ],
        opacity: instance.opacity,
        reserved: [0; 3],
        outline: [instance.outline.width, instance.outline.offset, 0.0, 0.0],
        shadow_0: [
            shadow_0.offset.x,
            shadow_0.offset.y,
            shadow_0.blur,
            shadow_0.spread,
        ],
        shadow_1: [
            shadow_1.offset.x,
            shadow_1.offset.y,
            shadow_1.blur,
            shadow_1.spread,
        ],
        outline_shadow_colors: [
            pack(instance.outline.color),
            pack(shadow_0.color),
            pack(shadow_1.color),
            shadows.len() as u32,
        ],
    }
}
fn convert_glyph(instance: &GlyphInstance) -> GpuGlyphInstance {
    GpuGlyphInstance {
        rect: rect(instance.rect),
        uv_texels: [
            instance.atlas_x as f32,
            instance.atlas_y as f32,
            instance.atlas_size.width as f32,
            instance.atlas_size.height as f32,
        ],
        color_spatial_clip_page: [
            pack(instance.color),
            instance.spatial.0,
            clip_slot(instance.clip.0),
            0,
        ],
        opacity: instance.opacity,
        flags: 0,
        reserved: [0; 2],
    }
}
fn convert_image(instance: &ImageInstance, alpha: Option<ImageAlphaMode>) -> GpuImageInstance {
    let alpha = alpha.unwrap_or(ImageAlphaMode::Straight);
    GpuImageInstance {
        rect: rect(instance.rect),
        uv_normalized: [0.0, 0.0, 1.0, 1.0],
        tint_spatial_clip_texture: [
            pack(
                instance
                    .tint
                    .unwrap_or(ColorRgba8::rgba(255, 255, 255, 255)),
            ),
            instance.spatial.0,
            clip_slot(instance.clip.0),
            instance.image.0,
        ],
        opacity: instance.opacity,
        sampler_key: 0,
        flags: match alpha {
            ImageAlphaMode::Straight => 0,
            ImageAlphaMode::Premultiplied => 1,
            ImageAlphaMode::Opaque => 2,
        } | if instance.tint.is_some() { 1 << 2 } else { 0 },
        reserved: 0,
    }
}
fn convert_clip(clip: &RenderClip) -> GpuClip {
    let rounded = clip.corner_radii.top_left > 0.0
        || clip.corner_radii.top_right > 0.0
        || clip.corner_radii.bottom_right > 0.0
        || clip.corner_radii.bottom_left > 0.0;
    GpuClip {
        view_bounds: rect(clip.rect),
        local_rect: rect(clip.rect),
        local_from_view_0: [1.0, 0.0, 0.0, 0.0],
        local_from_view_1: [0.0, 1.0, 0.0, 0.0],
        radii: [
            clip.corner_radii.top_left,
            clip.corner_radii.top_right,
            clip.corner_radii.bottom_right,
            clip.corner_radii.bottom_left,
        ],
        mask_uv_from_view_0: [0.0; 4],
        mask_uv_from_view_1: [0.0; 4],
        mode_mask_flags: [
            if rounded {
                CLIP_ANALYTIC_ROUNDED_RECT
            } else {
                CLIP_SCISSOR
            },
            NO_GPU_SLOT,
            0,
            0,
        ],
    }
}
fn none_clip() -> GpuClip {
    GpuClip {
        mode_mask_flags: [CLIP_NONE, NO_GPU_SLOT, 0, 0],
        ..GpuClip::zeroed()
    }
}
fn convert_spatial(spatial: &RenderSpatialNode) -> GpuSpatial {
    let transform = spatial.transform;
    GpuSpatial {
        local_to_view_0: [transform.m11, transform.m21, transform.tx, 0.0],
        local_to_view_1: [transform.m12, transform.m22, transform.ty, 0.0],
    }
}
fn identity_spatial() -> GpuSpatial {
    GpuSpatial {
        local_to_view_0: [1.0, 0.0, 0.0, 0.0],
        local_to_view_1: [0.0, 1.0, 0.0, 0.0],
    }
}
fn rect(value: crate::foundation::RectF) -> [f32; 4] {
    [value.x, value.y, value.width, value.height]
}
fn clip_slot(id: u32) -> u32 {
    if id == 0 { NO_GPU_SLOT } else { id }
}
fn pack(color: ColorRgba8) -> u32 {
    pack_srgba8(color.r, color.g, color.b, color.a)
}
fn byte_len<T>(values: &[T]) -> u64 {
    values.len().saturating_mul(size_of::<T>()) as u64
}
fn range_bytes(ranges: &DirtyRanges, stride: usize) -> u64 {
    ranges
        .ranges
        .iter()
        .map(|range| range.end.saturating_sub(range.start).saturating_mul(stride) as u64)
        .sum()
}

pub(crate) fn validate_draw_order(order: &[DrawItem]) -> Result<(), &'static str> {
    for draw in order {
        let expected = match draw.kind {
            PrimitiveKind::Box => PipelineKind::AnalyticBox,
            PrimitiveKind::Glyph => PipelineKind::Glyph,
            PrimitiveKind::Image => PipelineKind::Image,
            PrimitiveKind::Material => PipelineKind::Material,
        };
        if draw.batch.pipeline != expected
            && !(draw.kind == PrimitiveKind::Material
                && matches!(
                    draw.batch.pipeline,
                    PipelineKind::LiquidGlass | PipelineKind::GaussianBlur
                ))
        {
            return Err("Vulkan draw item primitive and pipeline kinds disagree");
        }
        if draw.batch.target != 0 {
            return Err("Vulkan retained path accepts only the primary target batch");
        }
    }
    Ok(())
}
pub(crate) fn validate_texture_count(order: &[DrawItem]) -> Result<(), &'static str> {
    let images = order
        .iter()
        .filter(|draw| {
            draw.kind == PrimitiveKind::Image
                || matches!(
                    draw.batch.pipeline,
                    PipelineKind::LiquidGlass | PipelineKind::GaussianBlur
                )
        })
        .map(|draw| (draw.kind as u8, draw.batch.resource))
        .collect::<BTreeSet<_>>()
        .len();
    let reserves_atlas_slot =
        usize::from(images != 0 || order.iter().any(|draw| draw.kind == PrimitiveKind::Glyph));
    if images + reserves_atlas_slot > MAX_TEXTURE_SETS {
        Err("Vulkan scene exceeds the per-frame sampled-image descriptor capacity")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
