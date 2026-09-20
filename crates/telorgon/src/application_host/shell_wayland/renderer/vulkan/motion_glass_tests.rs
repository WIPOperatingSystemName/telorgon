use super::super::super::super::motion::{WindowMotionController, WindowState};
use super::super::super::super::scene::{ShellComposition, ShellLayer};
use super::*;
use crate::render::{MaterialKind, RoundedClip};
use crate::theme::MotionPreference;

fn veil() -> ShellPlacement {
    ShellPlacement {
        key: ShellLayerKey::ResizeVeil(1),
        scene: ShellSceneKey::ResizeVeil(1),
        target: full_rect(SizeI {
            width: 400,
            height: 300,
        }),
        clip: None,
        rounded_clips: [None; 2],
    }
}
fn capture(id: u64) -> SnapshotCommand {
    SnapshotCommand {
        id,
        extent: SizeI {
            width: 400,
            height: 300,
        },
        content: SnapshotContent::Capture(vec![veil()]),
    }
}
fn styles() -> BTreeMap<ShellSceneKey, crate::GlassStyle> {
    BTreeMap::from([(veil().scene, crate::GlassStyle::liquid())])
}
#[test]
fn glass_pixels_never_enter_capture_or_interrupted_mix_snapshots() {
    let mut recipes = BTreeMap::new();
    let body = extract_recipe(&capture(1), &styles(), &mut recipes);
    assert!(body.is_empty());
    assert_eq!(recipes[&1].len(), 1);
    // Mix with ordinary content, then interrupt that fade with the glass endpoint.
    extract_recipe(
        &SnapshotCommand {
            id: 2,
            extent: SizeI {
                width: 900,
                height: 600,
            },
            content: SnapshotContent::Mix(vec![(1, 0.25), (99, 0.75)]),
        },
        &BTreeMap::new(),
        &mut recipes,
    );
    extract_recipe(
        &SnapshotCommand {
            id: 3,
            extent: SizeI {
                width: 500,
                height: 400,
            },
            content: SnapshotContent::Mix(vec![(2, 0.4), (1, 0.6)]),
        },
        &BTreeMap::new(),
        &mut recipes,
    );
    assert_eq!(
        recipes[&3].len(),
        1,
        "identical endpoints stay merged across interruptions"
    );
    assert!((recipes[&3][0].weight - 0.7).abs() < 1e-6);
    assert_eq!(
        recipes[&3][0].extent,
        capture(1).extent,
        "mixes must not round or rescale optical coordinates"
    );
    let mut state = MotionGlass {
        recipes,
        ..Default::default()
    };
    let frame = MotionFrame {
        live: [3].into_iter().collect(),
        ..Default::default()
    };
    retire_recipes(&mut state, &frame);
    assert_eq!(state.recipes.len(), 1);
    assert!((state.recipes[&3][0].weight - 0.7).abs() < 1e-6);
}

#[test]
fn a_shared_screen_pixel_keeps_its_backdrop_uv_while_the_lens_moves_and_resizes() {
    let sample = GlassSample {
        border: None,
        placement: veil(),
        extent: capture(1).extent,
        style: crate::GlassStyle::liquid(),
        weight: 0.5,
    };
    let extent = SizeI {
        width: 1920,
        height: 1080,
    };
    let screen = [450.0, 300.0];
    let mut uvs = Vec::new();
    for target in [
        RectI {
            x: 200,
            y: 100,
            width: 600,
            height: 500,
        },
        RectI {
            x: 100,
            y: 50,
            width: 1000,
            height: 700,
        },
    ] {
        let outer = RoundedClip::new(
            RectF {
                x: target.x as f32,
                y: target.y as f32,
                width: target.width as f32,
                height: target.height as f32,
            },
            crate::CornerRadii::all(20.0),
        );
        let output = ShellPlacement {
            key: ShellLayerKey::Motion(1),
            scene: ShellSceneKey::Motion(1),
            target,
            clip: None,
            rounded_clips: [Some(outer), None],
        };
        let lens = lens_placement(&sample, output);
        let mut description = RenderScene::default();
        glass::update_lens(&mut description, extent, lens, sample.style);
        let delta = description.take_delta().unwrap();
        let crate::render::MaterialResourceDelta::Upsert(resource) = delta.material_resources[0]
        else {
            panic!("lens resource")
        };
        let MaterialKind::LiquidGlass(parameters) = resource.kind else {
            panic!("lens parameters")
        };
        let rect = description.materials.values()[0].rect;
        let unit = [
            (screen[0] - rect.x) / rect.width,
            (screen[1] - rect.y) / rect.height,
        ];
        uvs.push([
            (rect.x + unit[0] * rect.width) * parameters.inverse_output_size[0],
            (rect.y + unit[1] * rect.height) * parameters.inverse_output_size[1],
        ]);
        assert_eq!(parameters.radii, [20.0; 4]);
        assert_eq!(
            parameters.refraction, sample.style.refraction,
            "pixel optics do not stretch with a snapshot"
        );
        let draw = lens_draw(lens, output, extent);
        assert_eq!(draw.target.x, -target.x);
        assert_eq!(draw.target.y, -target.y);
        assert_eq!(
            draw.clip,
            Some(full_rect(SizeI {
                width: target.width,
                height: target.height
            }))
        );
        assert_eq!(
            draw.rounded_clips, [None; 2],
            "outer coverage is applied once on final output"
        );
    }
    for axis in 0..2 {
        assert!((uvs[0][axis] - uvs[1][axis]).abs() < 1e-6);
    }
    assert!((uvs[0][0] - screen[0] / extent.width as f32).abs() < 1e-6);
}

fn window() -> WindowState {
    WindowState {
        bounds: RectI {
            x: 100,
            y: 80,
            width: 400,
            height: 300,
        },
        maximized: false,
        tiled: None,
        minimized: false,
        veiled: false,
        interactive: false,
        move_pointer: None,
        style: crate::WindowMotion::smooth()
            .maximize(crate::tween_ms(200, crate::Easing::Linear))
            .maximize_content(crate::ContentFade::new(30, 40)),
        corner_radii: crate::CornerRadii::all(18.0),
        shadows: Default::default(),
    }
}
fn advance(
    composition: &mut ShellComposition,
    controller: &mut WindowMotionController,
    state: WindowState,
    now_ms: u64,
) -> ShellFrame {
    let extent = SizeI {
        width: 1000,
        height: 800,
    };
    let mut layers = vec![ShellLayer::solid(
        ShellLayerKey::Background,
        ShellSceneKey::Background,
        ColorRgba8::rgba(10, 20, 30, 255),
        full_rect(extent),
    )];
    if !state.minimized {
        let mut layer = if state.veiled {
            ShellLayer::solid(
                ShellLayerKey::ResizeVeil(1),
                ShellSceneKey::ResizeVeil(1),
                crate::GlassStyle::liquid().tint,
                state.bounds,
            )
        } else {
            ShellLayer::solid(
                ShellLayerKey::Surface(1),
                ShellSceneKey::Surface(1),
                ColorRgba8::rgba(255, 0, 0, 255),
                state.bounds,
            )
        };
        if state.veiled {
            layer.glass = Some(crate::GlassStyle::liquid());
        }
        layers.push(layer);
    }
    let mut frame = composition
        .synchronize_with_force(extent, layers, true)
        .unwrap();
    controller.apply(
        &mut frame,
        BTreeMap::from([(1, state)]),
        &BTreeMap::from([(1, 1)]),
        now_ms * 1_000_000,
        MotionPreference::default(),
    );
    frame
}
#[test]
fn early_ready_maximize_and_minimize_keep_live_glass_without_protocol_veil_metadata() {
    let extent = SizeI {
        width: 1000,
        height: 800,
    };
    let mut composition = ShellComposition::new(extent);
    let mut controller = WindowMotionController::default();
    let mut recipes = BTreeMap::new();
    let mut state = window();
    for ms in [0, 1, 60, 100, 140, 160] {
        if ms == 1 {
            state.maximized = true;
            state.veiled = true;
            state.bounds = full_rect(extent);
            state.corner_radii = Default::default();
        }
        if ms == 100 {
            state.veiled = false;
        }
        if ms == 140 {
            state.minimized = true;
        }
        let frame = advance(&mut composition, &mut controller, state, ms);
        for command in &frame.motion.snapshots {
            let body = extract_recipe(command, &frame.glass, &mut recipes);
            assert!(body.iter().all(|p| !frame.glass.contains_key(&p.scene)));
        }
        if ms >= 60 {
            let endpoint = &frame.motion.outputs[0];
            let sample = recipes[&endpoint.source]
                .first()
                .expect("held or minimizing glass recipe");
            let output = frame
                .placements
                .iter()
                .find(|p| p.scene == ShellSceneKey::Motion(endpoint.id))
                .unwrap();
            let lens = lens_placement(sample, *output);
            assert_eq!(lens.target, output.target);
            if ms >= 100 {
                assert!(
                    frame.glass.is_empty(),
                    "early app readiness removes protocol metadata"
                );
            }
            if ms == 160 {
                assert!(endpoint.opacity < 1.0 && endpoint.opacity > 0.0);
            }
        }
    }
}
#[test]
#[ignore = "requires TELORGON_TEST_MODE=developer-hardware and a Vulkan adapter"]
fn live_motion_glass_keeps_stripes_aligned_and_refreshes_without_recapture() {
    live_motion_glass_fixture(false);
}

#[test]
#[ignore = "requires TELORGON_TEST_MODE=developer-hardware and a Vulkan adapter"]
fn bordered_motion_glass_composes_border_before_weighted_fade() {
    live_motion_glass_fixture(true);
}

fn live_motion_glass_fixture(bordered: bool) {
    use crate::render::{ReadbackFormat, ReadbackRequest};
    use crate::renderer_vulkan::OffscreenVulkanTarget;
    assert_eq!(
        std::env::var("TELORGON_TEST_MODE").as_deref(),
        Ok("developer-hardware")
    );
    let config = VulkanConfig {
        enable_validation: true,
        ..Default::default()
    };
    let instance = VulkanInstance::load(&config, &[]).unwrap();
    let selection = DeviceSelection::best(&instance.adapters().unwrap()).unwrap();
    let device = VulkanDevice::create_owned(instance, &config, &selection, None).unwrap();
    let extent = SizeI {
        width: 64,
        height: 48,
    };
    let mut composition = ShellComposition::new(extent);
    let mut scenes = BTreeMap::new();
    let mut snapshots = BTreeMap::new();
    let mut spares = Vec::new();
    let mut state = MotionGlass::default();
    let mut caches = BTreeMap::new();
    let style = crate::GlassStyle {
        tint: ColorRgba8::rgba(0, 255, 0, 64),
        blur_radius: 0.0,
        refraction: 0.0,
        dispersion: 0.0,
        fresnel: 0.0,
        ..crate::GlassStyle::liquid()
    };
    let mut last_revision = 0;
    let mut last_epoch = 0;
    for step in 0..4 {
        let target = if step == 0 {
            RectI {
                x: 10,
                y: 4,
                width: 32,
                height: 24,
            }
        } else {
            RectI {
                x: 6,
                y: 2,
                width: 52,
                height: 40,
            }
        };
        let mut frame = composition
            .synchronize_with_force(
                extent,
                vec![
                    ShellLayer::solid(
                        ShellLayerKey::Background,
                        ShellSceneKey::Background,
                        ColorRgba8::rgba(255, 0, 0, 255),
                        full_rect(extent),
                    ),
                    ShellLayer::solid(
                        ShellLayerKey::Widget(9),
                        ShellSceneKey::Widget(9),
                        if step < 2 {
                            ColorRgba8::rgba(0, 0, 255, 255)
                        } else {
                            ColorRgba8::rgba(255, 255, 255, 255)
                        },
                        RectI {
                            x: 32,
                            y: 0,
                            width: 32,
                            height: 48,
                        },
                    ),
                ],
                true,
            )
            .unwrap();
        if step == 0 {
            frame.glass.insert(ShellSceneKey::ResizeVeil(1), style);
            if bordered {
                frame.preview_borders.insert(
                    ShellSceneKey::ResizeVeil(1),
                    crate::Border::all(3.0, ColorRgba8::rgba(0, 0, 0, 255)),
                );
            }
            frame.motion.snapshots.push(SnapshotCommand {
                id: 1,
                extent: SizeI {
                    width: 32,
                    height: 24,
                },
                content: SnapshotContent::Capture(vec![ShellPlacement {
                    target: full_rect(SizeI {
                        width: 32,
                        height: 24,
                    }),
                    ..veil()
                }]),
            });
        }
        frame.motion.live.insert(1);
        frame.motion.outputs.push(SnapshotOutput {
            id: 1,
            source: 1,
            extent: SizeI {
                width: target.width,
                height: target.height,
            },
            opacity: if step == 1 { 0.5 } else { 1.0 },
        });
        frame.placements.push(ShellPlacement {
            key: ShellLayerKey::Motion(1),
            scene: ShellSceneKey::Motion(1),
            target,
            clip: None,
            rounded_clips: [
                Some(RoundedClip::new(
                    RectF {
                        x: target.x as f32,
                        y: target.y as f32,
                        width: target.width as f32,
                        height: target.height as f32,
                    },
                    crate::CornerRadii::all(5.0),
                )),
                None,
            ],
        });
        for update in &frame.updates {
            let scene = scenes
                .entry(update.key)
                .or_insert_with(|| device.create_scene().unwrap());
            for delta in &update.deltas {
                device.apply_scene_delta(scene, delta).unwrap();
            }
        }
        let mut recording = device.begin_owned_frame().unwrap();
        assert!(
            motion::record_motion(
                &device,
                &mut scenes,
                &mut snapshots,
                &mut spares,
                &frame.motion,
                &frame.glass,
                &frame.preview_borders,
                &mut state,
                &mut recording.context_mut()
            )
            .unwrap()
        );
        let output = record_output(
            &device,
            &mut scenes,
            &snapshots,
            &mut spares,
            &mut state,
            &mut caches,
            &frame,
            &mut recording.context_mut(),
        )
        .unwrap()
        .unwrap();
        let final_target = OffscreenVulkanTarget::new(&device, extent).unwrap();
        let indices = scenes
            .keys()
            .enumerate()
            .map(|(i, k)| (*k, i))
            .collect::<BTreeMap<_, _>>();
        let placements = output
            .iter()
            .map(|p| VulkanCompositePlacement {
                scene_index: indices[&p.scene],
                target: p.target,
                clip: p.clip,
                rounded_clips: p.rounded_clips,
            })
            .collect::<Vec<_>>();
        let mut source_scenes = scenes
            .values_mut()
            .map(|scene| VulkanCompositeScene { scene })
            .collect::<Vec<_>>();
        device
            .render_composite(
                &mut source_scenes,
                &placements,
                &mut recording.context_mut(),
                &final_target.target(),
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .unwrap();
        let pending = recording
            .context_mut()
            .record_readback(
                &final_target.target(),
                &ReadbackRequest {
                    region: full_rect(extent),
                    format: ReadbackFormat::Rgba8,
                },
            )
            .unwrap();
        let pixels = pending
            .bind_to_submission(recording.finish().unwrap().submit().unwrap())
            .unwrap()
            .wait(Duration::from_secs(10))
            .unwrap()
            .pixels;
        if bordered {
            let x = (target.x + 1) as usize;
            let border_pixel = &pixels[(16 * 64 + x) * 4..(16 * 64 + x) * 4 + 4];
            if step == 1 {
                assert!(
                    border_pixel[0] > 170
                        && border_pixel[0] < 205
                        && border_pixel[1] < 5
                        && border_pixel[2] < 5,
                    "half-faded black border must blend with the red desktop: {border_pixel:?}"
                );
            } else {
                assert!(
                    border_pixel[..3].iter().all(|v| *v < 5),
                    "opaque border must cover glass: {border_pixel:?}"
                );
            }
        }
        let left = &pixels[(20 * 64 + 25) * 4..(20 * 64 + 25) * 4 + 4];
        let right = &pixels[(20 * 64 + 35) * 4..(20 * 64 + 35) * 4 + 4];
        assert!(
            left[0] > 200 && left[1] > 80 && left[2] < 5,
            "live tint on left stripe: {left:?}"
        );
        if step < 2 {
            assert!(
                right[2] > 200 && right[1] > 80 && right[0] < 5,
                "right stripe must stay at x=32 after resize: {right:?}"
            );
        } else {
            assert!(
                right[0] > 200 && right[1] > 250 && right[2] > 200,
                "changed backdrop must refresh held glass: {right:?}"
            );
        }
        assert_eq!(state.recipes[&1].len(), 1);
        let resolved = &state.resolved[&1];
        let epoch = scenes[&ShellSceneKey::Motion(1)].epoch();
        if step == 3 {
            assert_eq!(resolved.revision, last_revision, "idle reuses the resolve");
            assert_eq!(
                epoch, last_epoch,
                "idle does not invalidate upper backdrops"
            );
        }
        last_revision = resolved.revision;
        last_epoch = epoch;
    }
}

#[test]
fn live_glass_uses_current_frame_bounds_instead_of_old_snapshot_shadow_padding() {
    let sample = GlassSample {
        border: None,
        placement: ShellPlacement {
            target: RectI {
                x: 60,
                y: 60,
                width: 400,
                height: 300,
            },
            ..veil()
        },
        extent: SizeI {
            width: 520,
            height: 420,
        },
        style: crate::GlassStyle::liquid(),
        weight: 0.5,
    };
    let frame = RectF {
        x: 100.0,
        y: 50.0,
        width: 900.0,
        height: 700.0,
    };
    let output = ShellPlacement {
        key: ShellLayerKey::Motion(1),
        scene: ShellSceneKey::Motion(1),
        target: RectI {
            x: 80,
            y: 30,
            width: 940,
            height: 740,
        },
        clip: None,
        rounded_clips: [
            Some(RoundedClip::new(frame, crate::CornerRadii::all(18.0))),
            None,
        ],
    };
    let lens = lens_placement(&sample, output);
    assert_eq!(
        lens.target,
        RectI {
            x: 100,
            y: 50,
            width: 900,
            height: 700
        }
    );
    let draw = lens_draw(
        lens,
        output,
        SizeI {
            width: 1920,
            height: 1080,
        },
    );
    assert_eq!(
        draw.clip,
        Some(RectI {
            x: 20,
            y: 20,
            width: 900,
            height: 700
        })
    );
    assert_eq!(draw.rounded_clips, [None; 2]);
}

#[test]
fn tiling_widget_glass_uses_an_independent_live_recipe() {
    let mut material = veil();
    material.key = ShellLayerKey::TilePreview(1);
    material.scene = ShellSceneKey::TilePreview(1);
    let mut command = capture(9);
    command.content = SnapshotContent::Capture(vec![material]);
    let styles = BTreeMap::from([(material.scene, crate::GlassStyle::liquid())]);
    let mut recipes = BTreeMap::new();
    assert!(extract_recipe(&command, &styles, &mut recipes).is_empty());
    assert_eq!(
        recipes[&9][0].placement.scene,
        ShellSceneKey::TilePreview(1)
    );
    assert_ne!(recipes[&9][0].placement.scene, veil().scene);
}

#[test]
fn glass_preview_border_travels_with_optical_recipe_through_interrupted_fades() {
    let border = crate::Border::all(2.0, ColorRgba8::rgba(120, 180, 255, 128));
    let borders = BTreeMap::from([(veil().scene, border)]);
    let mut command = capture(1);
    command.content = SnapshotContent::Capture(vec![
        veil(),
        ShellPlacement {
            key: ShellLayerKey::ResizePreviewBorder(1),
            scene: ShellSceneKey::ResizePreviewBorder(1),
            ..veil()
        },
    ]);
    let mut recipes = BTreeMap::new();
    assert!(extract_recipe_with_borders(&command, &styles(), &borders, &mut recipes).is_empty());
    assert_eq!(recipes[&1][0].border, Some(border));
    let mixed = SnapshotCommand {
        id: 2,
        extent: command.extent,
        content: SnapshotContent::Mix(vec![(1, 0.4), (99, 0.6)]),
    };
    extract_recipe_with_borders(&mixed, &BTreeMap::new(), &BTreeMap::new(), &mut recipes);
    assert_eq!(recipes[&2][0].border, Some(border));
    assert!((recipes[&2][0].weight - 0.4).abs() < 1e-6);
    // Color/fallback captures must retain the ordinary border scene.
    assert_eq!(
        extract_recipe_with_borders(&command, &BTreeMap::new(), &borders, &mut recipes).len(),
        2
    );
}

#[test]
fn motion_resolve_budget_admits_bordered_4k_and_remains_bounded() {
    let output = SizeI {
        width: 3840,
        height: 2400,
    };
    let bordered = resolve_bytes(output, 1);
    assert!(
        bordered > 64 * 1024 * 1024,
        "reproduce the old failure at native output density"
    );
    assert!(resolve_fits(output, output, 1, 0));
    assert!(
        resolve_fits(output, output, 2, 0),
        "interrupted border fade"
    );
    assert!(
        resolve_fits(output, output, 1, bordered),
        "two simultaneous bordered windows"
    );
    assert!(!resolve_fits(output, output, 2, bordered));
    assert_eq!(
        resolve_budget(SizeI {
            width: 1280,
            height: 720
        }),
        MIN_RESOLVE_BYTES
    );
    assert_eq!(
        resolve_budget(SizeI {
            width: 7680,
            height: 4800
        }),
        MAX_RESOLVE_BYTES
    );
    assert!(!resolve_fits(output, output, usize::MAX, u64::MAX));
    assert!(
        resolve_fits(output, output, 1, 0),
        "rejected admission must not poison later attempts"
    );
}

#[test]
fn fallback_discards_old_captures_and_later_maximize_restore_still_animate() {
    let mut composition = ShellComposition::new(SizeI {
        width: 1000,
        height: 800,
    });
    let mut controller = WindowMotionController::default();
    let restored = WindowState {
        shadows: crate::ui::ShadowList::one(crate::ui::Shadow {
            offset: crate::PointF { x: 0.0, y: 2.0 },
            blur: 4.0,
            spread: 0.0,
            color: ColorRgba8::rgba(0, 0, 0, 128),
        }),
        ..window()
    };
    let maximized = WindowState {
        maximized: true,
        veiled: true,
        bounds: RectI {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
        },
        ..restored
    };
    advance(&mut composition, &mut controller, restored, 0);
    let failed = advance(&mut composition, &mut controller, maximized, 10);
    assert!(controller.active(10_000_000));
    let last_id = failed.motion.snapshots.iter().map(|s| s.id).max().unwrap();
    let shadow_epoch = failed
        .updates
        .iter()
        .find(|update| update.key == ShellSceneKey::MotionShadow(1))
        .unwrap()
        .deltas
        .last()
        .unwrap()
        .epoch;
    controller.reset_after_fallback();
    assert!(!controller.active(10_000_000));
    assert!(controller.input(1, 1.0).is_none());
    let restore = advance(
        &mut composition,
        &mut controller,
        WindowState {
            veiled: true,
            ..restored
        },
        1100,
    );
    let recovered_shadow_epoch = restore
        .updates
        .iter()
        .find(|update| update.key == ShellSceneKey::MotionShadow(1))
        .unwrap()
        .deltas
        .last()
        .unwrap()
        .epoch;
    assert!(
        recovered_shadow_epoch > shadow_epoch,
        "recovery must not rewind retained shadow epochs"
    );
    assert!(!restore.motion.snapshots.is_empty());
    assert!(restore.motion.snapshots.iter().all(|s| s.id > last_id));
    assert!(
        restore
            .motion
            .snapshots
            .iter()
            .all(|s| matches!(s.content, SnapshotContent::Capture(_))),
        "a restore immediately after fallback must never mix discarded captures"
    );
    assert!(controller.active(1_100_000_000));
    let placement = restore
        .placements
        .iter()
        .find(|p| p.key == ShellLayerKey::Motion(1))
        .unwrap();
    assert_eq!(placement.target.width, maximized.bounds.width);
    advance(&mut composition, &mut controller, restored, 2000);
    advance(&mut composition, &mut controller, restored, 2100);
    let maximize_again = advance(&mut composition, &mut controller, maximized, 2200);
    assert!(controller.active(2_200_000_000));
    let placement = maximize_again
        .placements
        .iter()
        .find(|p| p.key == ShellLayerKey::Motion(1))
        .unwrap();
    assert_eq!(placement.target.width, restored.bounds.width);
}

#[test]
fn shared_resize_backdrop_is_stable_under_divider_motion_and_keeps_blur_styles_separate() {
    let size = SizeI {
        width: 800,
        height: 600,
    };
    let mut frame = ShellComposition::new(size)
        .synchronize_with_force(
            size,
            vec![ShellLayer::solid(
                ShellLayerKey::Background,
                ShellSceneKey::Background,
                ColorRgba8::rgba(1, 2, 3, 255),
                full_rect(size),
            )],
            true,
        )
        .unwrap();
    let mut recipes = BTreeMap::new();
    for id in 1..=4_u32 {
        let mut p = veil();
        p.key = ShellLayerKey::Motion(id);
        p.scene = ShellSceneKey::Motion(u64::from(id));
        frame.placements.push(p);
        frame.motion.resize_group.insert(id);
        frame.motion.outputs.push(SnapshotOutput {
            id: u64::from(id),
            source: u64::from(id),
            extent: size,
            opacity: 1.0,
        });
        let mut capture = capture(u64::from(id));
        let SnapshotContent::Capture(ref mut placements) = capture.content else {
            unreachable!()
        };
        placements[0].key = ShellLayerKey::ResizeVeil(id);
        placements[0].scene = ShellSceneKey::ResizeVeil(id);
        let mut style = crate::GlassStyle::liquid();
        style.tint = ColorRgba8::rgba(id as u8, 0, 0, 128);
        let scene = placements[0].scene;
        extract_recipe(&capture, &BTreeMap::from([(scene, style)]), &mut recipes);
    }
    let plan = shared_backdrop(&frame, &recipes);
    assert_eq!(plan.lower.len(), 1);
    assert_eq!(plan.keys.len(), 4);
    assert!(
        plan.keys
            .values()
            .all(|key| *key == ShellSceneKey::ResizeVeil(1))
    );
    frame.placements[1].target.width += 13;
    frame.placements[2].target.x += 13;
    assert_eq!(shared_backdrop(&frame, &recipes).lower, plan.lower);
    recipes.get_mut(&4).unwrap()[0].style.blur_radius += 1.0;
    let distinct = shared_backdrop(&frame, &recipes);
    assert_ne!(
        distinct.keys[&ShellSceneKey::ResizeVeil(4)],
        distinct.keys[&ShellSceneKey::ResizeVeil(1)]
    );
    let mut conflicting = recipes[&1][0].clone();
    conflicting.style.blur_radius += 2.0;
    recipes.get_mut(&1).unwrap().push(conflicting);
    assert!(
        shared_backdrop(&frame, &recipes).keys.is_empty(),
        "interrupted radius changes must not alias incompatible caches"
    );
    frame.motion.resize_group.clear();
    assert!(shared_backdrop(&frame, &recipes).keys.is_empty());
}

#[test]
fn capacity_reuse_crops_pixels_without_stretching_and_releases_large_shrinks() {
    let extent = SizeI {
        width: 701,
        height: 403,
    };
    let capacity = target_capacity(extent);
    assert_eq!(
        capacity,
        SizeI {
            width: 704,
            height: 448
        }
    );
    assert!(capacity_fits(
        capacity,
        SizeI {
            width: 650,
            height: 400
        }
    ));
    assert!(!capacity_fits(
        capacity,
        SizeI {
            width: 705,
            height: 400
        }
    ));
    assert!(!capacity_fits(
        capacity,
        SizeI {
            width: 300,
            height: 200
        }
    ));
    let scene = capacity_image_scene(extent, capacity, 0.5, true);
    let image = scene.images.get(NodeId::new(1, 1)).unwrap();
    assert_eq!(scene.extent.width, extent.width as f32);
    assert_eq!(image.rect.width, capacity.width as f32);
    // The logical right edge samples the same physical texel, not the padded edge.
    let uv = extent.width as f32 / image.rect.width;
    assert!((uv * capacity.width as f32 - extent.width as f32).abs() < 0.001);
}

#[test]
fn transparent_body_reduction_preserves_interrupted_mix_algebra() {
    let empty = std::collections::BTreeSet::from([1, 2]);
    assert!(body_is_empty(
        &SnapshotContent::Capture(vec![veil()]),
        &[],
        &empty
    ));
    assert!(!body_is_empty(
        &SnapshotContent::Capture(vec![veil()]),
        &[veil()],
        &empty
    ));
    assert!(body_is_empty(
        &SnapshotContent::Mix(vec![(1, 0.3), (2, 0.7), (9, 0.0)]),
        &[],
        &empty
    ));
    assert!(!body_is_empty(
        &SnapshotContent::Mix(vec![(1, 0.99), (9, 0.01)]),
        &[],
        &empty
    ));
    assert!(!body_is_empty(
        &SnapshotContent::Mix(vec![(9, f32::NAN)]),
        &[],
        &empty
    ));
}

#[test]
fn direct_resolve_requires_zero_body_and_one_fully_weighted_lens() {
    let mut recipes = BTreeMap::new();
    extract_recipe(&capture(1), &styles(), &mut recipes);
    let samples = recipes.get_mut(&1).unwrap();
    assert!(can_resolve_directly(true, samples));
    assert!(!can_resolve_directly(false, samples));
    samples[0].weight = 0.5;
    assert!(!can_resolve_directly(true, samples));
    samples[0].weight = 1.0;
    samples.push(samples[0].clone());
    assert!(!can_resolve_directly(true, samples));
    assert!(!can_resolve_directly(true, &[]));
}
