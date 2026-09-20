use super::*;

#[test]
fn inverse_clip_flags_preserve_target_flags_and_reset_between_placements() {
    let clip = crate::graphics::render::RoundedClip::new(
        crate::foundation::RectF {
            x: 10.0,
            y: 12.0,
            width: 30.0,
            height: 40.0,
        },
        crate::ui::CornerRadii::all(6.0),
    );
    let mut view = GpuView {
        epoch_flags: [12, 0, 1, 1],
        ..GpuView::default()
    };
    set_placement_clips(&mut view, [Some(clip.inverse()), Some(clip)]);
    assert_eq!(view.epoch_flags, [12, 0, 1, 0b011]);
    assert_eq!(view.placement_clip_rects[0], [10.0, 12.0, 30.0, 40.0]);
    assert_eq!(view.placement_clip_radii[0], [6.0; 4]);
    set_placement_clips(&mut view, [None, Some(clip.inverse())]);
    assert_eq!(view.epoch_flags[3], 0b101);
    assert_eq!(view.placement_clip_rects[0][2], -1.0);
    set_placement_clips(&mut view, [None; 2]);
    assert_eq!(view.epoch_flags[3], 1);
    assert!(view.placement_clip_rects.iter().all(|r| r[2] < 0.0));
}
