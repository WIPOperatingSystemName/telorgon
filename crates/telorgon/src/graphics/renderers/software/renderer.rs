mod color;
mod image_blit;
use image_blit::draw_image;
mod box_paint;
use box_paint::draw_box;
#[cfg(test)]
use box_paint::rounded_coverage;
use color::{decode_target_channel, encode_target_channel, srgb_decode_byte};
#[cfg(any(target_os = "uefi", test))]
mod uefi_fill;

mod scaled;
#[cfg(any(target_os = "linux", test))]
mod composite;
#[cfg(any(target_os = "linux", test))]
mod frame_border;

use std::collections::BTreeMap;
use std::marker::PhantomData;

use crate::foundation::{ColorRgba8, RectF, RectI, SizeF, SizeI};

use crate::graphics::render::{
    BlendMode, Border, BoxInstance, ColorSpace, DamageRegion, DrawItem, GlyphInstance,
    ImageAlphaMode, ImageColorEncoding, ImageId, ImageInstance, ImagePixelFormat,
    ImageResourceDelta, MaterialId, MaterialInstance, MaterialKind, MaterialResource,
    MaterialResourceDelta, PrimitiveKind, ReadbackFormat, ReadbackImage, ReadbackRequest,
    RenderBackend, RenderClip, RenderError, RenderErrorKind, RenderReadback, RenderRequest,
    RenderResult, RenderSceneDelta, RenderSpatialNode, RenderStats, RenderTargetInfo,
    SceneUpdateStats, Shadow, SpatialId, TargetLoad, apply_patches,
};

#[derive(Clone, Debug)]
struct SoftwareImage {
    extent: SizeI,
    color_encoding: ImageColorEncoding,
    alpha_mode: ImageAlphaMode,
    pixel_format: ImagePixelFormat,
    pixels: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct SoftwareRenderer;

#[derive(Clone, Debug, Default)]
pub struct SoftwareScene {
    frame_border: Option<BoxInstance>,
    #[cfg(any(target_os = "linux", test))]
    frame_cache: std::sync::Arc<std::sync::Mutex<Option<frame_border::FrameCache>>>,
    coverage_normalization: Option<crate::graphics::render::frame_border::InteriorPaint>,
    epoch: u64,
    extent: SizeF,
    background: ColorRgba8,
    boxes: Vec<BoxInstance>,
    glyphs: Vec<GlyphInstance>,
    images: Vec<ImageInstance>,
    materials: Vec<MaterialInstance>,
    clips: Vec<RenderClip>,
    spatial: Vec<RenderSpatialNode>,
    draw_order: Vec<DrawItem>,
    atlas_extent: SizeI,
    atlas_a8: Vec<u8>,
    image_resources: BTreeMap<ImageId, SoftwareImage>,
    material_resources: BTreeMap<MaterialId, MaterialResource>,
    pending_damage: DamageRegion,
}

#[derive(Clone, Debug, Default)]
pub struct SoftwareSurface {
    presented_damage: DamageRegion,
    framebuffer_extent: SizeI,
    framebuffer_rgba8: Vec<u8>,
}

pub struct SoftwareFrameContext<'frame> {
    surface: &'frame mut SoftwareSurface,
}

#[cfg(any(target_os = "linux", test))]
pub(crate) struct SoftwareCompositeLayer<'scene> {
    pub scene: &'scene SoftwareScene,
    pub target: RectI,
    pub clip: Option<RectI>,
    pub rounded_clips: [Option<crate::graphics::render::RoundedClip>; 2],
}

#[derive(Copy, Clone, Debug)]
pub struct SoftwareTarget<'frame> {
    info: RenderTargetInfo,
    marker: PhantomData<&'frame ()>,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct SoftwareReadback;

impl RenderBackend for SoftwareRenderer {
    type Scene = SoftwareScene;
    type FrameContext<'frame> = SoftwareFrameContext<'frame>;
    type Target<'frame> = SoftwareTarget<'frame>;

    fn create_scene(&self) -> RenderResult<Self::Scene> {
        Ok(SoftwareScene::default())
    }

    fn apply_scene_delta(
        &self,
        scene: &mut Self::Scene,
        delta: &RenderSceneDelta,
    ) -> RenderResult<SceneUpdateStats> {
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("delta.apply");
        if delta.epoch <= scene.epoch {
            return Ok(SceneUpdateStats {
                epoch: scene.epoch,
                ..SceneUpdateStats::default()
            });
        }
        scene.epoch = delta.epoch;
        scene.extent = delta.extent;
        scene.background = delta.background;
        apply_patches(&mut scene.boxes, &delta.boxes, delta.box_len);
        apply_patches(&mut scene.glyphs, &delta.glyphs, delta.glyph_len);
        apply_patches(&mut scene.images, &delta.images, delta.image_len);
        apply_patches(&mut scene.materials, &delta.materials, delta.material_len);
        apply_patches(&mut scene.clips, &delta.clips, delta.clip_len);
        apply_patches(&mut scene.spatial, &delta.spatial_nodes, delta.spatial_len);
        if let Some(order) = &delta.draw_order {
            scene.draw_order.clear();
            scene.draw_order.extend_from_slice(order);
        }
        scene.atlas_extent = delta.atlas_extent;
        let atlas_len =
            (delta.atlas_extent.width.max(1) * delta.atlas_extent.height.max(1)) as usize;
        if scene.atlas_a8.len() != atlas_len {
            scene.atlas_a8.resize(atlas_len, 0);
        }
        for page in &delta.atlas_pages {
            for row in 0..page.height {
                let source = row as usize * page.width as usize;
                let target = ((page.y + row) * delta.atlas_extent.width + page.x) as usize;
                scene.atlas_a8[target..target + page.width as usize]
                    .copy_from_slice(&page.pixels_a8[source..source + page.width as usize]);
            }
        }
        for update in &delta.image_resources {
            match update {
                ImageResourceDelta::Write(update) => {
                    let image = scene
                        .image_resources
                        .entry(update.image)
                        .or_insert_with(|| SoftwareImage {
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
                        });
                    if image.extent != update.extent || image.pixel_format != update.pixel_format {
                        image.extent = update.extent;
                        image.pixel_format = update.pixel_format;
                        image.pixels.resize(
                            update.extent.width as usize * update.extent.height as usize * 4,
                            0,
                        );
                    }
                    image.color_encoding = update.color_encoding;
                    image.alpha_mode = update.alpha_mode;
                    let destination_stride = update.extent.width as usize * 4;
                    let copy_bytes = update.rect.width as usize * 4;
                    for row in 0..update.rect.height as usize {
                        let source = row * update.row_bytes;
                        let target = (update.rect.y as usize + row) * destination_stride
                            + update.rect.x as usize * 4;
                        image.pixels[target..target + copy_bytes]
                            .copy_from_slice(&update.pixels[source..source + copy_bytes]);
                    }
                }
                ImageResourceDelta::Remove(image) => {
                    scene.image_resources.remove(image);
                }
            }
        }
        for update in &delta.material_resources {
            match update {
                MaterialResourceDelta::Upsert(resource) => {
                    scene
                        .material_resources
                        .insert(resource.material, *resource);
                }
                MaterialResourceDelta::Remove(material) => {
                    scene.material_resources.remove(material);
                }
            }
        }
        if delta.damage.full {
            scene.pending_damage.full = true;
            scene.pending_damage.rects.clear();
        } else {
            for rect in &delta.damage.rects {
                scene.pending_damage.add(*rect, delta.extent);
            }
        }
        Ok(SceneUpdateStats {
            epoch: scene.epoch,
            upload_bytes_queued: 0,
            descriptor_writes_queued: 0,
        })
    }

    fn render<'frame>(
        &self,
        scene: &mut Self::Scene,
        frame: &mut Self::FrameContext<'frame>,
        target: &Self::Target<'frame>,
        request: &RenderRequest,
    ) -> RenderResult<RenderStats> {
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("software.raster.detail");
        validate_target(target.info)?;
        let render_region = request.region.unwrap_or(target.info.region);
        if !rect_contains(target.info.region, render_region) {
            return Err(RenderError::new(
                RenderErrorKind::InvalidTarget,
                "software render region lies outside the target region",
            ));
        }
        let clear = match request.load {
            TargetLoad::Clear(color) => color,
            TargetLoad::Preserve => {
                return Err(RenderError::new(
                    RenderErrorKind::Unsupported,
                    "the software reference backend cannot preserve a host-provided target",
                ));
            }
        };
        let resized = frame.surface.ensure_framebuffer(target.info.extent);
        if request.force || resized {
            scene.pending_damage.full = true;
            scene.pending_damage.rects.clear();
        }
        let recorded = !scene.pending_damage.is_empty();
        let batches = if recorded {
            scene
                .draw_order
                .iter()
                .enumerate()
                .filter(|(index, item)| {
                    *index == 0 || scene.draw_order[index - 1].batch != item.batch
                })
                .count() as u32
        } else {
            0
        };
        if recorded {
            std::mem::swap(
                &mut scene.pending_damage,
                &mut frame.surface.presented_damage,
            );
            scene.pending_damage.full = false;
            scene.pending_damage.rects.clear();
            scene.rasterize_presented_damage(
                frame.surface,
                render_region,
                clear,
                target.info.color_space,
            );
        } else {
            frame.surface.presented_damage.full = false;
            frame.surface.presented_damage.rects.clear();
        }
        let stats = RenderStats {
            recorded,
            epoch: scene.epoch,
            upload_bytes_recorded: 0,
            buffer_copies: 0,
            buffer_allocations: 0,
            descriptor_writes: 0,
            passes: u32::from(recorded),
            barriers: 0,
            batches,
            draws: batches,
            dispatches: 0,
            damage_area: if recorded {
                damage_area(
                    frame.surface.presented_damage.full,
                    &frame.surface.presented_damage.rects,
                    render_region,
                )
            } else {
                0.0
            },
        };
        Ok(stats)
    }
}


impl RenderReadback<SoftwareRenderer> for SoftwareReadback {
    type Pending = ReadbackImage;

    fn record_readback<'frame>(
        &self,
        _backend: &SoftwareRenderer,
        frame: &mut SoftwareFrameContext<'frame>,
        target: &SoftwareTarget<'frame>,
        request: &ReadbackRequest,
    ) -> RenderResult<Self::Pending> {
        if !rect_contains(target.info.region, request.region) {
            return Err(RenderError::new(
                RenderErrorKind::InvalidTarget,
                "software readback region lies outside the target region",
            ));
        }
        frame.surface.readback(request)
    }
}

impl SoftwareSurface {
    pub fn begin_frame(&mut self) -> SoftwareFrameContext<'_> {
        SoftwareFrameContext { surface: self }
    }

    pub fn framebuffer_extent(&self) -> SizeI {
        self.framebuffer_extent
    }

    pub fn pixels_rgba8(&self) -> &[u8] {
        &self.framebuffer_rgba8
    }

    pub fn presented_damage(&self) -> &DamageRegion {
        &self.presented_damage
    }

    pub fn readback(&self, request: &ReadbackRequest) -> RenderResult<ReadbackImage> {
        if request.format != ReadbackFormat::Rgba8 {
            return Err(RenderError::new(
                RenderErrorKind::Unsupported,
                "unsupported software readback format",
            ));
        }
        let bounds = RectI {
            x: 0,
            y: 0,
            width: self.framebuffer_extent.width,
            height: self.framebuffer_extent.height,
        };
        if !rect_contains(bounds, request.region) {
            return Err(RenderError::new(
                RenderErrorKind::InvalidTarget,
                "software readback region lies outside the framebuffer",
            ));
        }
        let row_bytes = request.region.width as usize * 4;
        let mut pixels = Vec::with_capacity(row_bytes * request.region.height as usize);
        let stride = self.framebuffer_extent.width as usize * 4;
        for row in request.region.y..request.region.bottom() {
            let start = row as usize * stride + request.region.x as usize * 4;
            pixels.extend_from_slice(&self.framebuffer_rgba8[start..start + row_bytes]);
        }
        Ok(ReadbackImage {
            extent: SizeI {
                width: request.region.width,
                height: request.region.height,
            },
            row_bytes,
            pixels,
        })
    }

    pub(crate) fn ensure_framebuffer(&mut self, extent: SizeI) -> bool {
        if self.framebuffer_extent == extent && !self.framebuffer_rgba8.is_empty() {
            return false;
        }
        self.framebuffer_extent = extent;
        self.framebuffer_rgba8.resize(
            extent.width.max(1) as usize * extent.height.max(1) as usize * 4,
            0,
        );
        true
    }
}

impl SoftwareTarget<'_> {
    pub fn new(info: RenderTargetInfo) -> Self {
        Self {
            info,
            marker: PhantomData,
        }
    }

    pub fn info(&self) -> RenderTargetInfo {
        self.info
    }
}

impl SoftwareScene {
    pub fn background(&self) -> ColorRgba8 {
        self.background
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn discard_pending_damage(&mut self) {
        self.pending_damage = DamageRegion::default();
    }

    fn rasterize_presented_damage(
        &self,
        surface: &mut SoftwareSurface,
        render_region: RectI,
        clear: ColorRgba8,
        color_space: ColorSpace,
    ) {
        let render_region = rect_i_to_f(render_region);
        if surface.presented_damage.full {
            self.rasterize_region(surface, render_region, clear, color_space);
            return;
        }
        let rects = std::mem::take(&mut surface.presented_damage.rects);
        for rect in rects.iter().copied() {
            if let Some(region) = rect.intersection(render_region) {
                self.rasterize_region(surface, region, clear, color_space);
            }
        }
        surface.presented_damage.rects = rects;
    }

    fn rasterize_region(
        &self,
        surface: &mut SoftwareSurface,
        region: RectF,
        clear: ColorRgba8,
        color_space: ColorSpace,
    ) {
        let width = surface.framebuffer_extent.width.max(1) as usize;
        let height = surface.framebuffer_extent.height.max(1) as usize;
        clear_region(
            &mut surface.framebuffer_rgba8,
            width,
            height,
            region,
            clear,
            color_space,
        );
        let mut target = RasterTarget {
            pixels: &mut surface.framebuffer_rgba8,
            width,
            height,
            origin: crate::foundation::PointI::default(),
            blend_mode: BlendMode::Alpha,
            color_space,
            rounded_clips: [None; 2],
            coverage_normalization: self.coverage_normalization,
        };
        self.draw_region(&mut target, region);
    }

    fn draw_region(&self, target: &mut RasterTarget<'_>, region: RectF) {
        for item in &self.draw_order {
            target.blend_mode = item.batch.blend;
            let item_clip = self
                .clips
                .iter()
                .find(|clip| clip.id == item.batch.clip)
                .filter(|_| item.batch.clip.0 != 0);
            match item.kind {
                PrimitiveKind::Box => {
                    if let Some(instance) = self.boxes.get(item.index as usize) {
                        draw_box(
                            target,
                            instance,
                            self.spatial_for(instance.spatial),
                            item_clip,
                            region,
                        );
                    }
                }
                PrimitiveKind::Glyph => {
                    if let Some(instance) = self.glyphs.get(item.index as usize) {
                        draw_glyph(
                            target,
                            instance,
                            self.spatial_for(instance.spatial),
                            item_clip,
                            region,
                            &self.atlas_a8,
                            self.atlas_extent,
                        );
                    }
                }
                PrimitiveKind::Image => {
                    if let Some(instance) = self.images.get(item.index as usize)
                        && let Some(image) = self.image_resources.get(&instance.image)
                    {
                        draw_image(
                            target,
                            instance,
                            self.spatial_for(instance.spatial),
                            item_clip,
                            region,
                            image,
                        );
                    }
                }
                PrimitiveKind::Material => {
                    if let Some(instance) = self.materials.get(item.index as usize)
                        && let Some(material) = self.material_resources.get(&instance.material)
                    {
                        draw_material(
                            target,
                            instance,
                            self.spatial_for(instance.spatial),
                            item_clip,
                            region,
                            material,
                        );
                    }
                }
            }
        }
    }

    fn spatial_for(&self, id: SpatialId) -> Option<&RenderSpatialNode> {
        self.spatial.iter().find(|spatial| spatial.id == id)
    }
}

struct RasterTarget<'a> {
    coverage_normalization: Option<crate::graphics::render::frame_border::InteriorPaint>,
    pixels: &'a mut [u8],
    width: usize,
    height: usize,
    origin: crate::foundation::PointI,
    blend_mode: BlendMode,
    color_space: ColorSpace,
    rounded_clips: [Option<crate::graphics::render::RoundedClip>; 2],
}

impl RasterTarget<'_> {
    fn interior_paint_coverage(&self, x: i32, y: i32) -> f32 {
        self.coverage_normalization.map_or(1.0, |paint| paint.coverage(
            crate::foundation::PointF { x: x as f32 + 0.5, y: y as f32 + 0.5 }
        ))
    }
    fn normalize_coverage(&self, coverage: f32, x: i32, y: i32) -> f32 {
        let Some(clip) = self.coverage_normalization else { return coverage; };
        let amount = clip.contour.coverage(crate::foundation::PointF {
            x: x as f32 + 0.5, y: y as f32 + 0.5,
        });
        if amount <= 0.0 { 0.0 } else { (coverage / amount).clamp(0.0, 1.0) }
    }
    fn blend_srgba(&mut self, x: i32, y: i32, source: ColorRgba8, opacity: f32) {
        let alpha = (f32::from(source.a) / 255.0) * opacity.clamp(0.0, 1.0);
        let rgb = [
            srgb_decode_byte(source.r) * alpha,
            srgb_decode_byte(source.g) * alpha,
            srgb_decode_byte(source.b) * alpha,
        ];
        self.blend_linear_premultiplied(x, y, rgb, alpha);
    }

    fn blend_linear_premultiplied(
        &mut self,
        x: i32,
        y: i32,
        source_rgb: [f32; 3],
        source_alpha: f32,
    ) {
        let coverage = self.placement_coverage(x, y);
        if coverage <= 0.0 {
            return;
        }
        self.blend_covered_linear_premultiplied(
            x,
            y,
            source_rgb.map(|c| c * coverage),
            source_alpha * coverage,
        );
    }

    fn placement_coverage(&self, x: i32, y: i32) -> f32 {
        self.placement_coverage_with_shadow_overlap(x, y, false)
    }

    fn placement_coverage_with_shadow_overlap(&self, x: i32, y: i32, shadow: bool) -> f32 {
        let x = x.saturating_add(self.origin.x);
        let y = y.saturating_add(self.origin.y);
        self.rounded_clips
            .iter()
            .flatten()
            .fold(1.0_f32, |amount, clip| {
                let coverage = clip.coverage(crate::foundation::PointF {
                    x: x as f32 + 0.5,
                    y: y as f32 + 0.5,
                });
                // The foreground supplies edge AA; exclude only fully interior shadow pixels.
                amount.min(if shadow && clip.inverted {
                    if coverage > 0.0 { 1.0 } else { 0.0 }
                } else {
                    coverage
                })
            })
    }

    /// Blend a source whose geometric coverage already includes the placement clips.
    fn blend_covered_linear_premultiplied(
        &mut self,
        x: i32,
        y: i32,
        source_rgb: [f32; 3],
        source_alpha: f32,
    ) {
        let x = x.saturating_add(self.origin.x);
        let y = y.saturating_add(self.origin.y);
        if x < 0
            || y < 0
            || x as usize >= self.width
            || y as usize >= self.height
            || (source_alpha <= 0.0 && self.blend_mode == BlendMode::Alpha)
        {
            return;
        }
        let index = (y as usize * self.width + x as usize) * 4;
        if index + 3 >= self.pixels.len() {
            return;
        }
        let source_alpha = source_alpha.clamp(0.0, 1.0);
        let inverse = if self.blend_mode == BlendMode::Add {
            1.0
        } else {
            1.0 - source_alpha
        };
        let (rgb, alpha) = if self.blend_mode == BlendMode::Opaque
            && self.rounded_clips.iter().all(Option::is_none)
        {
            (source_rgb, source_alpha)
        } else {
            (
                [
                    source_rgb[0]
                        + decode_target_channel(self.pixels[index], self.color_space) * inverse,
                    source_rgb[1]
                        + decode_target_channel(self.pixels[index + 1], self.color_space) * inverse,
                    source_rgb[2]
                        + decode_target_channel(self.pixels[index + 2], self.color_space) * inverse,
                ],
                source_alpha + f32::from(self.pixels[index + 3]) / 255.0 * inverse,
            )
        };
        self.pixels[index] = encode_target_channel(rgb[0], self.color_space);
        self.pixels[index + 1] = encode_target_channel(rgb[1], self.color_space);
        self.pixels[index + 2] = encode_target_channel(rgb[2], self.color_space);
        self.pixels[index + 3] = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

fn validate_target(info: RenderTargetInfo) -> RenderResult<()> {
    let bounds = RectI {
        x: 0,
        y: 0,
        width: info.extent.width,
        height: info.extent.height,
    };
    if info.extent.width <= 0
        || info.extent.height <= 0
        || info.sample_count != 1
        || !rect_contains(bounds, info.region)
    {
        return Err(RenderError::new(
            RenderErrorKind::InvalidTarget,
            "software target has invalid extent, region, or sample count",
        ));
    }
    if !matches!(info.color_space, ColorSpace::Linear | ColorSpace::Srgb) {
        return Err(RenderError::new(
            RenderErrorKind::Unsupported,
            "software target supports only linear and sRGB color spaces",
        ));
    }
    Ok(())
}

fn rect_contains(outer: RectI, inner: RectI) -> bool {
    inner.width > 0
        && inner.height > 0
        && inner.x >= outer.x
        && inner.y >= outer.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

#[cfg(any(target_os = "linux", test))]
fn intersect_rect_i(left: RectI, right: RectI) -> Option<RectI> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = left.right().min(right.right());
    let bottom = left.bottom().min(right.bottom());
    (right_edge > x && bottom > y).then_some(RectI {
        x,
        y,
        width: right_edge - x,
        height: bottom - y,
    })
}

fn rect_i_to_f(rect: RectI) -> RectF {
    RectF {
        x: rect.x as f32,
        y: rect.y as f32,
        width: rect.width as f32,
        height: rect.height as f32,
    }
}

fn damage_area(full: bool, rects: &[RectF], region: RectI) -> f32 {
    let region = rect_i_to_f(region);
    if full {
        region.area()
    } else {
        rects
            .iter()
            .filter_map(|rect| rect.intersection(region))
            .map(RectF::area)
            .sum()
    }
}

fn clear_region(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    region: RectF,
    color: ColorRgba8,
    color_space: ColorSpace,
) {
    let Some(region) = clip_to_target(region, width, height) else {
        return;
    };
    let alpha = f32::from(color.a) / 255.0;
    let encoded = [
        encode_target_channel(srgb_decode_byte(color.r) * alpha, color_space),
        encode_target_channel(srgb_decode_byte(color.g) * alpha, color_space),
        encode_target_channel(srgb_decode_byte(color.b) * alpha, color_space),
        color.a,
    ];
    let left = region.x.floor().max(0.0) as usize;
    let top = region.y.floor().max(0.0) as usize;
    let right = region.right().ceil().min(width as f32) as usize;
    let bottom = region.bottom().ceil().min(height as f32) as usize;
    for y in top..bottom {
        for pixel in pixels[(y * width + left) * 4..(y * width + right) * 4].chunks_exact_mut(4) {
            pixel.copy_from_slice(&encoded);
        }
    }
}

fn draw_glyph(
    raster: &mut RasterTarget<'_>,
    glyph: &GlyphInstance,
    spatial: Option<&RenderSpatialNode>,
    clip: Option<&RenderClip>,
    region: RectF,
    atlas: &[u8],
    atlas_extent: SizeI,
) {
    let transform = spatial.map_or(crate::foundation::Affine2D::IDENTITY, |value| value.transform);
    let Some(inverse) = transform.inverse() else {
        return;
    };
    let rect = transform.transform_rect(glyph.rect);
    let Some(target) = clip_to_target(
        intersect(
            intersect(rect, clip.map_or(region, |clip| clip.rect)),
            region,
        ),
        raster.width,
        raster.height,
    ) else {
        return;
    };
    for y in target.y.floor() as i32..target.bottom().ceil() as i32 {
        for x in target.x.floor() as i32..target.right().ceil() as i32 {
            let point_x = x as f32 + 0.5;
            let point_y = y as f32 + 0.5;
            let clip_amount = raster.normalize_coverage(clip_coverage(point_x, point_y, clip).min(raster.interior_paint_coverage(x, y)), x, y);
            if clip_amount <= 0.0 {
                continue;
            }
            let local = inverse.transform_point(crate::foundation::PointF {
                x: point_x,
                y: point_y,
            });
            if !glyph.rect.contains(local) {
                continue;
            }
            let atlas_x = glyph.atlas_x as f32
                + (local.x - glyph.rect.x) * glyph.atlas_size.width as f32 / glyph.rect.width;
            let atlas_y = glyph.atlas_y as f32
                + (local.y - glyph.rect.y) * glyph.atlas_size.height as f32 / glyph.rect.height;
            let coverage = sample_a8_linear(
                atlas,
                atlas_extent.width,
                atlas_extent.height,
                atlas_x,
                atlas_y,
            );
            raster.blend_srgba(x, y, glyph.color, coverage * glyph.opacity * clip_amount);
        }
    }
}

fn draw_material(
    raster: &mut RasterTarget<'_>,
    instance: &MaterialInstance,
    spatial: Option<&RenderSpatialNode>,
    clip: Option<&RenderClip>,
    region: RectF,
    material: &MaterialResource,
) {
    let transform = spatial.map_or(crate::foundation::Affine2D::IDENTITY, |value| value.transform);
    let Some(inverse) = transform.inverse() else {
        return;
    };
    let rect = transform.transform_rect(instance.rect);
    let Some(target) = clip_to_target(
        intersect(
            intersect(rect, clip.map_or(region, |clip| clip.rect)),
            region,
        ),
        raster.width,
        raster.height,
    ) else {
        return;
    };
    for y in target.y.floor() as i32..target.bottom().ceil() as i32 {
        for x in target.x.floor() as i32..target.right().ceil() as i32 {
            let point_x = x as f32 + 0.5;
            let point_y = y as f32 + 0.5;
            let clip_amount = raster.normalize_coverage(clip_coverage(point_x, point_y, clip).min(raster.interior_paint_coverage(x, y)), x, y);
            if clip_amount <= 0.0 {
                continue;
            }
            let local = inverse.transform_point(crate::foundation::PointF {
                x: point_x,
                y: point_y,
            });
            let amount = match material.kind {
                MaterialKind::Solid | MaterialKind::LiquidGlass(_) | MaterialKind::GaussianBlur(_) => 0.0,
                MaterialKind::LinearGradientHorizontal => {
                    (local.x - instance.rect.x) / instance.rect.width
                }
                MaterialKind::LinearGradientVertical => {
                    (local.y - instance.rect.y) / instance.rect.height
                }
            }
            .clamp(0.0, 1.0);
            raster.blend_srgba(
                x,
                y,
                lerp_color(material.colors[0], material.colors[1], amount),
                instance.opacity * clip_amount,
            );
        }
    }
}

fn clip_coverage(x: f32, y: f32, clip: Option<&RenderClip>) -> f32 {
    let Some(clip) = clip else {
        return 1.0;
    };
    let point = crate::foundation::PointF { x, y };
    crate::graphics::render::RoundedClip::new(clip.rect, clip.corner_radii).coverage(point)
}

fn lerp_color(first: ColorRgba8, second: ColorRgba8, amount: f32) -> ColorRgba8 {
    let channel = |first: u8, second: u8| {
        (first as f32 + (second as f32 - first as f32) * amount).round() as u8
    };
    ColorRgba8::rgba(
        channel(first.r, second.r),
        channel(first.g, second.g),
        channel(first.b, second.b),
        channel(first.a, second.a),
    )
}

fn intersect(a: RectF, b: RectF) -> RectF {
    a.intersection(b).unwrap_or(RectF::ZERO)
}
fn clip_to_target(rect: RectF, width: usize, height: usize) -> Option<RectF> {
    rect.intersection(RectF {
        x: 0.0,
        y: 0.0,
        width: width as f32,
        height: height as f32,
    })
}

fn sample_a8_linear(pixels: &[u8], width: i32, height: i32, texel_x: f32, texel_y: f32) -> f32 {
    if width <= 0 || height <= 0 || pixels.is_empty() {
        return 0.0;
    }
    let x = texel_x - 0.5;
    let y = texel_y - 0.5;
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let amount_x = x - x.floor();
    let amount_y = y - y.floor();
    let fetch = |x: i32, y: i32| {
        let x = x.clamp(0, width - 1) as usize;
        let y = y.clamp(0, height - 1) as usize;
        pixels
            .get(y * width as usize + x)
            .map_or(0.0, |value| f32::from(*value) / 255.0)
    };
    let top = lerp(fetch(x0, y0), fetch(x0 + 1, y0), amount_x);
    let bottom = lerp(fetch(x0, y0 + 1), fetch(x0 + 1, y0 + 1), amount_x);
    lerp(top, bottom, amount_y)
}

fn sample_image_linear(
    pixels: &[u8],
    extent: SizeI,
    encoding: ImageColorEncoding,
    pixel_format: ImagePixelFormat,
    u: f32,
    v: f32,
) -> [f32; 4] {
    if extent.width <= 0 || extent.height <= 0 || pixels.is_empty() {
        return [0.0; 4];
    }
    let x = u * extent.width as f32 - 0.5;
    let y = v * extent.height as f32 - 0.5;
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let amount_x = x - x.floor();
    let amount_y = y - y.floor();
    let fetch = |x: i32, y: i32| {
        let x = x.clamp(0, extent.width - 1) as usize;
        let y = y.clamp(0, extent.height - 1) as usize;
        let index = (y * extent.width as usize + x) * 4;
        let decode = |channel: u8| match encoding {
            ImageColorEncoding::Linear => f32::from(channel) / 255.0,
            ImageColorEncoding::Srgb => srgb_decode_byte(channel),
        };
        let channels = &pixels[index..index + 4];
        let (red, green, blue) = match pixel_format {
            ImagePixelFormat::Rgba8 => (channels[0], channels[1], channels[2]),
            ImagePixelFormat::Bgra8 => (channels[2], channels[1], channels[0]),
        };
        [
            decode(red),
            decode(green),
            decode(blue),
            f32::from(channels[3]) / 255.0,
        ]
    };
    let top_left = fetch(x0, y0);
    let top_right = fetch(x0 + 1, y0);
    let bottom_left = fetch(x0, y0 + 1);
    let bottom_right = fetch(x0 + 1, y0 + 1);
    std::array::from_fn(|channel| {
        lerp(
            lerp(top_left[channel], top_right[channel], amount_x),
            lerp(bottom_left[channel], bottom_right[channel], amount_x),
            amount_y,
        )
    })
}

fn lerp(first: f32, second: f32, amount: f32) -> f32 {
    first + (second - first) * amount
}

#[cfg(test)]
mod tests;
