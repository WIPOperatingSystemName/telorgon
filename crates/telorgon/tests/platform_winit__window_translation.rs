#![cfg(any(feature = "application-software", feature = "application-vulkan"))]

use std::num::NonZeroU16;

use telorgon::platform::{
    CloseRequestReason, ForcedDestructionPhase, PhysicalExtent, ViewId, ViewLifetime, ViewState,
};
use telorgon::platform_winit::{
    ViewRegistry, WindowTranslationError, WinitWindowFact, WinitWindowObservationKind,
    translate_window_event, translate_window_fact,
};
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::window::WindowId;

fn window(value: u64) -> WindowId {
    WindowId::from(value)
}

fn registry(maximum: u16) -> ViewRegistry {
    ViewRegistry::new(NonZeroU16::new(maximum).unwrap()).unwrap()
}

fn state(view: ViewId) -> ViewState {
    let mut state = ViewState::new(view);
    state.observe_lifetime(ViewLifetime::Live).unwrap();
    state
}

#[test]
fn resize_preserves_physical_units_and_explicit_zero_extent() {
    let mut registry = registry(1);
    let registration = registry.register(window(11)).unwrap();
    let state = state(registration.view);
    let event = WindowEvent::Resized(PhysicalSize::new(0, 720));

    let observation =
        translate_window_event(&registry, registration.window, &state.snapshot(), &event)
            .unwrap()
            .unwrap();

    assert_eq!(observation.source_window(), window(11));
    assert_eq!(observation.view(), registration.view);
    let WinitWindowObservationKind::Resized { physical_extent } = observation.kind() else {
        panic!("resize event must produce a resize observation");
    };
    assert_eq!(physical_extent, PhysicalExtent::new(0, 720));
    assert!(!physical_extent.is_renderable());
}

#[test]
fn scale_is_validated_after_preserving_the_original_winit_observation() {
    let mut registry = registry(1);
    let registration = registry.register(window(11)).unwrap();
    let state = state(registration.view);
    let snapshot = state.snapshot();

    let observation = translate_window_fact(
        &registry,
        registration.window,
        &snapshot,
        WinitWindowFact::ScaleFactorChanged { scale_factor: 1.5 },
    )
    .unwrap();
    let WinitWindowObservationKind::ScaleFactorChanged { scale_factor } = observation.kind() else {
        panic!("scale fact must produce a scale observation");
    };
    assert_eq!(scale_factor.get(), 1.5);

    for observed in [0.0, f64::INFINITY, f64::NAN, f64::MIN_POSITIVE] {
        assert!(matches!(
            translate_window_fact(
                &registry,
                registration.window,
                &snapshot,
                WinitWindowFact::ScaleFactorChanged {
                    scale_factor: observed,
                },
            ),
            Err(WindowTranslationError::InvalidScaleFactor { view, observed: rejected })
                if view == registration.view
                    && (rejected == observed || rejected.is_nan() && observed.is_nan())
        ));
    }
}

#[test]
fn focus_occlusion_and_unrelated_events_remain_distinct_typed_facts() {
    let mut registry = registry(1);
    let registration = registry.register(window(11)).unwrap();
    let state = state(registration.view);
    let snapshot = state.snapshot();

    let focused = translate_window_event(
        &registry,
        registration.window,
        &snapshot,
        &WindowEvent::Focused(false),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        focused.kind(),
        WinitWindowObservationKind::FocusChanged { focused: false }
    );

    let occluded = translate_window_event(
        &registry,
        registration.window,
        &snapshot,
        &WindowEvent::Occluded(true),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        occluded.kind(),
        WinitWindowObservationKind::OcclusionChanged { occluded: true }
    );

    assert_eq!(
        translate_window_event(
            &registry,
            registration.window,
            &snapshot,
            &WindowEvent::RedrawRequested,
        )
        .unwrap(),
        None
    );
}

#[test]
fn cancellable_close_and_forced_destruction_preserve_exact_snapshot_revision() {
    let mut registry = registry(1);
    let registration = registry.register(window(11)).unwrap();
    let state = state(registration.view);
    let snapshot = state.snapshot();

    let close = translate_window_event(
        &registry,
        registration.window,
        &snapshot,
        &WindowEvent::CloseRequested,
    )
    .unwrap()
    .unwrap();
    let WinitWindowObservationKind::CloseRequested(request) = close.kind() else {
        panic!("close event must remain cancellable");
    };
    assert_eq!(request.view(), snapshot.view());
    assert_eq!(request.observed_revision(), snapshot.revision());
    assert_eq!(request.reason(), CloseRequestReason::User);

    let destroyed = translate_window_event(
        &registry,
        registration.window,
        &snapshot,
        &WindowEvent::Destroyed,
    )
    .unwrap()
    .unwrap();
    let WinitWindowObservationKind::Destroyed(destruction) = destroyed.kind() else {
        panic!("destroyed event must remain unanswerable");
    };
    assert_eq!(destruction.view(), snapshot.view());
    assert_eq!(destruction.observed_revision(), snapshot.revision());
    assert_eq!(destruction.phase(), ForcedDestructionPhase::Destroyed);
}

#[test]
fn stale_native_identity_and_mismatched_snapshot_are_rejected_before_translation() {
    let mut registry = registry(2);
    let first = registry.register(window(11)).unwrap();
    let second = registry.register(window(22)).unwrap();
    let first_state = state(first.view);
    let second_state = state(second.view);
    registry
        .replace_window(first.view, first.window, window(12))
        .unwrap();

    assert_eq!(
        translate_window_fact(
            &registry,
            first.window,
            &first_state.snapshot(),
            WinitWindowFact::CloseRequested,
        ),
        Err(WindowTranslationError::WindowUnavailable {
            window: first.window,
        })
    );
    assert_eq!(
        translate_window_fact(
            &registry,
            window(12),
            &second_state.snapshot(),
            WinitWindowFact::Destroyed,
        ),
        Err(WindowTranslationError::SnapshotViewMismatch {
            window: window(12),
            registered_view: first.view,
            snapshot_view: second.view,
        })
    );
}
