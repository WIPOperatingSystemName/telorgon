use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use crate::foundation::{Affine2D, ColorRgba8, RectF, SizeF, SizeI};
use crate::ui::layout::{ClipId, SpatialId};
use crate::graphics::scene::{NodeId, SparseSet};
use crate::ui::text::AtlasPageUpdate;
use crate::ui::{Border, CornerRadii, ImageId, MaterialId, Outline, ShadowList};

use crate::graphics::render::{RenderError, RenderErrorKind, RenderResult};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirtyRanges {
    pub ranges: Vec<Range<usize>>,
}
impl DirtyRanges {
    pub fn add(&mut self, mut range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        let mut index = 0;
        while index < self.ranges.len() {
            if self.ranges[index].end < range.start {
                index += 1;
                continue;
            }
            if self.ranges[index].start > range.end {
                break;
            }
            let current = self.ranges.remove(index);
            range.start = range.start.min(current.start);
            range.end = range.end.max(current.end);
        }
        self.ranges.insert(index, range);
    }
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }
}

#[cfg(test)]
mod dirty_range_tests;

#[derive(Clone, Debug)]
pub struct DenseInstances<T> {
    owners: Vec<NodeId>,
    values: Vec<T>,
    sparse: SparseSet<u32>,
    dirty: DirtyRanges,
}
impl<T> Default for DenseInstances<T> {
    fn default() -> Self {
        Self {
            owners: Vec::new(),
            values: Vec::new(),
            sparse: SparseSet::default(),
            dirty: DirtyRanges::default(),
        }
    }
}
impl<T> DenseInstances<T> {
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    pub fn values(&self) -> &[T] {
        &self.values
    }
    pub fn owners(&self) -> &[NodeId] {
        &self.owners
    }
    pub fn index(&self, owner: NodeId) -> Option<usize> {
        self.sparse.get(owner).copied().map(|value| value as usize)
    }
    pub fn get(&self, owner: NodeId) -> Option<&T> {
        self.index(owner).and_then(|index| self.values.get(index))
    }
    pub fn remove(&mut self, owner: NodeId) -> Option<T> {
        let index = self.index(owner)?;
        self.sparse.remove(owner);
        self.owners.swap_remove(index);
        let removed = self.values.swap_remove(index);
        if index < self.owners.len() {
            self.sparse.insert(self.owners[index], index as u32);
        }
        self.dirty.add(index..self.values.len().max(index + 1));
        Some(removed)
    }
}
impl<T: PartialEq> DenseInstances<T> {
    pub fn upsert(&mut self, owner: NodeId, value: T) -> bool {
        if let Some(index) = self.index(owner) {
            if self.values[index] == value {
                return false;
            }
            self.values[index] = value;
            self.dirty.add(index..index + 1);
            true
        } else {
            let index = self.values.len();
            self.owners.push(owner);
            self.values.push(value);
            self.sparse.insert(owner, index as u32);
            self.dirty.add(index..index + 1);
            true
        }
    }
}
impl<T: Clone> DenseInstances<T> {
    fn take_patches(&mut self) -> Vec<RangePatch<T>> {
        self.dirty
            .take()
            .ranges
            .into_iter()
            .map(|range| {
                let end = range.end.min(self.values.len());
                let start = range.start.min(end);
                RangePatch {
                    start,
                    values: self.values[start..end].to_vec().into(),
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoxInstance {
    pub node: NodeId,
    /// Primitive-local geometry transformed by `spatial` during rendering.
    pub rect: RectF,
    /// Conservative view-space bounds used only for damage and culling.
    pub view_bounds: RectF,
    pub background: Option<ColorRgba8>,
    pub border: Border,
    pub outline: Outline,
    pub corner_radii: CornerRadii,
    pub shadows: ShadowList,
    pub opacity: f32,
    pub clip: ClipId,
    pub spatial: SpatialId,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GlyphInstance {
    pub node: NodeId,
    pub rect: RectF,
    pub view_bounds: RectF,
    pub atlas_x: i32,
    pub atlas_y: i32,
    /// Physical atlas texels, independent of the logical glyph rectangle.
    pub atlas_size: SizeI,
    pub color: ColorRgba8,
    pub opacity: f32,
    pub clip: ClipId,
    pub spatial: SpatialId,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ImageColorEncoding {
    Linear,
    #[default]
    Srgb,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ImageAlphaMode {
    #[default]
    Straight,
    Premultiplied,
    Opaque,
}

/// Byte order of one four-channel image texel in CPU memory.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ImagePixelFormat {
    #[default]
    Rgba8,
    Bgra8,
}

impl ImagePixelFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        4
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageResource {
    pub image: ImageId,
    pub content_version: u64,
    pub extent: SizeI,
    pub color_encoding: ImageColorEncoding,
    pub alpha_mode: ImageAlphaMode,
    pub pixel_format: ImagePixelFormat,
    pub pixels: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageResourceUpdate {
    pub image: ImageId,
    pub content_version: u64,
    pub extent: SizeI,
    pub rect: crate::foundation::RectI,
    pub row_bytes: usize,
    pub color_encoding: ImageColorEncoding,
    pub alpha_mode: ImageAlphaMode,
    pub pixel_format: ImagePixelFormat,
    pub pixels: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ImageResourceDelta {
    Write(ImageResourceUpdate),
    Remove(ImageId),
}

/// Render-native liquid-glass parameters. Distances use the material's local pixel space.
/// The shader receives 16 packed f32 words (64 bytes), separate from image ownership.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LiquidGlassMaterial {
    pub backdrop: ImageId,
    /// Full-resolution unfiltered source for the refracted rim.
    pub sharp_backdrop: ImageId,
    /// Top-left, top-right, bottom-right, bottom-left.
    pub radii: [f32; 4],
    pub inverse_output_size: [f32; 2],
    pub inverse_bevel: f32,
    pub blend_softness: f32,
    pub refraction: f32,
    pub dispersion: f32,
    pub fresnel: f32,
    /// Linear RGB already multiplied by tint strength; W is remaining transmission.
    pub tint: [f32; 4],
}

impl LiquidGlassMaterial {
    pub(crate) fn words(self) -> [u32; 15] {
        [
            self.radii[0], self.radii[1], self.radii[2], self.radii[3],
            self.inverse_output_size[0], self.inverse_output_size[1],
            self.inverse_bevel, self.refraction,
            self.dispersion, self.fresnel,
            self.tint[0], self.tint[1], self.tint[2], self.tint[3],
            self.blend_softness,
        ].map(f32::to_bits)
    }

    pub(crate) fn valid(self) -> bool {
        self.words().into_iter().all(|v| f32::from_bits(v).is_finite())
            && self.radii.into_iter().all(|v| v >= 0.0)
            && self.inverse_output_size.into_iter().all(|v| v > 0.0)
            && self.inverse_bevel > 0.0
            && self.blend_softness >= 0.0
            && self.refraction >= 0.0
            && self.dispersion >= 0.0
            && (0.0..=1.0).contains(&self.fresnel)
            && self.tint.into_iter().all(|v| (0.0..=1.0).contains(&v))
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum MaterialKind {
    #[default]
    Solid,
    LinearGradientHorizontal,
    LinearGradientVertical,
    /// Sampled rounded-window lens; the backdrop must be an owned image in this scene.
    LiquidGlass(LiquidGlassMaterial),
    /// One axis of a full-resolution Gaussian filter over an owned image.
    GaussianBlur(super::GaussianBlurMaterial),
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct MaterialResource {
    pub material: MaterialId,
    pub content_version: u64,
    pub kind: MaterialKind,
    pub colors: [ColorRgba8; 2],
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum MaterialResourceDelta {
    Upsert(MaterialResource),
    Remove(MaterialId),
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ImageInstance {
    pub node: NodeId,
    pub image: ImageId,
    pub tint: Option<ColorRgba8>,
    pub rect: RectF,
    pub view_bounds: RectF,
    pub content_version: u64,
    pub opacity: f32,
    pub clip: ClipId,
    pub spatial: SpatialId,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MaterialInstance {
    pub node: NodeId,
    pub material: MaterialId,
    pub rect: RectF,
    pub view_bounds: RectF,
    pub opacity: f32,
    pub clip: ClipId,
    pub spatial: SpatialId,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RenderClip {
    pub id: ClipId,
    pub rect: RectF,
    pub corner_radii: CornerRadii,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RenderSpatialNode {
    pub id: SpatialId,
    pub transform: Affine2D,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrimitiveKind {
    Box,
    Glyph,
    Image,
    Material,
}
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PipelineKind {
    GaussianBlur,
    LiquidGlass,
    AnalyticBox,
    Glyph,
    Image,
    Material,
}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlendMode {
    /// Sum premultiplied contributions, used for isolated weighted crossfades.
    Add,
    Opaque,
    #[default]
    Alpha,
}
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BatchKey {
    pub pipeline: PipelineKind,
    pub resource: u32,
    pub clip: ClipId,
    pub blend: BlendMode,
    pub target: u16,
}
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DrawItem {
    pub kind: PrimitiveKind,
    pub index: u32,
    pub batch: BatchKey,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DamageRegion {
    pub rects: Vec<RectF>,
    pub full: bool,
    pub merge_distance: f32,
    pub full_threshold: f32,
}
impl Default for DamageRegion {
    fn default() -> Self {
        Self {
            rects: Vec::with_capacity(8),
            full: false,
            merge_distance: 4.0,
            full_threshold: 0.6,
        }
    }
}
impl DamageRegion {
    pub fn add(&mut self, rect: RectF, extent: SizeF) {
        if self.full || rect.area() <= 0.0 {
            return;
        }
        let expanded = rect.inflate(1.0);
        if let Some(existing) = self.rects.iter_mut().find(|item| {
            item.inflate(self.merge_distance)
                .intersection(expanded)
                .is_some()
        }) {
            *existing = existing.union(expanded);
        } else {
            self.rects.push(expanded);
        }
        let damaged: f32 = self.rects.iter().map(|rect| rect.area()).sum();
        if damaged >= extent.width * extent.height * self.full_threshold {
            self.full = true;
            self.rects.clear();
        }
    }
    pub fn is_empty(&self) -> bool {
        !self.full && self.rects.is_empty()
    }
    pub fn take(&mut self) -> Self {
        let mut next = Self::default();
        std::mem::swap(self, &mut next);
        next
    }
}

#[derive(Clone, Debug)]
pub struct RangePatch<T> {
    pub start: usize,
    pub values: Arc<[T]>,
}

#[derive(Clone, Debug)]
pub struct RenderSceneDelta {
    pub epoch: u64,
    pub extent: SizeF,
    pub background: ColorRgba8,
    pub boxes: Vec<RangePatch<BoxInstance>>,
    pub box_len: usize,
    pub glyphs: Vec<RangePatch<GlyphInstance>>,
    pub glyph_len: usize,
    pub images: Vec<RangePatch<ImageInstance>>,
    pub image_len: usize,
    pub materials: Vec<RangePatch<MaterialInstance>>,
    pub material_len: usize,
    pub clips: Vec<RangePatch<RenderClip>>,
    pub clip_len: usize,
    pub spatial_nodes: Vec<RangePatch<RenderSpatialNode>>,
    pub spatial_len: usize,
    pub draw_order: Option<Arc<[DrawItem]>>,
    pub damage: DamageRegion,
    pub atlas_extent: SizeI,
    pub atlas_pages: Vec<AtlasPageUpdate>,
    pub image_resources: Vec<ImageResourceDelta>,
    pub material_resources: Vec<MaterialResourceDelta>,
}

#[derive(Clone, Debug, PartialEq)]
struct RetainedImageResource {
    image: ImageId,
    content_version: u64,
    extent: SizeI,
    color_encoding: ImageColorEncoding,
    alpha_mode: ImageAlphaMode,
    pixel_format: ImagePixelFormat,
    pixels: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub struct RenderScene {
    pub extent: SizeF,
    pub background: ColorRgba8,
    pub draw_order: Vec<DrawItem>,
    pub boxes: DenseInstances<BoxInstance>,
    pub glyphs: Vec<GlyphInstance>,
    pub images: DenseInstances<ImageInstance>,
    pub materials: DenseInstances<MaterialInstance>,
    pub clips: DenseInstances<RenderClip>,
    pub spatial_nodes: DenseInstances<RenderSpatialNode>,
    pub dirty_ranges: DirtyRanges,
    pub damage: DamageRegion,
    epoch: u64,
    glyph_dirty: DirtyRanges,
    draw_order_dirty: bool,
    atlas_extent: SizeI,
    atlas_pages: Vec<AtlasPageUpdate>,
    image_resources: BTreeMap<ImageId, RetainedImageResource>,
    image_resource_updates: Vec<ImageResourceDelta>,
    material_resources: BTreeMap<MaterialId, MaterialResource>,
    material_resource_updates: Vec<MaterialResourceDelta>,
}
impl Default for RenderScene {
    fn default() -> Self {
        Self {
            extent: SizeF::default(),
            background: ColorRgba8::default(),
            draw_order: Vec::new(),
            boxes: DenseInstances::default(),
            glyphs: Vec::new(),
            images: DenseInstances::default(),
            materials: DenseInstances::default(),
            clips: DenseInstances::default(),
            spatial_nodes: DenseInstances::default(),
            dirty_ranges: DirtyRanges::default(),
            damage: DamageRegion::default(),
            epoch: 0,
            glyph_dirty: DirtyRanges::default(),
            draw_order_dirty: true,
            atlas_extent: SizeI {
                width: 1,
                height: 1,
            },
            atlas_pages: Vec::new(),
            image_resources: BTreeMap::new(),
            image_resource_updates: Vec::new(),
            material_resources: BTreeMap::new(),
            material_resource_updates: Vec::new(),
        }
    }
}
impl RenderScene {
    pub fn set_image_resource(&mut self, resource: ImageResource) -> RenderResult<()> {
        validate_image_resource(&resource)?;
        if self
            .image_resources
            .get(&resource.image)
            .is_some_and(|current| resource.content_version < current.content_version)
        {
            return Err(RenderError::new(
                RenderErrorKind::InvalidScene,
                "image resource content version cannot move backwards",
            ));
        }
        if self
            .image_resources
            .get(&resource.image)
            .is_some_and(|current| {
                current.content_version == resource.content_version
                    && current.extent == resource.extent
                    && current.color_encoding == resource.color_encoding
                    && current.alpha_mode == resource.alpha_mode
                    && current.pixel_format == resource.pixel_format
                    && current.pixels.as_ref() == resource.pixels.as_ref()
            })
        {
            return Ok(());
        }
        let row_bytes = resource.extent.width as usize * 4;
        let update = ImageResourceUpdate {
            image: resource.image,
            content_version: resource.content_version,
            extent: resource.extent,
            rect: crate::foundation::RectI {
                x: 0,
                y: 0,
                width: resource.extent.width,
                height: resource.extent.height,
            },
            row_bytes,
            color_encoding: resource.color_encoding,
            alpha_mode: resource.alpha_mode,
            pixel_format: resource.pixel_format,
            pixels: Arc::clone(&resource.pixels),
        };
        let image = resource.image;
        let extent = update.extent;
        let rect = update.rect;
        self.image_resources.insert(
            image,
            RetainedImageResource {
                image: resource.image,
                content_version: resource.content_version,
                extent: resource.extent,
                color_encoding: resource.color_encoding,
                alpha_mode: resource.alpha_mode,
                pixel_format: resource.pixel_format,
                pixels: Arc::clone(&resource.pixels),
            },
        );
        self.image_resource_updates
            .push(ImageResourceDelta::Write(update));
        self.damage_image_instance_regions(image, extent, rect);
        Ok(())
    }

    pub fn update_image_resource_region(
        &mut self,
        update: ImageResourceUpdate,
    ) -> RenderResult<()> {
        validate_image_update(&update)?;
        let resource = self.image_resources.get_mut(&update.image).ok_or_else(|| {
            RenderError::new(
                RenderErrorKind::InvalidScene,
                "image region update references a missing resource",
            )
        })?;
        if resource.extent != update.extent
            || resource.color_encoding != update.color_encoding
            || resource.alpha_mode != update.alpha_mode
            || resource.pixel_format != update.pixel_format
            || update.content_version < resource.content_version
        {
            return Err(RenderError::new(
                RenderErrorKind::InvalidScene,
                "image region update metadata does not match the retained resource",
            ));
        }
        let destination_stride = resource.extent.width as usize * 4;
        let copy_bytes = update.rect.width as usize * 4;
        let pixels = Arc::make_mut(&mut resource.pixels);
        for row in 0..update.rect.height as usize {
            let source = row * update.row_bytes;
            let target =
                (update.rect.y as usize + row) * destination_stride + update.rect.x as usize * 4;
            pixels[target..target + copy_bytes]
                .copy_from_slice(&update.pixels[source..source + copy_bytes]);
        }
        let image = update.image;
        let extent = update.extent;
        let rect = update.rect;
        resource.content_version = update.content_version;
        self.image_resource_updates
            .push(ImageResourceDelta::Write(update));
        self.damage_image_instance_regions(image, extent, rect);
        Ok(())
    }

    pub fn remove_image_resource(&mut self, image: ImageId) -> bool {
        let removed = self.image_resources.remove(&image).is_some();
        if removed {
            self.image_resource_updates
                .push(ImageResourceDelta::Remove(image));
            self.damage_image_instances(image);
        }
        removed
    }

    fn damage_image_instances(&mut self, image: ImageId) {
        let bounds = self
            .images
            .values()
            .iter()
            .filter(|instance| instance.image == image)
            .map(|instance| instance.view_bounds)
            .collect::<Vec<_>>();
        for bounds in bounds {
            self.damage.add(bounds, self.extent);
        }
    }

    fn damage_image_instance_regions(
        &mut self,
        image: ImageId,
        source_extent: SizeI,
        source_damage: crate::foundation::RectI,
    ) {
        let scale_x = 1.0 / source_extent.width as f32;
        let scale_y = 1.0 / source_extent.height as f32;
        let bounds = self
            .images
            .values()
            .iter()
            .filter(|instance| instance.image == image)
            .filter_map(|instance| {
                let damage = crate::foundation::RectF {
                    x: instance.rect.x + source_damage.x as f32 * scale_x * instance.rect.width,
                    y: instance.rect.y + source_damage.y as f32 * scale_y * instance.rect.height,
                    width: source_damage.width as f32 * scale_x * instance.rect.width,
                    height: source_damage.height as f32 * scale_y * instance.rect.height,
                };
                damage.intersection(instance.view_bounds)
            })
            .collect::<Vec<_>>();
        for bounds in bounds {
            self.damage.add(bounds, self.extent);
        }
    }

    pub fn set_material_resource(&mut self, resource: MaterialResource) {
        if self.material_resources.get(&resource.material) == Some(&resource) {
            return;
        }
        self.material_resources.insert(resource.material, resource);
        self.material_resource_updates
            .push(MaterialResourceDelta::Upsert(resource));
        self.damage.full = true;
        self.damage.rects.clear();
    }

    pub fn remove_material_resource(&mut self, material: MaterialId) -> bool {
        let removed = self.material_resources.remove(&material).is_some();
        if removed {
            self.material_resource_updates
                .push(MaterialResourceDelta::Remove(material));
            self.damage.full = true;
            self.damage.rects.clear();
        }
        removed
    }

    pub(crate) fn patch_glyph(&mut self, index: usize, glyph: GlyphInstance) {
        if self.glyphs[index] != glyph {
            self.draw_order_dirty |= self.glyphs[index].clip != glyph.clip;
            self.glyphs[index] = glyph;
            self.glyph_dirty.add(index..index + 1);
        }
    }

    pub fn set_glyphs(&mut self, glyphs: Vec<GlyphInstance>) {
        if self.glyphs != glyphs {
            let end = self.glyphs.len().max(glyphs.len());
            self.glyphs = glyphs;
            self.glyph_dirty.add(0..end);
            self.draw_order_dirty = true;
        }
    }
    pub fn replace_glyphs(&mut self, mut glyphs: Vec<GlyphInstance>) -> Vec<GlyphInstance> {
        if self.glyphs == glyphs {
            glyphs.clear();
            return glyphs;
        }
        let end = self.glyphs.len().max(glyphs.len());
        std::mem::swap(&mut self.glyphs, &mut glyphs);
        self.glyph_dirty.add(0..end);
        self.draw_order_dirty = true;
        glyphs.clear();
        glyphs
    }
    pub fn set_draw_order(&mut self, order: Vec<DrawItem>) {
        if self.draw_order != order {
            self.draw_order = order;
            self.draw_order_dirty = true;
        }
    }
    pub fn replace_draw_order(&mut self, mut order: Vec<DrawItem>) -> Vec<DrawItem> {
        if self.draw_order == order {
            order.clear();
            return order;
        }
        std::mem::swap(&mut self.draw_order, &mut order);
        self.draw_order_dirty = true;
        order.clear();
        order
    }
    pub fn set_atlas_updates(&mut self, extent: SizeI, pages: Vec<AtlasPageUpdate>) {
        self.atlas_extent = extent;
        self.atlas_pages.extend(pages);
    }
    pub fn take_delta(&mut self) -> Option<RenderSceneDelta> {
        if self.damage.is_empty()
            && !self.draw_order_dirty
            && self.boxes.dirty.ranges.is_empty()
            && self.glyph_dirty.ranges.is_empty()
            && self.images.dirty.ranges.is_empty()
            && self.materials.dirty.ranges.is_empty()
            && self.clips.dirty.ranges.is_empty()
            && self.spatial_nodes.dirty.ranges.is_empty()
            && self.atlas_pages.is_empty()
            && self.image_resource_updates.is_empty()
            && self.material_resource_updates.is_empty()
        {
            return None;
        }
        self.epoch += 1;
        let glyphs = self
            .glyph_dirty
            .take()
            .ranges
            .into_iter()
            .map(|range| {
                let end = range.end.min(self.glyphs.len());
                let start = range.start.min(end);
                RangePatch {
                    start,
                    values: self.glyphs[start..end].to_vec().into(),
                }
            })
            .collect();
        Some(RenderSceneDelta {
            epoch: self.epoch,
            extent: self.extent,
            background: self.background,
            boxes: self.boxes.take_patches(),
            box_len: self.boxes.len(),
            glyphs,
            glyph_len: self.glyphs.len(),
            images: self.images.take_patches(),
            image_len: self.images.len(),
            materials: self.materials.take_patches(),
            material_len: self.materials.len(),
            clips: self.clips.take_patches(),
            clip_len: self.clips.len(),
            spatial_nodes: self.spatial_nodes.take_patches(),
            spatial_len: self.spatial_nodes.len(),
            draw_order: self
                .draw_order_dirty
                .then(|| self.draw_order.clone().into()),
            damage: self.damage.take(),
            atlas_extent: self.atlas_extent,
            atlas_pages: std::mem::take(&mut self.atlas_pages),
            image_resources: std::mem::take(&mut self.image_resource_updates),
            material_resources: std::mem::take(&mut self.material_resource_updates),
        })
        .inspect(|_| self.draw_order_dirty = false)
    }

    pub fn snapshot_delta(
        &mut self,
        atlas_extent: SizeI,
        atlas_pages: Vec<AtlasPageUpdate>,
    ) -> RenderSceneDelta {
        self.epoch += 1;
        RenderSceneDelta {
            epoch: self.epoch,
            extent: self.extent,
            background: self.background,
            boxes: vec![RangePatch {
                start: 0,
                values: self.boxes.values.clone().into(),
            }],
            box_len: self.boxes.len(),
            glyphs: vec![RangePatch {
                start: 0,
                values: self.glyphs.clone().into(),
            }],
            glyph_len: self.glyphs.len(),
            images: vec![RangePatch {
                start: 0,
                values: self.images.values.clone().into(),
            }],
            image_len: self.images.len(),
            materials: vec![RangePatch {
                start: 0,
                values: self.materials.values.clone().into(),
            }],
            material_len: self.materials.len(),
            clips: vec![RangePatch {
                start: 0,
                values: self.clips.values.clone().into(),
            }],
            clip_len: self.clips.len(),
            spatial_nodes: vec![RangePatch {
                start: 0,
                values: self.spatial_nodes.values.clone().into(),
            }],
            spatial_len: self.spatial_nodes.len(),
            draw_order: Some(self.draw_order.clone().into()),
            damage: DamageRegion {
                full: true,
                ..DamageRegion::default()
            },
            atlas_extent,
            atlas_pages,
            image_resources: self
                .image_resources
                .values()
                .map(full_image_update)
                .map(ImageResourceDelta::Write)
                .collect(),
            material_resources: self
                .material_resources
                .values()
                .copied()
                .map(MaterialResourceDelta::Upsert)
                .collect(),
        }
    }
}

fn full_image_update(resource: &RetainedImageResource) -> ImageResourceUpdate {
    ImageResourceUpdate {
        image: resource.image,
        content_version: resource.content_version,
        extent: resource.extent,
        rect: crate::foundation::RectI {
            x: 0,
            y: 0,
            width: resource.extent.width,
            height: resource.extent.height,
        },
        row_bytes: resource.extent.width as usize * 4,
        color_encoding: resource.color_encoding,
        alpha_mode: resource.alpha_mode,
        pixel_format: resource.pixel_format,
        pixels: Arc::clone(&resource.pixels),
    }
}

fn validate_image_resource(resource: &ImageResource) -> RenderResult<()> {
    if resource.extent.width <= 0 || resource.extent.height <= 0 {
        return Err(RenderError::new(
            RenderErrorKind::InvalidScene,
            "image resource extent must be positive",
        ));
    }
    let required = resource.extent.width as usize * resource.extent.height as usize * 4;
    if resource.pixels.len() != required {
        return Err(RenderError::new(
            RenderErrorKind::InvalidScene,
            "full image resource payload must be tightly packed four-channel texels",
        ));
    }
    Ok(())
}

fn validate_image_update(update: &ImageResourceUpdate) -> RenderResult<()> {
    let rect = update.rect;
    if update.extent.width <= 0
        || update.extent.height <= 0
        || rect.x < 0
        || rect.y < 0
        || rect.width <= 0
        || rect.height <= 0
        || rect.right() > update.extent.width
        || rect.bottom() > update.extent.height
        || update.row_bytes < rect.width as usize * 4
        || !update.row_bytes.is_multiple_of(4)
        || update.pixels.len() < update.row_bytes * rect.height as usize
    {
        return Err(RenderError::new(
            RenderErrorKind::InvalidScene,
            "image resource update has invalid extent, rectangle, stride, or payload",
        ));
    }
    Ok(())
}

#[doc(hidden)]
pub fn apply_patches<T: Clone>(target: &mut Vec<T>, patches: &[RangePatch<T>], final_len: usize) {
    for patch in patches {
        let required = patch.start + patch.values.len();
        if target.len() < required
            && let Some(seed) = patch.values.first()
        {
            target.resize(required, seed.clone());
        }
        if !patch.values.is_empty() {
            target[patch.start..required].clone_from_slice(&patch.values);
        }
    }
    target.truncate(final_len);
}

#[cfg(test)]
mod tests;
