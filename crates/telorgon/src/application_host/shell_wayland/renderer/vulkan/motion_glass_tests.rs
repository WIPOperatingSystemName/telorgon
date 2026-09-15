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
