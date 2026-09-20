use super::*;

const POINTER: PointerId = PointerId::new(7);
const ORIGIN: PointF = PointF { x: 10.0, y: 20.0 };

fn down() -> GestureInput {
    GestureInput::PointerDown {
        pointer: POINTER,
        button: PointerButton::PRIMARY,
        position: ORIGIN,
    }
}

fn moved(x: f32, y: f32) -> GestureInput {
    GestureInput::PointerMoved {
        pointer: POINTER,
        position: PointF { x, y },
    }
}

fn up(x: f32, y: f32) -> GestureInput {
    GestureInput::PointerUp {
        pointer: POINTER,
        button: PointerButton::PRIMARY,
        position: PointF { x, y },
    }
}

#[test]
fn arena_eager_winner_resolves_on_close_and_notifies_every_member_once() {
    let mut arena = GestureArena::new();
    arena.add(POINTER, "tap").unwrap();
    arena.add(POINTER, "drag").unwrap();
    assert!(arena.accept(POINTER, "drag").unwrap().is_empty());
    assert_eq!(
        arena.close(POINTER).unwrap(),
        vec![
            GestureArenaDecision::Lost {
                participant: "tap",
                reason: GestureArenaLossReason::Winner("drag"),
            },
            GestureArenaDecision::Won {
                participant: "drag",
                reason: GestureArenaWinReason::Accepted,
            },
        ]
    );
    assert!(!arena.is_active(POINTER));
    assert_eq!(arena.diagnostics().wins, 1);
    assert_eq!(arena.diagnostics().losses, 1);
}

#[test]
fn arena_last_nonrejecting_member_wins_and_duplicates_are_rejected_atomically() {
    let mut arena = GestureArena::new();
    arena.add(POINTER, 1_u32).unwrap();
    assert_eq!(
        arena.add(POINTER, 1),
        Err(GestureArenaError::DuplicateParticipant {
            pointer: POINTER,
            participant: 1,
        })
    );
    arena.add(POINTER, 2).unwrap();
    arena.close(POINTER).unwrap();
    assert_eq!(
        arena.reject(POINTER, 1).unwrap(),
        vec![
            GestureArenaDecision::Lost {
                participant: 1,
                reason: GestureArenaLossReason::SelfRejected,
            },
            GestureArenaDecision::Won {
                participant: 2,
                reason: GestureArenaWinReason::LastRemaining,
            },
        ]
    );
}

#[test]
fn held_sweep_waits_for_release_and_preserves_canonical_first_winner() {
    let mut arena = GestureArena::new();
    arena.add(POINTER, 4_u32).unwrap();
    arena.add(POINTER, 5).unwrap();
    arena.hold(POINTER).unwrap();
    arena.close(POINTER).unwrap();
    assert!(arena.sweep(POINTER).unwrap().is_empty());
    assert_eq!(
        arena.release(POINTER).unwrap(),
        vec![
            GestureArenaDecision::Lost {
                participant: 5,
                reason: GestureArenaLossReason::Winner(4),
            },
            GestureArenaDecision::Won {
                participant: 4,
                reason: GestureArenaWinReason::Swept,
            },
        ]
    );
}

#[test]
fn arena_cancellation_rejects_all_remaining_members() {
    let mut arena = GestureArena::new();
    arena.add(POINTER, 1_u32).unwrap();
    arena.add(POINTER, 2).unwrap();
    assert_eq!(
        arena
            .cancel(POINTER, GestureCancelReason::PointerCancelled)
            .unwrap(),
        vec![
            GestureArenaDecision::Lost {
                participant: 1,
                reason: GestureArenaLossReason::Cancelled(
                    GestureCancelReason::PointerCancelled
                ),
            },
            GestureArenaDecision::Lost {
                participant: 2,
                reason: GestureArenaLossReason::Cancelled(
                    GestureCancelReason::PointerCancelled
                ),
            },
        ]
    );
}

#[test]
fn tap_claims_on_up_and_recognizes_only_after_winning() {
    let mut tap = TapRecognizer::new(8.0, true).unwrap();
    tap.handle(down()).unwrap();
    assert_eq!(
        tap.handle(up(11.0, 21.0)).unwrap().arena,
        GestureArenaRequest::Accept(POINTER)
    );
    assert_eq!(tap.diagnostics().recognized, 0);
    assert_eq!(
        tap.handle(GestureInput::ArenaWon { pointer: POINTER })
            .unwrap()
            .transition,
        GestureTransition::TapRecognized {
            pointer: POINTER,
            position: PointF { x: 11.0, y: 21.0 },
        }
    );
}

#[test]
fn early_tap_arena_win_still_waits_for_pointer_up() {
    let mut tap = TapRecognizer::new(8.0, true).unwrap();
    tap.handle(down()).unwrap();
    assert_eq!(
        tap.handle(GestureInput::ArenaWon { pointer: POINTER })
            .unwrap(),
        GestureOutcome::ignored()
    );
    assert_eq!(tap.state(), GestureRecognizerState::Accepted);
    assert!(matches!(
        tap.handle(up(11.0, 21.0)).unwrap().transition,
        GestureTransition::TapRecognized { .. }
    ));
}

#[test]
fn tap_rejects_after_slop_and_cannot_emit_a_later_tap() {
    let mut tap = TapRecognizer::new(4.0, true).unwrap();
    tap.handle(down()).unwrap();
    let cancelled = tap.handle(moved(20.0, 20.0)).unwrap();
    assert_eq!(cancelled.arena, GestureArenaRequest::Reject(POINTER));
    assert!(matches!(
        cancelled.transition,
        GestureTransition::Cancelled {
            reason: GestureCancelReason::SlopExceeded,
            ..
        }
    ));
    assert_eq!(
        tap.handle(up(20.0, 20.0)).unwrap(),
        GestureOutcome::ignored()
    );
}

#[test]
fn tap_up_alone_still_checks_slop() {
    let mut tap = TapRecognizer::new(4.0, true).unwrap();
    tap.handle(down()).unwrap();
    let cancelled = tap.handle(up(20.0, 20.0)).unwrap();
    assert!(matches!(
        cancelled.transition,
        GestureTransition::Cancelled {
            reason: GestureCancelReason::SlopExceeded,
            ..
        }
    ));
    assert_eq!(cancelled.arena, GestureArenaRequest::Reject(POINTER));
}

#[test]
fn long_press_deadline_is_host_owned_generation_safe_and_arena_gated() {
    let mut long = LongPressRecognizer::new(6.0, Duration::from_millis(500), true).unwrap();
    let started = long.handle(down()).unwrap();
    let GestureDeadlineRequest::Schedule { id, after } = started.deadline else {
        panic!("expected deadline request");
    };
    assert_eq!(after, Duration::from_millis(500));
    assert_eq!(
        long.handle(GestureInput::DeadlineElapsed(
            GestureDeadlineId::from_raw(POINTER, id.generation() + 1).unwrap(),
        ))
        .unwrap(),
        GestureOutcome::ignored()
    );
    assert_eq!(long.diagnostics().stale_deadlines, 1);
    assert_eq!(
        long.handle(GestureInput::DeadlineElapsed(id))
            .unwrap()
            .arena,
        GestureArenaRequest::Accept(POINTER)
    );
    assert_eq!(long.diagnostics().recognized, 0);
    assert!(matches!(
        long.handle(GestureInput::ArenaWon { pointer: POINTER })
            .unwrap()
            .transition,
        GestureTransition::LongPressStarted { .. }
    ));
}

#[test]
fn early_arena_win_does_not_bypass_long_press_deadline() {
    let mut long = LongPressRecognizer::new(6.0, Duration::from_millis(500), true).unwrap();
    let started = long.handle(down()).unwrap();
    let GestureDeadlineRequest::Schedule { id, .. } = started.deadline else {
        panic!("expected deadline request");
    };
    assert_eq!(
        long.handle(GestureInput::ArenaWon { pointer: POINTER })
            .unwrap(),
        GestureOutcome::ignored()
    );
    assert_eq!(long.state(), GestureRecognizerState::Possible);
    assert!(matches!(
        long.handle(GestureInput::DeadlineElapsed(id))
            .unwrap()
            .transition,
        GestureTransition::LongPressStarted { .. }
    ));
}

#[test]
fn recognized_long_press_reports_update_and_end_without_a_second_claim() {
    let mut long = LongPressRecognizer::new(6.0, Duration::from_millis(500), true).unwrap();
    let started = long.handle(down()).unwrap();
    let GestureDeadlineRequest::Schedule { id, .. } = started.deadline else {
        panic!("expected deadline request");
    };
    long.handle(GestureInput::ArenaWon { pointer: POINTER })
        .unwrap();
    long.handle(GestureInput::DeadlineElapsed(id)).unwrap();
    let update = long.handle(moved(12.0, 23.0)).unwrap();
    assert_eq!(update.arena, GestureArenaRequest::None);
    assert!(matches!(
        update.transition,
        GestureTransition::LongPressUpdated {
            delta: GestureDelta { x: 2.0, y: 3.0 },
            total: GestureDelta { x: 2.0, y: 3.0 },
            ..
        }
    ));
    assert!(matches!(
        long.handle(up(14.0, 25.0)).unwrap().transition,
        GestureTransition::LongPressEnded {
            total: GestureDelta { x: 4.0, y: 5.0 },
            ..
        }
    ));
}

#[test]
fn releasing_before_long_press_cancels_deadline_and_rejects_arena() {
    let mut long = LongPressRecognizer::new(6.0, Duration::from_millis(500), true).unwrap();
    let started = long.handle(down()).unwrap();
    let GestureDeadlineRequest::Schedule { id, .. } = started.deadline else {
        panic!("expected deadline request");
    };
    let cancelled = long.handle(up(10.0, 20.0)).unwrap();
    assert_eq!(cancelled.deadline, GestureDeadlineRequest::Cancel(id));
    assert_eq!(cancelled.arena, GestureArenaRequest::Reject(POINTER));
}

#[test]
fn drag_claims_after_axis_slop_then_reports_begin_update_end() {
    let mut drag = DragRecognizer::new(DragAxis::Horizontal, 5.0, true).unwrap();
    drag.handle(down()).unwrap();
    assert_eq!(
        drag.handle(moved(13.0, 40.0)).unwrap(),
        GestureOutcome::ignored()
    );
    assert_eq!(
        drag.handle(moved(16.0, 40.0)).unwrap().arena,
        GestureArenaRequest::Accept(POINTER)
    );
    assert!(matches!(
        drag.handle(GestureInput::ArenaWon { pointer: POINTER })
            .unwrap()
            .transition,
        GestureTransition::DragStarted {
            total: GestureDelta { x: 6.0, y: 20.0 },
            ..
        }
    ));
    assert!(matches!(
        drag.handle(moved(18.0, 43.0)).unwrap().transition,
        GestureTransition::DragUpdated {
            delta: GestureDelta { x: 2.0, y: 3.0 },
            ..
        }
    ));
    assert!(matches!(
        drag.handle(up(20.0, 45.0)).unwrap().transition,
        GestureTransition::DragEnded {
            total: GestureDelta { x: 10.0, y: 25.0 },
            ..
        }
    ));
}

#[test]
fn drag_loss_and_active_cancellation_never_emit_end() {
    let mut possible = DragRecognizer::new(DragAxis::Both, 2.0, true).unwrap();
    possible.handle(down()).unwrap();
    possible.handle(moved(13.0, 20.0)).unwrap();
    assert!(matches!(
        possible
            .handle(GestureInput::ArenaLost { pointer: POINTER })
            .unwrap()
            .transition,
        GestureTransition::Cancelled {
            reason: GestureCancelReason::ArenaLost,
            ..
        }
    ));

    let mut active = DragRecognizer::new(DragAxis::Both, 2.0, true).unwrap();
    active.handle(down()).unwrap();
    active
        .handle(GestureInput::ArenaWon { pointer: POINTER })
        .unwrap();
    active.handle(moved(13.0, 20.0)).unwrap();
    assert!(matches!(
        active
            .handle(GestureInput::PointerCancelled { pointer: POINTER })
            .unwrap()
            .transition,
        GestureTransition::Cancelled {
            reason: GestureCancelReason::PointerCancelled,
            ..
        }
    ));
}

#[test]
fn disable_cancels_and_unmount_is_terminal_for_every_recognizer() {
    let mut tap = TapRecognizer::new(2.0, true).unwrap();
    tap.handle(down()).unwrap();
    assert!(matches!(
        tap.handle(GestureInput::SetEnabled(false))
            .unwrap()
            .transition,
        GestureTransition::Cancelled {
            reason: GestureCancelReason::Disabled,
            ..
        }
    ));
    tap.handle(GestureInput::Unmount).unwrap();
    assert_eq!(tap.state(), GestureRecognizerState::Dead);
    tap.handle(GestureInput::ViewDeactivated).unwrap();
    assert_eq!(tap.state(), GestureRecognizerState::Dead);
    assert_eq!(tap.handle(down()).unwrap(), GestureOutcome::ignored());

    let mut long = LongPressRecognizer::new(2.0, Duration::from_millis(500), true).unwrap();
    long.handle(GestureInput::Unmount).unwrap();
    assert_eq!(long.state(), GestureRecognizerState::Dead);

    let mut drag = DragRecognizer::new(DragAxis::Both, 2.0, true).unwrap();
    drag.handle(GestureInput::Unmount).unwrap();
    assert_eq!(drag.state(), GestureRecognizerState::Dead);
}

#[test]
fn view_and_capture_loss_cancel_without_recognition() {
    let mut long = LongPressRecognizer::new(2.0, Duration::from_millis(500), true).unwrap();
    let started = long.handle(down()).unwrap();
    let GestureDeadlineRequest::Schedule { id, .. } = started.deadline else {
        panic!("expected deadline request");
    };
    let cancelled = long.handle(GestureInput::ViewDeactivated).unwrap();
    assert_eq!(cancelled.deadline, GestureDeadlineRequest::Cancel(id));
    assert_eq!(cancelled.arena, GestureArenaRequest::Reject(POINTER));
    assert!(matches!(
        cancelled.transition,
        GestureTransition::Cancelled {
            reason: GestureCancelReason::ViewDeactivated,
            ..
        }
    ));

    let mut drag = DragRecognizer::new(DragAxis::Both, 2.0, true).unwrap();
    drag.handle(down()).unwrap();
    let cancelled = drag
        .handle(GestureInput::PointerCaptureLost { pointer: POINTER })
        .unwrap();
    assert_eq!(cancelled.arena, GestureArenaRequest::None);
    assert!(matches!(
        cancelled.transition,
        GestureTransition::Cancelled {
            reason: GestureCancelReason::CaptureLost,
            ..
        }
    ));
}

#[test]
fn invalid_configuration_and_positions_are_rejected_without_starting() {
    assert!(matches!(
        TapRecognizer::new(f32::NAN, true),
        Err(GestureRecognizerError::InvalidSlop(value)) if value.is_nan()
    ));
    let mut drag = DragRecognizer::new(DragAxis::Both, 2.0, true).unwrap();
    assert!(matches!(
        drag.handle(GestureInput::PointerDown {
            pointer: POINTER,
            button: PointerButton::PRIMARY,
            position: PointF {
                x: f32::INFINITY,
                y: 0.0,
            },
        }),
        Err(GestureRecognizerError::NonFinitePosition(_))
    ));
    assert_eq!(drag.state(), GestureRecognizerState::Idle);
}
