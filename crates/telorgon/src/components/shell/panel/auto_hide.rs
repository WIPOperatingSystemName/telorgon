//! Clock-free panel auto-hide transition boundary.

use std::fmt;
use std::time::Duration;

use crate::runtime::MonotonicInstant;
use crate::shell::InputSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelAutoHideState {
    Hidden,
    RevealArmed,
    Revealing,
    Shown,
    Hiding,
}

impl PanelAutoHideState {
    pub const fn requires_deadline(self) -> bool {
        matches!(self, Self::RevealArmed | Self::Revealing | Self::Hiding)
    }
}

/// The normalized path that caused an edge reveal. All variants share the same transition rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelRevealSource {
    Mouse,
    Touch,
    Pen,
    Eraser,
    Keyboard,
    Directional,
    Accessibility,
    Programmatic,
}

impl PanelRevealSource {
    pub const fn from_input_source(source: InputSource) -> Self {
        match source {
            InputSource::Mouse => Self::Mouse,
            InputSource::Touch => Self::Touch,
            InputSource::Pen => Self::Pen,
            InputSource::Eraser => Self::Eraser,
            InputSource::Keyboard => Self::Keyboard,
            InputSource::Accessibility => Self::Accessibility,
            InputSource::Programmatic => Self::Programmatic,
        }
    }

    pub const fn input_source(self) -> Option<InputSource> {
        match self {
            Self::Mouse => Some(InputSource::Mouse),
            Self::Touch => Some(InputSource::Touch),
            Self::Pen => Some(InputSource::Pen),
            Self::Eraser => Some(InputSource::Eraser),
            Self::Keyboard => Some(InputSource::Keyboard),
            Self::Directional => None,
            Self::Accessibility => Some(InputSource::Accessibility),
            Self::Programmatic => Some(InputSource::Programmatic),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelAutoHideInput {
    Reveal(PanelRevealSource),
    Conceal,
    DeadlineElapsed,
}

/// Caller-owned state at one monotonic instant. No timer is created by this package.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PanelAutoHideSnapshot {
    state: PanelAutoHideState,
    entered_at: MonotonicInstant,
    deadline: Option<MonotonicInstant>,
}

impl PanelAutoHideSnapshot {
    pub fn new(
        state: PanelAutoHideState,
        entered_at: MonotonicInstant,
        deadline: Option<MonotonicInstant>,
    ) -> Result<Self, PanelAutoHideError> {
        if state.requires_deadline() != deadline.is_some() {
            return Err(PanelAutoHideError::InvalidDeadlineShape { state });
        }
        if deadline.is_some_and(|deadline| deadline < entered_at) {
            return Err(PanelAutoHideError::DeadlineBeforeState);
        }
        Ok(Self {
            state,
            entered_at,
            deadline,
        })
    }

    pub const fn hidden(at: MonotonicInstant) -> Self {
        Self {
            state: PanelAutoHideState::Hidden,
            entered_at: at,
            deadline: None,
        }
    }

    pub const fn state(self) -> PanelAutoHideState {
        self.state
    }

    pub const fn entered_at(self) -> MonotonicInstant {
        self.entered_at
    }

    pub const fn deadline(self) -> Option<MonotonicInstant> {
        self.deadline
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelAutoHidePolicy {
    reveal_arm_delay: Duration,
    reveal_duration: Duration,
    hide_duration: Duration,
}

impl PanelAutoHidePolicy {
    pub const MAX_DURATION: Duration = Duration::from_secs(86_400);

    pub fn new(
        reveal_arm_delay: Duration,
        reveal_duration: Duration,
        hide_duration: Duration,
    ) -> Result<Self, PanelAutoHideError> {
        if reveal_arm_delay > Self::MAX_DURATION
            || reveal_duration.is_zero()
            || reveal_duration > Self::MAX_DURATION
            || hide_duration.is_zero()
            || hide_duration > Self::MAX_DURATION
        {
            return Err(PanelAutoHideError::InvalidDuration);
        }
        Ok(Self {
            reveal_arm_delay,
            reveal_duration,
            hide_duration,
        })
    }

    pub const fn reveal_arm_delay(self) -> Duration {
        self.reveal_arm_delay
    }

    pub const fn reveal_duration(self) -> Duration {
        self.reveal_duration
    }

    pub const fn hide_duration(self) -> Duration {
        self.hide_duration
    }

    pub fn transition(
        self,
        current: PanelAutoHideSnapshot,
        input: PanelAutoHideInput,
        now: MonotonicInstant,
    ) -> Result<PanelAutoHideTransition, PanelAutoHideError> {
        if now < current.entered_at {
            return Err(PanelAutoHideError::TimeMovedBackward);
        }
        let next = match input {
            PanelAutoHideInput::Reveal(_) => match current.state {
                PanelAutoHideState::Hidden => Self::snapshot_with_deadline(
                    PanelAutoHideState::RevealArmed,
                    now,
                    self.reveal_arm_delay,
                )?,
                PanelAutoHideState::Hiding => Self::snapshot_with_deadline(
                    PanelAutoHideState::Revealing,
                    now,
                    self.reveal_duration,
                )?,
                _ => current,
            },
            PanelAutoHideInput::Conceal => match current.state {
                PanelAutoHideState::RevealArmed => PanelAutoHideSnapshot::hidden(now),
                PanelAutoHideState::Revealing | PanelAutoHideState::Shown => {
                    Self::snapshot_with_deadline(
                        PanelAutoHideState::Hiding,
                        now,
                        self.hide_duration,
                    )?
                }
                PanelAutoHideState::Hidden | PanelAutoHideState::Hiding => current,
            },
            PanelAutoHideInput::DeadlineElapsed => {
                let deadline = current
                    .deadline
                    .ok_or(PanelAutoHideError::UnexpectedDeadline)?;
                if now < deadline {
                    return Err(PanelAutoHideError::DeadlineNotReached { deadline, now });
                }
                match current.state {
                    PanelAutoHideState::RevealArmed => Self::snapshot_with_deadline(
                        PanelAutoHideState::Revealing,
                        now,
                        self.reveal_duration,
                    )?,
                    PanelAutoHideState::Revealing => {
                        PanelAutoHideSnapshot::new(PanelAutoHideState::Shown, now, None)?
                    }
                    PanelAutoHideState::Hiding => PanelAutoHideSnapshot::hidden(now),
                    PanelAutoHideState::Hidden | PanelAutoHideState::Shown => {
                        return Err(PanelAutoHideError::UnexpectedDeadline);
                    }
                }
            }
        };
        Ok(PanelAutoHideTransition {
            previous: current,
            next,
            input,
        })
    }

    fn snapshot_with_deadline(
        state: PanelAutoHideState,
        now: MonotonicInstant,
        after: Duration,
    ) -> Result<PanelAutoHideSnapshot, PanelAutoHideError> {
        let deadline = now
            .checked_add(after)
            .ok_or(PanelAutoHideError::DeadlineOverflow)?;
        PanelAutoHideSnapshot::new(state, now, Some(deadline))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PanelAutoHideTransition {
    previous: PanelAutoHideSnapshot,
    next: PanelAutoHideSnapshot,
    input: PanelAutoHideInput,
}

impl PanelAutoHideTransition {
    pub const fn previous(self) -> PanelAutoHideSnapshot {
        self.previous
    }

    pub const fn next(self) -> PanelAutoHideSnapshot {
        self.next
    }

    pub const fn input(self) -> PanelAutoHideInput {
        self.input
    }

    pub fn changed(self) -> bool {
        self.previous.state != self.next.state
            || self.previous.deadline != self.next.deadline
            || self.previous.entered_at != self.next.entered_at
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelAutoHideError {
    InvalidDuration,
    InvalidDeadlineShape {
        state: PanelAutoHideState,
    },
    DeadlineBeforeState,
    TimeMovedBackward,
    UnexpectedDeadline,
    DeadlineNotReached {
        deadline: MonotonicInstant,
        now: MonotonicInstant,
    },
    DeadlineOverflow,
}

impl fmt::Display for PanelAutoHideError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid panel auto-hide transition: {self:?}")
    }
}

impl std::error::Error for PanelAutoHideError {}
