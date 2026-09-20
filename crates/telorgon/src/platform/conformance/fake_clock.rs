//! Manually controlled monotonic clock for deterministic conformance traces.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use crate::platform::contracts::{MonotonicClock, MonotonicInstant};

/// Failure to advance a deterministic clock without violating its domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FakeClockError {
    Regression {
        current: MonotonicInstant,
        requested: MonotonicInstant,
    },
    Overflow {
        current: MonotonicInstant,
        duration: Duration,
    },
}

impl fmt::Display for FakeClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Regression { .. } => "deterministic clock cannot move backward",
            Self::Overflow { .. } => "deterministic clock advance overflowed",
        })
    }
}

impl Error for FakeClockError {}

/// One manually advanced monotonic clock domain.
///
/// Sampling never advances time. Only explicit [`Self::set`] or [`Self::advance`] calls change the
/// observation, and both reject invalid movement atomically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FakeClock {
    current: MonotonicInstant,
}

impl FakeClock {
    pub const fn new(current: MonotonicInstant) -> Self {
        Self { current }
    }

    pub const fn from_nanos(nanos: u64) -> Self {
        Self::new(MonotonicInstant::from_nanos(nanos))
    }

    pub const fn current(self) -> MonotonicInstant {
        self.current
    }

    pub fn set(&mut self, requested: MonotonicInstant) -> Result<(), FakeClockError> {
        if requested < self.current {
            return Err(FakeClockError::Regression {
                current: self.current,
                requested,
            });
        }
        self.current = requested;
        Ok(())
    }

    pub fn advance(&mut self, duration: Duration) -> Result<MonotonicInstant, FakeClockError> {
        let next = self
            .current
            .checked_add(duration)
            .ok_or(FakeClockError::Overflow {
                current: self.current,
                duration,
            })?;
        self.current = next;
        Ok(next)
    }
}

impl MonotonicClock for FakeClock {
    fn now(&mut self) -> MonotonicInstant {
        self.current
    }
}
