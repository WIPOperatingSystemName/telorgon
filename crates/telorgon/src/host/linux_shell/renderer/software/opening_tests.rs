use super::*;

#[test]
fn opening_fades_the_composed_window_without_resizing_and_settles() {
    let mut h = Harness::new();
    h.state.style = WindowMotion::none().open(crate::tween_ms(100, crate::Easing::Linear));
    let start = h.draw(0, RED, false, true);
    assert_eq!(h.pixel(), [0, 255, 0, 255]);
    assert!(h.motion.active(0));
    let middle = h.draw(50_000_000, RED, false, true);
    let pixel = h.pixel();
    assert_eq!(pixel[0], 0, "overlapping content must fade as one composed image");
    assert!(pixel[1] > 0 && pixel[1] < 255 && pixel[2] > 0 && pixel[2] < 255, "{pixel:?}");
    assert!((i16::from(pixel[1]) - i16::from(pixel[2])).abs() <= 1, "halfway blend: {pixel:?}");
    for frame in [&start, &middle] {
        let placement = frame.placements.iter().find(|p| p.key == ShellLayerKey::Motion(1)).unwrap();
        assert_eq!(placement.target, h.state.bounds);
    }
    h.draw(100_000_000, RED, false, true);
    assert_eq!(h.pixel(), [0, 0, 255, 255]);
    assert!(!h.motion.active(100_000_000));
    h.draw(150_000_000, RED, false, false);
    assert_eq!(h.pixel(), [255, 0, 0, 255]);
}

#[test]
fn opening_skips_reduced_motion_and_does_not_replay_on_recreation() {
    let mut h = Harness::new();
    h.state.style = WindowMotion::none().open(crate::tween_ms(100, crate::Easing::Linear)).open_from_scale(0.5);
    h.draw(0, RED, true, false);
    assert_eq!(h.pixel(), [255, 0, 0, 255]);
    h.draw(10_000_000, RED, false, false);
    assert_eq!(h.pixel(), [255, 0, 0, 255]);
    h.motion.withdraw_from_primary(1);
    h.draw(20_000_000, RED, false, false);
    assert_eq!(h.pixel(), [255, 0, 0, 255]);
    h.motion.close(1, 30_000_000);
    h.draw(40_000_000, BLUE, false, false);
    assert_eq!(h.pixel(), [0, 255, 0, 255], "a remapped window starts a fresh fade");
}

#[test]
fn opening_interrupted_by_close_starts_from_current_opacity() {
    let mut h = Harness::new();
    h.state.style = WindowMotion::none()
        .open(crate::tween_ms(100, crate::Easing::Linear))
        .open_from_scale(0.5)
        .close(crate::Minimize::shrink_and_fade(100));
    h.draw(0, RED, false, false);
    let before_frame = h.draw(50_000_000, RED, false, false);
    let before = h.pixel();
    h.motion.close(1, 50_000_000);
    h.mapped = false;
    let closed_frame = h.draw(50_000_000, RED, false, false);
    assert_eq!(h.pixel(), before);
    let target = |frame: &ShellFrame| frame.placements.iter().find(|p| p.key == ShellLayerKey::Motion(1)).unwrap().target;
    assert_eq!(target(&closed_frame), target(&before_frame));
    h.draw(150_000_000, RED, false, false);
    assert_eq!(h.pixel(), [0, 255, 0, 255]);
    assert!(!h.motion.active(150_000_000));
}

#[test]
fn opening_grows_centered_with_fade_and_settles_at_exact_bounds() {
    let mut h = Harness::new();
    h.state.style = WindowMotion::none()
        .open(crate::tween_ms(100, crate::Easing::Linear))
        .open_from_scale(0.5);
    for (now, width, x, opacity) in [(0, 8, 8, 0.0), (50_000_000, 12, 6, 0.5), (100_000_000, 16, 4, 1.0)] {
        let frame = h.draw(now, RED, false, true);
        let placement = frame.placements.iter().find(|p| p.key == ShellLayerKey::Motion(1)).unwrap();
        assert_eq!(placement.target, RectI { x, y: x, width, height: width });
        assert_eq!(frame.motion.outputs[0].opacity, opacity);
        assert_eq!(h.state.bounds, RectI { x: 4, y: 4, width: 16, height: 16 });
    }
    assert!(!h.motion.active(100_000_000));
}

#[test]
fn opening_growth_with_zero_duration_is_immediate() {
    let mut h = Harness::new();
    h.state.style = WindowMotion::none()
        .open(crate::tween_ms(0, crate::Easing::EaseOut))
        .open_from_scale(0.5);
    h.draw(0, RED, false, false);
    assert_eq!(h.pixel(), [255, 0, 0, 255]);
    assert!(!h.motion.active(0));
}

#[test]
fn close_waits_for_its_first_render_after_a_long_event_processing_delay() {
    let mut h = Harness::new();
    h.state.style = WindowMotion::none().close(crate::Minimize::shrink_and_fade(100));
    h.draw(0, RED, false, false);
    h.motion.close(1, 10_000_000);
    h.mapped = false;
    let start = h.draw(500_000_000, BLUE, false, false);
    assert_eq!(start.motion.outputs[0].opacity, 1.0);
    assert_eq!(h.pixel(), [255, 0, 0, 255]);
    let middle = h.draw(550_000_000, BLUE, false, false);
    assert!(middle.motion.outputs[0].opacity > 0.0 && middle.motion.outputs[0].opacity < 1.0);
    let end = h.draw(600_000_000, BLUE, false, false);
    assert!(end.motion.outputs.is_empty());
    assert_eq!(h.pixel(), [0, 255, 0, 255]);
}
