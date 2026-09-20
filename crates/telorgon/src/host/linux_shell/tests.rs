use super::*;

#[test]
fn atomic_cursor_motion_coalesces_while_a_commit_is_in_flight() {
    let mut cursor = CursorCommitTracker::default();
    cursor.move_to(PointI { x: 10, y: 20 });
    assert_eq!(cursor.desired_submission(), None);

    cursor.show(0, PointI::default());
    let submitted = cursor
        .desired_submission()
        .expect("visible cursor is dirty");
    cursor.mark_submitted(submitted).unwrap();
    cursor.move_to(PointI { x: 30, y: 40 });
    assert_eq!(cursor.desired_submission(), None);

    cursor.mark_completed(submitted).unwrap();
    let coalesced = cursor
        .desired_submission()
        .expect("newest position follows the completed generation");
    assert_eq!(coalesced.position, PointI { x: 30, y: 40 });
    assert_eq!(coalesced.buffer, Some(0));
}

#[test]
fn atomic_cursor_staging_never_reuses_current_or_in_flight_buffers() {
    let mut cursor = CursorCommitTracker::default();
    cursor.show(0, PointI::default());
    let first = cursor.desired_submission().unwrap();
    cursor.mark_submitted(first).unwrap();
    cursor.mark_completed(first).unwrap();

    cursor.show(1, PointI::default());
    let second = cursor.desired_submission().unwrap();
    cursor.mark_submitted(second).unwrap();

    assert_eq!(cursor.current_buffer, Some(0));
    assert_eq!(cursor.in_flight.and_then(|state| state.buffer), Some(1));
    assert_eq!(
        cursor.reusable_buffer(HARDWARE_CURSOR_BUFFER_COUNT),
        Some(2)
    );
}

#[test]
fn hardware_cursor_fallback_waits_for_plane_disable_completion() {
    let mut cursor = CursorCommitTracker::default();
    cursor.show(0, PointI::default());
    let visible = cursor.desired_submission().unwrap();
    cursor.mark_submitted(visible).unwrap();
    cursor.mark_completed(visible).unwrap();

    cursor.request_composited_fallback();
    assert!(!cursor.ready_to_retire());
    let hidden = cursor.desired_submission().expect("plane disable is dirty");
    assert!(!hidden.visible);
    cursor.mark_submitted(hidden).unwrap();
    cursor.mark_completed(hidden).unwrap();
    assert!(cursor.ready_to_retire());
}

#[test]
fn vulkan_staging_budget_covers_one_full_hd_upload_per_slot() {
    let slots = 2;
    let budget = vulkan_staging_budget_bytes(
        SizeI {
            width: 1920,
            height: 1080,
        },
        slots,
    )
    .expect("Full HD staging budget should fit");
    let bytes_per_slot = budget / slots as u64;

    assert_eq!(
        bytes_per_slot,
        1920 * 1080 * 4 + VULKAN_STAGING_HEADROOM_BYTES_PER_SLOT
    );
    // This is the upload size reported by the original Raspberry Pi Full HD failure.
    assert!(bytes_per_slot >= 8_294_628);
}

#[test]
fn vulkan_staging_budget_preserves_headroom_for_small_outputs() {
    let slots = 3;
    let budget = vulkan_staging_budget_bytes(
        SizeI {
            width: 640,
            height: 480,
        },
        slots,
    )
    .expect("small scanout staging budget should fit");

    assert_eq!(
        budget,
        (640 * 480 * 4 + VULKAN_STAGING_HEADROOM_BYTES_PER_SLOT) * slots as u64
    );
    assert!(budget >= VULKAN_STAGING_MIN_BYTES_PER_SLOT * slots as u64);
}

#[test]
fn vulkan_staging_budget_rejects_total_slot_overflow() {
    assert!(
        vulkan_staging_budget_bytes(
            SizeI {
                width: i32::MAX,
                height: i32::MAX,
            },
            usize::MAX,
        )
        .is_err()
    );
}

#[test]
fn accumulated_damage_unions_every_change_since_the_target_version() {
    let history = VecDeque::from([
        (1, None),
        (
            2,
            Some(RectI {
                x: 10,
                y: 20,
                width: 30,
                height: 40,
            }),
        ),
        (
            3,
            Some(RectI {
                x: 35,
                y: 45,
                width: 20,
                height: 10,
            }),
        ),
    ]);

    assert_eq!(
        accumulated_damage(
            1,
            3,
            &history,
            SizeI {
                width: 100,
                height: 100,
            },
        ),
        Some(RectI {
            x: 10,
            y: 20,
            width: 45,
            height: 40,
        })
    );
}

#[test]
fn accumulated_damage_requires_full_redraw_after_a_full_change_or_history_gap() {
    let extent = SizeI {
        width: 100,
        height: 100,
    };
    let full_change = VecDeque::from([
        (
            2,
            Some(RectI {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            }),
        ),
        (3, None),
    ]);
    let history_gap = VecDeque::from([(
        4,
        Some(RectI {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        }),
    )]);

    assert_eq!(accumulated_damage(1, 3, &full_change, extent), None);
    assert_eq!(accumulated_damage(1, 4, &history_gap, extent), None);
    assert_eq!(accumulated_damage(0, 4, &history_gap, extent), None);
}

#[test]
fn every_wayland_cursor_shape_round_trips_through_pointer_icons() {
    for shape in 1..=36 {
        let icon = cursor_shape_pointer_icon(shape).expect("shape must be supported");
        assert_eq!(pointer_icon_cursor_shape(icon), shape);
        assert!(cursor_shape_icon_name(shape).is_some());
    }
}

#[test]
fn cursor_image_transitions_require_presentation_without_scene_damage() {
    assert!(cursor_transition_requires_presentation(
        CursorImage::Hidden,
        CursorImage::Shape(pointer_icon_cursor_shape(PointerIcon::Move)),
    ));
    assert!(cursor_transition_requires_presentation(
        CursorImage::Shape(pointer_icon_cursor_shape(PointerIcon::Move)),
        CursorImage::TelorgonDefault,
    ));
    assert!(!cursor_transition_requires_presentation(
        CursorImage::TelorgonDefault,
        CursorImage::TelorgonDefault,
    ));
}

#[test]
fn client_commits_do_not_roll_back_a_newer_requested_resize() {
    let committed = SizeI {
        width: 640,
        height: 480,
    };
    let requested = SizeI {
        width: 720,
        height: 540,
    };

    assert_eq!(retained_requested_size(None, committed), committed);
    assert_eq!(
        retained_requested_size(Some(requested), committed),
        requested
    );
}

#[test]
fn resize_drag_geometry_is_derived_from_the_press_time_baseline() {
    let position = PointI { x: 100, y: 80 };
    let size = SizeI {
        width: 400,
        height: 300,
    };
    let output = SizeI {
        width: 1920,
        height: 1080,
    };

    assert_eq!(
        resize_drag_geometry(
            position,
            size,
            ResizeEdge::Right,
            PointI { x: 50, y: 0 },
            output,
        ),
        (
            position,
            SizeI {
                width: 450,
                height: 300,
            },
        )
    );
    assert_eq!(
        resize_drag_geometry(
            position,
            size,
            ResizeEdge::Right,
            PointI { x: 20, y: 0 },
            output,
        )
        .1
        .width,
        420
    );
    assert_eq!(
        resize_drag_geometry(
            position,
            size,
            ResizeEdge::Bottom,
            PointI { x: 0, y: 70 },
            output,
        )
        .1
        .height,
        370
    );
}

#[test]
fn left_and_top_resize_edges_keep_the_opposite_edges_anchored() {
    let position = PointI { x: 100, y: 80 };
    let size = SizeI {
        width: 400,
        height: 300,
    };
    let output = SizeI {
        width: 1920,
        height: 1080,
    };
    let (resized_position, resized_size) = resize_drag_geometry(
        position,
        size,
        ResizeEdge::TopLeft,
        PointI { x: 40, y: 30 },
        output,
    );

    assert_eq!(resized_position, PointI { x: 140, y: 110 });
    assert_eq!(
        resized_size,
        SizeI {
            width: 360,
            height: 270,
        }
    );
    assert_eq!(
        resized_position.x + resized_size.width,
        position.x + size.width
    );
    assert_eq!(
        resized_position.y + resized_size.height,
        position.y + size.height
    );

    let (minimum_position, minimum_size) = resize_drag_geometry(
        position,
        size,
        ResizeEdge::TopLeft,
        PointI {
            x: 10_000,
            y: 10_000,
        },
        output,
    );
    assert_eq!(
        minimum_size,
        SizeI {
            width: 1,
            height: 1
        }
    );
    assert_eq!(
        minimum_position.x + minimum_size.width,
        position.x + size.width
    );
    assert_eq!(
        minimum_position.y + minimum_size.height,
        position.y + size.height
    );
}
