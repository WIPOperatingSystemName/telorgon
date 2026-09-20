//! Side-effect-free translation of Winit cursor, mouse-button, and mouse-wheel observations.

use std::error::Error;
use std::fmt;

use crate::foundation::PointF;
use crate::input::{
    ButtonState, PhysicalPointerPosition, PhysicalScrollDelta, PointerButton, PointerButtonSet,
    PointerButtonSetError, PointerCoordinateError, PointerDeviceKind, PointerEvent,
    PointerEventError, PointerEventKind, PointerInputEvent, PointerPosition, PointerStateSnapshot,
    ScrollDelta, ScrollEvent, ScrollMomentumPhase, ScrollPhase, ScrollPrecision, ScrollUnit,
    ScrollValueError,
};
use crate::platform::contracts::{MetricsCitation, MetricsRevision, ViewId, ViewSnapshot};
use winit::dpi::PhysicalPosition;
use winit::event::{
    DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::window::WindowId;

use crate::platform::winit::ViewRegistry;

/// Minimal copied fact selected from one supported Winit pointer event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WinitPointerFact {
    CursorEntered {
        device_id: DeviceId,
    },
    CursorLeft {
        device_id: DeviceId,
    },
    CursorMoved {
        device_id: DeviceId,
        physical_position: PhysicalPosition<f64>,
    },
    MouseButton {
        device_id: DeviceId,
        button: MouseButton,
        state: ElementState,
    },
    MouseWheel {
        device_id: DeviceId,
        delta: MouseScrollDelta,
        phase: TouchPhase,
    },
}

impl WinitPointerFact {
    /// Copies only the supported portion of a borrowed Winit event.
    pub fn from_event(event: &WindowEvent) -> Option<Self> {
        match event {
            WindowEvent::CursorEntered { device_id } => Some(Self::CursorEntered {
                device_id: *device_id,
            }),
            WindowEvent::CursorLeft { device_id } => Some(Self::CursorLeft {
                device_id: *device_id,
            }),
            WindowEvent::CursorMoved {
                device_id,
                position,
            } => Some(Self::CursorMoved {
                device_id: *device_id,
                physical_position: *position,
            }),
            WindowEvent::MouseInput {
                device_id,
                button,
                state,
            } => Some(Self::MouseButton {
                device_id: *device_id,
                button: *button,
                state: *state,
            }),
            WindowEvent::MouseWheel {
                device_id,
                delta,
                phase,
            } => Some(Self::MouseWheel {
                device_id: *device_id,
                delta: *delta,
                phase: *phase,
            }),
            _ => None,
        }
    }

    pub const fn device_id(self) -> DeviceId {
        match self {
            Self::CursorEntered { device_id }
            | Self::CursorLeft { device_id }
            | Self::CursorMoved { device_id, .. }
            | Self::MouseButton { device_id, .. }
            | Self::MouseWheel { device_id, .. } => device_id,
        }
    }
}

/// Immutable caller-owned state needed because Winit pointer callbacks are intentionally partial.
///
/// The caller maps Winit's native device identity to the neutral identity in `state` and cites the
/// exact metrics revision under which any retained position was converted. Translation validates
/// both facts but never mutates this context.
#[derive(Clone, Debug, PartialEq)]
pub struct WinitPointerContext {
    source_device: DeviceId,
    state_metrics: MetricsRevision,
    state: PointerStateSnapshot,
}

impl WinitPointerContext {
    pub const fn new(
        source_device: DeviceId,
        state_metrics: MetricsRevision,
        state: PointerStateSnapshot,
    ) -> Self {
        Self {
            source_device,
            state_metrics,
            state,
        }
    }

    pub const fn source_device(&self) -> DeviceId {
        self.source_device
    }

    pub const fn state_metrics(&self) -> MetricsRevision {
        self.state_metrics
    }

    pub const fn state(&self) -> &PointerStateSnapshot {
        &self.state
    }
}

/// Typed rejection from contextual Winit pointer translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerTranslationError {
    WindowUnavailable {
        window: WindowId,
    },
    SnapshotViewMismatch {
        window: WindowId,
        registered_view: ViewId,
        snapshot_view: ViewId,
    },
    DeviceMismatch {
        view: ViewId,
        expected: DeviceId,
        observed: DeviceId,
    },
    StateMetricsMismatch {
        view: ViewId,
        expected: MetricsRevision,
        observed: MetricsRevision,
    },
    ContextDeviceKind {
        view: ViewId,
        observed: PointerDeviceKind,
    },
    InvalidPosition {
        view: ViewId,
        cause: PointerCoordinateError,
    },
    InvalidScrollDelta {
        view: ViewId,
        cause: ScrollValueError,
    },
    InvalidButtonState {
        view: ViewId,
        cause: PointerButtonSetError,
    },
    InvalidPointerEvent {
        view: ViewId,
        cause: PointerEventError,
    },
}

impl fmt::Display for PointerTranslationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WindowUnavailable { window } => write!(
                formatter,
                "Winit window {window:?} is stale, retired, or unknown during pointer translation"
            ),
            Self::SnapshotViewMismatch {
                window,
                registered_view,
                snapshot_view,
            } => write!(
                formatter,
                "Winit window {window:?} belongs to {registered_view}, not pointer snapshot view {snapshot_view}"
            ),
            Self::DeviceMismatch {
                view,
                expected,
                observed,
            } => write!(
                formatter,
                "Winit view {view} pointer context belongs to {expected:?}, not event device {observed:?}"
            ),
            Self::StateMetricsMismatch {
                view,
                expected,
                observed,
            } => write!(
                formatter,
                "Winit view {view} pointer state cites metrics {observed}, not current metrics {expected}"
            ),
            Self::ContextDeviceKind { view, observed } => write!(
                formatter,
                "Winit cursor/mouse translation for {view} requires Mouse state, not {observed:?}"
            ),
            Self::InvalidPosition { view, cause } => {
                write!(
                    formatter,
                    "Winit view {view} reported an invalid pointer position: {cause}"
                )
            }
            Self::InvalidScrollDelta { view, cause } => {
                write!(
                    formatter,
                    "Winit view {view} reported an invalid scroll delta: {cause}"
                )
            }
            Self::InvalidButtonState { view, cause } => {
                write!(
                    formatter,
                    "Winit view {view} produced invalid complete button state: {cause}"
                )
            }
            Self::InvalidPointerEvent { view, cause } => {
                write!(
                    formatter,
                    "Winit view {view} produced an invalid pointer event: {cause}"
                )
            }
        }
    }
}

impl Error for PointerTranslationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidPosition { cause, .. } => Some(cause),
            Self::InvalidScrollDelta { cause, .. } => Some(cause),
            Self::InvalidButtonState { cause, .. } => Some(cause),
            Self::InvalidPointerEvent { cause, .. } => Some(cause),
            Self::WindowUnavailable { .. }
            | Self::SnapshotViewMismatch { .. }
            | Self::DeviceMismatch { .. }
            | Self::StateMetricsMismatch { .. }
            | Self::ContextDeviceKind { .. } => None,
        }
    }
}

/// One immutable current-view neutral pointer observation.
#[derive(Clone, Debug, PartialEq)]
pub struct WinitPointerObservation {
    source_window: WindowId,
    view: ViewId,
    metrics: MetricsCitation,
    event: PointerInputEvent,
}

impl WinitPointerObservation {
    pub const fn source_window(&self) -> WindowId {
        self.source_window
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn metrics_citation(&self) -> MetricsCitation {
        self.metrics
    }

    pub const fn event(&self) -> &PointerInputEvent {
        &self.event
    }

    pub fn into_event(self) -> PointerInputEvent {
        self.event
    }
}

/// Translates a Winit mouse button into the neutral standardized/other namespaces.
pub const fn translate_mouse_button(button: MouseButton) -> PointerButton {
    match button {
        MouseButton::Left => PointerButton::PRIMARY,
        MouseButton::Right => PointerButton::SECONDARY,
        MouseButton::Middle => PointerButton::MIDDLE,
        MouseButton::Back => PointerButton::BACK,
        MouseButton::Forward => PointerButton::FORWARD,
        MouseButton::Other(value) => PointerButton::from_platform_other(value),
    }
}

/// Selects and translates one supported borrowed Winit pointer event.
///
/// Unsupported events return `Ok(None)`. No event-owned reference or native device identity is
/// retained in the returned neutral payload.
pub fn translate_pointer_event(
    registry: &ViewRegistry,
    source_window: WindowId,
    snapshot: &ViewSnapshot,
    context: &WinitPointerContext,
    event: &WindowEvent,
) -> Result<Option<WinitPointerObservation>, PointerTranslationError> {
    let Some(fact) = WinitPointerFact::from_event(event) else {
        return Ok(None);
    };
    translate_pointer_fact(registry, source_window, snapshot, context, fact).map(Some)
}

/// Translates one already-copied Winit pointer fact without mutating caller state.
pub fn translate_pointer_fact(
    registry: &ViewRegistry,
    source_window: WindowId,
    snapshot: &ViewSnapshot,
    context: &WinitPointerContext,
    fact: WinitPointerFact,
) -> Result<WinitPointerObservation, PointerTranslationError> {
    let view = registry.view_for_window(source_window).ok_or(
        PointerTranslationError::WindowUnavailable {
            window: source_window,
        },
    )?;
    if view != snapshot.view() {
        return Err(PointerTranslationError::SnapshotViewMismatch {
            window: source_window,
            registered_view: view,
            snapshot_view: snapshot.view(),
        });
    }
    if fact.device_id() != context.source_device {
        return Err(PointerTranslationError::DeviceMismatch {
            view,
            expected: context.source_device,
            observed: fact.device_id(),
        });
    }
    let metrics_revision = snapshot.metrics().revision();
    if metrics_revision != context.state_metrics {
        return Err(PointerTranslationError::StateMetricsMismatch {
            view,
            expected: metrics_revision,
            observed: context.state_metrics,
        });
    }
    if context.state.device() != PointerDeviceKind::Mouse {
        return Err(PointerTranslationError::ContextDeviceKind {
            view,
            observed: context.state.device(),
        });
    }

    let scale_factor = f64::from(snapshot.metrics().metrics().scale_factor().get());
    let (event, converted) = match fact {
        WinitPointerFact::CursorEntered { .. } => pointer_observation(
            view,
            PointerEventKind::Entered,
            context.state.clone(),
            context.state.position().is_some(),
        )?,
        WinitPointerFact::CursorLeft { .. } => pointer_observation(
            view,
            PointerEventKind::Left,
            context.state.clone(),
            context.state.position().is_some(),
        )?,
        WinitPointerFact::CursorMoved {
            physical_position, ..
        } => {
            let position = translate_position(view, physical_position, scale_factor)?;
            pointer_observation(
                view,
                PointerEventKind::Moved,
                context.state.clone().with_position(Some(position)),
                true,
            )?
        }
        WinitPointerFact::MouseButton { button, state, .. } => {
            let button = translate_mouse_button(button);
            let state = translate_button_state(state);
            let buttons = update_button_state(view, context.state.buttons(), button, state)?;
            pointer_observation(
                view,
                PointerEventKind::Button { button, state },
                context.state.clone().with_buttons(buttons),
                context.state.position().is_some(),
            )?
        }
        WinitPointerFact::MouseWheel { delta, phase, .. } => {
            let (delta, precision, delta_converted) =
                translate_scroll_delta(view, delta, scale_factor)?;
            let scroll = ScrollEvent::new(context.state.pointer(), context.state.device(), delta)
                .with_device_id(context.state.device_id())
                .with_position(context.state.position())
                .with_phase(translate_scroll_phase(phase))
                .with_momentum(ScrollMomentumPhase::None)
                .with_precision(precision)
                .with_source(context.state.source())
                .with_modifiers(context.state.modifiers());
            (
                PointerInputEvent::Scroll(scroll),
                delta_converted || context.state.position().is_some(),
            )
        }
    };

    Ok(WinitPointerObservation {
        source_window,
        view,
        metrics: if converted {
            MetricsCitation::converted_using(metrics_revision)
        } else {
            MetricsCitation::NOT_CONVERTED
        },
        event,
    })
}

fn pointer_observation(
    view: ViewId,
    kind: PointerEventKind,
    state: PointerStateSnapshot,
    converted: bool,
) -> Result<(PointerInputEvent, bool), PointerTranslationError> {
    PointerEvent::new(kind, state)
        .map(PointerInputEvent::Pointer)
        .map(|event| (event, converted))
        .map_err(|cause| PointerTranslationError::InvalidPointerEvent { view, cause })
}

fn translate_position(
    view: ViewId,
    position: PhysicalPosition<f64>,
    scale_factor: f64,
) -> Result<PointerPosition, PointerTranslationError> {
    let physical = PhysicalPointerPosition::new(position.x, position.y)
        .map_err(|cause| PointerTranslationError::InvalidPosition { view, cause })?;
    let logical = PointF {
        x: (position.x / scale_factor) as f32,
        y: (position.y / scale_factor) as f32,
    };
    PointerPosition::with_physical(logical, physical)
        .map_err(|cause| PointerTranslationError::InvalidPosition { view, cause })
}

fn update_button_state(
    view: ViewId,
    current: &PointerButtonSet,
    button: PointerButton,
    state: ButtonState,
) -> Result<PointerButtonSet, PointerTranslationError> {
    let mut buttons: Vec<_> = current.iter().collect();
    match (state, buttons.binary_search(&button)) {
        (ButtonState::Pressed, Err(index)) => buttons.insert(index, button),
        (ButtonState::Released, Ok(index)) => {
            buttons.remove(index);
        }
        (ButtonState::Pressed, Ok(_)) | (ButtonState::Released, Err(_)) => {}
    }
    PointerButtonSet::new(buttons)
        .map_err(|cause| PointerTranslationError::InvalidButtonState { view, cause })
}

fn translate_scroll_delta(
    view: ViewId,
    delta: MouseScrollDelta,
    scale_factor: f64,
) -> Result<(ScrollDelta, ScrollPrecision, bool), PointerTranslationError> {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => {
            ScrollDelta::new(f64::from(x), f64::from(y), ScrollUnit::Lines)
                .map(|delta| (delta, ScrollPrecision::Discrete, false))
                .map_err(|cause| PointerTranslationError::InvalidScrollDelta { view, cause })
        }
        MouseScrollDelta::PixelDelta(position) => {
            let physical = PhysicalScrollDelta::new(position.x, position.y)
                .map_err(|cause| PointerTranslationError::InvalidScrollDelta { view, cause })?;
            ScrollDelta::new(
                position.x / scale_factor,
                position.y / scale_factor,
                ScrollUnit::Pixels,
            )
            .and_then(|delta| delta.with_physical_pixels(physical))
            .map(|delta| (delta, ScrollPrecision::Precise, true))
            .map_err(|cause| PointerTranslationError::InvalidScrollDelta { view, cause })
        }
    }
}

const fn translate_button_state(state: ElementState) -> ButtonState {
    match state {
        ElementState::Pressed => ButtonState::Pressed,
        ElementState::Released => ButtonState::Released,
    }
}

const fn translate_scroll_phase(phase: TouchPhase) -> ScrollPhase {
    match phase {
        TouchPhase::Started => ScrollPhase::Began,
        TouchPhase::Moved => ScrollPhase::Changed,
        TouchPhase::Ended => ScrollPhase::Ended,
        TouchPhase::Cancelled => ScrollPhase::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_and_platform_other_buttons_have_distinct_neutral_codes() {
        assert_eq!(
            translate_mouse_button(MouseButton::Left),
            PointerButton::PRIMARY
        );
        assert_eq!(
            translate_mouse_button(MouseButton::Other(1)).platform_other_code(),
            Some(1)
        );
        assert_ne!(
            translate_mouse_button(MouseButton::Other(1)),
            PointerButton::PRIMARY
        );
    }

    #[test]
    fn all_winit_wheel_phases_have_explicit_neutral_meaning() {
        assert_eq!(
            translate_scroll_phase(TouchPhase::Started),
            ScrollPhase::Began
        );
        assert_eq!(
            translate_scroll_phase(TouchPhase::Moved),
            ScrollPhase::Changed
        );
        assert_eq!(
            translate_scroll_phase(TouchPhase::Ended),
            ScrollPhase::Ended
        );
        assert_eq!(
            translate_scroll_phase(TouchPhase::Cancelled),
            ScrollPhase::Cancelled
        );
    }
}
