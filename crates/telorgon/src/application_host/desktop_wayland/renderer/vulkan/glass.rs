//! Filtered backdrop pyramid, using the existing image shader and owned snapshot lifetime.
//! No readback, extra submission, or timer: lower-scene revisions drive cache invalidation.
use super::super::super::scene::{DesktopLayerKey, DesktopPlacement};
use super::*;
use crate::render::{
    LiquidGlassMaterial, MaterialId, MaterialInstance, MaterialKind, MaterialResource, RoundedClip,
};
use crate::renderer_vulkan::VulkanFrameContext;

#[derive(Clone, Debug, PartialEq)]
struct Signature {
    extent: SizeI,
    levels: usize,
    // Scene epoch tracks lens geometry/optics; backdrop revision tracks texture contents.
    sources: Vec<(DesktopPlacement, u64, u64)>,
}

pub(super) struct GlassCache {
    signature: Signature,
    pub(super) revision: u64,
    pub(super) targets: Vec<VulkanMaterializationTarget>,
    output: RenderScene,
    output_state: Option<(SizeI, DesktopPlacement, crate::GlassStyle)>,
}

pub(super) fn backdrop_sources(
    id: u32,
    lower: &[DesktopPlacement],
    scenes: &BTreeMap<DesktopSceneKey, VulkanScene>,
    caches: &BTreeMap<DesktopSceneKey, GlassCache>,
) -> Vec<(DesktopPlacement, u64, u64)> {
    lower
        .iter()
        .filter(|p| !is_own_shadow(p, id))
        .map(|p| {
            let epoch = scenes.get(&p.scene).map_or(0, |s| s.epoch());
            let backdrop_revision = match p.scene {
                DesktopSceneKey::ResizeGlass(owner) => caches
                    .get(&DesktopSceneKey::ResizeVeil(owner))
                    .map_or(0, |c| c.revision),
                _ => 0,
            };
            (*p, epoch, backdrop_revision)
        })
        .collect()
}

fn pyramid_extents(extent: SizeI, radius: f32) -> Vec<SizeI> {
    let radius = if radius.is_finite() {
        radius.clamp(0.0, 256.0)
    } else {
        4.0
    };
    let levels = if radius <= 0.0 {
        0
    } else {
        (radius / 2.0).log2().round().clamp(1.0, 7.0) as usize
    };
    let mut result = vec![extent];
    for _ in 0..levels {
        let last = *result.last().unwrap();
        if last.width == 1 && last.height == 1 {
            break;
        }
        result.push(SizeI {
            width: (last.width + 1) / 2,
            height: (last.height + 1) / 2,
        });
    }
    result
}

fn is_own_shadow(p: &DesktopPlacement, id: u32) -> bool {
    matches!(p.key, DesktopLayerKey::FrameShadow(owner) | DesktopLayerKey::MotionShadow(owner) if owner == id)
}

fn sample_scene(
    device: &VulkanDevice,
    source: &VulkanMaterializationTarget,
    extent: SizeI,
) -> AppResult<VulkanScene> {
    let image = ImageId(1);
    let mut description = super::super::super::motion::image_scene(extent, &[(image, 1.0)], false);
    let mut scene = device.create_scene().map_err(app_error)?;
    scene
        .bind_materialized_image(image, source, ImageAlphaMode::Opaque)
        .map_err(app_error)?;
    device
        .apply_scene_delta(&mut scene, &description.take_delta().unwrap())
        .map_err(app_error)?;
    Ok(scene)
}

/// Only this small retained material changes as the lens moves; the backdrop stays bound.
pub(super) fn update_lens(
    description: &mut RenderScene,
    extent: SizeI,
    placement: DesktopPlacement,
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
    placement: DesktopPlacement,
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
        radii,
        inverse_output_size: [1.0 / extent.width as f32, 1.0 / extent.height as f32],
        inverse_bevel: 1.0 / style.bevel_width.min(half).max(0.5),
        blend_softness: style.blend_softness,
        refraction: style.refraction,
        dispersion: style.dispersion,
        rim: style.rim,
        fresnel: style.fresnel,
        specular: style.specular,
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
pub(super) fn prepare_backdrop(
    device: &VulkanDevice,
    scenes: &mut BTreeMap<DesktopSceneKey, VulkanScene>,
    caches: &mut BTreeMap<DesktopSceneKey, GlassCache>,
    extent: SizeI,
    id: u32,
    style: crate::GlassStyle,
    lower: &[DesktopPlacement],
    context: &mut VulkanFrameContext<'_>,
) -> AppResult<bool> {
    let cache_key = DesktopSceneKey::ResizeVeil(id);
    let extents = pyramid_extents(extent, style.blur_radius);
    let sources = backdrop_sources(id, lower, scenes, caches);
    let signature = Signature {
        extent: extent,
        levels: extents.len(),
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
        if !reusable {
            // Bound live backdrop storage. Existing image pins protect retired in-flight work.
            let required = extents
                .iter()
                .map(|e| e.width as u64 * e.height as u64 * 4)
                .sum::<u64>();
            let other_bytes = caches
                .iter()
                .filter(|(key, _)| **key != cache_key)
                .flat_map(|(_, c)| &c.targets)
                .map(|t| t.allocated_bytes())
                .sum::<u64>();
            if required + other_bytes > 96 * 1024 * 1024 {
                return Ok(false);
            }
            let targets = extents
                .iter()
                .map(|extent| VulkanMaterializationTarget::new_traced(device, *extent, &mut |_| {}))
                .collect::<Result<Vec<_>, _>>();
            let Ok(targets) = targets else {
                return Ok(false);
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
                    output,
                    output_state,
                },
            );
        }
        let cache = caches.get_mut(&cache_key).unwrap();
        let indices = scenes
            .keys()
            .enumerate()
            .map(|(i, key)| (*key, i))
            .collect::<BTreeMap<_, _>>();
        let inputs = signature
            .sources
            .iter()
            .map(|(p, _, _)| {
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
        for level in 1..cache.targets.len() {
            let mut source = sample_scene(device, &cache.targets[level - 1], extents[level])?;
            draw(
                device,
                &mut [VulkanCompositeScene { scene: &mut source }],
                &[VulkanCompositePlacement {
                    scene_index: 0,
                    target: full_rect(extents[level]),
                    clip: None,
                    rounded_clips: [None::<RoundedClip>; 2],
                }],
                &mut cache.targets[level],
                context,
            )?;
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
    scenes: &mut BTreeMap<DesktopSceneKey, VulkanScene>,
    caches: &mut BTreeMap<DesktopSceneKey, GlassCache>,
    frame: &DesktopFrame,
    placements: &[DesktopPlacement],
    context: &mut VulkanFrameContext<'_>,
) -> AppResult<Vec<DesktopPlacement>> {
    let mut output = placements.to_vec();
    for (index, placement) in placements.iter().enumerate() {
        let Some(style) = frame.glass.get(&placement.scene) else {
            continue;
        };
        let DesktopSceneKey::ResizeVeil(id) = placement.scene else {
            continue;
        };
        if !prepare_backdrop(
            device,
            scenes,
            caches,
            frame.extent,
            id,
            *style,
            &output[..index],
            context,
        )? {
            continue;
        }
        let cache = caches.get_mut(&placement.scene).unwrap();
        let key = DesktopSceneKey::ResizeGlass(id);
        // A missing retained GPU scene needs a complete initial description.
        if !scenes.contains_key(&key) {
            cache.output = RenderScene::default();
            cache.output_state = None;
        }
        let state = (frame.extent, *placement, *style);
        if cache.output_state != Some(state) {
            update_lens(&mut cache.output, frame.extent, *placement, *style);
            cache.output_state = Some(state);
        }
        let scene = match scenes.entry(key) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(device.create_scene().map_err(app_error)?)
            }
        };
        scene
            .bind_materialized_image(
                ImageId(1),
                cache.targets.last().unwrap(),
                ImageAlphaMode::Opaque,
            )
            .map_err(app_error)?;
        if let Some(mut delta) = cache.output.take_delta() {
            delta.epoch = scene
                .epoch()
                .checked_add(1)
                .ok_or_else(|| AppError::new("liquid glass scene epoch exhausted"))?;
            device.apply_scene_delta(scene, &delta).map_err(app_error)?;
        }
        output[index] = DesktopPlacement {
            scene: key,
            target: full_rect(frame.extent),
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
    fn glass_style_invalid_inputs_normalize_to_stable_finite_values() {
        let style = crate::GlassStyle {
            blur_radius: f32::NAN,
            bevel_width: f32::INFINITY,
            blend_softness: f32::NAN,
            refraction: -10.0,
            dispersion: 100.0,
            rim: f32::NEG_INFINITY,
            fresnel: -1.0,
            specular: 5.0,
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
        assert_eq!((style.rim, style.fresnel, style.specular), (0.3, 0.0, 1.0));
        assert_eq!(style.blend_softness, 0.0);
        assert_eq!(crate::GlassStyle { blend_softness: 200.0, ..style }.normalized().blend_softness, 128.0);
        assert_eq!(crate::GlassStyle { blend_softness: -1.0, ..style }.normalized().blend_softness, 0.0);
    }

    fn test_placement() -> DesktopPlacement {
        DesktopPlacement {
            key: DesktopLayerKey::ResizeVeil(1),
            scene: DesktopSceneKey::ResizeVeil(1),
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
        delta
            .image_resources
            .push(ImageResourceDelta::Write(ImageResourceUpdate {
                image: ImageId(1),
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
        assert_eq!(retained.material_parameters.len(), 17);
        assert_eq!(retained.gpu_materials[0].params_spatial_clip[1], 17);
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
            68,
            "only 68 bytes of optical data change"
        );
        retained.commit_uploads(0, 0, 0, 0, 0);
        style.blend_softness = 16.0;
        update_lens(&mut description, extent, p, style);
        let softness = description.take_delta().unwrap();
        assert!(softness.materials.is_empty() && softness.image_resources.is_empty());
        assert_eq!(retained.apply_delta_checked(&softness).unwrap().upload_bytes_queued, 68);
        assert_eq!(f32::from_bits(retained.material_parameters[16]), 16.0);
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
        for bad in [0, 1, 2, 3] {
            let mut invalid = bound.clone();
            if bad != 1 {
                if let crate::render::MaterialResourceDelta::Upsert(r) =
                    &mut invalid.material_resources[0]
                {
                    if let MaterialKind::LiquidGlass(p) = &mut r.kind {
                        match bad {
                            0 => p.inverse_bevel = f32::NAN,
                            2 => p.blend_softness = f32::NAN,
                            _ => p.blend_softness = -1.0,
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
        let key = DesktopSceneKey::ResizeGlass(1);
        let lower = [DesktopPlacement {
            scene: key,
            ..test_placement()
        }];
        let mut scenes = BTreeMap::from([(key, scene)]);
        let mut caches = BTreeMap::from([(
            DesktopSceneKey::ResizeVeil(1),
            GlassCache {
                signature: Signature {
                    extent,
                    levels: 1,
                    sources: Vec::new(),
                },
                revision: 1,
                targets: Vec::new(),
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
            .get_mut(&DesktopSceneKey::ResizeVeil(1))
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
        assert_eq!(pyramid_extents(size, 0.0), vec![size]);
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
        let mut composition = super::super::super::super::scene::DesktopComposition::new(extent);
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
            use super::super::super::super::scene::DesktopLayer;
            let mut veil = DesktopLayer::solid(
                DesktopLayerKey::ResizeVeil(1),
                DesktopSceneKey::ResizeVeil(1),
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
                        DesktopLayer::solid(
                            DesktopLayerKey::Background,
                            DesktopSceneKey::Background,
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
            assert_eq!(output[1].scene, DesktopSceneKey::ResizeGlass(1));
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
                caches[&DesktopSceneKey::ResizeVeil(1)].revision,
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
        let mut p = DesktopPlacement {
            key: DesktopLayerKey::FrameShadow(7),
            scene: DesktopSceneKey::FrameShadow(7),
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
        p.key = DesktopLayerKey::MotionShadow(7);
        assert!(is_own_shadow(&p, 7));
    }

    #[test]
    fn pyramid_uses_filtered_half_steps_without_a_tint_pass() {
        let sizes = pyramid_extents(
            SizeI {
                width: 1920,
                height: 1080,
            },
            24.0,
        );
        assert_eq!(
            sizes[0],
            SizeI {
                width: 1920,
                height: 1080
            }
        );
        for pair in sizes.windows(2) {
            assert_eq!(pair[1].width, (pair[0].width + 1) / 2);
            assert_eq!(pair[1].height, (pair[0].height + 1) / 2);
        }
        assert!(sizes.last().unwrap().width <= 240);
    }
    #[test]
    fn tiny_outputs_and_invalid_radii_stay_bounded() {
        for radius in [f32::NAN, f32::INFINITY, -5.0, 0.0, 1e30] {
            let sizes = pyramid_extents(
                SizeI {
                    width: 1,
                    height: 1,
                },
                radius,
            );
            assert_eq!(
                sizes,
                vec![
                    SizeI {
                        width: 1,
                        height: 1
                    };
                    1
                ]
            );
        }
    }
}

#[cfg(test)]
#[path = "glass_optics_tests.rs"]
mod optics_tests;
