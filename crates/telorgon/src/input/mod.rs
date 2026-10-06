//! Platform-neutral input values and routing decisions.
//!
//! Native event conversion belongs to platform adapters. Component actions and mutable routing
//! state belong to the runtime.

pub mod activation;
pub mod composite;
mod event;
pub mod focus;
pub mod gesture;
mod keyboard;
mod pointer;
mod route;
mod text_input;
pub use text_input::TextInputEvent;
pub mod shortcut;

pub use activation::{
    Activation, ActivationCancelReason, ActivationInput, ActivationOutcome, ActivationPhase,
    ActivationStateMachine, ActivationTransition, ChangeSource, CompetingGesture,
    PointerCaptureRequest,
};
pub use composite::{
    CompositeChange, CompositeDiagnostics, CompositeEdgeBehavior, CompositeEntryReason,
    CompositeError, CompositeFocusTarget, CompositeHighlightReason, CompositeItem,
    CompositeNavigationCommand, CompositeNavigationPolicy, CompositeOrientation,
    CompositeSelectionBehavior, CompositeSelectionRequest, CompositeStateMachine,
    DisabledItemPolicy, WritingDirection,
};
pub use event::{DEFAULT_WHEEL_STEP, ButtonState, EventPhase, InputEvent, PointerInputEvent, ValueChangePhase};
pub use focus::{
    FocusCandidate, FocusChange, FocusClearReason, FocusDiagnostics, FocusError,
    FocusIndicatorPolicy, FocusInputModality, FocusMoveReason, FocusOrigin, FocusScopeId,
    FocusStateMachine, FocusTraversalDirection, FocusTraversalEdge,
};
pub use gesture::{
    DragAxis, DragRecognizer, GestureArena, GestureArenaDecision, GestureArenaDiagnostics,
    GestureArenaError, GestureArenaLossReason, GestureArenaRequest, GestureArenaWinReason,
    GestureCancelReason, GestureDeadlineId, GestureDeadlineRequest, GestureDelta, GestureInput,
    GestureKind, GestureOutcome, GestureRecognizerDiagnostics, GestureRecognizerError,
    GestureRecognizerState, GestureTransition, LongPressRecognizer, TapRecognizer,
};
pub use keyboard::{
    KeyEvent, KeyLocation, KeyText, KeyTextError, LogicalKey, MAX_KEY_TEXT_BYTES, Modifiers,
    NamedKey, PhysicalKey, PhysicalKeyCode,
};
pub use pointer::{
    MAX_PRESSED_POINTER_BUTTONS, PhysicalPointerPosition, PhysicalScrollDelta, PointerButton,
    PointerButtonSet, PointerButtonSetError, PointerCancelReason, PointerCaptureChange,
    PointerContactGeometry, PointerCoordinateError, PointerDeviceId, PointerDeviceKind,
    PointerEvent, PointerEventError, PointerEventKind, PointerEventSource, PointerId,
    PointerPosition, PointerPressure, PointerProperties, PointerPropertyError,
    PointerStateSnapshot, PointerTilt, PointerTwist, ScrollDelta, ScrollEvent, ScrollMomentumPhase,
    ScrollPhase, ScrollPrecision, ScrollUnit, ScrollValueError,
};
pub use route::{DefaultResponse, Propagation};
pub use shortcut::{
    ActiveShortcutScope, ShortcutBinding, ShortcutChord, ShortcutDiagnostics, ShortcutError,
    ShortcutMatcher, ShortcutRepeatPolicy, ShortcutResolution, ShortcutScopeId,
    ShortcutScopePolicy, ShortcutTrigger,
};
