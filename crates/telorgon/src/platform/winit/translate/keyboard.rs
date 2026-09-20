//! Side-effect-free translation of Winit keyboard observations.

use std::error::Error;
use std::fmt;

use crate::input::{
    ButtonState, KeyEvent, KeyLocation, KeyText, KeyTextError, LogicalKey, Modifiers, NamedKey,
    PhysicalKey, PhysicalKeyCode,
};
use crate::platform::contracts::ViewId;
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::{
    KeyCode as WinitKeyCode, KeyLocation as WinitKeyLocation,
    ModifiersState as WinitModifiersState, NamedKey as WinitNamedKey,
    PhysicalKey as WinitPhysicalKey,
};
use winit::window::WindowId;

use crate::platform::winit::ViewRegistry;

macro_rules! define_physical_mapping {
    ($($name:ident),+ $(,)?) => {
        /// Translates a Winit physical key without retaining native unidentified codes.
        pub fn translate_physical_key(key: WinitPhysicalKey) -> PhysicalKey {
            let WinitPhysicalKey::Code(code) = key else {
                return PhysicalKey::UNIDENTIFIED;
            };
            match code {
                $(WinitKeyCode::$name => PhysicalKey::from_code(PhysicalKeyCode::$name),)+
                _ => PhysicalKey::UNIDENTIFIED,
            }
        }

        #[cfg(test)]
        const PHYSICAL_MAPPING: &[(WinitKeyCode, PhysicalKeyCode)] = &[
            $((WinitKeyCode::$name, PhysicalKeyCode::$name),)+
        ];
    };
}

define_physical_mapping!(
    Backquote,
    Backslash,
    BracketLeft,
    BracketRight,
    Comma,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Equal,
    IntlBackslash,
    IntlRo,
    IntlYen,
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Minus,
    Period,
    Quote,
    Semicolon,
    Slash,
    AltLeft,
    AltRight,
    Backspace,
    CapsLock,
    ContextMenu,
    ControlLeft,
    ControlRight,
    Enter,
    SuperLeft,
    SuperRight,
    ShiftLeft,
    ShiftRight,
    Space,
    Tab,
    Convert,
    KanaMode,
    Lang1,
    Lang2,
    Lang3,
    Lang4,
    Lang5,
    NonConvert,
    Delete,
    End,
    Help,
    Home,
    Insert,
    PageDown,
    PageUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    NumLock,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadAdd,
    NumpadBackspace,
    NumpadClear,
    NumpadClearEntry,
    NumpadComma,
    NumpadDecimal,
    NumpadDivide,
    NumpadEnter,
    NumpadEqual,
    NumpadHash,
    NumpadMemoryAdd,
    NumpadMemoryClear,
    NumpadMemoryRecall,
    NumpadMemoryStore,
    NumpadMemorySubtract,
    NumpadMultiply,
    NumpadParenLeft,
    NumpadParenRight,
    NumpadStar,
    NumpadSubtract,
    Escape,
    Fn,
    FnLock,
    PrintScreen,
    ScrollLock,
    Pause,
    BrowserBack,
    BrowserFavorites,
    BrowserForward,
    BrowserHome,
    BrowserRefresh,
    BrowserSearch,
    BrowserStop,
    Eject,
    LaunchApp1,
    LaunchApp2,
    LaunchMail,
    MediaPlayPause,
    MediaSelect,
    MediaStop,
    MediaTrackNext,
    MediaTrackPrevious,
    Power,
    Sleep,
    AudioVolumeDown,
    AudioVolumeMute,
    AudioVolumeUp,
    WakeUp,
    Meta,
    Hyper,
    Turbo,
    Abort,
    Resume,
    Suspend,
    Again,
    Copy,
    Cut,
    Find,
    Open,
    Paste,
    Props,
    Select,
    Undo,
    Hiragana,
    Katakana,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    F25,
    F26,
    F27,
    F28,
    F29,
    F30,
    F31,
    F32,
    F33,
    F34,
    F35,
);

macro_rules! define_named_mapping {
    ($($name:ident),+ $(,)?) => {
        /// Translates one standardized Winit named key.
        ///
        /// `None` defensively represents a future Winit value absent from this adapter revision.
        pub fn translate_named_key(key: WinitNamedKey) -> Option<NamedKey> {
            match key {
                $(WinitNamedKey::$name => Some(NamedKey::$name),)+
                _ => None,
            }
        }

        #[cfg(test)]
        const NAMED_MAPPING: &[(WinitNamedKey, NamedKey)] = &[
            $((WinitNamedKey::$name, NamedKey::$name),)+
        ];
    };
}

define_named_mapping!(
    Alt,
    AltGraph,
    CapsLock,
    Control,
    Fn,
    FnLock,
    NumLock,
    ScrollLock,
    Shift,
    Symbol,
    SymbolLock,
    Meta,
    Hyper,
    Super,
    Enter,
    Tab,
    Space,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    End,
    Home,
    PageDown,
    PageUp,
    Backspace,
    Clear,
    Copy,
    CrSel,
    Cut,
    Delete,
    EraseEof,
    ExSel,
    Insert,
    Paste,
    Redo,
    Undo,
    Accept,
    Again,
    Attn,
    Cancel,
    ContextMenu,
    Escape,
    Execute,
    Find,
    Help,
    Pause,
    Play,
    Props,
    Select,
    ZoomIn,
    ZoomOut,
    BrightnessDown,
    BrightnessUp,
    Eject,
    LogOff,
    Power,
    PowerOff,
    PrintScreen,
    Hibernate,
    Standby,
    WakeUp,
    AllCandidates,
    Alphanumeric,
    CodeInput,
    Compose,
    Convert,
    FinalMode,
    GroupFirst,
    GroupLast,
    GroupNext,
    GroupPrevious,
    ModeChange,
    NextCandidate,
    NonConvert,
    PreviousCandidate,
    Process,
    SingleCandidate,
    HangulMode,
    HanjaMode,
    JunjaMode,
    Eisu,
    Hankaku,
    Hiragana,
    HiraganaKatakana,
    KanaMode,
    KanjiMode,
    Katakana,
    Romaji,
    Zenkaku,
    ZenkakuHankaku,
    Soft1,
    Soft2,
    Soft3,
    Soft4,
    ChannelDown,
    ChannelUp,
    Close,
    MailForward,
    MailReply,
    MailSend,
    MediaClose,
    MediaFastForward,
    MediaPause,
    MediaPlay,
    MediaPlayPause,
    MediaRecord,
    MediaRewind,
    MediaStop,
    MediaTrackNext,
    MediaTrackPrevious,
    New,
    Open,
    Print,
    Save,
    SpellCheck,
    Key11,
    Key12,
    AudioBalanceLeft,
    AudioBalanceRight,
    AudioBassBoostDown,
    AudioBassBoostToggle,
    AudioBassBoostUp,
    AudioFaderFront,
    AudioFaderRear,
    AudioSurroundModeNext,
    AudioTrebleDown,
    AudioTrebleUp,
    AudioVolumeDown,
    AudioVolumeUp,
    AudioVolumeMute,
    MicrophoneToggle,
    MicrophoneVolumeDown,
    MicrophoneVolumeUp,
    MicrophoneVolumeMute,
    SpeechCorrectionList,
    SpeechInputToggle,
    LaunchApplication1,
    LaunchApplication2,
    LaunchCalendar,
    LaunchContacts,
    LaunchMail,
    LaunchMediaPlayer,
    LaunchMusicPlayer,
    LaunchPhone,
    LaunchScreenSaver,
    LaunchSpreadsheet,
    LaunchWebBrowser,
    LaunchWebCam,
    LaunchWordProcessor,
    BrowserBack,
    BrowserFavorites,
    BrowserForward,
    BrowserHome,
    BrowserRefresh,
    BrowserSearch,
    BrowserStop,
    AppSwitch,
    Call,
    Camera,
    CameraFocus,
    EndCall,
    GoBack,
    GoHome,
    HeadsetHook,
    LastNumberRedial,
    Notification,
    MannerMode,
    VoiceDial,
    TV,
    TV3DMode,
    TVAntennaCable,
    TVAudioDescription,
    TVAudioDescriptionMixDown,
    TVAudioDescriptionMixUp,
    TVContentsMenu,
    TVDataService,
    TVInput,
    TVInputComponent1,
    TVInputComponent2,
    TVInputComposite1,
    TVInputComposite2,
    TVInputHDMI1,
    TVInputHDMI2,
    TVInputHDMI3,
    TVInputHDMI4,
    TVInputVGA1,
    TVMediaContext,
    TVNetwork,
    TVNumberEntry,
    TVPower,
    TVRadioService,
    TVSatellite,
    TVSatelliteBS,
    TVSatelliteCS,
    TVSatelliteToggle,
    TVTerrestrialAnalog,
    TVTerrestrialDigital,
    TVTimer,
    AVRInput,
    AVRPower,
    ColorF0Red,
    ColorF1Green,
    ColorF2Yellow,
    ColorF3Blue,
    ColorF4Grey,
    ColorF5Brown,
    ClosedCaptionToggle,
    Dimmer,
    DisplaySwap,
    DVR,
    Exit,
    FavoriteClear0,
    FavoriteClear1,
    FavoriteClear2,
    FavoriteClear3,
    FavoriteRecall0,
    FavoriteRecall1,
    FavoriteRecall2,
    FavoriteRecall3,
    FavoriteStore0,
    FavoriteStore1,
    FavoriteStore2,
    FavoriteStore3,
    Guide,
    GuideNextDay,
    GuidePreviousDay,
    Info,
    InstantReplay,
    Link,
    ListProgram,
    LiveContent,
    Lock,
    MediaApps,
    MediaAudioTrack,
    MediaLast,
    MediaSkipBackward,
    MediaSkipForward,
    MediaStepBackward,
    MediaStepForward,
    MediaTopMenu,
    NavigateIn,
    NavigateNext,
    NavigateOut,
    NavigatePrevious,
    NextFavoriteChannel,
    NextUserProfile,
    OnDemand,
    Pairing,
    PinPDown,
    PinPMove,
    PinPToggle,
    PinPUp,
    PlaySpeedDown,
    PlaySpeedReset,
    PlaySpeedUp,
    RandomToggle,
    RcLowBattery,
    RecordSpeedNext,
    RfBypass,
    ScanChannelsToggle,
    ScreenModeNext,
    Settings,
    SplitScreenToggle,
    STBInput,
    STBPower,
    Subtitle,
    Teletext,
    VideoModeNext,
    Wink,
    ZoomToggle,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    F25,
    F26,
    F27,
    F28,
    F29,
    F30,
    F31,
    F32,
    F33,
    F34,
    F35,
);

/// Borrowed logical meaning copied from a Winit key event.
///
/// Native unidentified payloads are deliberately collapsed before crossing this boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WinitLogicalKey<'a> {
    Named(WinitNamedKey),
    Character(&'a str),
    Dead(Option<char>),
    Unidentified,
}

impl fmt::Debug for WinitLogicalKey<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(key) => formatter.debug_tuple("Named").field(key).finish(),
            Self::Character(text) => formatter
                .debug_struct("Character")
                .field("byte_len", &text.len())
                .field("redacted", &true)
                .finish(),
            Self::Dead(character) => formatter
                .debug_struct("Dead")
                .field("has_character", &character.is_some())
                .field("character_redacted", &character.is_some())
                .finish(),
            Self::Unidentified => formatter.write_str("Unidentified"),
        }
    }
}

/// Borrowed, callback-scoped keyboard fields selected from Winit.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct WinitKeyboardInput<'a> {
    pub physical_key: WinitPhysicalKey,
    pub logical_key: WinitLogicalKey<'a>,
    pub text: Option<&'a str>,
    pub location: WinitKeyLocation,
    pub state: ElementState,
    pub repeat: bool,
    pub synthetic: bool,
}

impl WinitKeyboardInput<'_> {
    /// Borrows the supported fields from a Winit keyboard event.
    pub fn from_event(event: &WindowEvent) -> Option<WinitKeyboardInput<'_>> {
        let WindowEvent::KeyboardInput {
            event,
            is_synthetic,
            ..
        } = event
        else {
            return None;
        };
        let logical_key = match &event.logical_key {
            winit::keyboard::Key::Named(key) => WinitLogicalKey::Named(*key),
            winit::keyboard::Key::Character(text) => WinitLogicalKey::Character(text.as_str()),
            winit::keyboard::Key::Dead(character) => WinitLogicalKey::Dead(*character),
            winit::keyboard::Key::Unidentified(_) => WinitLogicalKey::Unidentified,
        };
        Some(WinitKeyboardInput {
            physical_key: event.physical_key,
            logical_key,
            text: event.text.as_ref().map(|text| text.as_str()),
            location: event.location,
            state: event.state,
            repeat: event.repeat,
            synthetic: *is_synthetic,
        })
    }
}

impl fmt::Debug for WinitKeyboardInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WinitKeyboardInput")
            .field("physical_key", &self.physical_key)
            .field("logical_key", &self.logical_key)
            .field("text_byte_len", &self.text.map(str::len))
            .field("text_redacted", &self.text.is_some())
            .field("location", &self.location)
            .field("state", &self.state)
            .field("repeat", &self.repeat)
            .field("synthetic", &self.synthetic)
            .finish()
    }
}

/// Whether Winit's produced key text may enter the neutral event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardTextPolicy {
    /// Preserve Winit's optional produced text after enforcing the neutral bound.
    Preserve,
    /// Omit produced text because an active IME composition is the only text mutation path.
    SuppressDuringImeComposition,
}

/// Caller-supplied keyboard state that Winit does not carry inside each key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WinitKeyboardContext {
    modifiers: Modifiers,
    text_policy: KeyboardTextPolicy,
}

impl WinitKeyboardContext {
    pub const fn new(modifiers: Modifiers, text_policy: KeyboardTextPolicy) -> Self {
        Self {
            modifiers,
            text_policy,
        }
    }

    pub const fn modifiers(self) -> Modifiers {
        self.modifiers
    }

    pub const fn text_policy(self) -> KeyboardTextPolicy {
        self.text_policy
    }
}

/// Which textual keyboard field exceeded the neutral hard bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardTextField {
    LogicalCharacter,
    ProducedText,
}

/// Typed rejection from contextual Winit keyboard translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardTranslationError {
    WindowUnavailable {
        window: WindowId,
    },
    TextTooLong {
        view: ViewId,
        field: KeyboardTextField,
        byte_len: usize,
        maximum_bytes: usize,
    },
}

impl fmt::Display for KeyboardTranslationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WindowUnavailable { window } => write!(
                formatter,
                "Winit window {window:?} is stale, retired, or unknown during keyboard translation"
            ),
            Self::TextTooLong {
                view,
                field,
                byte_len,
                maximum_bytes,
            } => write!(
                formatter,
                "Winit view {view} reported {byte_len} UTF-8 bytes for {field:?}; maximum is {maximum_bytes}"
            ),
        }
    }
}

impl Error for KeyboardTranslationError {}

/// One immutable, view-scoped neutral keyboard observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WinitKeyboardObservation {
    source_window: WindowId,
    view: ViewId,
    event: KeyEvent,
}

impl WinitKeyboardObservation {
    pub const fn source_window(&self) -> WindowId {
        self.source_window
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn event(&self) -> &KeyEvent {
        &self.event
    }

    pub fn into_event(self) -> KeyEvent {
        self.event
    }
}

/// Selects and translates one borrowed Winit keyboard event.
///
/// Non-keyboard events return `Ok(None)`. No native handle, event-owned value, or text reference is
/// retained in the returned observation.
pub fn translate_keyboard_event(
    registry: &ViewRegistry,
    source_window: WindowId,
    context: WinitKeyboardContext,
    event: &WindowEvent,
) -> Result<Option<WinitKeyboardObservation>, KeyboardTranslationError> {
    let Some(input) = WinitKeyboardInput::from_event(event) else {
        return Ok(None);
    };
    translate_keyboard_input(registry, source_window, context, input).map(Some)
}

/// Translates already-borrowed Winit keyboard fields into a neutral view-scoped event.
pub fn translate_keyboard_input(
    registry: &ViewRegistry,
    source_window: WindowId,
    context: WinitKeyboardContext,
    input: WinitKeyboardInput<'_>,
) -> Result<WinitKeyboardObservation, KeyboardTranslationError> {
    let view = registry.view_for_window(source_window).ok_or(
        KeyboardTranslationError::WindowUnavailable {
            window: source_window,
        },
    )?;

    let logical_key = match input.logical_key {
        WinitLogicalKey::Named(key) => translate_named_key(key)
            .map(LogicalKey::Named)
            .unwrap_or(LogicalKey::Unidentified),
        WinitLogicalKey::Character(text) => LogicalKey::Character(bounded_text(
            view,
            KeyboardTextField::LogicalCharacter,
            text,
        )?),
        WinitLogicalKey::Dead(character) => LogicalKey::Dead(character),
        WinitLogicalKey::Unidentified => LogicalKey::Unidentified,
    };
    let text = match (context.text_policy, input.text) {
        (KeyboardTextPolicy::SuppressDuringImeComposition, _) | (_, None) => None,
        (KeyboardTextPolicy::Preserve, Some(text)) => {
            Some(bounded_text(view, KeyboardTextField::ProducedText, text)?)
        }
    };
    let event = KeyEvent {
        physical_key: translate_physical_key(input.physical_key),
        logical_key,
        text,
        location: translate_location(input.location),
        state: translate_state(input.state),
        repeat: input.repeat,
        synthetic: input.synthetic,
        modifiers: context.modifiers,
    };
    Ok(WinitKeyboardObservation {
        source_window,
        view,
        event,
    })
}

/// Translates a Winit modifier snapshot without inventing unavailable lock or side state.
pub fn translate_modifiers_state(state: WinitModifiersState) -> Modifiers {
    let mut modifiers = Modifiers::empty();
    if state.shift_key() {
        modifiers = modifiers.union(Modifiers::SHIFT);
    }
    if state.control_key() {
        modifiers = modifiers.union(Modifiers::CONTROL);
    }
    if state.alt_key() {
        modifiers = modifiers.union(Modifiers::ALT);
    }
    if state.super_key() {
        modifiers = modifiers.union(Modifiers::SUPER);
    }
    modifiers
}

/// Selects a Winit modifier-change event and translates its aggregate state.
pub fn translate_modifiers_event(event: &WindowEvent) -> Option<Modifiers> {
    match event {
        WindowEvent::ModifiersChanged(modifiers) => {
            Some(translate_modifiers_state(modifiers.state()))
        }
        _ => None,
    }
}

fn translate_location(location: WinitKeyLocation) -> KeyLocation {
    match location {
        WinitKeyLocation::Standard => KeyLocation::Standard,
        WinitKeyLocation::Left => KeyLocation::Left,
        WinitKeyLocation::Right => KeyLocation::Right,
        WinitKeyLocation::Numpad => KeyLocation::Numpad,
    }
}

fn translate_state(state: ElementState) -> ButtonState {
    match state {
        ElementState::Pressed => ButtonState::Pressed,
        ElementState::Released => ButtonState::Released,
    }
}

fn bounded_text(
    view: ViewId,
    field: KeyboardTextField,
    text: &str,
) -> Result<KeyText, KeyboardTranslationError> {
    KeyText::new(text).map_err(|error| match error {
        KeyTextError::TooLong {
            byte_len,
            maximum_bytes,
        } => KeyboardTranslationError::TextTooLong {
            view,
            field,
            byte_len,
            maximum_bytes,
        },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn mapping_tables_cover_every_current_standardized_winit_value_once() {
        assert_eq!(PHYSICAL_MAPPING.len(), 194);
        assert_eq!(NAMED_MAPPING.len(), 306);

        let physical: HashSet<_> = PHYSICAL_MAPPING.iter().map(|(_, key)| *key).collect();
        let named: HashSet<_> = NAMED_MAPPING.iter().map(|(_, key)| *key).collect();
        assert_eq!(physical.len(), PHYSICAL_MAPPING.len());
        assert_eq!(named.len(), NAMED_MAPPING.len());

        for (source, expected) in PHYSICAL_MAPPING {
            assert_eq!(
                translate_physical_key(WinitPhysicalKey::Code(*source)).code(),
                Some(*expected)
            );
        }
        for (source, expected) in NAMED_MAPPING {
            assert_eq!(translate_named_key(*source), Some(*expected));
        }
    }

    #[test]
    fn borrowed_input_debug_redacts_character_dead_and_produced_text() {
        let input = WinitKeyboardInput {
            physical_key: WinitPhysicalKey::Code(WinitKeyCode::KeyA),
            logical_key: WinitLogicalKey::Character("private-character"),
            text: Some("private-produced"),
            location: WinitKeyLocation::Standard,
            state: ElementState::Pressed,
            repeat: false,
            synthetic: false,
        };
        let debug = format!("{input:?}");
        assert!(debug.contains("redacted"));
        assert!(!debug.contains("private-character"));
        assert!(!debug.contains("private-produced"));

        let dead = format!("{:?}", WinitLogicalKey::Dead(Some('`')));
        assert!(dead.contains("character_redacted"));
        assert!(!dead.contains('`'));
    }

    #[test]
    fn text_bound_constant_remains_the_neutral_owner() {
        assert_eq!(crate::input::MAX_KEY_TEXT_BYTES, 4 * 1_024);
    }
}
