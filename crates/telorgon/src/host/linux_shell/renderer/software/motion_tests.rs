use super::*;
use crate::host::linux_shell::motion::{WindowMotionController, WindowState};
use crate::host::linux_shell::scene::{
    ShellComposition, ShellLayer, ShellLayerKey,
};
use crate::foundation::SizeI;
use crate::theme::MotionPreference;
use crate::{ContentFade, WindowMotion};
#[test]
fn empty_retained_sources_are_published_before_motion_capture_and_after_recreation() {
    let size = SizeI {
        width: 32,
        height: 32,
    };
    let mut composition = ShellComposition::new(size);
    let mut renderer = SoftwareShellRenderer::new(1);
    let mut motion = WindowMotionController::default();
    for cycle in 0..2 {
        for tick in 0..3 {
            let mut source = crate::graphics::render::RenderScene::default();
            source.extent = crate::foundation::SizeF {
                width: 32.0,
                height: 32.0,
            };
            source.background = RED;
            let deltas = if tick == 2 {
                source.take_delta().into_iter().collect()
            } else {
                Vec::new()
            };
            let mut frame = composition
                .synchronize_with_force(
                    size,
                    vec![ShellLayer::retained(
                        ShellLayerKey::Widget(1),
                        ShellSceneKey::Widget(1),
                        deltas,
                        size,
                        crate::foundation::PointI::default(),
                        true,
                    )],
                    true,
                )
                .unwrap();
            assert_eq!(frame.updates.len(), usize::from(tick != 1), "cycle {cycle}");
            motion.apply_widget_visibility(&mut frame, &[(1, 0.5)]);
            if cycle == 0 && tick == 0 {
                let mut unpublished = frame.clone();
                unpublished.updates.clear();
                let err = SoftwareShellRenderer::new(1)
                    .render(0, unpublished)
                    .unwrap_err();
                assert!(format!("{err:?}").contains("scene Widget(1) missing"));
            }
            renderer.render(0, frame).unwrap();
            if tick == 2 {
                assert!(
                    renderer.pixels()[0] > 0,
                    "first real delta must not be discarded as stale"
                );
            }
        }
        let removed = composition
            .synchronize_with_force(size, Vec::new(), true)
            .unwrap();
        renderer.render(0, removed).unwrap();
    }
}

struct Harness {
    composition: ShellComposition,
    motion: WindowMotionController,
    renderer: SoftwareShellRenderer,
    state: WindowState,
    mapped: bool,
    overlay: bool,
}
impl Harness {
    fn new() -> Self {
        Self {
            composition: ShellComposition::new(SizeI {
                width: 32,
                height: 32,
            }),
            mapped: true,
            overlay: false,
            motion: Default::default(),
            renderer: SoftwareShellRenderer::new(1),
            state: WindowState {
                bounds: RectI {
                    x: 4,
                    y: 4,
                    width: 16,
                    height: 16,
                },
                maximized: false,
                tiled: None,
                interactive: false,
                move_pointer: None,
                minimized: false,
                veiled: false,
                style: WindowMotion::smooth().resize_content(ContentFade::new(100, 100)),
                corner_radii: Default::default(),
                shadows: Default::default(),
            },
        }
    }
    fn draw(
        &mut self,
        now: u64,
        color: ColorRgba8,
        reduced: bool,
        overlap: bool,
    ) -> ShellFrame {
        let mut layers = vec![ShellLayer::solid(
            ShellLayerKey::Background,
            ShellSceneKey::Background,
            ColorRgba8::rgba(0, 255, 0, 255),
            RectI {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            },
        )];
        if self.mapped && !self.state.minimized {
            layers.push(ShellLayer::solid(
                ShellLayerKey::ContentBackground(1),
                ShellSceneKey::ContentBackground(1),
                color,
                self.state.bounds,
            ));
            if overlap {
                layers.push(ShellLayer::solid(
                    ShellLayerKey::ResizeVeil(1),
                    ShellSceneKey::ResizeVeil(1),
                    ColorRgba8::rgba(0, 0, 255, 255),
                    self.state.bounds,
                ));
            }
        }
        if self.overlay {
            layers.push(ShellLayer::solid(
                ShellLayerKey::Cursor,
                ShellSceneKey::CursorImage,
                BLUE,
                self.state.bounds,
            ));
        }
        let mut frame = self
            .composition
            .synchronize_with_force(
                SizeI {
                    width: 32,
                    height: 32,
                },
                layers,
                true,
            )
            .unwrap();
        self.motion.apply(
            &mut frame,
            if self.mapped {
                BTreeMap::from([(1, self.state)])
            } else {
                BTreeMap::new()
            },
            &BTreeMap::from([(1, 1)]),
            now,
            if reduced {
                MotionPreference::Reduced
            } else {
                MotionPreference::Full
            },
        );
        self.renderer.render(0, frame.clone()).unwrap();
        self.renderer.mark_copied(0);
        frame
    }
    fn pixel(&self) -> [u8; 4] {
        self.renderer.pixels()[(12 * 32 + 12) * 4..(12 * 32 + 12) * 4 + 4]
            .try_into()
            .unwrap()
    }
}
const RED: ColorRgba8 = ColorRgba8::rgba(255, 0, 0, 255);
const BLUE: ColorRgba8 = ColorRgba8::rgba(0, 0, 255, 255);
#[test]
fn close_fades_last_image_without_input_and_retires_on_the_final_frame() {
    let mut h = Harness::new();
    h.state.style = WindowMotion::none().close(crate::Minimize::shrink_and_fade(100));
    h.draw(0, RED, false, false);
    h.motion.close(1, 10_000_000);
    h.mapped = false;
    let start = h.draw(10_000_000, BLUE, false, false);
    assert_eq!(h.pixel(), [255, 0, 0, 255]);
    assert!(h.motion.input(1, 1.0).is_none());
    let middle = h.draw(60_000_000, BLUE, false, false);
    assert_eq!(middle.motion.outputs[0].opacity, 0.125);
    assert_eq!(
        middle.placements.last().unwrap().target,
        crate::host::linux_shell::motion::minimize_rect(
            start.placements.last().unwrap().target,
            0.125
        )
    );
    let pixel = h.pixel();
    assert!(pixel[0] > 0 && pixel[1] > 0 && pixel[2] == 0, "{pixel:?}");
    assert!(
        h.motion.active(110_000_000),
        "request the final clearing frame"
    );
    let end = h.draw(110_000_000, BLUE, false, false);
    assert!(end.damage.is_none());
    assert!(end.motion.outputs.is_empty());
    assert!(end.motion.live.is_empty());
    assert!(!h.motion.active(110_000_000));
    assert_eq!(h.pixel(), [0, 255, 0, 255]);
}

#[test]
fn close_matches_configured_minimize_geometry_and_opacity() {
    let mut closing = Harness::new();
    let mut minimizing = Harness::new();
    for h in [&mut closing, &mut minimizing] {
        h.state.style = WindowMotion::smooth().minimize(crate::Minimize::shrink_and_fade(300));
        h.draw(0, RED, false, false);
    }
    closing.motion.close(1, 10_000_000);
    closing.mapped = false;
    minimizing.state.minimized = true;
    for now in [10_000_000, 110_000_000, 210_000_000] {
        let a = closing.draw(now, RED, false, false);
        let b = minimizing.draw(now, RED, false, false);
        assert_eq!(a.motion.outputs[0].opacity, b.motion.outputs[0].opacity);
        assert_eq!(
            a.placements.last().unwrap().target,
            b.placements.last().unwrap().target
        );
        assert_eq!(closing.pixel(), minimizing.pixel());
    }
}

#[test]
fn close_stays_below_overlays_after_its_original_layers_are_removed() {
    let mut h = Harness::new();
    h.overlay = true;
    h.draw(0, RED, false, false);
    h.motion.close(1, 10_000_000);
    h.mapped = false;
    h.draw(60_000_000, RED, false, false);
    assert_eq!(h.pixel(), [0, 0, 255, 255]);
}

#[test]
fn disabled_close_and_already_minimized_windows_leave_no_exit_image() {
    for minimized in [false, true] {
        let mut h = Harness::new();
        if !minimized {
            h.state.style = WindowMotion::smooth().close(crate::Minimize::shrink_and_fade(0));
        }
        h.draw(0, RED, false, false);
        if minimized {
            h.state.minimized = true;
            h.draw(10_000_000, RED, false, false);
            h.draw(500_000_000, RED, false, false);
        }
        h.motion.close(1, 510_000_000);
        h.mapped = false;
        let frame = h.draw(520_000_000, RED, false, false);
        assert!(frame.motion.outputs.is_empty());
        assert!(!h.motion.active(520_000_000));
    }
}

#[test]
fn close_cancels_for_reduced_motion_remapping_lock_and_fallback() {
    for mode in 0..4 {
        let mut h = Harness::new();
        h.draw(0, RED, false, false);
        h.motion.close(1, 10_000_000);
        h.mapped = mode == 1;
        if mode == 2 {
            h.motion.cancel_closing();
        }
        if mode == 3 {
            h.motion.reset_after_fallback();
        }
        let frame = h.draw(20_000_000, BLUE, mode == 0, false);
        assert_eq!(
            h.pixel(),
            if h.mapped {
                [0, 0, 255, 255]
            } else {
                [0, 255, 0, 255]
            }
        );
        if !h.mapped {
            assert!(frame.motion.outputs.is_empty());
        }
        assert!(!h.motion.active(20_000_000));
    }
}

#[test]
fn close_during_content_fade_keeps_the_last_sampled_mix() {
    let mut h = Harness::new();
    h.draw(0, RED, false, false);
    h.state.veiled = true;
    h.draw(10_000_000, BLUE, false, false);
    h.draw(60_000_000, BLUE, false, false);
    let before = h.pixel();
    h.motion.close(1, 60_000_000);
    h.mapped = false;
    h.draw(60_000_000, RED, false, false);
    assert_eq!(h.pixel(), before);
}

#[test]
fn tiling_switches_use_maximize_timing_and_floating_uses_restore_timing() {
    let mut h = Harness::new();
    h.state.style = WindowMotion::smooth()
        .maximize(crate::tween_ms(200, crate::Easing::Linear))
        .restore(crate::tween_ms(400, crate::Easing::Linear))
        .maximize_content(ContentFade::new(0, 0));
    let normal = h.state.bounds;
    h.draw(0, RED, false, false);
    h.state.tiled = Some(crate::TileTarget::Left);
    h.state.veiled = true;
    h.state.bounds = RectI {
        x: 0,
        y: 0,
        width: 16,
        height: 32,
    };
    let start = h.draw(10_000_000, BLUE, false, false);
    assert_eq!(start.placements.last().unwrap().target, normal);
    let middle = h.draw(110_000_000, BLUE, false, false);
    assert_eq!(
        middle.placements.last().unwrap().target,
        RectI {
            x: 2,
            y: 2,
            width: 16,
            height: 24
        }
    );
    h.draw(210_000_000, BLUE, false, false);
    h.state.veiled = false;
    h.draw(250_000_000, RED, false, false);
    h.state.tiled = Some(crate::TileTarget::Right);
    h.state.bounds.x = 16;
    h.state.veiled = true;
    h.draw(300_000_000, BLUE, false, false);
    let switching = h.draw(400_000_000, BLUE, false, false);
    assert_eq!(switching.placements.last().unwrap().target.x, 8);
    h.draw(500_000_000, BLUE, false, false);
    h.state.veiled = false;
    h.draw(550_000_000, RED, false, false);
    h.state.tiled = None;
    h.state.bounds = normal;
    h.state.veiled = true;
    h.draw(600_000_000, BLUE, false, false);
    let restoring = h.draw(800_000_000, BLUE, false, false);
    assert_eq!(
        restoring.placements.last().unwrap().target,
        RectI {
            x: 10,
            y: 2,
            width: 16,
            height: 24
        }
    );
    let settled = h.draw(1_000_000_000, BLUE, false, false);
    assert_eq!(settled.placements.last().unwrap().target, normal);
    h.state.maximized = true;
    h.state.bounds = RectI {
        x: 0,
        y: 0,
        width: 32,
        height: 32,
    };
    h.draw(1_100_000_000, BLUE, false, false);
    h.draw(1_300_000_000, BLUE, false, false);
    h.state.veiled = false;
    h.draw(1_350_000_000, RED, false, false);
    h.state.maximized = false;
    h.state.tiled = Some(crate::TileTarget::Left);
    h.state.bounds.width = 16;
    h.state.veiled = true;
    h.draw(1_400_000_000, BLUE, false, false);
    let max_to_tile = h.draw(1_500_000_000, BLUE, false, false);
    assert_eq!(
        max_to_tile.placements.last().unwrap().target.width,
        24,
        "entering a tile uses maximize timing even from a maximized window"
    );
}

#[test]
fn shared_resize_membership_survives_release_until_content_fade_finishes() {
    let mut h = Harness::new();
    h.state.tiled = Some(crate::TileTarget::Left);
    h.draw(0, RED, false, false);
    h.state.interactive = true;
    h.state.veiled = true;
    assert!(
        h.draw(10_000_000, BLUE, false, false)
            .motion
            .resize_group
            .contains(&1)
    );
    h.draw(110_000_000, BLUE, false, false);
    h.state.interactive = false;
    assert!(
        h.draw(120_000_000, BLUE, false, false)
            .motion
            .resize_group
            .contains(&1)
    );
    h.state.veiled = false;
    assert!(
        h.draw(130_000_000, RED, false, false)
            .motion
            .resize_group
            .contains(&1)
    );
    assert!(
        h.draw(180_000_000, RED, false, false)
            .motion
            .resize_group
            .contains(&1)
    );
    assert!(
        h.draw(230_000_000, RED, false, false)
            .motion
            .resize_group
            .is_empty()
    );
}

#[test]
fn shared_divider_uses_resize_content_fades_and_waits_for_ready_content() {
    let mut h = Harness::new();
    h.state.tiled = Some(crate::TileTarget::Left);
    h.state.style = WindowMotion::smooth()
        .maximize_content(ContentFade::new(0, 0))
        .resize_content(ContentFade::new(100, 200));
    h.draw(0, RED, false, false);
    h.state.interactive = true;
    h.state.veiled = true;
    h.state.bounds.width = 23;
    h.draw(10_000_000, BLUE, false, false);
    h.draw(60_000_000, BLUE, false, false);
    let entry = h.pixel();
    assert!(entry[0] > 0 && entry[2] > 0, "{entry:?}");
    h.draw(110_000_000, BLUE, false, false);
    assert_eq!(h.pixel(), [0, 0, 255, 255]);
    h.state.interactive = false;
    h.draw(310_000_000, BLUE, false, false);
    assert_eq!(h.pixel(), [0, 0, 255, 255], "release is not readiness");
    h.state.veiled = false;
    h.draw(320_000_000, RED, false, false);
    h.draw(420_000_000, RED, false, false);
    let ready = h.pixel();
    assert!(ready[0] > 0 && ready[2] > 0, "{ready:?}");
    h.draw(520_000_000, RED, false, false);
    assert_eq!(h.pixel(), [255, 0, 0, 255]);
}

#[test]
fn shared_divider_and_reduced_motion_keep_direct_tile_geometry() {
    let mut h = Harness::new();
    h.state.tiled = Some(crate::TileTarget::Left);
    h.draw(0, RED, false, false);
    h.state.interactive = true;
    h.state.veiled = true;
    h.state.bounds.width = 23;
    let dragging = h.draw(10_000_000, BLUE, false, false);
    assert_eq!(dragging.placements.last().unwrap().target, h.state.bounds);
    h.state.interactive = false;
    let released = h.draw(20_000_000, BLUE, false, false);
    assert_eq!(released.placements.last().unwrap().target, h.state.bounds);
    h.state.tiled = Some(crate::TileTarget::Right);
    h.state.bounds.x = 16;
    let reduced = h.draw(30_000_000, BLUE, true, false);
    assert!(reduced.motion.outputs.is_empty());
    assert!(
        reduced
            .placements
            .iter()
            .any(|p| p.target == h.state.bounds)
    );
}

#[test]
fn fluid_geometry_overshoots_and_still_waits_for_ready_content() {
    for tiled in [false, true] {
        let mut h = Harness::new();
        h.state.style = WindowMotion::fluid()
            .maximize_spring(
                crate::Spring::new()
                    .initial_velocity(2.0)
                    .damping_ratio(0.3)
                    .angular_frequency(8.0)
                    .settle_within_ms(800),
            )
            .maximize_content(ContentFade::new(0, 130));
        h.draw(0, RED, false, false);
        h.state.maximized = !tiled;
        h.state.tiled = tiled.then_some(crate::TileTarget::Left);
        h.state.veiled = true;
        h.state.bounds = RectI {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        };
        h.draw(1, BLUE, false, false);
        let overshoot = h.draw(400_000_001, BLUE, false, false);
        assert!(
            overshoot.placements.last().unwrap().target.width > 32,
            "geometry must not clamp spring progress at one"
        );
        h.state.veiled = false;
        h.draw(450_000_001, RED, false, false);
        assert_eq!(h.pixel(), [0, 0, 255, 255]);
        let settled = h.draw(800_000_001, RED, false, false);
        assert_eq!(settled.placements.last().unwrap().target, h.state.bounds);
        assert_eq!(h.pixel(), [0, 0, 255, 255]);
        h.draw(930_000_001, RED, false, false);
        assert_eq!(h.pixel(), [255, 0, 0, 255]);
        assert!(!h.motion.active(930_000_001));
    }
}
#[test]
fn shadow_follows_maximize_restore_and_manual_resize_bounds() {
    let mut h = Harness::new();
    let shadows = crate::ui::ShadowList::one(crate::ui::Shadow {
        offset: crate::foundation::PointF { x: 0.0, y: 2.0 },
        blur: 2.0,
        spread: 1.0,
        color: ColorRgba8::rgba(0, 0, 0, 160),
    });
    h.state.shadows = shadows;
    h.state.corner_radii = crate::ui::CornerRadii::all(3.0);
    h.draw(0, RED, false, false);
    h.state.maximized = true;
    h.state.veiled = true;
    h.state.shadows = Default::default();
    h.state.bounds = RectI {
        x: 0,
        y: 0,
        width: 32,
        height: 32,
    };
    h.draw(1, BLUE, false, false);
    assert!(
        h.renderer.pixels()[(12 * 32 + 3) * 4 + 1] < 255,
        "the shadow must darken the desktop outside the starting window"
    );
    let check = |frame: &ShellFrame| {
        let shadow = frame
            .placements
            .iter()
            .position(|p| p.key == ShellLayerKey::MotionShadow(1))
            .expect("independent shadow");
        let window = &frame.placements[shadow + 1];
        assert_eq!(window.key, ShellLayerKey::Motion(1));
        let cutout = frame.placements[shadow].rounded_clips[0].unwrap();
        assert!(cutout.inverted);
        assert_eq!(cutout.rect.width, window.target.width as f32);
        assert_eq!(cutout.rect.height, window.target.height as f32);
        assert_eq!(cutout.rect.x, window.target.x as f32);
        assert_eq!(
            frame.placements[shadow].target.width - window.target.width,
            10,
            "shadow reach must remain fixed as the window changes size"
        );
        for update in &frame.updates {
            if update.key == ShellSceneKey::MotionShadow(1) {
                for delta in &update.deltas {
                    assert!(
                        delta.image_resources.is_empty(),
                        "shadow must remain analytic"
                    );
                }
            }
        }
    };
    check(&h.draw(100_000_001, BLUE, false, false));
    check(&h.draw(200_000_001, BLUE, false, false));
    h.draw(290_000_001, BLUE, false, false);
    h.state.maximized = false;
    h.state.shadows = shadows;
    h.state.bounds = RectI {
        x: 4,
        y: 4,
        width: 16,
        height: 16,
    };
    h.draw(300_000_001, BLUE, false, false);
    check(&h.draw(450_000_001, BLUE, false, false));
    h.state.interactive = true;
    for (i, width) in [20, 24, 18].into_iter().enumerate() {
        h.state.bounds.width = width;
        check(&h.draw(500_000_001 + i as u64 * 10_000_000, BLUE, false, false));
    }
}
#[test]
fn opaque_motion_crossfade_never_exposes_background_and_settles() {
    let mut h = Harness::new();
    h.draw(0, RED, false, false);
    h.state.veiled = true;
    h.draw(1, BLUE, false, false);
    assert_eq!(h.pixel(), [255, 0, 0, 255]);
    let frame = h.draw(50_000_001, BLUE, false, false);
    assert_eq!(
        h.pixel()[1],
        0,
        "desktop green must not leak through the crossfade"
    );
    let snapshot = frame.motion.outputs[0].source;
    assert_eq!(
        h.renderer.motion_snapshots[&snapshot].pixels_rgba8()[3],
        255
    );
    assert!(
        h.motion.active(100_000_001),
        "one final sample must still be requested"
    );
    h.draw(100_000_001, BLUE, false, false);
    assert_eq!(h.pixel(), [0, 0, 255, 255]);
    assert!(!h.motion.active(100_000_001));
    assert_eq!(h.renderer.motion_snapshots.len(), 1);
}
#[test]
fn transparent_placeholder_discards_old_pixels_and_ready_fade_reverses() {
    let mut h = Harness::new();
    h.draw(0, RED, false, false);
    h.state.veiled = true;
    h.draw(1, ColorRgba8::rgba(0, 0, 0, 0), false, false);
    h.draw(50_000_001, ColorRgba8::rgba(0, 0, 0, 0), false, false);
    let before = h.pixel();
    h.state.veiled = false;
    h.draw(50_000_001, BLUE, false, false);
    let after = h.pixel();
    for (a, b) in before.into_iter().zip(after) {
        assert!(
            a.abs_diff(b) <= 1,
            "interruption must preserve displayed pixels"
        );
    }
    h.draw(150_000_001, BLUE, false, false);
    assert_eq!(h.pixel(), [0, 0, 255, 255]);
    h.state.veiled = true;
    h.draw(160_000_001, ColorRgba8::rgba(0, 0, 0, 0), false, false);
    h.draw(260_000_001, ColorRgba8::rgba(0, 0, 0, 0), false, false);
    assert_eq!(h.pixel(), [0, 255, 0, 255]);
}
#[test]
fn minimize_fades_composed_group_and_unminimize_can_reverse() {
    let mut h = Harness::new();
    h.draw(0, RED, false, true);
    h.state.minimized = true;
    h.draw(1, RED, false, true);
    h.draw(90_000_001, RED, false, true);
    let before = h.pixel();
    assert_eq!(before[0], 0, "hidden red child must not bleed through blue");
    assert!(before[1] > 0 && before[2] > 0);
    h.state.minimized = false;
    h.draw(90_000_001, RED, false, true);
    assert_eq!(h.pixel(), before);
    h.draw(270_000_001, RED, false, true);
    assert_eq!(h.pixel(), [0, 0, 255, 255]);
    h.state.minimized = true;
    h.draw(280_000_001, RED, false, true);
    h.draw(460_000_001, RED, false, true);
    assert_eq!(h.pixel(), [0, 255, 0, 255]);
    assert!(h.renderer.motion_snapshots.is_empty());
    assert!(!h.motion.active(460_000_001));
}
#[test]
fn maximize_keeps_target_state_and_maps_displayed_input() {
    let mut h = Harness::new();
    h.draw(0, RED, false, false);
    h.state.maximized = true;
    h.state.bounds = RectI {
        x: 0,
        y: 0,
        width: 32,
        height: 32,
    };
    let frame = h.draw(1, RED, false, false);
    assert_eq!(frame.placements.last().unwrap().target.width, 16);
    assert_eq!(h.state.bounds.width, 32);
    let input = h.motion.input(1, 1.0).unwrap();
    assert!(input.block_content);
    let local = input.map(crate::foundation::PointF { x: 4.0, y: 4.0 });
    assert_eq!(local, crate::foundation::PointF { x: 0.0, y: 0.0 });
    h.draw(240_000_001, RED, false, false);
    assert!(!h.motion.input(1, 1.0).unwrap().block_content);
    assert!(!h.motion.active(240_000_001));
}
#[test]
fn maximize_fades_before_growth_and_holds_early_ready_content() {
    for tiled in [false, true] {
        let mut h = Harness::new();
        h.state.corner_radii = crate::ui::CornerRadii::all(4.0);
        h.draw(0, RED, false, false);
        h.state.maximized = !tiled;
        h.state.tiled = tiled.then_some(crate::TileTarget::Left);
        h.state.veiled = true;
        h.state.corner_radii = Default::default();
        h.state.bounds = RectI {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        };
        h.draw(1, BLUE, false, false);
        let entry = h.draw(25_000_001, BLUE, false, false);
        assert_eq!(entry.placements.last().unwrap().target.width, 16);
        assert!(entry.motion.snapshots.iter().all(|s| s.extent.width <= 16));
        let moving = h.draw(100_000_001, BLUE, false, false);
        let placement = moving.placements.last().unwrap();
        assert!(placement.target.width > 16 && placement.target.width < 32);
        assert_eq!(
            placement.rounded_clips[0].unwrap().radii,
            crate::ui::CornerRadii::all(4.0)
        );
        assert!(
            moving.motion.snapshots.is_empty(),
            "movement must reuse the placeholder"
        );
        h.state.veiled = false; // App redraw arrives before movement completes.
        let early = h.draw(150_000_001, RED, false, false);
        assert!(
            early.motion.snapshots.is_empty(),
            "early client frames stay hidden"
        );
        assert_eq!(h.pixel(), [0, 0, 255, 255]);
        assert!(h.motion.input(1, 1.0).unwrap().block_content);
        h.draw(290_000_001, RED, false, false);
        assert_eq!(
            h.pixel(),
            [0, 0, 255, 255],
            "reveal starts only after movement"
        );
        h.draw(420_000_001, RED, false, false);
        assert_eq!(h.pixel(), [255, 0, 0, 255]);
        assert!(!h.motion.active(420_000_001));
        // Restore uses the destination's rounded contour, even though maximize was square.
        h.state.maximized = false;
        h.state.tiled = None;
        h.state.veiled = true;
        h.state.corner_radii = crate::ui::CornerRadii::all(4.0);
        h.state.bounds = RectI {
            x: 4,
            y: 4,
            width: 16,
            height: 16,
        };
        h.draw(500_000_001, BLUE, false, false);
        let restoring = h.draw(650_000_001, BLUE, false, false);
        assert_eq!(
            restoring.placements.last().unwrap().rounded_clips[0]
                .unwrap()
                .radii,
            crate::ui::CornerRadii::all(4.0)
        );
        h.draw(790_000_001, BLUE, false, false);
        assert_eq!(
            h.pixel(),
            [0, 0, 255, 255],
            "late client keeps placeholder at destination"
        );
        assert!(
            !h.motion.active(790_000_001),
            "waiting for the app must not spin"
        );
        h.state.veiled = false;
        h.draw(800_000_001, RED, false, false);
        let settled = h.draw(930_000_001, RED, false, false);
        assert_eq!(
            settled.damage, None,
            "restore completion must repaint all scanout slots"
        );
        assert_eq!(
            settled.placements.last().unwrap().rounded_clips,
            [None; 2],
            "the final animation frame must already include the exterior shadow"
        );
        // A later hover repaint must not be the first frame to remove the animation clip.
        let hovered = h.draw(940_000_001, BLUE, false, false);
        assert_eq!(
            settled.placements.last().unwrap().rounded_clips,
            hovered.placements.last().unwrap().rounded_clips
        );
    }
}
#[test]
fn leaving_output_cancels_the_old_visual_instead_of_leaving_a_ghost() {
    let mut h = Harness::new();
    h.draw(0, RED, false, false);
    h.state.bounds.x = 100;
    h.draw(1, RED, false, false);
    assert_eq!(h.pixel(), [0, 255, 0, 255]);
    assert!(h.renderer.motion_snapshots.is_empty());
    assert!(!h.motion.active(1));
}
#[test]
fn direct_manipulation_cancels_geometry_easing() {
    let mut h = Harness::new();
    h.draw(0, RED, false, false);
    h.state.maximized = true;
    h.state.bounds = RectI {
        x: 0,
        y: 0,
        width: 32,
        height: 32,
    };
    h.draw(1, RED, false, false);
    h.state.interactive = true;
    h.state.maximized = false;
    h.state.bounds = RectI {
        x: 2,
        y: 2,
        width: 20,
        height: 20,
    };
    let frame = h.draw(20_000_001, RED, false, false);
    assert_eq!(frame.placements.last().unwrap().target, h.state.bounds);
    assert!(!h.motion.active(20_000_001));
}
#[test]
fn titlebar_drag_restores_size_while_tracking_pointer_without_restarting() {
    for tiled in [false, true] {
        for style in [WindowMotion::smooth(), WindowMotion::fluid()] {
            let mut h = Harness::new();
            h.state.style = style;
            h.state.maximized = !tiled;
            h.state.tiled = tiled.then_some(crate::TileTarget::Left);
            h.state.bounds = RectI {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            };
            h.draw(0, RED, false, false);
            h.state.maximized = false;
            h.state.tiled = None;
            h.state.veiled = true;
            h.state.interactive = true;
            h.state.move_pointer = Some(crate::foundation::PointF { x: 16.0, y: 2.0 });
            h.state.bounds = RectI {
                x: 8,
                y: 0,
                width: 16,
                height: 16,
            };
            let start = h.draw(1, BLUE, false, false);
            assert_eq!(
                start.placements.last().unwrap().target.width,
                32,
                "titlebar restore must not snap to the final size"
            );
            h.state.bounds.x += 4;
            h.state.bounds.y += 3;
            h.state.move_pointer = Some(crate::foundation::PointF { x: 20.0, y: 5.0 });
            let moving = h.draw(150_000_001, BLUE, false, false);
            let bounds = moving.placements.last().unwrap().target;
            assert!(bounds.width > 16 && bounds.width < 32);
            assert!((bounds.x as f32 + bounds.width as f32 * 0.5 - 20.0).abs() <= 1.0);
            assert_eq!(
                bounds.y, 3,
                "vertical dragging must follow the pointer immediately"
            );
            h.state.interactive = false;
            h.state.move_pointer = None;
            let released = h.draw(200_000_001, BLUE, false, false);
            assert!(
                released.placements.last().unwrap().target.width > 16,
                "release must not snap an unfinished restore"
            );
            let finish_time = 1 + u64::from(
                style.maximize_transition(false).duration_ms()
                    + style.maximize_content_transition(true).duration_ms,
            ) * 1_000_000;
            let finished = h.draw(finish_time, BLUE, false, false);
            assert_eq!(finished.placements.last().unwrap().target, h.state.bounds);
            assert!(
                !h.motion.active(finish_time),
                "pointer moves must not extend the deadline"
            );
            assert!(
                h.motion.input(1, 1.0).unwrap().block_content,
                "the app's readiness gate still controls the reveal"
            );
        }
    }
}

#[test]
fn early_ready_drag_restore_keeps_snapshot_and_clip_in_the_same_coordinates() {
    for tiled in [false, true] {
        for style in [WindowMotion::smooth(), WindowMotion::fluid()] {
            let mut h = Harness::new();
            h.state.style = style;
            h.state.maximized = !tiled;
            h.state.tiled = tiled.then_some(crate::TileTarget::Left);
            h.state.bounds = RectI {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            };
            h.draw(0, RED, false, false);
            h.state.maximized = false;
            h.state.tiled = None;
            h.state.veiled = true;
            h.state.interactive = true;
            h.state.bounds = RectI {
                x: 8,
                y: 0,
                width: 16,
                height: 16,
            };
            h.state.move_pointer = Some(crate::foundation::PointF { x: 16.0, y: 2.0 });
            h.draw(1, BLUE, false, false);
            h.draw(60_000_001, BLUE, false, false);
            h.state.veiled = false; // Ready client is retained while the placeholder finishes moving.
            for (i, (x, y)) in [(10, 3), (6, 6), (9, 2)].into_iter().enumerate() {
                h.state.bounds.x = x;
                h.state.bounds.y = y;
                h.state.move_pointer = Some(crate::foundation::PointF {
                    x: x as f32 + 8.0,
                    y: y as f32 + 2.0,
                });
                let frame = h.draw(100_000_001 + i as u64 * 40_000_000, RED, false, false);
                let placement = frame.placements.last().unwrap();
                let clip = placement.rounded_clips[0].unwrap().rect;
                assert_eq!(
                    placement.target.x as f32, clip.x,
                    "snapshot must move with its clip"
                );
                assert_eq!(
                    placement.target.y as f32, clip.y,
                    "snapshot must move with its clip"
                );
                assert_eq!(placement.target.width as f32, clip.width);
                assert!(
                    frame.motion.snapshots.is_empty(),
                    "dragging must reuse the held placeholder"
                );
                let sample_x = placement.target.x.max(0) as usize + 2;
                let sample_y = placement.target.y.max(0) as usize + 2;
                let offset = (sample_y * 32 + sample_x) * 4;
                assert_eq!(
                    &h.renderer.pixels()[offset..offset + 4],
                    &[0, 0, 255, 255],
                    "placeholder must cover the moving frame without an inner gap"
                );
            }
        }
    }
}

#[test]
fn reduced_motion_cancels_snapshots_and_preserves_placeholder() {
    let mut h = Harness::new();
    h.draw(0, RED, false, false);
    h.state.veiled = true;
    h.draw(1, BLUE, false, false);
    let frame = h.draw(2, BLUE, true, false);
    assert!(frame.motion.outputs.is_empty());
    assert!(h.renderer.motion_snapshots.is_empty());
    assert_eq!(h.pixel(), [0, 0, 255, 255]);
    assert!(h.state.veiled);
}
