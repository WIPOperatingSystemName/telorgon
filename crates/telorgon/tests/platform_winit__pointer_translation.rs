#![cfg(any(
    feature = "application-software",
    feature = "application-vulkan"
))]

use std::num::NonZeroU16;

use telorgon::core::{PointF, SizeF};
use telorgon::input::{
    ButtonState, MAX_PRESSED_POINTER_BUTTONS, Modifiers, PhysicalPointerPosition, PointerButton,
    PointerButtonSet, PointerDeviceId, PointerDeviceKind, PointerEventKind, PointerEventSource,
    PointerId, PointerInputEvent, PointerPosition, PointerStateSnapshot, ScrollMomentumPhase,
    ScrollPhase, ScrollPrecision, ScrollUnit,
};
use telorgon::platform::{
    DisplayProperties, MetricsCitation, PhysicalExtent, ScaleFactor, ViewMetrics, ViewState,
};
use telorgon::platform_winit::{
    PointerTranslationError, ViewRegistry, WinitPointerContext, WinitPointerFact,
    translate_pointer_event, translate_pointer_fact,
};
use winit::dpi::PhysicalPosition;
use winit::event::{
    DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::window::WindowId;

fn window(value: u64) -> WindowId {
    WindowId::from(value)
}

fn fixture() -> (ViewRegistry, WindowId, ViewState) {
    let mut registry = ViewRegistry::new(NonZeroU16::new(2).unwrap()).unwrap();
    let registration = registry.register(window(11)).unwrap();
    let metrics = ViewMetrics::new(
        PhysicalExtent::new(800, 600),
        ScaleFactor::new(2.0).unwrap(),
        DisplayProperties::default(),
    )
    .unwrap();
    let state = ViewState::with_metrics(registration.view, metrics);
    (registry, registration.window, state)
}

fn context(state: &ViewState, position: Option<PointerPosition>) -> WinitPointerContext {
    let snapshot = state.snapshot();
    WinitPointerContext::new(
        DeviceId::dummy(),
        snapshot.metrics().revision(),
        PointerStateSnapshot::new(PointerId::new(7), PointerDeviceKind::Mouse)
            .with_device_id(PointerDeviceId::from_raw(3, 2))
            .with_position(position)
            .with_primary_contact(true)
            .with_source(PointerEventSource::Native)
            .with_modifiers(Modifiers::SHIFT.union(Modifiers::CAPS_LOCK)),
    )
}

fn current_position() -> PointerPosition {
    PointerPosition::with_physical(
        PointF { x: 10.0, y: 15.0 },
        PhysicalPointerPosition::new(20.0, 30.0).unwrap(),
    )
    .unwrap()
}

#[test]
fn cursor_move_converts_with_exact_metrics_and_preserves_complete_identity_state() {
    let (registry, source_window, state) = fixture();
    let snapshot = state.snapshot();
    let context = context(&state, None);
    let observation = translate_pointer_fact(
        &registry,
        source_window,
        &snapshot,
        &context,
        WinitPointerFact::CursorMoved {
            device_id: DeviceId::dummy(),
            physical_position: PhysicalPosition::new(100.0, 50.0),
        },
    )
    .unwrap();

    assert_eq!(observation.source_window(), source_window);
    assert_eq!(observation.view(), snapshot.view());
    assert_eq!(
        observation.metrics_citation(),
        MetricsCitation::converted_using(snapshot.metrics().revision())
    );
    let PointerInputEvent::Pointer(event) = observation.event() else {
        panic!("cursor move must produce a pointer event");
    };
    assert_eq!(event.kind(), PointerEventKind::Moved);
    assert_eq!(event.state().pointer(), PointerId::new(7));
    assert_eq!(event.state().device_id(), PointerDeviceId::from_raw(3, 2));
    assert!(event.state().primary_contact());
    assert!(event.state().modifiers().contains(Modifiers::CAPS_LOCK));
    let position = event.state().position().unwrap();
    assert_eq!(position.view_logical(), PointF { x: 50.0, y: 25.0 });
    assert_eq!(position.physical().unwrap().x(), 100.0);
    assert_eq!(context.state().position(), None);
}

#[test]
fn enter_and_leave_preserve_explicit_optional_position_without_sentinels() {
    let (registry, source_window, state) = fixture();
    let snapshot = state.snapshot();
    let absent = context(&state, None);
    let entered = translate_pointer_fact(
        &registry,
        source_window,
        &snapshot,
        &absent,
        WinitPointerFact::CursorEntered {
            device_id: DeviceId::dummy(),
        },
    )
    .unwrap();
    assert_eq!(entered.metrics_citation(), MetricsCitation::NOT_CONVERTED);
    let PointerInputEvent::Pointer(entered) = entered.event() else {
        panic!("enter must produce a pointer event");
    };
    assert_eq!(entered.kind(), PointerEventKind::Entered);
    assert_eq!(entered.state().position(), None);

    let present = context(&state, Some(current_position()));
    let left = translate_pointer_fact(
        &registry,
        source_window,
        &snapshot,
        &present,
        WinitPointerFact::CursorLeft {
            device_id: DeviceId::dummy(),
        },
    )
    .unwrap();
    assert_eq!(
        left.metrics_citation().revision(),
        Some(snapshot.metrics().revision())
    );
    let PointerInputEvent::Pointer(left) = left.event() else {
        panic!("leave must produce a pointer event");
    };
    assert_eq!(left.kind(), PointerEventKind::Left);
    assert_eq!(left.state().position(), Some(current_position()));
}

#[test]
fn button_edges_update_complete_state_and_namespace_platform_other_codes() {
    let (registry, source_window, state) = fixture();
    let snapshot = state.snapshot();
    let context = context(&state, Some(current_position()));
    let pressed = translate_pointer_fact(
        &registry,
        source_window,
        &snapshot,
        &context,
        WinitPointerFact::MouseButton {
            device_id: DeviceId::dummy(),
            button: MouseButton::Other(1),
            state: ElementState::Pressed,
        },
    )
    .unwrap();
    let PointerInputEvent::Pointer(pressed) = pressed.event() else {
        panic!("mouse button must produce a pointer event");
    };
    let PointerEventKind::Button {
        button,
        state: edge,
    } = pressed.kind()
    else {
        panic!("mouse button must retain its edge");
    };
    assert_eq!(button.platform_other_code(), Some(1));
    assert_ne!(button, PointerButton::PRIMARY);
    assert_eq!(edge, ButtonState::Pressed);
    assert!(pressed.state().buttons().contains(button));

    let released_context = WinitPointerContext::new(
        DeviceId::dummy(),
        snapshot.metrics().revision(),
        pressed.state().clone(),
    );
    let released = translate_pointer_fact(
        &registry,
        source_window,
        &snapshot,
        &released_context,
        WinitPointerFact::MouseButton {
            device_id: DeviceId::dummy(),
            button: MouseButton::Other(1),
            state: ElementState::Released,
        },
    )
    .unwrap();
    let PointerInputEvent::Pointer(released) = released.event() else {
        panic!("mouse release must produce a pointer event");
    };
    assert!(!released.state().buttons().contains(button));
}

#[test]
fn button_state_bound_rejects_atomically_without_mutating_context() {
    let (registry, source_window, state) = fixture();
    let snapshot = state.snapshot();
    let full = PointerButtonSet::new(
        (1..=MAX_PRESSED_POINTER_BUTTONS).map(|value| PointerButton::new(value as u16)),
    )
    .unwrap();
    let state = PointerStateSnapshot::new(PointerId::PRIMARY, PointerDeviceKind::Mouse)
        .with_buttons(full.clone());
    let context = WinitPointerContext::new(DeviceId::dummy(), snapshot.metrics().revision(), state);
    assert!(matches!(
        translate_pointer_fact(
            &registry,
            source_window,
            &snapshot,
            &context,
            WinitPointerFact::MouseButton {
                device_id: DeviceId::dummy(),
                button: MouseButton::Other(99),
                state: ElementState::Pressed,
            },
        ),
        Err(PointerTranslationError::InvalidButtonState { .. })
    ));
    assert_eq!(context.state().buttons(), &full);
}

#[test]
fn wheel_preserves_axes_units_phase_precision_and_physical_source() {
    let (registry, source_window, state) = fixture();
    let snapshot = state.snapshot();
    let without_position = context(&state, None);
    let lines = translate_pointer_fact(
        &registry,
        source_window,
        &snapshot,
        &without_position,
        WinitPointerFact::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(1.25, -2.5),
            phase: TouchPhase::Moved,
        },
    )
    .unwrap();
    assert_eq!(lines.metrics_citation(), MetricsCitation::NOT_CONVERTED);
    let PointerInputEvent::Scroll(lines) = lines.event() else {
        panic!("wheel must produce a scroll event");
    };
    assert_eq!(lines.delta().unit(), ScrollUnit::Lines);
    assert_eq!((lines.delta().x(), lines.delta().y()), (1.25, -2.5));
    assert_eq!(lines.delta().physical_pixels(), None);
    assert_eq!(lines.phase(), ScrollPhase::Changed);
    assert_eq!(lines.momentum(), ScrollMomentumPhase::None);
    assert_eq!(lines.precision(), ScrollPrecision::Discrete);

    let pixels = translate_pointer_fact(
        &registry,
        source_window,
        &snapshot,
        &without_position,
        WinitPointerFact::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(12.0, -20.0)),
            phase: TouchPhase::Started,
        },
    )
    .unwrap();
    assert_eq!(
        pixels.metrics_citation().revision(),
        Some(snapshot.metrics().revision())
    );
    let PointerInputEvent::Scroll(pixels) = pixels.event() else {
        panic!("pixel wheel must produce a scroll event");
    };
    assert_eq!(pixels.delta().unit(), ScrollUnit::Pixels);
    assert_eq!((pixels.delta().x(), pixels.delta().y()), (6.0, -10.0));
    assert_eq!(pixels.delta().physical_pixels().unwrap().x(), 12.0);
    assert_eq!(pixels.phase(), ScrollPhase::Began);
    assert_eq!(pixels.precision(), ScrollPrecision::Precise);
}

#[test]
fn stale_window_snapshot_metrics_device_kind_and_nonfinite_values_fail_typed() {
    let (mut registry, source_window, mut state) = fixture();
    let first_snapshot = state.snapshot();
    let stale_context = context(&state, None);
    let view = first_snapshot.view();
    registry
        .replace_window(view, source_window, window(12))
        .unwrap();
    let invalid_move = WinitPointerFact::CursorMoved {
        device_id: DeviceId::dummy(),
        physical_position: PhysicalPosition::new(f64::NAN, 2.0),
    };
    assert_eq!(
        translate_pointer_fact(
            &registry,
            source_window,
            &first_snapshot,
            &stale_context,
            invalid_move,
        ),
        Err(PointerTranslationError::WindowUnavailable {
            window: source_window,
        })
    );

    state
        .observe_metrics(
            ViewMetrics::new(
                PhysicalExtent::new(800, 600),
                ScaleFactor::new(1.0).unwrap(),
                DisplayProperties::default(),
            )
            .unwrap(),
        )
        .unwrap();
    let current_snapshot = state.snapshot();
    assert!(matches!(
        translate_pointer_fact(
            &registry,
            window(12),
            &current_snapshot,
            &stale_context,
            WinitPointerFact::CursorEntered {
                device_id: DeviceId::dummy(),
            },
        ),
        Err(PointerTranslationError::StateMetricsMismatch { .. })
    ));

    let current_context = context(&state, None);
    assert!(matches!(
        translate_pointer_fact(
            &registry,
            window(12),
            &current_snapshot,
            &current_context,
            invalid_move,
        ),
        Err(PointerTranslationError::InvalidPosition { .. })
    ));

    let pen_context = WinitPointerContext::new(
        DeviceId::dummy(),
        current_snapshot.metrics().revision(),
        PointerStateSnapshot::new(PointerId::PRIMARY, PointerDeviceKind::Pen),
    );
    assert_eq!(
        translate_pointer_fact(
            &registry,
            window(12),
            &current_snapshot,
            &pen_context,
            WinitPointerFact::CursorEntered {
                device_id: DeviceId::dummy(),
            },
        ),
        Err(PointerTranslationError::ContextDeviceKind {
            view,
            observed: PointerDeviceKind::Pen,
        })
    );
}

#[test]
fn borrowed_event_selection_covers_supported_events_and_ignores_unrelated_ones() {
    let (registry, source_window, state) = fixture();
    let snapshot = state.snapshot();
    let context = context(&state, None);
    let device_id = DeviceId::dummy();
    let supported = [
        WindowEvent::CursorEntered { device_id },
        WindowEvent::CursorLeft { device_id },
        WindowEvent::CursorMoved {
            device_id,
            position: PhysicalPosition::new(2.0, 4.0),
        },
        WindowEvent::MouseInput {
            device_id,
            state: ElementState::Pressed,
            button: MouseButton::Left,
        },
        WindowEvent::MouseWheel {
            device_id,
            delta: MouseScrollDelta::LineDelta(0.0, 1.0),
            phase: TouchPhase::Ended,
        },
    ];
    for event in supported {
        assert!(
            translate_pointer_event(&registry, source_window, &snapshot, &context, &event)
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(
        translate_pointer_event(
            &registry,
            source_window,
            &snapshot,
            &context,
            &WindowEvent::RedrawRequested,
        )
        .unwrap(),
        None
    );
}

#[test]
fn snapshot_view_mismatch_is_rejected_before_translation() {
    let (mut registry, source_window, state) = fixture();
    let other = registry.register(window(22)).unwrap();
    let other_state = ViewState::new(other.view);
    let context = context(&state, None);
    assert_eq!(
        translate_pointer_fact(
            &registry,
            source_window,
            &other_state.snapshot(),
            &context,
            WinitPointerFact::CursorEntered {
                device_id: DeviceId::dummy(),
            },
        ),
        Err(PointerTranslationError::SnapshotViewMismatch {
            window: source_window,
            registered_view: state.view(),
            snapshot_view: other.view,
        })
    );
}

#[test]
fn logical_extent_is_not_used_to_clamp_outside_cursor_positions() {
    let (registry, source_window, state) = fixture();
    let snapshot = state.snapshot();
    assert_eq!(
        snapshot.metrics().metrics().logical_extent(),
        SizeF {
            width: 400.0,
            height: 300.0,
        }
    );
    let observation = translate_pointer_fact(
        &registry,
        source_window,
        &snapshot,
        &context(&state, None),
        WinitPointerFact::CursorMoved {
            device_id: DeviceId::dummy(),
            physical_position: PhysicalPosition::new(-20.0, 700.0),
        },
    )
    .unwrap();
    let PointerInputEvent::Pointer(event) = observation.event() else {
        panic!("cursor move must produce a pointer event");
    };
    assert_eq!(
        event.state().position().unwrap().view_logical(),
        PointF { x: -10.0, y: 350.0 }
    );
}
