//! Named-function and explicitly authorized desktop-action compositor shortcuts.

use super::{ShellKeyAction, ShellKeyEvent};

/// A layout-resolved XKB symbol, not a physical key position.
///
/// Named letters (`A`–`Z`) match either ASCII case; express Shift on [`KeyChord`].
/// The `ascii()` and `from_keysym()` constructors retain exact, case-sensitive matching.
/// Use [`Self::from_keysym`] for non-ASCII symbols and additional named keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutKey {
    keysym: u32,
    fold_ascii_case: bool,
}

#[allow(non_upper_case_globals)]
impl ShortcutKey {
    pub const A: Self = Self::letter(b'a');
    pub const B: Self = Self::letter(b'b');
    pub const C: Self = Self::letter(b'c');
    pub const D: Self = Self::letter(b'd');
    pub const E: Self = Self::letter(b'e');
    pub const F: Self = Self::letter(b'f');
    pub const G: Self = Self::letter(b'g');
    pub const H: Self = Self::letter(b'h');
    pub const I: Self = Self::letter(b'i');
    pub const J: Self = Self::letter(b'j');
    pub const K: Self = Self::letter(b'k');
    pub const L: Self = Self::letter(b'l');
    pub const M: Self = Self::letter(b'm');
    pub const N: Self = Self::letter(b'n');
    pub const O: Self = Self::letter(b'o');
    pub const P: Self = Self::letter(b'p');
    pub const Q: Self = Self::letter(b'q');
    pub const R: Self = Self::letter(b'r');
    pub const S: Self = Self::letter(b's');
    pub const T: Self = Self::letter(b't');
    pub const U: Self = Self::letter(b'u');
    pub const V: Self = Self::letter(b'v');
    pub const W: Self = Self::letter(b'w');
    pub const X: Self = Self::letter(b'x');
    pub const Y: Self = Self::letter(b'y');
    pub const Z: Self = Self::letter(b'z');

    /// Digit symbols are layout-resolved, not number-row positions.
    /// Shifted punctuation requires its own symbol binding.
    pub const Digit0: Self = Self::ascii('0');
    pub const Digit1: Self = Self::ascii('1');
    pub const Digit2: Self = Self::ascii('2');
    pub const Digit3: Self = Self::ascii('3');
    pub const Digit4: Self = Self::ascii('4');
    pub const Digit5: Self = Self::ascii('5');
    pub const Digit6: Self = Self::ascii('6');
    pub const Digit7: Self = Self::ascii('7');
    pub const Digit8: Self = Self::ascii('8');
    pub const Digit9: Self = Self::ascii('9');

    pub const F1: Self = Self::from_keysym(0xffbe);
    pub const F2: Self = Self::from_keysym(0xffbf);
    pub const F3: Self = Self::from_keysym(0xffc0);
    pub const F4: Self = Self::from_keysym(0xffc1);
    pub const F5: Self = Self::from_keysym(0xffc2);
    pub const F6: Self = Self::from_keysym(0xffc3);
    pub const F7: Self = Self::from_keysym(0xffc4);
    pub const F8: Self = Self::from_keysym(0xffc5);
    pub const F9: Self = Self::from_keysym(0xffc6);
    pub const F10: Self = Self::from_keysym(0xffc7);
    pub const F11: Self = Self::from_keysym(0xffc8);
    pub const F12: Self = Self::from_keysym(0xffc9);

    pub const AudioVolumeDown: Self = Self::from_keysym(0x1008ff11);
    pub const AudioVolumeMute: Self = Self::from_keysym(0x1008ff12);
    pub const AudioVolumeUp: Self = Self::from_keysym(0x1008ff13);
    pub const MicrophoneMute: Self = Self::from_keysym(0x1008ffb2);

    pub const Space: Self = Self::from_keysym(0x20);
    pub const Enter: Self = Self::from_keysym(0xff0d);
    pub const Escape: Self = Self::from_keysym(0xff1b);
    pub const Tab: Self = Self::from_keysym(0xff09);
    pub const Backspace: Self = Self::from_keysym(0xff08);

    /// Creates a key from an XKB keysym value.
    pub const fn from_keysym(keysym: u32) -> Self {
        Self {
            keysym,
            fold_ascii_case: false,
        }
    }

    /// Creates a printable ASCII symbol.
    ///
    /// # Panics
    /// Panics if `character` is outside the printable ASCII range (space through tilde).
    pub const fn ascii(character: char) -> Self {
        assert!(
            character >= ' ' && character <= '~',
            "shortcut character must be printable ASCII"
        );
        Self::from_keysym(character as u32)
    }

    const fn letter(lowercase: u8) -> Self {
        Self {
            keysym: lowercase as u32,
            fold_ascii_case: true,
        }
    }

    fn matches(self, keysym: u32) -> bool {
        let symbol = if self.fold_ascii_case && (u32::from('A')..=u32::from('Z')).contains(&keysym)
        {
            keysym + 32
        } else {
            keysym
        };
        self.keysym == symbol
    }

    fn overlaps(self, other: Self) -> bool {
        self.matches(other.keysym) || other.matches(self.keysym)
    }
}

/// A shortcut key and exact Control/Shift/Alt/Super modifier combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyChord {
    key: ShortcutKey,
    control: bool,
    shift: bool,
    alt: bool,
    logo: bool,
}

impl KeyChord {
    pub const fn new(key: ShortcutKey) -> Self {
        Self {
            key,
            control: false,
            shift: false,
            alt: false,
            logo: false,
        }
    }

    pub const fn control(mut self) -> Self {
        self.control = true;
        self
    }

    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    /// Requires the Super/Windows modifier (the desktop event's `logo` modifier).
    pub const fn super_key(mut self) -> Self {
        self.logo = true;
        self
    }

    fn overlaps(self, other: Self) -> bool {
        self.key.overlaps(other.key)
            && self.control == other.control
            && self.shift == other.shift
            && self.alt == other.alt
            && self.logo == other.logo
    }

    fn matches(self, event: ShellKeyEvent) -> bool {
        self.key.matches(event.keysym)
            && self.control == event.control
            && self.shift == event.shift
            && self.alt == event.alt
            && self.logo == event.logo
    }
}

/// Compositor shortcuts backed by ordinary `fn()` pointers or opt-in desktop audio actions.
///
/// Matched keys run once per fresh press and consume that key's press, repeats, and release.
/// Unmatched keys forward normally. The desktop host disables shortcuts during session lock.
/// Functions run on the host thread and must remain short and nonblocking.
/// Bind [`crate::request_exit`] directly, or call it from a handler, to quit cleanly.
///
/// ```
/// use telorgon::app::{Compositor, KeyBindings, KeyChord, ShortcutKey};
///
/// fn open_launcher() { /* Request your application's launcher. */ }
/// fn open_terminal() { /* Request your application's terminal. */ }
///
/// let shortcuts = KeyBindings::new()
///     .bind(KeyChord::new(ShortcutKey::Space).super_key(), open_launcher)
///     .bind(KeyChord::new(ShortcutKey::T).control(), open_terminal);
/// let compositor = Compositor::new().keybindings(shortcuts);
/// ```
#[derive(Clone, Debug, Default)]
pub struct KeyBindings {
    bindings: Vec<(KeyChord, ShortcutHandler)>,
}

#[derive(Clone)]
enum ShortcutHandler {
    #[cfg(all(target_os = "linux", feature = "desktop-audio-linux"))]
    Mixer { handle: super::audio_mixer::AudioMixerHandle, action: crate::services::audio::AudioSystemAction, on_error: fn(crate::integrations::pipewire::MediaError) },
    Function(fn()),
    #[cfg(all(target_os = "linux", feature = "desktop-audio-linux"))]
    Audio {
        handle: super::desktop_audio::DesktopAudioHandle,
        action: crate::services::audio::AudioSystemAction,
        on_error: fn(crate::integrations::pipewire::MediaError),
    },
}
impl std::fmt::Debug for ShortcutHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Function(_) => f.write_str("Function"),
            #[cfg(all(target_os = "linux", feature = "desktop-audio-linux"))]
            Self::Mixer { action, .. } => f.debug_tuple("Mixer").field(action).finish(),
            #[cfg(all(target_os = "linux", feature = "desktop-audio-linux"))]
            Self::Audio { action, .. } => f.debug_tuple("Audio").field(action).finish(),
        }
    }
}
impl ShortcutHandler {
    fn invoke(&self) {
        match self {
            Self::Function(handler) => handler(),
            #[cfg(all(target_os = "linux", feature = "desktop-audio-linux"))]
            Self::Mixer { handle, action, on_error } => {
                if let Err(error) = handle.execute(super::audio_mixer::MixerAction::System(*action)) { on_error(error); }
            },
            #[cfg(all(target_os = "linux", feature = "desktop-audio-linux"))]
            Self::Audio {
                handle,
                action,
                on_error,
            } => {
                if let Err(error) = handle.execute(*action) {
                    on_error(error);
                }
            }
        }
    }
}

impl KeyBindings {
    pub const fn new() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }

    /// Adds a shortcut backed by a named function.
    ///
    /// # Panics
    /// Panics if a chord overlaps an existing binding, including named letters paired
    /// with exact lowercase or uppercase symbols using the same modifiers.
    pub fn bind(mut self, chord: KeyChord, handler: fn()) -> Self {
        assert!(
            !self
                .bindings
                .iter()
                .any(|(existing, _)| existing.overlaps(chord)),
            "duplicate compositor shortcut: {chord:?}"
        );
        self.bindings
            .push((chord, ShortcutHandler::Function(handler)));
        self
    }

    /// Bind media keys to the reconnecting mixer shared with shell widgets.
    #[cfg(all(target_os = "linux", feature = "desktop-audio-linux"))]
    pub fn mixer_media_keys(mut self, handle: super::audio_mixer::AudioMixerHandle, step_ui: f32,
        on_error: fn(crate::integrations::pipewire::MediaError)) -> Result<Self, crate::integrations::pipewire::MediaError> {
        use crate::services::audio::{AudioControlTarget as Target, AudioSystemAction as Action, Amplification};
        if !step_ui.is_finite() || step_ui <= 0.0 || step_ui > 1.0 { return Err(crate::integrations::pipewire::MediaError::InvalidArgument("audio key volume step")); }
        for (key, action) in [
            (ShortcutKey::AudioVolumeDown, Action::AdjustVolume { target: Target::DefaultOutput, delta_ui: -step_ui, amplification: Amplification::Forbid }),
            (ShortcutKey::AudioVolumeUp, Action::AdjustVolume { target: Target::DefaultOutput, delta_ui: step_ui, amplification: Amplification::Forbid }),
            (ShortcutKey::AudioVolumeMute, Action::ToggleMute { target: Target::DefaultOutput }),
            (ShortcutKey::MicrophoneMute, Action::ToggleMute { target: Target::DefaultInput }),
        ] {
            let chord = KeyChord::new(key);
            assert!(!self.bindings.iter().any(|(existing, _)| existing.overlaps(chord)), "duplicate compositor shortcut: {chord:?}");
            self.bindings.push((chord, ShortcutHandler::Mixer { handle: handle.clone(), action, on_error }));
        }
        Ok(self)
    }

    /// Bind an explicit audio capability; construction never creates a connection or sends
    /// a request. Matched presses remain consumed even if admission fails, preventing a
    /// second handler from applying them. Admission errors call on_error on the host thread;
    /// native completion/errors remain visible in the shared DesktopAudio snapshot.
    /// Keep on_error short and nonblocking. Duplicate chords panic, as with bind().
    #[cfg(all(target_os = "linux", feature = "desktop-audio-linux"))]
    pub fn bind_audio(
        mut self,
        chord: KeyChord,
        handle: super::desktop_audio::DesktopAudioHandle,
        action: crate::services::audio::AudioSystemAction,
        on_error: fn(crate::integrations::pipewire::MediaError),
    ) -> Self {
        assert!(
            !self
                .bindings
                .iter()
                .any(|(existing, _)| existing.overlaps(chord)),
            "duplicate compositor shortcut: {chord:?}"
        );
        self.bindings.push((
            chord,
            ShortcutHandler::Audio {
                handle,
                action,
                on_error,
            },
        ));
        self
    }

    /// Opt into the four unmodified XF86 output volume/mute and microphone-mute keys.
    /// Each fresh press uses the widget service queue; held-key repeats remain suppressed
    /// by the shell's shortcut contract. Do not enable on devices where firmware already
    /// performs the same operation. Keep the DesktopAudio/Connection owners alive.
    #[cfg(all(target_os = "linux", feature = "desktop-audio-linux"))]
    pub fn audio_media_keys(
        self,
        handle: super::desktop_audio::DesktopAudioHandle,
        step_ui: f32,
        on_error: fn(crate::integrations::pipewire::MediaError),
    ) -> Result<Self, crate::integrations::pipewire::MediaError> {
        use crate::services::audio::{Amplification, AudioControlTarget, AudioSystemAction};
        if !step_ui.is_finite() || !(0.0..=1.0).contains(&step_ui) || step_ui == 0.0 {
            return Err(crate::integrations::pipewire::MediaError::InvalidArgument(
                "audio key volume step",
            ));
        }
        let mut bindings = self;
        for (key, delta_ui) in [
            (ShortcutKey::AudioVolumeDown, -step_ui),
            (ShortcutKey::AudioVolumeUp, step_ui),
        ] {
            bindings = bindings.bind_audio(
                KeyChord::new(key),
                handle.clone(),
                AudioSystemAction::AdjustVolume {
                    target: AudioControlTarget::DefaultOutput,
                    delta_ui,
                    amplification: Amplification::Forbid,
                },
                on_error,
            );
        }
        for (key, target) in [
            (
                ShortcutKey::AudioVolumeMute,
                AudioControlTarget::DefaultOutput,
            ),
            (
                ShortcutKey::MicrophoneMute,
                AudioControlTarget::DefaultInput,
            ),
        ] {
            bindings = bindings.bind_audio(
                KeyChord::new(key),
                handle.clone(),
                AudioSystemAction::ToggleMute { target },
                on_error,
            );
        }
        Ok(bindings)
    }

    pub(crate) fn handle(&self, event: ShellKeyEvent) -> ShellKeyAction {
        if let Some((_, handler)) = self.bindings.iter().find(|(chord, _)| chord.matches(event)) {
            handler.invoke();
            ShellKeyAction::Consume
        } else {
            ShellKeyAction::Forward
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop() {}

    #[test]
    fn named_letters_ignore_case_but_require_exact_shift_state() {
        let letters = [
            ShortcutKey::A,
            ShortcutKey::B,
            ShortcutKey::C,
            ShortcutKey::D,
            ShortcutKey::E,
            ShortcutKey::F,
            ShortcutKey::G,
            ShortcutKey::H,
            ShortcutKey::I,
            ShortcutKey::J,
            ShortcutKey::K,
            ShortcutKey::L,
            ShortcutKey::M,
            ShortcutKey::N,
            ShortcutKey::O,
            ShortcutKey::P,
            ShortcutKey::Q,
            ShortcutKey::R,
            ShortcutKey::S,
            ShortcutKey::T,
            ShortcutKey::U,
            ShortcutKey::V,
            ShortcutKey::W,
            ShortcutKey::X,
            ShortcutKey::Y,
            ShortcutKey::Z,
        ];
        for (key, lower) in letters.into_iter().zip(b'a'..=b'z') {
            for shifted in [false, true] {
                let mut chord = KeyChord::new(key).control();
                if shifted {
                    chord = chord.shift();
                }
                let bindings = KeyBindings::new().bind(chord, noop);
                // Caps Lock may invert the case with either Shift state.
                for symbol in [lower, lower.to_ascii_uppercase()] {
                    let event = ShellKeyEvent {
                        keysym: u32::from(symbol),
                        control: true,
                        shift: shifted,
                        ..Default::default()
                    };
                    assert_eq!(bindings.handle(event), ShellKeyAction::Consume);
                    assert_eq!(
                        bindings.handle(ShellKeyEvent {
                            shift: !shifted,
                            ..event
                        }),
                        ShellKeyAction::Forward
                    );
                    assert_eq!(
                        bindings.handle(ShellKeyEvent {
                            control: false,
                            ..event
                        }),
                        ShellKeyAction::Forward
                    );
                    assert_eq!(
                        bindings.handle(ShellKeyEvent {
                            keysym: u32::from('é'),
                            ..event
                        }),
                        ShellKeyAction::Forward
                    );
                }
            }
        }
    }

    #[test]
    fn named_digits_and_function_keys_keep_exact_symbols() {
        let digits = [
            ShortcutKey::Digit0,
            ShortcutKey::Digit1,
            ShortcutKey::Digit2,
            ShortcutKey::Digit3,
            ShortcutKey::Digit4,
            ShortcutKey::Digit5,
            ShortcutKey::Digit6,
            ShortcutKey::Digit7,
            ShortcutKey::Digit8,
            ShortcutKey::Digit9,
        ];
        for (key, symbol) in digits.into_iter().zip(b'0'..=b'9') {
            assert_eq!(key, ShortcutKey::ascii(char::from(symbol)));
        }
        let functions = [
            ShortcutKey::F1,
            ShortcutKey::F2,
            ShortcutKey::F3,
            ShortcutKey::F4,
            ShortcutKey::F5,
            ShortcutKey::F6,
            ShortcutKey::F7,
            ShortcutKey::F8,
            ShortcutKey::F9,
            ShortcutKey::F10,
            ShortcutKey::F11,
            ShortcutKey::F12,
        ];
        for (key, symbol) in functions.into_iter().zip(0xffbe..=0xffc9) {
            let chord = KeyChord::new(key);
            assert!(chord.matches(ShellKeyEvent {
                keysym: symbol,
                ..Default::default()
            }));
            assert!(!chord.matches(ShellKeyEvent {
                keysym: symbol + 1,
                ..Default::default()
            }));
        }
        assert!(
            !KeyChord::new(ShortcutKey::Digit1)
                .shift()
                .matches(ShellKeyEvent {
                    keysym: u32::from('!'),
                    shift: true,
                    ..Default::default()
                })
        );
    }

    #[test]
    fn named_and_exact_letter_overlaps_are_rejected_in_either_order() {
        for exact in [
            ShortcutKey::ascii('q'),
            ShortcutKey::ascii('Q'),
            ShortcutKey::from_keysym(u32::from('q')),
            ShortcutKey::from_keysym(u32::from('Q')),
        ] {
            for (first, second) in [(ShortcutKey::Q, exact), (exact, ShortcutKey::Q)] {
                assert!(
                    std::panic::catch_unwind(|| {
                        KeyBindings::new()
                            .bind(KeyChord::new(first).control(), noop)
                            .bind(KeyChord::new(second).control(), noop);
                    })
                    .is_err()
                );
            }
        }
        // Different modifiers remain independent; exact symbols retain case sensitivity.
        KeyBindings::new()
            .bind(KeyChord::new(ShortcutKey::Q).control(), noop)
            .bind(KeyChord::new(ShortcutKey::Q).control().shift(), noop);
        for constructor in [
            ShortcutKey::ascii('q'),
            ShortcutKey::from_keysym(u32::from('q')),
        ] {
            let chord = KeyChord::new(constructor);
            assert!(chord.matches(ShellKeyEvent {
                keysym: u32::from('q'),
                ..Default::default()
            }));
            assert!(!chord.matches(ShellKeyEvent {
                keysym: u32::from('Q'),
                ..Default::default()
            }));
        }
        KeyBindings::new()
            .bind(KeyChord::new(ShortcutKey::ascii('q')), noop)
            .bind(KeyChord::new(ShortcutKey::ascii('Q')), noop);
    }

    #[test]
    fn matching_requires_exact_modifiers_and_symbol() {
        let bindings = KeyBindings::new().bind(
            KeyChord::new(ShortcutKey::ascii('Q'))
                .control()
                .shift()
                .alt()
                .super_key(),
            noop,
        );
        let matched = ShellKeyEvent {
            keysym: u32::from('Q'),
            control: true,
            shift: true,
            alt: true,
            logo: true,
            ..Default::default()
        };
        assert_eq!(bindings.handle(matched), ShellKeyAction::Consume);
        for event in [
            ShellKeyEvent {
                control: false,
                ..matched
            },
            ShellKeyEvent {
                shift: false,
                ..matched
            },
            ShellKeyEvent {
                alt: false,
                ..matched
            },
            ShellKeyEvent {
                logo: false,
                ..matched
            },
            ShellKeyEvent {
                keysym: u32::from('q'),
                ..matched
            },
        ] {
            assert_eq!(bindings.handle(event), ShellKeyAction::Forward);
        }
        let plain = KeyBindings::new().bind(KeyChord::new(ShortcutKey::Space), noop);
        assert_eq!(
            plain.handle(ShellKeyEvent {
                keysym: 0x20,
                logo: true,
                ..Default::default()
            }),
            ShellKeyAction::Forward
        );
        assert_eq!(KeyBindings::new().handle(matched), ShellKeyAction::Forward);
    }

    #[test]
    #[should_panic(expected = "duplicate compositor shortcut")]
    fn aliases_cannot_register_duplicate_chords() {
        KeyBindings::new()
            .bind(KeyChord::new(ShortcutKey::Space), noop)
            .bind(KeyChord::new(ShortcutKey::ascii(' ')), noop);
    }

    #[test]
    #[should_panic(expected = "printable ASCII")]
    fn ascii_rejects_non_ascii() {
        ShortcutKey::ascii('é');
    }
}
