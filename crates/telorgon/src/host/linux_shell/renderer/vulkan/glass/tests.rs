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
fn bind_test_backdrop(delta: &mut crate::graphics::render::RenderSceneDelta) {
    use crate::graphics::render::*;
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
            if let crate::graphics::render::MaterialResourceDelta::Upsert(r) =
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
