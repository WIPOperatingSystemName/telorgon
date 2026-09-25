use super::*;

#[test]
fn client_preview_contour_applies_during_handoff_but_not_normal_movement() {
    let mut h = Harness::new();
    h.state.client_decorated = true;
    h.state.corner_radii = crate::CornerRadii::all(6.0);
    h.state.interactive = true;
    let corner = |h: &Harness| h.renderer.pixels()[(4 * 32 + 4) * 4..(4 * 32 + 4) * 4 + 4].to_vec();
    let idle = h.draw(0, RED, false, false);
    let placement = idle
        .placements
        .iter()
        .find(|p| p.key == ShellLayerKey::Motion(1))
        .unwrap();
    assert_eq!(placement.rounded_clips, [None; 2]);
    assert_eq!(corner(&h), [255, 0, 0, 255]);

    h.state.veiled = true;
    h.draw(10_000_000, BLUE, false, false);
    let preview = h.draw(120_000_000, BLUE, false, false);
    let placement = preview
        .placements
        .iter()
        .find(|p| p.key == ShellLayerKey::Motion(1))
        .unwrap();
    assert_eq!(
        placement.rounded_clips[0].unwrap().radii,
        crate::CornerRadii::all(6.0)
    );
    assert_eq!(corner(&h), [0, 255, 0, 255]);
    assert_eq!(h.pixel(), [0, 0, 255, 255]);

    h.state.veiled = false;
    h.draw(130_000_000, RED, false, false);
    assert_eq!(corner(&h), [0, 255, 0, 255]);
    let ready = h.draw(240_000_000, RED, false, false);
    let placement = ready
        .placements
        .iter()
        .find(|p| p.key == ShellLayerKey::Motion(1))
        .unwrap();
    assert_eq!(placement.rounded_clips, [None; 2]);
    assert_eq!(corner(&h), [255, 0, 0, 255]);
}

// The capture includes asymmetric transparent client shadow margins, whereas the
// preview covers only the window geometry. Check pixels, not just placement bounds.
fn draw_shadowed_preview(h: &mut Harness, now: u64) {
    let size = SizeI {
        width: 32,
        height: 32,
    };
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
    if !h.state.veiled {
        layers.push(ShellLayer::solid(
            ShellLayerKey::Surface(1),
            ShellSceneKey::Surface(1),
            ColorRgba8::rgba(0, 0, 0, 0),
            RectI {
                x: 1,
                y: 2,
                width: 26,
                height: 27,
            },
        ));
    }
    layers.push(ShellLayer::solid(
        ShellLayerKey::ContentBackground(1),
        ShellSceneKey::ContentBackground(1),
        if h.state.veiled { BLUE } else { RED },
        h.state.bounds,
    ));
    let mut frame = h
        .composition
        .synchronize_with_force(size, layers, true)
        .unwrap();
    h.motion.apply(
        &mut frame,
        BTreeMap::from([(1, h.state)]),
        &BTreeMap::from([(1, 1)]),
        now,
        MotionPreference::Full,
    );
    h.renderer.render(0, frame).unwrap();
    h.renderer.mark_copied(0);
}

#[test]
fn shadow_padding_does_not_shrink_content_on_preview_entry_exit_or_interruption() {
    let mut h = Harness::new();
    h.state.client_decorated = true;
    h.state.interactive = true;
    // Interior edge pixels must stay covered throughout the fade. Previously the
    // wider source was squeezed inward and the green desktop appeared here.
    let covered = |h: &Harness| {
        for (x, y) in [(5, 12), (18, 12), (12, 5), (12, 18)] {
            let pixel = &h.renderer.pixels()[(y * 32 + x) * 4..][..4];
            assert_eq!(pixel[1], 0, "content shrank at {x},{y}: {pixel:?}");
        }
    };
    draw_shadowed_preview(&mut h, 0);
    covered(&h);
    for (time, veiled) in [
        (10, true),
        (60, true),
        (65, false),
        (100, false),
        (105, true),
        (160, true),
        (220, true),
        (230, false),
        (280, false),
        (340, false),
    ] {
        h.state.veiled = veiled;
        draw_shadowed_preview(&mut h, time * 1_000_000);
        covered(&h);
    }
}

#[test]
fn widget_fade_repaints_outside_unrelated_damage_including_final_opaque_frame() {
    let size = SizeI {
        width: 32,
        height: 32,
    };
    let bounds = RectI {
        x: 8,
        y: 8,
        width: 16,
        height: 16,
    };
    let mut composition = ShellComposition::new(size);
    let mut motion = WindowMotionController::default();
    let mut renderer = SoftwareShellRenderer::new(1);
    let mut previous_blue = 0;
    for opacity in [0.0, 0.25, 0.75, 1.0, 0.5, 0.0] {
        let mut frame = composition
            .synchronize_with_force(
                size,
                vec![
                    ShellLayer::solid(
                        ShellLayerKey::Background,
                        ShellSceneKey::Background,
                        ColorRgba8::rgba(0, 255, 0, 255),
                        RectI {
                            x: 0,
                            y: 0,
                            width: 32,
                            height: 32,
                        },
                    ),
                    ShellLayer::solid(
                        ShellLayerKey::Widget(2),
                        ShellSceneKey::Widget(2),
                        BLUE,
                        bounds,
                    ),
                ],
                true,
            )
            .unwrap();
        // A moving client/cursor supplies partial damage that excludes the preview.
        // The retained preview's geometry and source pixels themselves have not changed.
        frame.damage = Some(RectI {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        });
        motion.apply_widget_visibility(&mut frame, &[(2, opacity)]);
        renderer.render(0, frame).unwrap();
        renderer.mark_copied(0);
        let pixel = &renderer.pixels()[(12 * 32 + 12) * 4..][..4];
        if opacity == 0.0 {
            assert_eq!(pixel, &[0, 255, 0, 255]);
        } else if opacity == 1.0 {
            assert_eq!(pixel, &[0, 0, 255, 255]);
        } else if opacity != 0.5 {
            assert!(pixel[2] > previous_blue, "fade did not repaint: {pixel:?}");
        } else {
            assert!(
                pixel[2] < previous_blue,
                "fade-out did not repaint: {pixel:?}"
            );
        }
        previous_blue = pixel[2];
    }
}
