use crate::foundation::PointF;

use crate::input::{
    KeyEvent, PointerButton, PointerDeviceKind, PointerEvent, PointerId, ScrollEvent,
    ScrollPrecision,
};

/// Platform-neutral pressed/released state shared by pointer buttons and keyboard keys.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ButtonState {
    Released,
    Pressed,
}

/// Lifecycle phase for a continuous controlled-value interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueChangePhase {
    Begin,
    Update,
    Commit,
    Cancel,
}

/// Complete canonical pointer and scroll observations produced by platform adapters.
///
/// This richer value path is intentionally separate from [`InputEvent`] while the mounted runtime
/// still consumes its compatibility pointer variants. Keeping the boundary explicit lets adapters
/// preserve complete state without silently changing current routing behavior.
#[derive(Clone, Debug, PartialEq)]
pub enum PointerInputEvent {
    Pointer(PointerEvent),
    Scroll(ScrollEvent),
}

impl From<PointerEvent> for PointerInputEvent {
    fn from(event: PointerEvent) -> Self {
        Self::Pointer(event)
    }
}

impl From<ScrollEvent> for PointerInputEvent {
    fn from(event: ScrollEvent) -> Self {
        Self::Scroll(event)
    }
}

impl ButtonState {
    pub const fn is_pressed(self) -> bool {
        matches!(self, Self::Pressed)
    }
}

/// The current stage of a routed UI input event.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EventPhase {
    Capture,
    Target,
    Bubble,
}

/// A platform-neutral input value accepted by the mounted runtime.
///
/// View identity, revisioned metrics, and host event stamps are added by the later neutral platform
/// spine. This initial Epoch D vocabulary replaces the Linux-shaped core event without importing a
/// native event type or component action.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    PointerMoved {
        pointer: PointerId,
        device: PointerDeviceKind,
        position: PointF,
    },
    PointerButton {
        pointer: PointerId,
        device: PointerDeviceKind,
        button: PointerButton,
        state: ButtonState,
    },
    Scroll {
        pointer: PointerId,
        device: PointerDeviceKind,
        /// Content movement in logical pixels (positive moves content down/right).
        delta: PointF,
        precision: ScrollPrecision,
    },
    Key(KeyEvent),
}

impl InputEvent {
    pub const fn mouse_moved(position: PointF) -> Self {
        Self::PointerMoved {
            pointer: PointerId::PRIMARY,
            device: PointerDeviceKind::Mouse,
            position,
        }
    }

    pub const fn mouse_button(button: PointerButton, state: ButtonState) -> Self {
        Self::PointerButton {
            pointer: PointerId::PRIMARY,
            device: PointerDeviceKind::Mouse,
            button,
            state,
        }
    }

    /// Precise logical-pixel input, including platform-supplied momentum. No extra smoothing.
    pub const fn mouse_scroll(delta: PointF) -> Self {
        Self::Scroll {
            pointer: PointerId::PRIMARY,
            device: PointerDeviceKind::Mouse,
            delta,
            precision: ScrollPrecision::Precise,
        }
    }

    /// Discrete wheel steps; fractional steps from high-resolution wheels are preserved.
    pub const fn mouse_wheel(steps: PointF) -> Self {
        Self::Scroll {
            pointer: PointerId::PRIMARY,
            device: PointerDeviceKind::Mouse,
            delta: PointF {
                x: steps.x * DEFAULT_WHEEL_STEP,
                y: steps.y * DEFAULT_WHEEL_STEP,
            },
            precision: ScrollPrecision::Discrete,
        }
    }
}

/// Default logical distance for one normalized wheel step.
pub const DEFAULT_WHEEL_STEP: f32 = 48.0;
