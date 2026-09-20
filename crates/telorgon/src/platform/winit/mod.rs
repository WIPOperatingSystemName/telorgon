//! Winit-specific platform adaptation, separate from renderers and presenters.
//!
//! The implemented slices own the bounded, generation-aware relationship between Winit window
//! identities and neutral [`crate::platform::contracts::ViewId`] values plus typed cross-thread completion
//! delivery through a caller-provided Winit event-loop proxy, pure post-turn scheduling plans, and
//! side-effect-free window, keyboard, and pointer/scroll observation translation. They create no
//! window or event loop, implement no application handler, and perform no rendering, presentation,
//! runtime dispatch, or fallback selection.

mod event_proxy;
mod schedule;
pub mod translate;
mod view_registry;

pub use event_proxy::{
    CompletionEvent, CompletionEventProxy, CompletionSendError, CompletionSendErrorKind,
};
pub use schedule::{
    RedrawTarget, WinitClockObservation, WinitScheduleError, WinitSchedulePlan, WinitWakeIntent,
    interpret_schedule,
};
pub use translate::keyboard::{
    KeyboardTextField, KeyboardTextPolicy, KeyboardTranslationError, WinitKeyboardContext,
    WinitKeyboardInput, WinitKeyboardObservation, WinitLogicalKey, translate_keyboard_event,
    translate_keyboard_input, translate_modifiers_event, translate_modifiers_state,
    translate_named_key, translate_physical_key,
};
pub use translate::pointer::{
    PointerTranslationError, WinitPointerContext, WinitPointerFact, WinitPointerObservation,
    translate_mouse_button, translate_pointer_event, translate_pointer_fact,
};
pub use translate::window::{
    WindowTranslationError, WinitWindowFact, WinitWindowObservation, WinitWindowObservationKind,
    translate_window_event, translate_window_fact,
};
pub use view_registry::{
    MAX_WINIT_VIEWS, RetiredView, ViewRegistration, ViewRegistry, ViewRegistryError,
    ViewRegistryLimitError, WindowReplacement,
};
