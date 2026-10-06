#![cfg(any(feature = "application-software", feature = "application-vulkan"))]

use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use telorgon::platform::{
    MonotonicInstant, PendingHostFacts, PostTurnSchedule, RemainingWork, ViewId,
};
use telorgon::platform_winit::{
    ViewRegistry, WinitClockObservation, WinitScheduleError, WinitWakeIntent, interpret_schedule,
};
use winit::event_loop::ControlFlow;
use winit::window::WindowId;

fn window(value: u64) -> WindowId {
    WindowId::from(value)
}

fn registry(maximum: u16) -> ViewRegistry {
    ViewRegistry::new(NonZeroU16::new(maximum).unwrap()).unwrap()
}

fn schedule(
    work: RemainingWork,
    redraw_views: &[ViewId],
    deadline: Option<MonotonicInstant>,
    pending: PendingHostFacts,
) -> PostTurnSchedule {
    PostTurnSchedule::new(work, redraw_views, deadline, pending).unwrap()
}

#[test]
fn idle_and_future_deadline_plans_use_only_the_supplied_observation() {
    let registry = registry(1);
    let native_now = Instant::now();
    let observation = WinitClockObservation::new(MonotonicInstant::from_nanos(100), native_now);

    let idle = interpret_schedule(
        &schedule(RemainingWork::NONE, &[], None, PendingHostFacts::NONE),
        &registry,
        observation,
    )
    .unwrap();
    assert_eq!(idle.control_flow(), ControlFlow::Wait);
    assert_eq!(idle.wake_intent(), WinitWakeIntent::NoWake);

    let timed = interpret_schedule(
        &schedule(
            RemainingWork::NONE,
            &[],
            Some(MonotonicInstant::from_nanos(150)),
            PendingHostFacts::NONE,
        ),
        &registry,
        observation,
    )
    .unwrap();
    assert_eq!(
        timed.control_flow(),
        ControlFlow::WaitUntil(native_now + Duration::from_nanos(50))
    );
    assert_eq!(timed.wake_intent(), WinitWakeIntent::NoWake);
}

#[test]
fn due_deadline_and_unfinished_work_request_one_wake_without_polling() {
    let registry = registry(1);
    let observation = WinitClockObservation::new(MonotonicInstant::from_nanos(100), Instant::now());

    let due = interpret_schedule(
        &schedule(
            RemainingWork::NONE,
            &[],
            Some(MonotonicInstant::from_nanos(100)),
            PendingHostFacts::NONE,
        ),
        &registry,
        observation,
    )
    .unwrap();
    assert_eq!(due.control_flow(), ControlFlow::Wait);
    assert_eq!(due.wake_intent(), WinitWakeIntent::RequestWake);
    assert!(due.wake_intent().requires_request());

    let unfinished = interpret_schedule(
        &schedule(
            RemainingWork::new(true, false, false, false),
            &[],
            Some(MonotonicInstant::from_nanos(150)),
            PendingHostFacts::NONE,
        ),
        &registry,
        observation,
    )
    .unwrap();
    assert_eq!(unfinished.control_flow(), ControlFlow::Wait);
    assert_eq!(unfinished.wake_intent(), WinitWakeIntent::RequestWake);
    assert_ne!(unfinished.control_flow(), ControlFlow::Poll);
}

#[test]
fn an_existing_pending_wake_is_not_requested_twice() {
    let registry = registry(1);
    let plan = interpret_schedule(
        &schedule(
            RemainingWork::new(false, true, false, false),
            &[],
            None,
            PendingHostFacts::new(true, true),
        ),
        &registry,
        WinitClockObservation::new(MonotonicInstant::ZERO, Instant::now()),
    )
    .unwrap();

    assert_eq!(plan.control_flow(), ControlFlow::Wait);
    assert_eq!(plan.wake_intent(), WinitWakeIntent::WakeAlreadyPending);
    assert!(plan.wake_intent().is_already_pending());
    assert!(!plan.wake_intent().requires_request());
}

#[test]
fn redraws_resolve_current_multi_view_windows_after_native_replacement() {
    let mut registry = registry(2);
    let first = registry.register(window(11)).unwrap();
    let second = registry.register(window(22)).unwrap();
    registry
        .replace_window(first.view, first.window, window(12))
        .unwrap();

    let plan = interpret_schedule(
        &schedule(
            RemainingWork::NONE,
            &[second.view, first.view],
            None,
            PendingHostFacts::NONE,
        ),
        &registry,
        WinitClockObservation::new(MonotonicInstant::ZERO, Instant::now()),
    )
    .unwrap();

    assert_eq!(plan.redraw_targets().len(), 2);
    assert_eq!(plan.redraw_targets()[0].view(), first.view);
    assert_eq!(plan.redraw_targets()[0].window(), window(12));
    assert_eq!(plan.redraw_targets()[1].view(), second.view);
    assert_eq!(plan.redraw_targets()[1].window(), window(22));
    assert_eq!(plan.control_flow(), ControlFlow::Wait);
}

#[test]
fn stale_redraw_generation_cannot_target_a_reused_registry_slot() {
    let mut registry = registry(1);
    let stale = registry.register(window(11)).unwrap();
    registry.retire(stale.view, stale.window).unwrap();
    let current = registry.register(window(22)).unwrap();
    assert_eq!(current.view.slot(), stale.view.slot());
    assert_ne!(current.view, stale.view);

    let error = interpret_schedule(
        &schedule(
            RemainingWork::NONE,
            &[stale.view],
            None,
            PendingHostFacts::NONE,
        ),
        &registry,
        WinitClockObservation::new(MonotonicInstant::ZERO, Instant::now()),
    )
    .unwrap_err();

    assert_eq!(
        error,
        WinitScheduleError::RedrawViewUnavailable { view: stale.view }
    );
    assert_eq!(registry.window_for_view(current.view), Some(window(22)));
}
