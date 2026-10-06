#![cfg(any(feature = "application-software", feature = "application-vulkan"))]

use std::num::NonZeroU16;

use telorgon::input::{
    ButtonState, KeyLocation, LogicalKey, MAX_KEY_TEXT_BYTES, Modifiers, NamedKey, PhysicalKeyCode,
};
use telorgon::platform_winit::{
    KeyboardTextField, KeyboardTextPolicy, KeyboardTranslationError, ViewRegistry,
    WinitKeyboardContext, WinitKeyboardInput, WinitLogicalKey, translate_keyboard_event,
    translate_keyboard_input, translate_modifiers_event, translate_modifiers_state,
};
use winit::event::{ElementState, Modifiers as WinitModifiers, WindowEvent};
use winit::keyboard::{
    KeyCode as WinitKeyCode, KeyLocation as WinitKeyLocation,
    ModifiersState as WinitModifiersState, NamedKey as WinitNamedKey, NativeKeyCode,
    PhysicalKey as WinitPhysicalKey,
};
use winit::window::WindowId;

fn window(value: u64) -> WindowId {
    WindowId::from(value)
}

fn registry() -> (ViewRegistry, WindowId) {
    let mut registry = ViewRegistry::new(NonZeroU16::new(2).unwrap()).unwrap();
    let registration = registry.register(window(11)).unwrap();
    (registry, registration.window)
}

fn context(text_policy: KeyboardTextPolicy) -> WinitKeyboardContext {
    WinitKeyboardContext::new(
        Modifiers::SHIFT
            .union(Modifiers::ALT_GRAPH)
            .union(Modifiers::CAPS_LOCK),
        text_policy,
    )
}

#[test]
fn complete_key_fields_translate_into_one_current_view_observation() {
    let (registry, source_window) = registry();
    let input = WinitKeyboardInput {
        physical_key: WinitPhysicalKey::Code(WinitKeyCode::NumpadEnter),
        logical_key: WinitLogicalKey::Named(WinitNamedKey::Enter),
        text: Some("\r"),
        location: WinitKeyLocation::Numpad,
        state: ElementState::Pressed,
        repeat: true,
        synthetic: true,
    };

    let observation = translate_keyboard_input(
        &registry,
        source_window,
        context(KeyboardTextPolicy::Preserve),
        input,
    )
    .unwrap();
    let event = observation.event();

    assert_eq!(observation.source_window(), source_window);
    assert_eq!(
        observation.view(),
        registry.view_for_window(source_window).unwrap()
    );
    assert_eq!(
        event.physical_key.code(),
        Some(PhysicalKeyCode::NumpadEnter)
    );
    assert_eq!(event.logical_key, LogicalKey::Named(NamedKey::Enter));
    assert_eq!(event.text.as_ref().unwrap().as_str(), "\r");
    assert_eq!(event.location, KeyLocation::Numpad);
    assert_eq!(event.state, ButtonState::Pressed);
    assert!(event.repeat);
    assert!(event.synthetic);
    assert!(event.modifiers.contains(Modifiers::ALT_GRAPH));
    assert!(event.modifiers.contains(Modifiers::CAPS_LOCK));
}

#[test]
fn character_dead_released_and_unidentified_meanings_remain_distinct() {
    let (registry, source_window) = registry();
    let character = translate_keyboard_input(
        &registry,
        source_window,
        context(KeyboardTextPolicy::Preserve),
        WinitKeyboardInput {
            physical_key: WinitPhysicalKey::Code(WinitKeyCode::KeyE),
            logical_key: WinitLogicalKey::Character("é"),
            text: Some("é"),
            location: WinitKeyLocation::Standard,
            state: ElementState::Released,
            repeat: false,
            synthetic: false,
        },
    )
    .unwrap();
    assert!(matches!(
        &character.event().logical_key,
        LogicalKey::Character(text) if text.as_str() == "é"
    ));
    assert_eq!(character.event().state, ButtonState::Released);

    let dead = translate_keyboard_input(
        &registry,
        source_window,
        context(KeyboardTextPolicy::Preserve),
        WinitKeyboardInput {
            physical_key: WinitPhysicalKey::Code(WinitKeyCode::Quote),
            logical_key: WinitLogicalKey::Dead(Some('\'')),
            text: None,
            location: WinitKeyLocation::Standard,
            state: ElementState::Pressed,
            repeat: false,
            synthetic: false,
        },
    )
    .unwrap();
    assert_eq!(dead.event().logical_key, LogicalKey::Dead(Some('\'')));

    let unidentified = translate_keyboard_input(
        &registry,
        source_window,
        context(KeyboardTextPolicy::Preserve),
        WinitKeyboardInput {
            physical_key: WinitPhysicalKey::Unidentified(NativeKeyCode::Windows(0x1234)),
            logical_key: WinitLogicalKey::Unidentified,
            text: None,
            location: WinitKeyLocation::Standard,
            state: ElementState::Pressed,
            repeat: false,
            synthetic: false,
        },
    )
    .unwrap();
    assert!(unidentified.event().physical_key.is_unidentified());
    assert_eq!(unidentified.event().logical_key, LogicalKey::Unidentified);
}

#[test]
fn active_ime_suppresses_only_produced_text() {
    let (registry, source_window) = registry();
    let oversized_produced = "x".repeat(MAX_KEY_TEXT_BYTES + 1);
    let observation = translate_keyboard_input(
        &registry,
        source_window,
        context(KeyboardTextPolicy::SuppressDuringImeComposition),
        WinitKeyboardInput {
            physical_key: WinitPhysicalKey::Code(WinitKeyCode::KeyA),
            logical_key: WinitLogicalKey::Character("a"),
            text: Some(&oversized_produced),
            location: WinitKeyLocation::Standard,
            state: ElementState::Pressed,
            repeat: false,
            synthetic: false,
        },
    )
    .unwrap();

    assert!(matches!(
        &observation.event().logical_key,
        LogicalKey::Character(text) if text.as_str() == "a"
    ));
    assert_eq!(observation.event().text, None);
}

#[test]
fn logical_and_produced_text_bounds_fail_with_redacted_typed_fields() {
    let (registry, source_window) = registry();
    let view = registry.view_for_window(source_window).unwrap();
    let private = "secret".repeat(MAX_KEY_TEXT_BYTES);

    let logical_error = translate_keyboard_input(
        &registry,
        source_window,
        context(KeyboardTextPolicy::Preserve),
        WinitKeyboardInput {
            physical_key: WinitPhysicalKey::Code(WinitKeyCode::KeyA),
            logical_key: WinitLogicalKey::Character(&private),
            text: None,
            location: WinitKeyLocation::Standard,
            state: ElementState::Pressed,
            repeat: false,
            synthetic: false,
        },
    )
    .unwrap_err();
    assert_eq!(
        logical_error,
        KeyboardTranslationError::TextTooLong {
            view,
            field: KeyboardTextField::LogicalCharacter,
            byte_len: private.len(),
            maximum_bytes: MAX_KEY_TEXT_BYTES,
        }
    );
    assert!(!format!("{logical_error:?}").contains("secret"));

    let produced_error = translate_keyboard_input(
        &registry,
        source_window,
        context(KeyboardTextPolicy::Preserve),
        WinitKeyboardInput {
            physical_key: WinitPhysicalKey::Code(WinitKeyCode::Enter),
            logical_key: WinitLogicalKey::Named(WinitNamedKey::Enter),
            text: Some(&private),
            location: WinitKeyLocation::Standard,
            state: ElementState::Pressed,
            repeat: false,
            synthetic: false,
        },
    )
    .unwrap_err();
    assert!(matches!(
        produced_error,
        KeyboardTranslationError::TextTooLong {
            field: KeyboardTextField::ProducedText,
            ..
        }
    ));
}

#[test]
fn stale_window_rejects_before_text_translation_and_unrelated_events_are_ignored() {
    let (mut registry, source_window) = registry();
    let view = registry.view_for_window(source_window).unwrap();
    registry
        .replace_window(view, source_window, window(12))
        .unwrap();
    let oversized = "x".repeat(MAX_KEY_TEXT_BYTES + 1);

    assert_eq!(
        translate_keyboard_input(
            &registry,
            source_window,
            context(KeyboardTextPolicy::Preserve),
            WinitKeyboardInput {
                physical_key: WinitPhysicalKey::Code(WinitKeyCode::KeyA),
                logical_key: WinitLogicalKey::Character(&oversized),
                text: None,
                location: WinitKeyLocation::Standard,
                state: ElementState::Pressed,
                repeat: false,
                synthetic: false,
            },
        ),
        Err(KeyboardTranslationError::WindowUnavailable {
            window: source_window,
        })
    );
    assert_eq!(
        translate_keyboard_event(
            &registry,
            window(12),
            context(KeyboardTextPolicy::Preserve),
            &WindowEvent::RedrawRequested,
        )
        .unwrap(),
        None
    );
}

#[test]
fn modifier_events_map_only_the_state_winit_actually_reports() {
    let state = WinitModifiersState::SHIFT
        | WinitModifiersState::CONTROL
        | WinitModifiersState::ALT
        | WinitModifiersState::SUPER;
    let expected = Modifiers::SHIFT
        .union(Modifiers::CONTROL)
        .union(Modifiers::ALT)
        .union(Modifiers::SUPER);
    assert_eq!(translate_modifiers_state(state), expected);
    assert_eq!(
        translate_modifiers_event(&WindowEvent::ModifiersChanged(WinitModifiers::from(state))),
        Some(expected)
    );
    assert_eq!(
        translate_modifiers_event(&WindowEvent::RedrawRequested),
        None
    );
}
