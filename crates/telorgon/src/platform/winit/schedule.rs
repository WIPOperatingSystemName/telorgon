//! Pure interpretation of neutral post-turn facts for a future Winit `about_to_wait` callback.

use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};

use crate::platform::contracts::{MonotonicInstant, PostTurnSchedule, ViewId};
use winit::event_loop::ControlFlow;
use winit::window::WindowId;

use crate::platform::winit::ViewRegistry;

/// One generation-checked Winit redraw target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RedrawTarget {
    view: ViewId,
    window: WindowId,
}

impl RedrawTarget {
    /// Returns the exact logical view generation cited by the neutral schedule.
    pub const fn view(self) -> ViewId {
        self.view
    }

    /// Returns the current Winit identity resolved for that logical view.
    pub const fn window(self) -> WindowId {
        self.window
    }
}

/// Explicit same-turn observation of Telorgon's host clock and `std`'s native instant domain.
///
/// The caller must sample both values together from the clock domain used to produce the neutral
/// schedule. This value does not read either clock or infer an origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WinitClockObservation {
    monotonic: MonotonicInstant,
    native: Instant,
}

impl WinitClockObservation {
    /// Records caller-observed instants from the paired clock domains.
    pub const fn new(monotonic: MonotonicInstant, native: Instant) -> Self {
        Self { monotonic, native }
    }

    /// Returns the caller-observed neutral monotonic instant.
    pub const fn monotonic(self) -> MonotonicInstant {
        self.monotonic
    }

    /// Returns the caller-observed native instant.
    pub const fn native(self) -> Instant {
        self.native
    }
}

/// Whether the future application handler must arrange an immediate owner-thread turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WinitWakeIntent {
    /// No immediate owner-thread turn is required.
    NoWake,
    /// The handler must send or otherwise arrange one coalescible wake.
    RequestWake,
    /// The neutral schedule reports that an owner wake is already pending.
    WakeAlreadyPending,
}

impl WinitWakeIntent {
    /// Reports whether the application handler must arrange a new wake.
    pub const fn requires_request(self) -> bool {
        matches!(self, Self::RequestWake)
    }

    /// Reports whether a previously arranged owner wake is already pending.
    pub const fn is_already_pending(self) -> bool {
        matches!(self, Self::WakeAlreadyPending)
    }
}

/// Typed failure while constructing a Winit schedule plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WinitScheduleError {
    /// A redraw cites a stale, retired, or unknown view generation.
    RedrawViewUnavailable {
        /// Unresolvable logical view identity.
        view: ViewId,
    },
    /// The future neutral deadline cannot be represented in the native instant domain.
    NativeDeadlineOverflow {
        /// Explicit neutral observation used as the mapping origin.
        observed: MonotonicInstant,
        /// Future neutral deadline that exceeded native instant range.
        deadline: MonotonicInstant,
    },
}

impl fmt::Display for WinitScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RedrawViewUnavailable { view } => write!(
                formatter,
                "Winit schedule redraw cites stale, retired, or unknown view {view}"
            ),
            Self::NativeDeadlineOverflow { observed, deadline } => write!(
                formatter,
                "neutral deadline {} cannot be represented from observation {} in the native instant domain",
                deadline.as_nanos(),
                observed.as_nanos()
            ),
        }
    }
}

impl Error for WinitScheduleError {}

/// Immutable output for a future Winit `about_to_wait` callback to enact.
///
/// Construction calls no Winit method. The control flow is always [`ControlFlow::Wait`] or
/// [`ControlFlow::WaitUntil`]; continuous [`ControlFlow::Poll`] requires a separate explicit host
/// profile and is never inferred from neutral runtime work.
#[must_use = "a Winit schedule plan must be enacted or explicitly discarded"]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WinitSchedulePlan {
    control_flow: ControlFlow,
    wake_intent: WinitWakeIntent,
    redraw_targets: Vec<RedrawTarget>,
}

impl WinitSchedulePlan {
    /// Returns the Winit wait policy selected from the neutral facts.
    pub const fn control_flow(&self) -> ControlFlow {
        self.control_flow
    }

    /// Returns the independent owner-thread wake intention.
    pub const fn wake_intent(&self) -> WinitWakeIntent {
        self.wake_intent
    }

    /// Returns the bounded, generation-checked redraw targets in neutral schedule order.
    pub fn redraw_targets(&self) -> &[RedrawTarget] {
        &self.redraw_targets
    }

    /// Consumes the plan and returns its generation-checked redraw targets.
    pub fn into_redraw_targets(self) -> Vec<RedrawTarget> {
        self.redraw_targets
    }
}

/// Purely interprets one neutral post-turn schedule for a future Winit callback.
///
/// Redraw views are resolved through the exact current registry generation before a plan is
/// returned. Due deadlines and immediate runtime/host work request an owner wake and keep Winit in
/// `Wait`; a future deadline becomes `WaitUntil`. Redraw targets do not themselves force polling,
/// because the future callback will request redraw on their current windows.
pub fn interpret_schedule(
    schedule: &PostTurnSchedule,
    registry: &ViewRegistry,
    observation: WinitClockObservation,
) -> Result<WinitSchedulePlan, WinitScheduleError> {
    let mut redraw_targets = Vec::with_capacity(schedule.redraw_views().len());
    for &view in schedule.redraw_views() {
        let window = registry
            .window_for_view(view)
            .ok_or(WinitScheduleError::RedrawViewUnavailable { view })?;
        redraw_targets.push(RedrawTarget { view, window });
    }

    let deadline_due = schedule
        .next_deadline()
        .is_some_and(|deadline| deadline <= observation.monotonic);
    let immediate_turn =
        schedule.remaining_work().any() || schedule.pending_host().any() || deadline_due;

    let wake_intent = if !immediate_turn {
        WinitWakeIntent::NoWake
    } else if schedule.pending_host().wake_pending() {
        WinitWakeIntent::WakeAlreadyPending
    } else {
        WinitWakeIntent::RequestWake
    };

    let control_flow = if immediate_turn {
        ControlFlow::Wait
    } else if let Some(deadline) = schedule.next_deadline() {
        let duration = Duration::from_nanos(
            deadline
                .as_nanos()
                .checked_sub(observation.monotonic.as_nanos())
                .expect("non-immediate deadlines are later than the observation"),
        );
        let native_deadline = observation.native.checked_add(duration).ok_or(
            WinitScheduleError::NativeDeadlineOverflow {
                observed: observation.monotonic,
                deadline,
            },
        )?;
        ControlFlow::WaitUntil(native_deadline)
    } else {
        ControlFlow::Wait
    };

    Ok(WinitSchedulePlan {
        control_flow,
        wake_intent,
        redraw_targets,
    })
}
