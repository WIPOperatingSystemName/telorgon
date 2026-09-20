use super::*;

#[test]
fn maps_primary_mouse_buttons_to_stable_codes() {
    assert_eq!(mouse_button(MouseButton::Left), PointerButton::PRIMARY);
    assert_eq!(mouse_button(MouseButton::Right), PointerButton::SECONDARY);
    assert_eq!(mouse_button(MouseButton::Middle), PointerButton::MIDDLE);
}

#[test]
fn interactive_resize_bursts_apply_only_the_latest_extent_per_frame() {
    let now = Instant::now();
    let mut pending = PendingResize::default();
    let _ = pending.queue(SizeI {
        width: 640,
        height: 480,
    });
    let _ = pending.queue(SizeI {
        width: 960,
        height: 600,
    });
    let _ = pending.queue(SizeI {
        width: 1280,
        height: 720,
    });
    assert_eq!(
        pending.take(now),
        Some(SizeI {
            width: 1280,
            height: 720,
        })
    );
    assert_eq!(pending.take(now), None);
}

#[test]
fn live_resize_frames_are_refresh_paced_without_dropping_the_final_extent() {
    let started = Instant::now();
    let interval = Duration::from_millis(16);
    let mut pending = PendingResize::default();
    let _ = pending.queue(SizeI {
        width: 800,
        height: 600,
    });
    assert!(pending.is_due(started, interval));
    assert!(pending.take(started).is_some());
    let _ = pending.queue(SizeI {
        width: 900,
        height: 700,
    });
    assert!(!pending.is_due(started + Duration::from_millis(8), interval));
    assert_eq!(pending.next_due_at(interval), Some(started + interval));
    let _ = pending.queue(SizeI {
        width: 1000,
        height: 800,
    });
    assert!(pending.is_due(started + interval, interval));
    assert_eq!(
        pending.take(started + interval),
        Some(SizeI {
            width: 1000,
            height: 800,
        })
    );
    assert_eq!(pending.next_due_at(interval), None);
}

#[test]
fn native_resize_timer_does_not_requeue_an_already_applied_extent() {
    let now = Instant::now();
    let extent = SizeI {
        width: 1000,
        height: 700,
    };
    let mut pending = PendingResize::default();
    assert!(pending.queue(extent));
    assert_eq!(pending.take(now), Some(extent));
    assert!(!pending.queue(extent));
    assert!(!pending.is_pending());

    let changed = SizeI {
        width: 1001,
        height: 700,
    };
    assert!(pending.queue(changed));
    assert!(pending.is_pending());
}

#[test]
fn post_native_barrier_can_repeat_an_expansion_extent() {
    let now = Instant::now();
    let extent = SizeI {
        width: 1200,
        height: 800,
    };
    let mut pending = PendingResize::default();
    assert!(pending.queue(extent));
    assert_eq!(pending.take(now), Some(extent));
    assert!(!pending.queue(extent));
    assert!(pending.queue_for_barrier(extent));
    assert_eq!(pending.take(now), Some(extent));
}

#[test]
fn monitor_refresh_rate_defines_the_managed_frame_interval() {
    assert_eq!(
        frame_interval_for_refresh_rate(Some(60_000)),
        Duration::from_nanos(16_666_667)
    );
    assert_eq!(
        frame_interval_for_refresh_rate(Some(120_000)),
        Duration::from_nanos(8_333_334)
    );
    assert_eq!(
        frame_interval_for_refresh_rate(Some(144_000)),
        Duration::from_nanos(6_944_445)
    );
    assert_eq!(
        frame_interval_for_refresh_rate(None),
        frame_interval_for_refresh_rate(Some(0))
    );
}

#[test]
fn frame_pacer_waits_for_refresh_and_never_issues_catch_up_frames() {
    let started = Instant::now();
    let interval = Duration::from_millis(16);
    let mut pacer = FramePacer::default();
    assert_eq!(pacer.throttle_deadline(started, interval), None);

    pacer.frame_started(started);
    assert_eq!(
        pacer.throttle_deadline(started + Duration::from_millis(8), interval),
        Some(started + interval)
    );
    assert_eq!(pacer.throttle_deadline(started + interval, interval), None);

    let late = started + Duration::from_millis(100);
    pacer.frame_started(late);
    assert_eq!(
        pacer.throttle_deadline(late + Duration::from_millis(1), interval),
        Some(late + interval)
    );
}

#[test]
fn only_active_windows_resize_frames_are_applied_synchronously() {
    assert!(!should_apply_resize_synchronously(false, true));
    assert!(!should_apply_resize_synchronously(true, false));
    assert_eq!(
        should_apply_resize_synchronously(true, true),
        cfg!(target_os = "windows")
    );
}

#[test]
fn windows_live_resize_rejects_the_parallel_or_stale_winit_stream() {
    let current = SizeI {
        width: 1001,
        height: 700,
    };
    let stale = SizeI {
        width: 980,
        height: 700,
    };
    assert_eq!(
        should_accept_winit_resize(true, current, current),
        !cfg!(target_os = "windows")
    );
    assert_eq!(
        should_accept_winit_resize(false, stale, current),
        !cfg!(target_os = "windows")
    );
    assert!(should_accept_winit_resize(false, current, current));
}

#[test]
fn exact_wm_size_barrier_accepts_only_responsive_resize_commits() {
    let update = ResizeUpdate {
        generation: 4,
        metrics_revision: 17,
        phase: super::super::resize::ResizeInteractionPhase::Updating,
        extent: SizeI {
            width: 1001,
            height: 700,
        },
        surface: SurfaceResizeAction::Commit,
    };
    assert_eq!(
        resize_revision_to_synchronize(RedrawSource::SynchronousResizeBarrier, update),
        Some(17)
    );
    assert_eq!(
        resize_revision_to_synchronize(RedrawSource::SynchronousResize, update),
        None
    );
    assert_eq!(
        resize_revision_to_synchronize(
            RedrawSource::SynchronousResizeBarrier,
            ResizeUpdate {
                surface: SurfaceResizeAction::KeepCurrent,
                ..update
            }
        ),
        None
    );
}

#[test]
fn redraw_demand_merges_reasons_and_deduplicates_native_requests() {
    let mut demand = RedrawDemand::default();
    assert!(demand.mark(RedrawReason::Input));
    assert!(!demand.mark(RedrawReason::Input));
    assert!(demand.mark(RedrawReason::Animation));
    assert!(demand.has_demand());
    assert!(demand.queue_native_request());
    assert!(!demand.queue_native_request());

    assert!(demand.native_callback_started());
    assert!(!demand.native_callback_started());
    assert!(demand.queue_native_request());
    let reasons = demand.take_reasons();
    assert!(!reasons.is_empty());
    assert!(!reasons.force_present());
    assert!(!demand.has_demand());
}

#[test]
fn resize_release_retires_a_stale_native_redraw_request() {
    let mut demand = RedrawDemand::default();
    demand.mark(RedrawReason::Resize);
    assert!(demand.queue_native_request());

    // Live-resize frames run synchronously and consume demand without receiving the native
    // callback that normally retires this request.
    let live_resize_reasons = demand.take_reasons();
    assert!(live_resize_reasons.force_present());
    demand.mark(RedrawReason::Resize);
    assert!(!demand.queue_native_request());

    // WM_EXITSIZEMOVE retires the stale request before its mandatory synchronous frame.
    demand.cancel_native_request();
    let release_reasons = demand.take_reasons();
    assert!(release_reasons.force_present());
    demand.mark(RedrawReason::Animation);
    assert!(demand.queue_native_request());
}

#[test]
fn ordinary_runtime_reasons_share_pacing_while_lifecycle_presentation_bypasses_it() {
    for reason in [
        RedrawReason::Runtime,
        RedrawReason::Input,
        RedrawReason::Command,
        RedrawReason::ExternalWake,
        RedrawReason::Animation,
        RedrawReason::Timer,
        RedrawReason::PointerMove,
    ] {
        let mut demand = RedrawDemand::default();
        demand.mark(reason);
        assert!(!demand.requires_immediate_presentation());
    }

    let mut demand = RedrawDemand::default();
    demand.mark(RedrawReason::Expose);
    assert!(demand.requires_immediate_presentation());
}

#[test]
fn expose_and_recovery_reasons_force_presentation() {
    let mut reasons = RedrawReasons::default();
    reasons.insert(RedrawReason::Expose);
    assert!(reasons.force_present());

    let mut reasons = RedrawReasons::default();
    reasons.insert(RedrawReason::Recovery);
    assert!(reasons.force_present());
}

#[test]
fn pointer_move_only_frames_allow_the_generic_runtime_reason_but_no_other_trigger() {
    let mut reasons = RedrawReasons::default();
    reasons.insert(RedrawReason::PointerMove);
    reasons.insert(RedrawReason::Runtime);
    assert!(reasons.pointer_move_only());

    reasons.insert(RedrawReason::Animation);
    assert!(!reasons.pointer_move_only());

    let mut reasons = RedrawReasons::default();
    reasons.insert(RedrawReason::Input);
    assert!(!reasons.pointer_move_only());
}

#[test]
fn resize_deadline_only_shortens_waiting_control_flow() {
    let now = Instant::now();
    let later = now + Duration::from_millis(20);
    assert_eq!(
        earlier_wait_deadline(ControlFlow::Poll, now),
        ControlFlow::Poll
    );
    assert_eq!(
        earlier_wait_deadline(ControlFlow::Wait, later),
        ControlFlow::WaitUntil(later)
    );
    assert_eq!(
        earlier_wait_deadline(ControlFlow::WaitUntil(later), now),
        ControlFlow::WaitUntil(now)
    );
    assert_eq!(
        earlier_wait_deadline(ControlFlow::WaitUntil(now), later),
        ControlFlow::WaitUntil(now)
    );
}
