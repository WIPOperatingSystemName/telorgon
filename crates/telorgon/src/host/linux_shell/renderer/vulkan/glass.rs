//! Native sharp captures and scale-aware separable filtering with owned snapshot lifetime.
//! No readback, extra submission, or timer: lower-scene revisions drive cache invalidation.
use super::super::super::scene::{ShellLayerKey, ShellPlacement};
use super::*;
use crate::graphics::render::{
    LiquidGlassMaterial, MaterialId, MaterialInstance, MaterialKind, MaterialResource, RoundedClip,
};
use crate::graphics::renderers::vulkan::VulkanFrameContext;

// Four simultaneous quadrant previews each own a sharp capture and two blur targets.
// Reserve 25% headroom for Vulkan image padding/alignment while retaining a hard ceiling.
const MAX_BACKDROP_BUDGET_BYTES: u64 = 512 * 1024 * 1024;
fn backdrop_budget(output: SizeI) -> u64 {
    (output.width.max(0) as u64)
        .saturating_mul(output.height.max(0) as u64)
        .saturating_mul(4 * 3 * 5)
        .clamp(64 * 1024 * 1024, MAX_BACKDROP_BUDGET_BYTES)
}

fn backdrop_fits_budget(output: SizeI, extents: &[SizeI], other_bytes: u64) -> bool {
    extents.iter().fold(other_bytes, |bytes, extent| {
        bytes.saturating_add(
            (extent.width.max(0) as u64)
                .saturating_mul(extent.height.max(0) as u64)
                .saturating_mul(4),
        )
    }) <= backdrop_budget(output)
}

#[derive(Clone, Debug, PartialEq)]
struct Signature {
    extent: SizeI,
    region: RectI,
    blur_radius: f32,
    // Scene epoch tracks lens geometry/optics; backdrop revision tracks texture contents.
    sources: Vec<(ShellPlacement, u64, u64)>,
}

impl Signature {
    fn same_capture(&self, other: &Self) -> bool {
        self.extent == other.extent && self.region == other.region && self.sources == other.sources
    }
}

pub(super) struct GlassCache {
    signature: Signature,
    pub(super) revision: u64,
    pub(super) targets: Vec<VulkanMaterializationTarget>,
    filters: Vec<VulkanScene>,
    output: RenderScene,
    output_state: Option<(SizeI, ShellPlacement, crate::GlassStyle)>,
}

/// Pad for the finite Gaussian kernel, bounded optical displacement and reconstruction.
/// Quantized outward edges avoid reallocating for every physical pointer pixel.
pub(super) fn backdrop_region(output: SizeI, lens: RectI, style: crate::GlassStyle) -> RectI {
    let margin = (normalized_blur_radius(style.blur_radius) * 1.5
        + style.refraction.abs()
        + style.dispersion.abs()
        + 4.0)
        .ceil() as i32;
    let left = lens.x.saturating_sub(margin).max(0) / 64 * 64;
    let top = lens.y.saturating_sub(margin).max(0) / 64 * 64;
    let right = lens.right().saturating_add(margin).saturating_add(63) / 64 * 64;
    let bottom = lens.bottom().saturating_add(margin).saturating_add(63) / 64 * 64;
    intersect_rect(
        RectI {
            x: left,
            y: top,
            width: right.saturating_sub(left),
            height: bottom.saturating_sub(top),
        },
        full_rect(output),
    )
    .unwrap_or_else(|| full_rect(output))
}

pub(super) fn localize(mut p: ShellPlacement, region: RectI) -> ShellPlacement {
    p.target.x -= region.x;
    p.target.y -= region.y;
    if let Some(clip) = &mut p.clip {
        clip.x -= region.x;
        clip.y -= region.y;
    }
    for clip in p.rounded_clips.iter_mut().flatten() {
        clip.rect.x -= region.x as f32;
        clip.rect.y -= region.y as f32;
    }
    p
}

fn cropped_sources(
    sources: Vec<(ShellPlacement, u64, u64)>,
    region: RectI,
) -> Vec<(ShellPlacement, u64, u64)> {
    sources
        .into_iter()
        .filter(|(p, _, _)| {
            let visible = match p.clip {
                Some(clip) => intersect_rect(p.target, clip),
                None => Some(p.target),
            };
            visible.is_some_and(|r| intersect_rect(r, region).is_some())
        })
        .collect()
}

pub(super) fn backdrop_sources(
    id: u32,
    lower: &[ShellPlacement],
    scenes: &BTreeMap<ShellSceneKey, VulkanScene>,
    caches: &BTreeMap<ShellSceneKey, GlassCache>,
) -> Vec<(ShellPlacement, u64, u64)> {
    lower
        .iter()
        .filter(|p| !is_own_shadow(p, id))
        .map(|p| {
            let epoch = scenes.get(&p.scene).map_or(0, |s| s.epoch());
            let backdrop_revision = match p.scene {
                ShellSceneKey::ResizeGlass(owner) => caches
                    .get(&ShellSceneKey::ResizeVeil(owner))
                    .map_or(0, |c| c.revision),
                ShellSceneKey::TileGlass(owner) => caches
                    .get(&ShellSceneKey::TilePreview(owner))
                    .map_or(0, |c| c.revision),
                _ => 0,
            };
            (*p, epoch, backdrop_revision)
        })
        .collect()
}

pub(super) fn normalized_blur_radius(radius: f32) -> f32 {
    if radius.is_finite() {
        radius.clamp(0.0, 256.0)
    } else {
        4.0
    }
}

fn backdrop_extents(extent: SizeI, radius: f32) -> Vec<SizeI> {
    let radius = normalized_blur_radius(radius);
    if radius == 0.0 {
        return vec![extent];
    }
    // Keep sharp optics at native resolution. Prefilter before reducing the broad blur;
    // small kernels remain native to avoid softening detail beyond the requested radius.
    if radius < 8.0 {
        return vec![extent; 3];
    }
    let half = SizeI {
        width: (extent.width + 1) / 2,
        height: (extent.height + 1) / 2,
    };
    vec![extent, half, half, half]
}

fn filter_parameters(extents: &[SizeI], level: usize, radius: f32) -> (SizeI, f32, bool) {
    let source = extents[level - 1];
    if extents.len() == 4 && level == 1 {
        // A zero-radius linear sample at half resolution averages a 2x2 footprint.
        return (source, 0.0, true);
    }
    let horizontal = level == if extents.len() == 4 { 2 } else { 1 };
    let ratio = if horizontal {
        source.width as f32 / extents[0].width as f32
    } else {
        source.height as f32 / extents[0].height as f32
    };
    (source, radius * ratio, horizontal)
}

fn is_own_shadow(p: &ShellPlacement, id: u32) -> bool {
    matches!(p.key, ShellLayerKey::FrameShadow(owner) | ShellLayerKey::MotionShadow(owner) if owner == id)
}

fn blur_scene(
    device: &VulkanDevice,
    source: &VulkanMaterializationTarget,
    extent: SizeI,
    radius: f32,
    horizontal: bool,
) -> AppResult<VulkanScene> {
    let mut description = RenderScene::default();
    description.extent = SizeF {
        width: extent.width as f32,
        height: extent.height as f32,
    };
    description.background = ColorRgba8::rgba(0, 0, 0, 0);
    description.set_material_resource(MaterialResource {
        material: MaterialId(1),
        content_version: 1,
        kind: MaterialKind::GaussianBlur(crate::graphics::render::GaussianBlurMaterial {
            source: ImageId(1),
            inverse_size: [1.0 / extent.width as f32, 1.0 / extent.height as f32],
            sigma: radius * 0.5,
            horizontal,
        }),
        colors: [ColorRgba8::rgba(255, 255, 255, 255); 2],
    });
    let node = NodeId::new(1, 1);
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: extent.width as f32,
        height: extent.height as f32,
    };
    description.materials.upsert(
        node,
        MaterialInstance {
            node,
            material: MaterialId(1),
            rect,
            view_bounds: rect,
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(0),
        },
    );
    description.set_draw_order(vec![DrawItem {
        kind: PrimitiveKind::Material,
        index: 0,
        batch: BatchKey {
            pipeline: PipelineKind::GaussianBlur,
            resource: 1,
            clip: ClipId(0),
            blend: BlendMode::Opaque,
            target: 0,
        },
    }]);
    let mut scene = device.create_scene().map_err(app_error)?;
    scene
        .bind_materialized_image(ImageId(1), source, ImageAlphaMode::Opaque)
        .map_err(app_error)?;
    device
        .apply_scene_delta(&mut scene, &description.take_delta().unwrap())
        .map_err(app_error)?;
    Ok(scene)
}

pub(super) fn bind_backdrops(scene: &mut VulkanScene, cache: &GlassCache) -> AppResult<()> {
    scene
        .bind_materialized_image(
            ImageId(1),
            cache.targets.last().unwrap(),
            ImageAlphaMode::Opaque,
        )
        .map_err(app_error)?;
    scene
        .bind_materialized_image(ImageId(2), &cache.targets[0], ImageAlphaMode::Opaque)
        .map_err(app_error)
}

/// Only this small retained material changes as the lens moves; the backdrop stays bound.
pub(super) fn update_lens(
    description: &mut RenderScene,
    extent: SizeI,
    placement: ShellPlacement,
    style: crate::GlassStyle,
) {
    update_lens_weighted(
        description,
        extent,
        placement,
        style,
        1.0,
        BlendMode::Opaque,
    );
}

pub(super) fn update_lens_weighted(
    description: &mut RenderScene,
    extent: SizeI,
    placement: ShellPlacement,
    style: crate::GlassStyle,
    opacity: f32,
    blend: BlendMode,
) {
    let rect = placement.rounded_clips[0]
        .filter(|clip| !clip.inverted)
        .map(|clip| clip.rect)
        .unwrap_or(RectF {
            x: placement.target.x as f32,
            y: placement.target.y as f32,
            width: placement.target.width as f32,
            height: placement.target.height as f32,
        });
    let half = 0.5 * rect.width.min(rect.height).max(1.0);
    let radii = placement.rounded_clips[0]
        .filter(|clip| !clip.inverted)
        .map(|clip| {
            [
                clip.radii.top_left,
                clip.radii.top_right,
                clip.radii.bottom_right,
                clip.radii.bottom_left,
            ]
        })
        .unwrap_or([0.0; 4])
        .map(|r| r.clamp(0.0, half));
    let alpha = style.tint.a as f32 / 255.0;
    let linear = |byte: u8| {
        let x = byte as f32 / 255.0;
        (if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }) * alpha
    };
    let parameters = LiquidGlassMaterial {
        backdrop: ImageId(1),
        sharp_backdrop: ImageId(2),
        radii,
        inverse_output_size: [1.0 / extent.width as f32, 1.0 / extent.height as f32],
        inverse_bevel: 1.0 / style.bevel_width.min(half).max(0.5),
        blend_softness: style.blend_softness,
        refraction: style.refraction,
        dispersion: style.dispersion,
        fresnel: style.fresnel,
        tint: [
            linear(style.tint.r),
            linear(style.tint.g),
            linear(style.tint.b),
            1.0 - alpha,
        ],
    };
    let size = SizeF {
        width: extent.width as f32,
        height: extent.height as f32,
    };
    if description.extent != size {
        description.extent = size;
        description.damage.full = true;
    }
    description.background = ColorRgba8::rgba(0, 0, 0, 0);
    description.set_material_resource(MaterialResource {
        material: MaterialId(1),
        content_version: 1,
        kind: MaterialKind::LiquidGlass(parameters),
        colors: [style.tint; 2],
    });
    let node = NodeId::new(1, 1);
    if description.materials.upsert(
        node,
        MaterialInstance {
            node,
            material: MaterialId(1),
            rect,
            view_bounds: rect,
            opacity,
            clip: ClipId(0),
            spatial: SpatialId(0),
        },
    ) {
        description.damage.full = true;
    }
    description.set_draw_order(vec![DrawItem {
        kind: PrimitiveKind::Material,
        index: 0,
        batch: BatchKey {
            pipeline: PipelineKind::LiquidGlass,
            resource: 1,
            clip: ClipId(0),
            blend,
            target: 0,
        },
    }]);
}

fn draw(
    device: &VulkanDevice,
    scenes: &mut [VulkanCompositeScene<'_>],
    placements: &[VulkanCompositePlacement],
    target: &mut VulkanMaterializationTarget,
    context: &mut VulkanFrameContext<'_>,
) -> AppResult<()> {
    context.core.images.push(target.image());
    device
        .render_composite(
            scenes,
            placements,
            &mut VulkanFrameContext {
                core: &mut *context.core,
            },
            &target.target(),
            &RenderRequest {
                force: true,
                load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                store: TargetStore::Store,
                region: None,
            },
        )
        .map_err(app_error)?;
    target.mark_initialized();
    Ok(())
}

/// Capture only the currently displayed lower placements. The lens geometry is not cached here.
pub(super) fn prepare_backdrop_key(
    device: &VulkanDevice,
    scenes: &mut BTreeMap<ShellSceneKey, VulkanScene>,
    caches: &mut BTreeMap<ShellSceneKey, GlassCache>,
    extent: SizeI,
    region: RectI,
    cache_key: ShellSceneKey,
    style: crate::GlassStyle,
    lower: &[ShellPlacement],
    context: &mut VulkanFrameContext<'_>,
) -> AppResult<bool> {
    let id = match cache_key {
        ShellSceneKey::ResizeVeil(id) => id,
        _ => 0,
    };
    let radius = normalized_blur_radius(style.blur_radius);
    let capture_extent = SizeI {
        width: region.width,
        height: region.height,
    };
    let extents = backdrop_extents(capture_extent, radius);
    let sources = cropped_sources(backdrop_sources(id, lower, scenes, caches), region);
    let signature = Signature {
        extent,
        region,
        blur_radius: radius,
        sources,
    };
    let rebuild = caches
        .get(&cache_key)
        .is_none_or(|cache| cache.signature != signature);
    if rebuild {
        let reusable = caches.get(&cache_key).is_some_and(|cache| {
            cache
                .targets
                .iter()
                .map(|t| t.extent())
                .eq(extents.iter().copied())
        });
        let capture_changed = !reusable
            || caches
                .get(&cache_key)
                .is_none_or(|cache| !cache.signature.same_capture(&signature));
        if !reusable {
            // Bound live backdrop storage. Existing image pins protect retired in-flight work.
            let other_bytes = caches
                .iter()
                .filter(|(key, _)| **key != cache_key)
                .flat_map(|(_, c)| &c.targets)
                .map(|t| t.allocated_bytes())
                .sum::<u64>();
            if !backdrop_fits_budget(extent, &extents, other_bytes) {
                eprintln!(
                    "telorgon-glass: backdrop budget rejected key={cache_key:?} output={extent:?} targets={extents:?} other_allocated_bytes={other_bytes} budget_bytes={}",
                    backdrop_budget(extent)
                );
                return Ok(false);
            }
            let targets = extents
                .iter()
                .map(|extent| VulkanMaterializationTarget::new_traced(device, *extent, &mut |_| {}))
                .collect::<Result<Vec<_>, _>>();
            let targets = match targets {
                Ok(targets) => targets,
                Err(error) => {
                    eprintln!(
                        "telorgon-glass: backdrop allocation failed key={cache_key:?}: {error:?}"
                    );
                    return Ok(false);
                }
            };
            let revision = caches.get(&cache_key).map_or(0, |cache| cache.revision);
            let (output, output_state) = caches
                .remove(&cache_key)
                .map(|c| (c.output, c.output_state))
                .unwrap_or_default();
            caches.insert(
                cache_key,
                GlassCache {
                    signature: signature.clone(),
                    revision,
                    targets,
                    filters: Vec::new(),
                    output,
                    output_state,
                },
            );
        }
        let cache = caches.get_mut(&cache_key).unwrap();
        if capture_changed {
            let gpu_scope = context.core.begin_gpu_scope("gpu.glass.capture");
            let indices = scenes
                .keys()
                .enumerate()
                .map(|(i, key)| (*key, i))
                .collect::<BTreeMap<_, _>>();
            let inputs = signature
                .sources
                .iter()
                .map(|(p, _, _)| {
                    let p = localize(*p, region);
                    Ok(VulkanCompositePlacement {
                        scene_index: *indices
                            .get(&p.scene)
                            .ok_or_else(|| AppError::new("glass backdrop scene missing"))?,
                        target: p.target,
                        clip: p.clip,
                        rounded_clips: p.rounded_clips,
                    })
                })
                .collect::<AppResult<Vec<_>>>()?;
            let mut source_scenes = scenes
                .values_mut()
                .map(|scene| VulkanCompositeScene { scene })
                .collect::<Vec<_>>();
            draw(
                device,
                &mut source_scenes,
                &inputs,
                &mut cache.targets[0],
                context,
            )?;
            context.core.end_gpu_scope(gpu_scope);
        }
        if cache.filters.len() + 1 != cache.targets.len() || cache.signature.blur_radius != radius {
            cache.filters.clear();
            for level in 1..cache.targets.len() {
                let (source_extent, filter_radius, horizontal) =
                    filter_parameters(&extents, level, radius);
                cache.filters.push(blur_scene(
                    device,
                    &cache.targets[level - 1],
                    source_extent,
                    filter_radius,
                    horizontal,
                )?);
            }
        }
        for level in 1..cache.targets.len() {
            let gpu_scope = context
                .core
                .begin_gpu_scope(if extents.len() == 4 && level == 1 {
                    "gpu.glass.prefilter"
                } else {
                    "gpu.glass.blur"
                });
            let source = &mut cache.filters[level - 1];
            draw(
                device,
                &mut [VulkanCompositeScene { scene: source }],
                &[VulkanCompositePlacement {
                    scene_index: 0,
                    target: full_rect(extents[level]),
                    clip: None,
                    rounded_clips: [None::<RoundedClip>; 2],
                }],
                &mut cache.targets[level],
                context,
            )?;
            context.core.end_gpu_scope(gpu_scope);
        }
        cache.signature = signature;
        cache.revision = cache.revision.wrapping_add(1).max(1);
    }
    Ok(true)
}

/// Returns output placements with glass replacing the flat veil, preserving its exact clip.
/// Allocation failure leaves the flat tint available. Recording errors propagate normally.
pub(super) fn record_glass(
    device: &VulkanDevice,
    scenes: &mut BTreeMap<ShellSceneKey, VulkanScene>,
    caches: &mut BTreeMap<ShellSceneKey, GlassCache>,
    frame: &ShellFrame,
    placements: &[ShellPlacement],
    context: &mut VulkanFrameContext<'_>,
) -> AppResult<Vec<ShellPlacement>> {
    let mut output = placements.to_vec();
    for (index, placement) in placements.iter().enumerate() {
        let Some(style) = frame.glass.get(&placement.scene) else {
            continue;
        };
        let key = match placement.scene {
            ShellSceneKey::ResizeVeil(id) => ShellSceneKey::ResizeGlass(id),
            ShellSceneKey::TilePreview(id) => ShellSceneKey::TileGlass(id),
            _ => continue,
        };
        let region = backdrop_region(frame.extent, placement.target, *style);
        let capture_extent = SizeI {
            width: region.width,
            height: region.height,
        };
        if !prepare_backdrop_key(
            device,
            scenes,
            caches,
            frame.extent,
            region,
            placement.scene,
            *style,
            &output[..index],
            context,
        )? {
            continue;
        }
        let cache = caches.get_mut(&placement.scene).unwrap();
        // A missing retained GPU scene needs a complete initial description.
        if !scenes.contains_key(&key) {
            cache.output = RenderScene::default();
            cache.output_state = None;
        }
        let state = (frame.extent, *placement, *style);
        if cache.output_state != Some(state) {
            update_lens(
                &mut cache.output,
                capture_extent,
                localize(*placement, region),
                *style,
            );
            cache.output_state = Some(state);
        }
        let scene = match scenes.entry(key) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(device.create_scene().map_err(app_error)?)
            }
        };
        bind_backdrops(scene, cache)?;
        if let Some(mut delta) = cache.output.take_delta() {
            delta.epoch = scene
                .epoch()
                .checked_add(1)
                .ok_or_else(|| AppError::new("liquid glass scene epoch exhausted"))?;
            device.apply_scene_delta(scene, &delta).map_err(app_error)?;
        }
        output[index] = ShellPlacement {
            scene: key,
            target: region,
            clip: Some(
                placement
                    .clip
                    .and_then(|clip| intersect_rect(clip, placement.target))
                    .unwrap_or(placement.target),
            ),
            ..*placement
        };
    }
    Ok(output)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "glass_optics_tests.rs"]
mod optics_tests;
