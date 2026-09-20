//! Native sharp captures and scale-aware separable filtering with owned snapshot lifetime.
//! No readback, extra submission, or timer: lower-scene revisions drive cache invalidation.
use super::super::super::scene::{ShellLayerKey, ShellPlacement};
use super::*;
use crate::render::{
    LiquidGlassMaterial, MaterialId, MaterialInstance, MaterialKind, MaterialResource, RoundedClip,
};
use crate::renderer_vulkan::VulkanFrameContext;

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
        kind: MaterialKind::GaussianBlur(crate::render::GaussianBlurMaterial {
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
mod tests {
    use super::*;
    #[test]
    fn full_resolution_glass_budget_admits_four_3840_by_2400_backdrops() {
        // Regression: the real 200%-scale output exceeded the old budget for even
        // one lens, silently selecting flat tint and losing all refraction.
        let extent = SizeI {
            width: 3840,
            height: 2400,
        };
        let targets = vec![extent; 3];
        let one_lens_bytes = 3840_u64 * 2400 * 4 * 3;
        assert!(one_lens_bytes > 96 * 1024 * 1024);
        assert!(backdrop_fits_budget(extent, &targets, 0));
        assert!(backdrop_fits_budget(extent, &targets, one_lens_bytes));
        assert!(backdrop_fits_budget(extent, &targets, one_lens_bytes * 2));
        assert!(backdrop_fits_budget(extent, &targets, one_lens_bytes * 3));
        // Previously even one byte of padding on the first three lenses rejected the fourth.
        let padded_lens = one_lens_bytes + 3 * 1024 * 1024;
        assert!(backdrop_fits_budget(extent, &targets, padded_lens * 3));
        assert!(!backdrop_fits_budget(extent, &targets, one_lens_bytes * 4));
        assert_eq!(
            backdrop_budget(SizeI {
                width: i32::MAX,
                height: i32::MAX
            }),
            MAX_BACKDROP_BUDGET_BYTES
        );
        assert!(!backdrop_fits_budget(extent, &targets, u64::MAX));
        let remaining = backdrop_budget(extent) - one_lens_bytes;
        assert!(backdrop_fits_budget(extent, &targets, remaining));
        assert!(!backdrop_fits_budget(extent, &targets, remaining + 1));
    }

    #[test]
    fn glass_style_invalid_inputs_normalize_to_stable_finite_values() {
        let style = crate::GlassStyle {
            blur_radius: f32::NAN,
            bevel_width: f32::INFINITY,
            blend_softness: f32::NAN,
            refraction: -10.0,
            dispersion: 100.0,
            fresnel: -1.0,
            ..crate::GlassStyle::liquid()
        }
        .normalized();
        assert_eq!(style, style.normalized());
        assert_eq!(
            (
                style.blur_radius,
                style.bevel_width,
                style.refraction,
                style.dispersion
            ),
            (4.0, 24.0, 0.0, 4.0)
        );
        assert_eq!(style.fresnel, 0.0);
        assert_eq!(style.blend_softness, 0.0);
        assert_eq!(
            crate::GlassStyle {
                blend_softness: 200.0,
                ..style
            }
            .normalized()
            .blend_softness,
            128.0
        );
        assert_eq!(
            crate::GlassStyle {
                blend_softness: -1.0,
                ..style
            }
            .normalized()
            .blend_softness,
            0.0
        );
    }

    fn test_placement() -> ShellPlacement {
        ShellPlacement {
            key: ShellLayerKey::ResizeVeil(1),
            scene: ShellSceneKey::ResizeVeil(1),
            target: RectI {
                x: 10,
                y: 20,
                width: 200,
                height: 100,
            },
            clip: None,
            rounded_clips: [None; 2],
        }
    }
    fn bind_test_backdrop(delta: &mut crate::render::RenderSceneDelta) {
        use crate::render::*;
        for image in [ImageId(1), ImageId(2)] {
            delta
                .image_resources
                .push(ImageResourceDelta::Write(ImageResourceUpdate {
                    image,
                    content_version: 1,
                    extent: SizeI {
                        width: 1,
                        height: 1,
                    },
                    rect: RectI {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    },
                    row_bytes: 4,
                    color_encoding: ImageColorEncoding::Srgb,
                    alpha_mode: ImageAlphaMode::Opaque,
                    pixel_format: ImagePixelFormat::Rgba8,
                    pixels: vec![128, 128, 128, 255].into(),
                }));
        }
    }
    #[test]
    fn liquid_glass_updates_geometry_and_optics_without_texture_uploads() {
        let extent = SizeI {
            width: 640,
            height: 480,
        };
        let mut p = test_placement();
        let mut style = crate::GlassStyle::liquid();
        let mut description = RenderScene::default();
        let mut retained = VulkanScene::default();
        update_lens(&mut description, extent, p, style);
        let mut delta = description.take_delta().unwrap();
        bind_test_backdrop(&mut delta);
        retained.apply_delta_checked(&delta).unwrap();
        assert_eq!(retained.material_parameters.len(), 15);
        assert_eq!(retained.gpu_materials[0].params_spatial_clip[1], 15);
        retained.commit_uploads(0, 0, 0, 0, 0);
        update_lens(&mut description, extent, p, style);
        assert!(
            description.take_delta().is_none(),
            "idle glass has no retained delta"
        );
        p.target.x += 25;
        p.target.width += 10;
        update_lens(&mut description, extent, p, style);
        let moved = description.take_delta().unwrap();
        assert!(moved.image_resources.is_empty());
        assert!(moved.material_resources.is_empty());
        assert_eq!(
            retained
                .apply_delta_checked(&moved)
                .unwrap()
                .upload_bytes_queued,
            64,
            "only one geometry record changes"
        );
        retained.commit_uploads(0, 0, 0, 0, 0);
        style.refraction = 8.0;
        style.tint.a = 64;
        update_lens(&mut description, extent, p, style);
        let tint = description.take_delta().unwrap();
        assert!(tint.materials.is_empty());
        assert!(tint.image_resources.is_empty());
        assert_eq!(
            retained
                .apply_delta_checked(&tint)
                .unwrap()
                .upload_bytes_queued,
            60,
            "only 60 bytes of optical data change"
        );
        retained.commit_uploads(0, 0, 0, 0, 0);
        style.blend_softness = 16.0;
        update_lens(&mut description, extent, p, style);
        let softness = description.take_delta().unwrap();
        assert!(softness.materials.is_empty() && softness.image_resources.is_empty());
        assert_eq!(
            retained
                .apply_delta_checked(&softness)
                .unwrap()
                .upload_bytes_queued,
            60
        );
        assert_eq!(f32::from_bits(retained.material_parameters[14]), 16.0);
    }
    #[test]
    fn liquid_glass_rejects_missing_images_bad_parameters_and_wrong_pipeline_atomically() {
        let mut description = RenderScene::default();
        update_lens(
            &mut description,
            SizeI {
                width: 640,
                height: 480,
            },
            test_placement(),
            crate::GlassStyle::liquid(),
        );
        let delta = description.take_delta().unwrap();
        let mut retained = VulkanScene::default();
        assert!(retained.apply_delta_checked(&delta).is_err());
        let mut bound = delta.clone();
        bind_test_backdrop(&mut bound);
        for bad in [0, 1, 2, 3, 4] {
            let mut invalid = bound.clone();
            if bad != 1 {
                if let crate::render::MaterialResourceDelta::Upsert(r) =
                    &mut invalid.material_resources[0]
                {
                    if let MaterialKind::LiquidGlass(p) = &mut r.kind {
                        match bad {
                            0 => p.inverse_bevel = f32::NAN,
                            2 => p.blend_softness = f32::NAN,
                            3 => p.blend_softness = -1.0,
                            _ => p.sharp_backdrop = ImageId(99),
                        }
                    }
                }
            } else {
                let mut order = invalid.draw_order.as_ref().unwrap().to_vec();
                order[0].batch.pipeline = PipelineKind::Material;
                invalid.draw_order = Some(order.into());
            }
            assert!(retained.apply_delta_checked(&invalid).is_err());
            assert_eq!(retained.epoch(), 0);
        }
        retained.apply_delta_checked(&bound).unwrap();
    }
    #[test]
    fn stacked_glass_tracks_lower_optics_and_backdrop_independently() {
        let extent = SizeI {
            width: 640,
            height: 480,
        };
        let mut lens = RenderScene::default();
        let mut scene = VulkanScene::default();
        update_lens(
            &mut lens,
            extent,
            test_placement(),
            crate::GlassStyle::liquid(),
        );
        let mut delta = lens.take_delta().unwrap();
        bind_test_backdrop(&mut delta);
        scene.apply_delta_checked(&delta).unwrap();
        let key = ShellSceneKey::ResizeGlass(1);
        let lower = [ShellPlacement {
            scene: key,
            ..test_placement()
        }];
        let mut scenes = BTreeMap::from([(key, scene)]);
        let mut caches = BTreeMap::from([(
            ShellSceneKey::ResizeVeil(1),
            GlassCache {
                signature: Signature {
                    extent,
                    region: full_rect(extent),
                    blur_radius: 0.0,
                    sources: Vec::new(),
                },
                revision: 1,
                targets: Vec::new(),
                filters: Vec::new(),
                output: RenderScene::default(),
                output_state: None,
            },
        )]);
        let first = backdrop_sources(2, &lower, &scenes, &caches);
        assert_eq!(first, backdrop_sources(2, &lower, &scenes, &caches));
        update_lens(
            &mut lens,
            extent,
            test_placement(),
            crate::GlassStyle {
                refraction: 5.0,
                ..crate::GlassStyle::liquid()
            },
        );
        scenes
            .get_mut(&key)
            .unwrap()
            .apply_delta_checked(&lens.take_delta().unwrap())
            .unwrap();
        let optics = backdrop_sources(2, &lower, &scenes, &caches);
        assert_ne!(
            first, optics,
            "lower lens optics invalidate the upper backdrop"
        );
        caches
            .get_mut(&ShellSceneKey::ResizeVeil(1))
            .unwrap()
            .revision += 1;
        let pixels = backdrop_sources(2, &lower, &scenes, &caches);
        assert_ne!(
            optics, pixels,
            "lower backdrop pixels invalidate without changing material epoch"
        );
    }

    #[test]
    fn liquid_glass_zero_blur_has_no_filter_passes() {
        let size = SizeI {
            width: 1920,
            height: 1080,
        };
        assert_eq!(backdrop_extents(size, 0.0), vec![size]);
    }

    #[test]
    #[ignore = "requires TELORGON_TEST_MODE=developer-hardware and a Vulkan adapter"]
    fn glass_records_reuses_and_refreshes_real_gpu_targets() {
        assert_eq!(
            std::env::var("TELORGON_TEST_MODE").as_deref(),
            Ok("developer-hardware")
        );
        let config = VulkanConfig {
            enable_validation: true,
            ..VulkanConfig::default()
        };
        let instance = VulkanInstance::load(&config, &[]).unwrap();
        let selection = DeviceSelection::best(&instance.adapters().unwrap()).unwrap();
        let device = VulkanDevice::create_owned(instance, &config, &selection, None).unwrap();
        let extent = SizeI {
            width: 64,
            height: 48,
        };
        let mut composition = super::super::super::super::scene::ShellComposition::new(extent);
        let mut scenes = BTreeMap::new();
        let mut caches = BTreeMap::new();
        let mut recording = device.begin_owned_frame().unwrap();
        let mut output_target = VulkanMaterializationTarget::new(&device, extent).unwrap();
        for (step, color) in [
            ColorRgba8::rgba(255, 0, 0, 255),
            ColorRgba8::rgba(255, 0, 0, 255),
            ColorRgba8::rgba(0, 0, 255, 255),
        ]
        .into_iter()
        .enumerate()
        {
            use super::super::super::super::scene::ShellLayer;
            let mut veil = ShellLayer::solid(
                ShellLayerKey::ResizeVeil(1),
                ShellSceneKey::ResizeVeil(1),
                crate::GlassStyle::default().tint,
                RectI {
                    x: 8 + step as i32,
                    y: 8,
                    width: 20,
                    height: 20,
                },
            );
            veil.glass = Some(crate::GlassStyle::default());
            let frame = composition
                .synchronize(
                    extent,
                    vec![
                        ShellLayer::solid(
                            ShellLayerKey::Background,
                            ShellSceneKey::Background,
                            color,
                            full_rect(extent),
                        ),
                        veil,
                    ],
                )
                .unwrap();
            for update in &frame.updates {
                let scene = scenes
                    .entry(update.key)
                    .or_insert_with(|| device.create_scene().unwrap());
                for delta in &update.deltas {
                    device.apply_scene_delta(scene, delta).unwrap();
                }
            }
            let output = record_glass(
                &device,
                &mut scenes,
                &mut caches,
                &frame,
                &frame.placements,
                &mut recording.context_mut(),
            )
            .unwrap();
            assert_eq!(output[1].scene, ShellSceneKey::ResizeGlass(1));
            let indices = scenes
                .keys()
                .enumerate()
                .map(|(i, k)| (*k, i))
                .collect::<BTreeMap<_, _>>();
            let draws = output
                .iter()
                .map(|p| VulkanCompositePlacement {
                    scene_index: indices[&p.scene],
                    target: p.target,
                    clip: p.clip,
                    rounded_clips: p.rounded_clips,
                })
                .collect::<Vec<_>>();
            let mut sources = scenes
                .values_mut()
                .map(|scene| VulkanCompositeScene { scene })
                .collect::<Vec<_>>();
            draw(
                &device,
                &mut sources,
                &draws,
                &mut output_target,
                &mut recording.context_mut(),
            )
            .unwrap();
            assert_eq!(
                caches[&ShellSceneKey::ResizeVeil(1)].revision,
                if step < 2 { 1 } else { 2 }
            );
        }
        // Destinations and sampled images remain pinned even when host owners disappear.
        scenes.clear();
        caches.clear();
        recording
            .finish()
            .unwrap()
            .submit()
            .unwrap()
            .wait(Duration::from_secs(2))
            .unwrap();
    }

    #[test]
    fn own_shadow_is_excluded_but_other_windows_are_kept() {
        let mut p = ShellPlacement {
            key: ShellLayerKey::FrameShadow(7),
            scene: ShellSceneKey::FrameShadow(7),
            target: RectI {
                x: 0,
                y: 0,
                width: 20,
                height: 20,
            },
            clip: None,
            rounded_clips: [None; 2],
        };
        assert!(is_own_shadow(&p, 7));
        assert!(!is_own_shadow(&p, 8));
        p.key = ShellLayerKey::MotionShadow(7);
        assert!(is_own_shadow(&p, 7));
    }

    #[test]
    fn fractional_blur_changes_refilter_but_reuse_the_same_capture() {
        let extent = SizeI {
            width: 1920,
            height: 1080,
        };
        let before = Signature {
            extent,
            region: full_rect(extent),
            blur_radius: 10.0,
            sources: vec![(test_placement(), 1, 1)],
        };
        let mut after = before.clone();
        after.blur_radius = 10.1;
        assert_ne!(
            before, after,
            "changes inside the old reduction bucket must refilter"
        );
        assert!(before.same_capture(&after));
        assert_eq!(
            backdrop_extents(extent, before.blur_radius),
            backdrop_extents(extent, after.blur_radius)
        );
        after.sources[0].2 += 1;
        assert!(
            !before.same_capture(&after),
            "lower glass pixels invalidate the capture"
        );
        after = before.clone();
        after.sources[0].0.target.x += 1;
        assert!(!before.same_capture(&after));
    }

    #[test]
    fn broad_blur_reduces_only_filtered_targets_and_preserves_native_optics() {
        for extent in [
            SizeI {
                width: 1920,
                height: 1080,
            },
            SizeI {
                width: 1931,
                height: 1081,
            },
            SizeI {
                width: 1,
                height: 1,
            },
        ] {
            for radius in [0.01, 4.0, 10.0, 24.0, 256.0] {
                let targets = backdrop_extents(extent, radius);
                assert_eq!(targets[0], extent);
                if radius < 8.0 {
                    assert_eq!(targets, vec![extent; 3]);
                } else {
                    assert_eq!(targets.len(), 4);
                    assert_eq!(targets[1].width, (extent.width + 1) / 2);
                    assert_eq!(targets[1].height, (extent.height + 1) / 2);
                }
            }
            assert_eq!(backdrop_extents(extent, 0.0), vec![extent]);
        }
    }
    #[test]
    fn cropped_capture_preserves_support_and_desktop_coordinates() {
        let output = SizeI {
            width: 1931,
            height: 1081,
        };
        let lens = RectI {
            x: 701,
            y: 403,
            width: 301,
            height: 201,
        };
        let style = crate::GlassStyle {
            blur_radius: 10.0,
            refraction: 10.0,
            dispersion: 0.2,
            ..crate::GlassStyle::liquid()
        };
        let region = backdrop_region(output, lens, style);
        assert!(region.x <= lens.x - 30 && region.y <= lens.y - 30);
        assert!(region.right() >= lens.right() + 30);
        assert!(region.bottom() >= lens.bottom() + 30);
        assert!(region.width * region.height < output.width * output.height / 4);
        let p = ShellPlacement {
            key: ShellLayerKey::ResizeVeil(1),
            scene: ShellSceneKey::ResizeVeil(1),
            target: lens,
            clip: Some(lens),
            rounded_clips: [
                Some(RoundedClip {
                    rect: RectF {
                        x: lens.x as f32,
                        y: lens.y as f32,
                        width: 301.0,
                        height: 201.0,
                    },
                    radii: crate::ui::CornerRadii::all(12.0),
                    inverted: false,
                }),
                None,
            ],
        };
        let local = localize(p, region);
        assert_eq!(local.target.x + region.x, lens.x);
        assert_eq!(local.clip.unwrap().y + region.y, lens.y);
        assert_eq!(
            local.rounded_clips[0].unwrap().rect.x + region.x as f32,
            lens.x as f32
        );
        let outside = ShellPlacement {
            target: RectI {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            clip: None,
            rounded_clips: [None; 2],
            ..p
        };
        assert_eq!(
            cropped_sources(vec![(outside, 1, 0), (p, 2, 0)], region),
            vec![(p, 2, 0)]
        );
        let edge = backdrop_region(
            output,
            RectI {
                x: 1850,
                y: 1000,
                width: 81,
                height: 81,
            },
            style,
        );
        assert_eq!(edge.right(), output.width);
        assert_eq!(edge.bottom(), output.height);
    }

    #[test]
    fn half_resolution_blur_preserves_physical_kernel_width_on_odd_outputs() {
        let extent = SizeI {
            width: 1931,
            height: 1081,
        };
        let targets = backdrop_extents(extent, 10.0);
        assert_eq!(filter_parameters(&targets, 1, 10.0).1, 0.0);
        for level in [2, 3] {
            let (source, radius, horizontal) = filter_parameters(&targets, level, 10.0);
            let scale = if horizontal {
                extent.width as f32 / source.width as f32
            } else {
                extent.height as f32 / source.height as f32
            };
            assert!((radius * scale - 10.0).abs() < 0.00001);
        }
        let pixels: i64 = targets
            .iter()
            .map(|e| i64::from(e.width) * i64::from(e.height))
            .sum();
        let native_pixels = i64::from(extent.width) * i64::from(extent.height);
        assert!(
            pixels < native_pixels * 2,
            "sharp plus filters use less than two native surfaces"
        );
    }

    #[test]
    fn invalid_blur_radii_are_normalized_before_caching() {
        assert_eq!(normalized_blur_radius(f32::NAN), 4.0);
        assert_eq!(normalized_blur_radius(f32::INFINITY), 4.0);
        assert_eq!(normalized_blur_radius(-5.0), 0.0);
        assert_eq!(normalized_blur_radius(1e30), 256.0);
    }
}

#[cfg(test)]
#[path = "glass_optics_tests.rs"]
mod optics_tests;
