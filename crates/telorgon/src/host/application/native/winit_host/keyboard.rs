use crate::input::{KeyEvent, Modifiers};
use crate::platform::winit::{
    KeyboardTextPolicy, KeyboardTranslationError, ViewRegistry, WinitKeyboardContext,
    WinitKeyboardInput, translate_keyboard_input, translate_modifiers_state,
};
use winit::keyboard::ModifiersState;
use winit::window::WindowId;

/// Winit supplies modifiers separately from key events; each managed view retains
/// that snapshot and passes it through the canonical platform adapter.
#[derive(Default)]
pub(super) struct NativeKeyboard {
    modifiers: Modifiers,
    alt_graph: bool,
    composing: bool,
}

impl NativeKeyboard {
    pub(super) fn modifiers_changed(&mut self, modifiers: ModifiersState) {
        self.modifiers = translate_modifiers_state(modifiers);
    }

    pub(super) fn modifiers(&self) -> Modifiers {
        if self.alt_graph {
            self.modifiers.union(Modifiers::ALT_GRAPH)
        } else {
            self.modifiers
        }
    }

    pub(super) fn set_composing(&mut self, composing: bool) {
        self.composing = composing;
    }

    pub(super) fn reset(&mut self) {
        self.modifiers = Modifiers::empty();
        self.alt_graph = false;
        self.composing = false;
    }

    pub(super) fn translate(
        &mut self,
        registry: &ViewRegistry,
        window: WindowId,
        input: WinitKeyboardInput<'_>,
    ) -> Result<KeyEvent, KeyboardTranslationError> {
        if matches!(
            input.logical_key,
            crate::platform::winit::WinitLogicalKey::Named(winit::keyboard::NamedKey::AltGraph)
        ) {
            self.alt_graph = input.state == winit::event::ElementState::Pressed;
        }
        translate_keyboard_input(
            registry,
            window,
            WinitKeyboardContext::new(
                self.modifiers(),
                if self.composing {
                    KeyboardTextPolicy::SuppressDuringImeComposition
                } else {
                    KeyboardTextPolicy::Preserve
                },
            ),
            input,
        )
        .map(|observation| observation.into_event())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{ButtonState, KeyLocation, LogicalKey, NamedKey, PhysicalKeyCode};
    use crate::platform::winit::WinitLogicalKey;
    use crate::services::clipboard::{ClipboardEditAction, ClipboardText};
    use std::num::NonZeroU16;
    use winit::event::ElementState;
    use winit::keyboard::{
        KeyCode, KeyLocation as NativeLocation, NamedKey as NativeNamedKey, PhysicalKey,
    };

    #[test]
    fn active_composition_suppresses_key_text_until_commit_or_reset() {
        let (registry, window) = registry();
        let mut keyboard = NativeKeyboard::default();
        keyboard.set_composing(true);
        let key = keyboard
            .translate(
                &registry,
                window,
                input(KeyCode::KeyE, WinitLogicalKey::Character("é"), Some("é")),
            )
            .unwrap();
        assert!(key.text.is_none());
        assert!(matches!(key.logical_key, LogicalKey::Character(_)));
        keyboard.set_composing(false);
        assert_eq!(
            keyboard
                .translate(
                    &registry,
                    window,
                    input(KeyCode::KeyE, WinitLogicalKey::Character("é"), Some("é"))
                )
                .unwrap()
                .text
                .unwrap()
                .as_str(),
            "é"
        );
        keyboard.set_composing(true);
        keyboard.reset();
        assert!(
            keyboard
                .translate(
                    &registry,
                    window,
                    input(KeyCode::KeyE, WinitLogicalKey::Character("e"), Some("e"))
                )
                .unwrap()
                .text
                .is_some()
        );
    }

    #[test]
    fn explicit_alt_graph_is_preserved_across_aggregate_modifier_updates_and_cleared_on_release() {
        let (registry, window) = registry();
        let mut keyboard = NativeKeyboard::default();
        keyboard
            .translate(
                &registry,
                window,
                input(
                    KeyCode::AltRight,
                    WinitLogicalKey::Named(NativeNamedKey::AltGraph),
                    None,
                ),
            )
            .unwrap();
        keyboard.modifiers_changed(ModifiersState::ALT | ModifiersState::CONTROL);
        let key = keyboard
            .translate(
                &registry,
                window,
                input(KeyCode::Digit2, WinitLogicalKey::Character("@"), Some("@")),
            )
            .unwrap();
        assert!(key.modifiers.contains(Modifiers::ALT_GRAPH));
        assert_eq!(key.text.unwrap().as_str(), "@");
        let mut release = input(
            KeyCode::AltRight,
            WinitLogicalKey::Named(NativeNamedKey::AltGraph),
            None,
        );
        release.state = ElementState::Released;
        keyboard.translate(&registry, window, release).unwrap();
        assert!(!keyboard.modifiers().contains(Modifiers::ALT_GRAPH));
        keyboard.reset();
        assert_eq!(keyboard.modifiers(), Modifiers::empty());
    }

    fn registry() -> (ViewRegistry, WindowId) {
        let window = WindowId::from(41);
        let mut registry = ViewRegistry::new(NonZeroU16::MIN).unwrap();
        registry.register(window).unwrap();
        (registry, window)
    }

    fn input<'a>(
        physical: KeyCode,
        logical_key: WinitLogicalKey<'a>,
        text: Option<&'a str>,
    ) -> WinitKeyboardInput<'a> {
        WinitKeyboardInput {
            physical_key: PhysicalKey::Code(physical),
            logical_key,
            text,
            location: NativeLocation::Standard,
            state: ElementState::Pressed,
            repeat: false,
            synthetic: false,
        }
    }

    #[test]
    fn native_locale_text_reaches_the_editor_with_complete_key_fields() {
        let (registry, window) = registry();
        let mut keyboard = NativeKeyboard::default();
        let mut native = input(KeyCode::KeyQ, WinitLogicalKey::Character("é"), Some("é"));
        native.repeat = true;
        native.synthetic = true;
        native.location = NativeLocation::Right;
        let key = keyboard.translate(&registry, window, native).unwrap();
        assert_eq!(key.physical_key.code(), Some(PhysicalKeyCode::KeyQ));
        assert!(matches!(&key.logical_key, LogicalKey::Character(value) if value.as_str() == "é"));
        assert_eq!(key.text.as_ref().unwrap().as_str(), "é");
        assert_eq!(key.location, KeyLocation::Right);
        assert_eq!(key.state, ButtonState::Pressed);
        assert!(key.repeat && key.synthetic);
        let mut editor = ClipboardText::default();
        assert_eq!(editor.key(&key), ClipboardEditAction::Changed);
        assert_eq!(editor.text, "é");
    }

    #[test]
    fn native_space_enter_and_navigation_remain_named_editing_keys() {
        let (registry, window) = registry();
        let mut keyboard = NativeKeyboard::default();
        let mut editor = ClipboardText::default();
        for (physical, native, neutral, text) in [
            (
                KeyCode::Space,
                NativeNamedKey::Space,
                NamedKey::Space,
                Some(" "),
            ),
            (
                KeyCode::Enter,
                NativeNamedKey::Enter,
                NamedKey::Enter,
                Some("\r"),
            ),
            (
                KeyCode::ArrowLeft,
                NativeNamedKey::ArrowLeft,
                NamedKey::ArrowLeft,
                None,
            ),
            (
                KeyCode::ArrowRight,
                NativeNamedKey::ArrowRight,
                NamedKey::ArrowRight,
                None,
            ),
            (
                KeyCode::Backspace,
                NativeNamedKey::Backspace,
                NamedKey::Backspace,
                None,
            ),
        ] {
            let key = keyboard
                .translate(
                    &registry,
                    window,
                    input(physical, WinitLogicalKey::Named(native), text),
                )
                .unwrap();
            assert_eq!(key.logical_key, LogicalKey::Named(neutral));
            editor.key(&key);
        }
        // Space inserts actual text, Enter's control text is filtered, and
        // Backspace still removes the inserted space after cursor navigation.
        assert!(editor.text.is_empty());
        assert_eq!(editor.cursor, 0);
    }

    #[test]
    fn modifier_snapshots_drive_selection_and_reset_on_view_loss() {
        let (registry, window) = registry();
        let mut keyboard = NativeKeyboard::default();
        keyboard.modifiers_changed(ModifiersState::CONTROL | ModifiersState::SHIFT);
        let select = keyboard
            .translate(
                &registry,
                window,
                input(KeyCode::KeyA, WinitLogicalKey::Character("a"), None),
            )
            .unwrap();
        assert!(select.modifiers.contains(Modifiers::CONTROL));
        assert!(select.modifiers.contains(Modifiers::SHIFT));
        let mut editor = ClipboardText::default();
        editor.text = "text".into();
        editor.cursor = 4;
        editor.anchor = 4;
        editor.key(&select);
        assert_eq!(editor.selection(), 0..4);
        keyboard.modifiers_changed(ModifiersState::SHIFT);
        let left = keyboard
            .translate(
                &registry,
                window,
                input(
                    KeyCode::ArrowLeft,
                    WinitLogicalKey::Named(NativeNamedKey::ArrowLeft),
                    None,
                ),
            )
            .unwrap();
        assert_eq!(left.modifiers, Modifiers::SHIFT);
        keyboard.reset();
        let insert = keyboard
            .translate(
                &registry,
                window,
                input(KeyCode::KeyC, WinitLogicalKey::Character("ç"), Some("ç")),
            )
            .unwrap();
        assert_eq!(insert.modifiers, Modifiers::empty());
        editor.key(&insert);
        assert_eq!(editor.text, "ç");
    }

    #[test]
    fn native_key_releases_preserve_state_without_inserting_text() {
        let (registry, window) = registry();
        let mut keyboard = NativeKeyboard::default();
        let mut native = input(KeyCode::KeyA, WinitLogicalKey::Character("a"), Some("a"));
        native.state = ElementState::Released;
        let key = keyboard.translate(&registry, window, native).unwrap();
        assert_eq!(key.state, ButtonState::Released);
        let mut editor = ClipboardText::default();
        assert_eq!(editor.key(&key), ClipboardEditAction::None);
        assert!(editor.text.is_empty());
    }
}
