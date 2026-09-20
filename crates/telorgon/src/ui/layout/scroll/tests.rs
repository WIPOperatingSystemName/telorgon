use super::*;

fn state() -> ScrollState {
    ScrollState::new(
        SizeF {
            width: 100.0,
            height: 80.0,
        },
        SizeF {
            width: 400.0,
            height: 300.0,
        },
    )
    .unwrap()
}

fn assert_point(actual: PointF, expected: PointF) {
    assert!((actual.x - expected.x).abs() < 0.001, "x: {actual:?}");
    assert!((actual.y - expected.y).abs() < 0.001, "y: {actual:?}");
}

#[test]
fn delta_reports_consumed_and_unconsumed_distance() {
    let mut scroll = state();
    let update = scroll
        .scroll_by(PointF { x: 350.0, y: -20.0 }, ScrollInputSource::Wheel)
        .unwrap();

    assert_point(update.consumed_delta, PointF { x: 300.0, y: 0.0 });
    assert_point(update.unconsumed_delta, PointF { x: 50.0, y: -20.0 });
    assert_eq!(scroll.diagnostics().boundary_hits, 1);
    assert!(!scroll.metrics().can_scroll_right());
    assert!(scroll.metrics().can_scroll_left());
}

#[test]
fn invalid_extent_update_is_atomic() {
    let mut scroll = state();
    scroll
        .scroll_to(PointF { x: 30.0, y: 40.0 }, ScrollInputSource::Programmatic)
        .unwrap();
    let before = scroll.metrics();

    assert_eq!(
        scroll.set_extents(
            SizeF {
                width: f32::NAN,
                height: 80.0,
            },
            before.content,
            ScrollExtentAnchor::default(),
        ),
        Err(ScrollError::InvalidExtent)
    );
    assert_eq!(scroll.metrics(), before);
    assert_eq!(scroll.diagnostics().invalid_requests, 1);
}

#[test]
fn extent_anchor_preserves_distance_from_end() {
    let mut scroll = state();
    scroll
        .scroll_to(
            PointF { x: 280.0, y: 200.0 },
            ScrollInputSource::Programmatic,
        )
        .unwrap();
    scroll
        .set_extents(
            SizeF {
                width: 100.0,
                height: 80.0,
            },
            SizeF {
                width: 500.0,
                height: 500.0,
            },
            ScrollExtentAnchor::END,
        )
        .unwrap();

    assert_point(scroll.metrics().offset, PointF { x: 380.0, y: 400.0 });
}

#[test]
fn extent_correction_stops_ballistic_motion_before_publishing_bounds() {
    let mut scroll = state();
    scroll
        .scroll_to(PointF { x: 250.0, y: 0.0 }, ScrollInputSource::Programmatic)
        .unwrap();
    scroll.begin_drag();
    let started = scroll
        .end_drag(PointF { x: 100.0, y: 0.0 }, ScrollPhysics::default(), false)
        .unwrap();
    let ScrollMotionRequest::Start(id) = started.motion else {
        panic!("motion should start");
    };

    let corrected = scroll
        .set_extents(
            SizeF {
                width: 100.0,
                height: 80.0,
            },
            SizeF {
                width: 150.0,
                height: 300.0,
            },
            ScrollExtentAnchor::default(),
        )
        .unwrap();
    assert_eq!(corrected.after.offset.x, 50.0);
    assert_eq!(corrected.motion, ScrollMotionRequest::Stop(id));
    assert_eq!(scroll.activity(), ScrollActivity::Idle);
}

#[test]
fn nearest_reveal_moves_minimally_and_visible_target_is_a_noop() {
    let mut scroll = state();
    let update = scroll
        .reveal(RevealRequest::nearest(RectF {
            x: 120.0,
            y: 90.0,
            width: 20.0,
            height: 20.0,
        }))
        .unwrap();
    assert_point(update.after.offset, PointF { x: 40.0, y: 30.0 });

    let second = scroll
        .reveal(RevealRequest::nearest(RectF {
            x: 50.0,
            y: 40.0,
            width: 10.0,
            height: 10.0,
        }))
        .unwrap();
    assert!(!second.changed());
    assert_eq!(scroll.diagnostics().reveal_noops, 1);
}

#[test]
fn oversized_target_spanning_viewport_does_not_jitter() {
    let mut scroll = state();
    scroll
        .scroll_to(PointF { x: 50.0, y: 60.0 }, ScrollInputSource::Programmatic)
        .unwrap();
    let before = scroll.metrics();

    let update = scroll
        .reveal(RevealRequest::nearest(RectF {
            x: 20.0,
            y: 20.0,
            width: 180.0,
            height: 160.0,
        }))
        .unwrap();
    assert_eq!(update.after, before);
}

#[test]
fn explicit_reveal_alignment_is_clamped() {
    let scroll = state();
    let target = scroll
        .reveal_target(RevealRequest {
            target: RectF {
                x: 390.0,
                y: 290.0,
                width: 10.0,
                height: 10.0,
            },
            horizontal: Some(RevealAlignment::Center),
            vertical: Some(RevealAlignment::Fraction(1.0)),
        })
        .unwrap();
    assert_point(target, PointF { x: 300.0, y: 220.0 });
}

#[test]
fn drag_handoff_uses_caller_owned_ballistic_steps() {
    let mut scroll = state();
    scroll.begin_drag();
    scroll.drag_by(PointF { x: 10.0, y: 0.0 }).unwrap();
    let started = scroll
        .end_drag(
            PointF { x: 100.0, y: 0.0 },
            ScrollPhysics::new(100.0, 0.0).unwrap(),
            false,
        )
        .unwrap();
    let ScrollMotionRequest::Start(id) = started.motion else {
        panic!("ballistic motion should start");
    };

    let first = scroll.step_motion(id, Duration::from_millis(500)).unwrap();
    assert_point(first.consumed_delta, PointF { x: 37.5, y: 0.0 });
    assert_eq!(first.motion, ScrollMotionRequest::Continue(id));
    let second = scroll.step_motion(id, Duration::from_millis(500)).unwrap();
    assert_point(second.consumed_delta, PointF { x: 12.5, y: 0.0 });
    assert_eq!(second.motion, ScrollMotionRequest::Stop(id));
    assert_eq!(scroll.activity(), ScrollActivity::Idle);
}

#[test]
fn reduced_motion_skips_ballistic_travel() {
    let mut scroll = state();
    scroll.begin_drag();
    let update = scroll
        .end_drag(
            PointF { x: 2_000.0, y: 0.0 },
            ScrollPhysics::default(),
            true,
        )
        .unwrap();

    assert_eq!(update.motion, ScrollMotionRequest::None);
    assert_eq!(scroll.activity(), ScrollActivity::Idle);
    assert_eq!(scroll.diagnostics().ballistics_started, 0);
}

#[test]
fn stale_motion_step_is_rejected_without_offset_mutation() {
    let mut scroll = state();
    scroll.begin_drag();
    let started = scroll
        .end_drag(PointF { x: 100.0, y: 0.0 }, ScrollPhysics::default(), false)
        .unwrap();
    let ScrollMotionRequest::Start(current) = started.motion else {
        panic!("motion should start");
    };
    let stale = ScrollMotionId::from_raw(current.generation() + 1).unwrap();
    let before = scroll.metrics();

    assert_eq!(
        scroll.step_motion(stale, Duration::from_millis(16)),
        Err(ScrollError::StaleMotion {
            expected: current,
            received: stale,
        })
    );
    assert_eq!(scroll.metrics(), before);
    assert_eq!(scroll.diagnostics().stale_motion_steps, 1);
}

#[test]
fn extreme_elapsed_motion_saturates_without_publishing_nonfinite_values() {
    let mut scroll = state();
    scroll.begin_drag();
    let started = scroll
        .end_drag(
            PointF {
                x: f32::MAX,
                y: 0.0,
            },
            ScrollPhysics::default(),
            false,
        )
        .unwrap();
    let ScrollMotionRequest::Start(id) = started.motion else {
        panic!("motion should start");
    };

    let update = scroll.step_motion(id, Duration::MAX).unwrap();
    assert!(update.after.offset.x.is_finite());
    assert!(update.consumed_delta.x.is_finite());
    assert!(update.unconsumed_delta.x.is_finite());
    assert_eq!(update.motion, ScrollMotionRequest::Stop(id));
}

#[test]
fn replacement_drag_stops_old_motion_generation() {
    let mut scroll = state();
    scroll.begin_drag();
    let started = scroll
        .end_drag(PointF { x: 100.0, y: 0.0 }, ScrollPhysics::default(), false)
        .unwrap();
    let ScrollMotionRequest::Start(id) = started.motion else {
        panic!("motion should start");
    };

    let replacement = scroll.begin_drag();
    assert_eq!(replacement.motion, ScrollMotionRequest::Stop(id));
    assert_eq!(scroll.activity(), ScrollActivity::Dragging);
    assert_eq!(scroll.diagnostics().cancellations, 1);
}

#[test]
fn invalid_reveal_does_not_interrupt_active_motion() {
    let mut scroll = state();
    scroll.begin_drag();
    let before = scroll.metrics();

    assert_eq!(
        scroll.reveal(RevealRequest {
            target: RectF {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            horizontal: Some(RevealAlignment::Fraction(2.0)),
            vertical: None,
        }),
        Err(ScrollError::InvalidRevealAlignment)
    );
    assert_eq!(scroll.metrics(), before);
    assert_eq!(scroll.activity(), ScrollActivity::Dragging);
}

#[test]
fn overflowing_derived_reveal_bounds_are_rejected_atomically() {
    let mut scroll = state();
    let before = scroll.metrics();

    assert_eq!(
        scroll.reveal(RevealRequest::nearest(RectF {
            x: f32::MAX,
            y: 0.0,
            width: f32::MAX,
            height: 10.0,
        })),
        Err(ScrollError::InvalidRevealTarget)
    );
    assert_eq!(scroll.metrics(), before);
}
