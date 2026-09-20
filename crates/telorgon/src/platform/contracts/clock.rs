//! Host-injected monotonic clock sampling.
//!
//! This module defines a clock boundary; it does not provide an ambient clock. Managed hosts may
//! adapt their event-loop clock, embedded hosts may adapt an engine clock, and deterministic hosts
//! may provide a manually controlled implementation. No implementation here reads wall time or
//! [`std::time::Instant`], schedules work, sleeps, or creates a timer or thread.

use std::error::Error;
use std::fmt;

pub use crate::foundation::MonotonicInstant;

/// A host-owned source of instants from one monotonic clock domain.
///
/// All values returned by one implementation instance must belong to the same domain. Values from
/// different instances are comparable only when the host knows that both instances adapt the exact
/// same underlying domain; Telorgon cannot infer or validate that relationship from
/// [`MonotonicInstant`] alone.
///
/// The receiver is mutable so deterministic implementations may advance a scripted observation
/// stream without interior mutability. The trait does not require `Send` or `Sync`: the managed or
/// embedded host decides which thread owns its clock and serializes access according to its runtime
/// ownership model.
pub trait MonotonicClock {
    /// Samples the host-selected monotonic clock.
    ///
    /// Implementations must not substitute wall-clock time. Consecutive values may be equal when
    /// the underlying clock has coarser resolution than the caller's sampling cadence.
    fn now(&mut self) -> MonotonicInstant;
}

/// A clock source returned a value earlier than its last accepted observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MonotonicClockError {
    previous: MonotonicInstant,
    observed: MonotonicInstant,
}

impl MonotonicClockError {
    pub const fn previous(self) -> MonotonicInstant {
        self.previous
    }

    pub const fn observed(self) -> MonotonicInstant {
        self.observed
    }
}

impl fmt::Display for MonotonicClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "host monotonic clock regressed from {}ns to {}ns",
            self.previous.as_nanos(),
            self.observed.as_nanos()
        )
    }
}

impl Error for MonotonicClockError {}

/// Non-cloneable checked owner for one host clock source and its domain.
///
/// Construction binds this owner to the supplied source. Callers must not replace the source with
/// an implementation backed by another clock domain through [`Self::clock_mut`]. The owner accepts
/// equal observations, rejects regressions, and updates [`Self::last_observed`] only after a sample
/// passes validation.
///
/// This type deliberately performs no synchronization. It belongs to the host-selected runtime
/// owner, which may be a managed application thread, an embedded engine thread, or a deterministic
/// test driver.
#[derive(Debug)]
pub struct MonotonicClockState<C> {
    clock: C,
    last_observed: Option<MonotonicInstant>,
}

impl<C> MonotonicClockState<C> {
    /// Binds a validation owner to one clock source and its monotonic domain.
    pub const fn new(clock: C) -> Self {
        Self {
            clock,
            last_observed: None,
        }
    }

    /// Returns the source without sampling it.
    pub const fn clock(&self) -> &C {
        &self.clock
    }

    /// Returns mutable access to the same source without sampling it.
    ///
    /// This supports manually advancing a deterministic clock. The source must continue to use the
    /// domain established at construction; assigning a different-domain source would make retained
    /// observations incomparable.
    pub const fn clock_mut(&mut self) -> &mut C {
        &mut self.clock
    }

    /// Returns the last accepted observation, or `None` before the first successful sample.
    pub const fn last_observed(&self) -> Option<MonotonicInstant> {
        self.last_observed
    }

    /// Ends validation ownership and returns the host clock source.
    pub fn into_clock(self) -> C {
        self.clock
    }
}

impl<C: MonotonicClock> MonotonicClockState<C> {
    /// Samples and validates the bound host clock.
    ///
    /// Equal observations are accepted. If the source regresses, the error reports both values and
    /// the retained last observation remains unchanged.
    pub fn observe_now(&mut self) -> Result<MonotonicInstant, MonotonicClockError> {
        let observed = self.clock.now();
        if let Some(previous) = self.last_observed
            && observed < previous
        {
            return Err(MonotonicClockError { previous, observed });
        }

        self.last_observed = Some(observed);
        Ok(observed)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    #[derive(Debug)]
    struct ScriptedClock {
        samples: VecDeque<MonotonicInstant>,
    }

    impl ScriptedClock {
        fn new(samples: impl IntoIterator<Item = u64>) -> Self {
            Self {
                samples: samples
                    .into_iter()
                    .map(MonotonicInstant::from_nanos)
                    .collect(),
            }
        }
    }

    impl MonotonicClock for ScriptedClock {
        fn now(&mut self) -> MonotonicInstant {
            self.samples.pop_front().expect("scripted clock exhausted")
        }
    }

    #[derive(Debug)]
    struct ManualClock {
        now: MonotonicInstant,
    }

    impl ManualClock {
        fn set(&mut self, nanos: u64) {
            self.now = MonotonicInstant::from_nanos(nanos);
        }
    }

    impl MonotonicClock for ManualClock {
        fn now(&mut self) -> MonotonicInstant {
            self.now
        }
    }

    #[test]
    fn checked_owner_accepts_initial_advancing_and_equal_samples() {
        let mut clock = MonotonicClockState::new(ScriptedClock::new([5, 5, 8]));

        assert_eq!(clock.last_observed(), None);
        assert_eq!(clock.observe_now().unwrap().as_nanos(), 5);
        assert_eq!(clock.observe_now().unwrap().as_nanos(), 5);
        assert_eq!(clock.observe_now().unwrap().as_nanos(), 8);
        assert_eq!(clock.last_observed(), Some(MonotonicInstant::from_nanos(8)));
    }

    #[test]
    fn regression_is_typed_and_leaves_the_last_observation_unchanged() {
        let mut clock = MonotonicClockState::new(ScriptedClock::new([10, 9, 11]));
        assert_eq!(clock.observe_now().unwrap().as_nanos(), 10);

        let error = clock.observe_now().unwrap_err();
        assert_eq!(error.previous(), MonotonicInstant::from_nanos(10));
        assert_eq!(error.observed(), MonotonicInstant::from_nanos(9));
        assert_eq!(
            clock.last_observed(),
            Some(MonotonicInstant::from_nanos(10))
        );

        assert_eq!(clock.observe_now().unwrap().as_nanos(), 11);
        assert_eq!(
            clock.last_observed(),
            Some(MonotonicInstant::from_nanos(11))
        );
    }

    #[test]
    fn deterministic_host_can_advance_its_bound_source_explicitly() {
        let source = ManualClock {
            now: MonotonicInstant::ZERO,
        };
        let mut clock = MonotonicClockState::new(source);

        assert_eq!(clock.observe_now().unwrap(), MonotonicInstant::ZERO);
        clock.clock_mut().set(25);
        assert_eq!(clock.observe_now().unwrap().as_nanos(), 25);

        let source = clock.into_clock();
        assert_eq!(source.now.as_nanos(), 25);
    }

    #[test]
    fn clock_boundary_is_object_safe_without_threading_requirements() {
        fn sample(clock: &mut dyn MonotonicClock) -> MonotonicInstant {
            clock.now()
        }

        let mut clock = ManualClock {
            now: MonotonicInstant::from_nanos(7),
        };
        assert_eq!(sample(&mut clock).as_nanos(), 7);
    }
}
