use super::*;

#[test]
fn transformed_damage_matches_full_redraw_through_resize_fade_close_and_cancellation() {
    let mut h = Harness::new();
    h.state.corner_radii = crate::ui::CornerRadii::all(3.0);
    h.state.shadows = crate::ui::ShadowList::one(crate::ui::Shadow {
        offset: crate::foundation::PointF { x: 0.0, y: 2.0 }, blur: 2.0, spread: 1.0,
        color: ColorRgba8::rgba(0, 0, 0, 160),
    });
    let mut reference = SoftwareShellRenderer::new(3);
    let mut rotating = SoftwareShellRenderer::new(3);
    let mut scanout = vec![vec![0; 32 * 32 * 4]; 3];
    for tick in 0..40 {
        let now = tick * 20_000_000;
        let mut color = RED;
        match tick {
            3 => h.state.veiled = true,
            5 => h.state.bounds = RectI { x: 8, y: 6, width: 20, height: 22 },
            8 => h.state.veiled = false,
            12 => { h.state.minimized = true; h.overlay = true; }
            14 => h.state.minimized = false,
            17 => { h.state.bounds.x = -4; h.overlay = false; }
            22 => h.state.bounds = RectI { x: 10, y: 10, width: 12, height: 10 },
            25 => h.state.veiled = true,
            27 => { h.motion.close(1, now); h.mapped = false; }
            _ => {}
        }
        if tick >= 8 { color = ColorRgba8::rgba(10, 40, 220, 150); }
        let mut frame = h.draw(now, color, tick == 24, false);
        let slot = (tick % 3) as usize;
        let damage = rotating.render(slot, frame.clone()).unwrap();
        for y in damage.y..damage.bottom() {
            let start = (y as usize * 32 + damage.x as usize) * 4;
            let end = start + damage.width as usize * 4;
            scanout[slot][start..end].copy_from_slice(&rotating.pixels()[start..end]);
        }
        rotating.mark_copied(slot);
        frame.damage = None;
        reference.render((tick % 3) as usize, frame).unwrap();
        assert_eq!(h.renderer.pixels(), reference.pixels(), "tick {tick}");
        assert_eq!(scanout[slot], reference.pixels(), "scanout slot {slot}, tick {tick}");
        reference.mark_copied((tick % 3) as usize);
    }
}

#[test]
fn opacity_and_geometry_updates_keep_immutable_image_bindings() {
    let mut h = Harness::new();
    h.draw(0, RED, false, false);
    h.state.minimized = true;
    let mut version = None;
    let mut pixels = None;
    for now in [1, 30_000_001, 60_000_001] {
        let frame = h.draw(now, RED, false, false);
        let output = &frame.motion.outputs[0];
        let current = h.renderer.motion_outputs[&output.id].source.image_resource_version(crate::graphics::render::ImageId(1));
        if let Some(version) = version { assert_eq!(current, version); }
        version = Some(current);
        let snapshot = &h.renderer.motion_snapshots[&output.source].pixels;
        if let Some(pixels) = &pixels { assert!(std::sync::Arc::ptr_eq(pixels, snapshot)); }
        pixels = Some(snapshot.clone());
    }
}

#[test]
fn crossfade_keeps_no_scratch_image_until_interrupted_and_retires_endpoints() {
    let mut h = Harness::new();
    h.draw(0, RED, false, false);
    h.state.veiled = true;
    h.draw(1, BLUE, false, false);
    let middle = h.draw(50_000_001, BLUE, false, false);
    let mixed = middle.motion.outputs[0].source;
    assert!(h.renderer.motion_mixes.contains_key(&mixed));
    assert!(!h.renderer.motion_snapshots.contains_key(&mixed));
    assert_eq!(h.renderer.motion_snapshots.len(), 2);
    let before = h.pixel();
    h.state.veiled = false;
    h.draw(50_000_001, RED, false, false);
    assert_eq!(h.pixel(), before);
    assert!(h.renderer.motion_snapshots.contains_key(&mixed));
    h.draw(150_000_001, RED, false, false);
    assert!(h.renderer.motion_mixes.is_empty());
    assert_eq!(h.renderer.motion_snapshots.len(), 1);
}
