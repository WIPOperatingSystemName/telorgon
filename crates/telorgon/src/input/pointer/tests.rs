use super::*;

#[test]
fn device_identity_is_generation_aware() {
    let first = PointerDeviceId::from_raw(7, 1).unwrap();
    let replacement = PointerDeviceId::from_raw(7, 2).unwrap();
    assert_ne!(first, replacement);
    assert_eq!(PointerDeviceId::from_raw(0, 1), None);
    assert_eq!(PointerDeviceId::from_raw(1, 0), None);
}

#[test]
fn complete_button_state_is_canonical_bounded_and_identified() {
    let buttons = PointerButtonSet::new([
        PointerButton::FORWARD,
        PointerButton::PRIMARY,
        PointerButton::MIDDLE,
    ])
    .unwrap();
    assert_eq!(
        buttons.iter().collect::<Vec<_>>(),
        vec![
            PointerButton::PRIMARY,
            PointerButton::MIDDLE,
            PointerButton::FORWARD,
        ]
    );
    assert_eq!(
        PointerButtonSet::new([PointerButton::PRIMARY, PointerButton::PRIMARY]),
        Err(PointerButtonSetError::DuplicateButton(
            PointerButton::PRIMARY
        ))
    );
    assert_eq!(
        PointerButtonSet::new([PointerButton::UNIDENTIFIED]),
        Err(PointerButtonSetError::UnidentifiedButton)
    );
    assert!(matches!(
        PointerButtonSet::new(
            (1..=MAX_PRESSED_POINTER_BUTTONS + 1).map(|value| PointerButton::new(value as u16))
        ),
        Err(PointerButtonSetError::TooManyButtons { .. })
    ));
    assert_ne!(
        PointerButton::from_platform_other(1),
        PointerButton::PRIMARY
    );
    assert_eq!(
        PointerButton::from_platform_other(u16::MAX).platform_other_code(),
        Some(u16::MAX)
    );
}

#[test]
fn coordinates_and_tool_properties_are_validated_without_clamping() {
    let logical = PointF { x: 12.5, y: 7.25 };
    let physical = PhysicalPointerPosition::new(25.0, 14.5).unwrap();
    let position = PointerPosition::with_physical(logical, physical).unwrap();
    assert_eq!(position.view_logical(), logical);
    assert_eq!(position.physical(), Some(physical));
    assert_eq!(
        PointerPosition::logical(PointF {
            x: f32::NAN,
            y: 0.0,
        }),
        Err(PointerCoordinateError::NonFiniteLogicalPosition)
    );

    assert_eq!(PointerPressure::new(0.5).unwrap().get(), 0.5);
    assert_eq!(PointerTilt::new(-90.0, 90.0).unwrap().y_degrees(), 90.0);
    assert_eq!(PointerTwist::new(359.0).unwrap().degrees(), 359.0);
    assert_eq!(
        PointerPressure::new(1.1),
        Err(PointerPropertyError::InvalidPressure { observed: 1.1 })
    );
    assert!(
        PointerContactGeometry::new(SizeF {
            width: 0.0,
            height: 2.0,
        })
        .is_err()
    );
}

#[test]
fn button_edges_and_cancellation_agree_with_complete_state() {
    let pointer = PointerId::new(4);
    let pressed = PointerButtonSet::new([PointerButton::PRIMARY]).unwrap();
    let down = PointerEvent::new(
        PointerEventKind::Button {
            button: PointerButton::PRIMARY,
            state: ButtonState::Pressed,
        },
        PointerStateSnapshot::new(pointer, PointerDeviceKind::Mouse)
            .with_buttons(pressed.clone()),
    )
    .unwrap();
    assert!(down.state().buttons().contains(PointerButton::PRIMARY));

    assert_eq!(
        PointerEvent::new(
            PointerEventKind::Button {
                button: PointerButton::PRIMARY,
                state: ButtonState::Released,
            },
            PointerStateSnapshot::new(pointer, PointerDeviceKind::Mouse)
                .with_buttons(pressed.clone()),
        ),
        Err(PointerEventError::ReleasedButtonStillPressed {
            button: PointerButton::PRIMARY,
        })
    );
    assert_eq!(
        PointerEvent::new(
            PointerEventKind::Cancelled(PointerCancelReason::FocusLost),
            PointerStateSnapshot::new(pointer, PointerDeviceKind::Mouse).with_buttons(pressed),
        ),
        Err(PointerEventError::CancellationRetainsButtons)
    );
}

#[test]
fn leave_and_cancel_support_absent_positions_without_sentinels() {
    let state = PointerStateSnapshot::new(PointerId::PRIMARY, PointerDeviceKind::Mouse);
    let left = PointerEvent::new(PointerEventKind::Left, state.clone()).unwrap();
    let cancelled = PointerEvent::new(
        PointerEventKind::Cancelled(PointerCancelReason::ViewSuspended),
        state,
    )
    .unwrap();
    assert_eq!(left.state().position(), None);
    assert_eq!(cancelled.state().position(), None);
}

#[test]
fn scroll_retains_units_axes_phase_momentum_precision_and_physical_source() {
    let physical = PhysicalScrollDelta::new(2.0, -4.0).unwrap();
    let delta = ScrollDelta::new(1.0, -2.0, ScrollUnit::Pixels)
        .unwrap()
        .with_physical_pixels(physical)
        .unwrap();
    let event = ScrollEvent::new(PointerId::PRIMARY, PointerDeviceKind::Mouse, delta)
        .with_phase(ScrollPhase::Changed)
        .with_momentum(ScrollMomentumPhase::Began)
        .with_precision(ScrollPrecision::Precise)
        .with_source(PointerEventSource::SynthesizedOther)
        .with_modifiers(Modifiers::CONTROL);
    assert_eq!(event.delta().unit(), ScrollUnit::Pixels);
    assert_eq!(event.delta().physical_pixels(), Some(physical));
    assert_eq!(event.phase(), ScrollPhase::Changed);
    assert_eq!(event.momentum(), ScrollMomentumPhase::Began);
    assert_eq!(event.precision(), ScrollPrecision::Precise);
    assert!(event.source().is_synthesized());
    assert!(event.modifiers().contains(Modifiers::CONTROL));

    assert_eq!(
        ScrollDelta::new(1.0, 2.0, ScrollUnit::Lines)
            .unwrap()
            .with_physical_pixels(physical),
        Err(ScrollValueError::PhysicalDeltaForNonPixelUnit {
            unit: ScrollUnit::Lines,
        })
    );
}
