//! Clock-domain-neutral monotonic time values.

use std::time::Duration;

/// An instant in a monotonic clock domain supplied by a host.
///
/// This value does not read a clock and does not establish an origin. A host chooses one clock
/// domain and injects monotonically nondecreasing nanosecond values wherever Telorgon needs time.
/// Values issued by different host clock domains are not comparable.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonotonicInstant(u64);

impl MonotonicInstant {
    pub const ZERO: Self = Self(0);

    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        let nanos = u64::try_from(duration.as_nanos()).ok()?;
        self.0.checked_add(nanos).map(Self)
    }

    pub fn saturating_add(self, duration: Duration) -> Self {
        let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        Self(self.0.saturating_add(nanos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_is_explicit_and_never_reads_a_clock() {
        let instant = MonotonicInstant::from_nanos(7);
        assert_eq!(instant.as_nanos(), 7);
        assert_eq!(
            instant.checked_add(Duration::from_nanos(5)),
            Some(MonotonicInstant::from_nanos(12))
        );
        assert_eq!(
            MonotonicInstant::from_nanos(u64::MAX).checked_add(Duration::from_nanos(1)),
            None
        );
        assert_eq!(
            MonotonicInstant::from_nanos(u64::MAX - 1).saturating_add(Duration::from_nanos(2)),
            MonotonicInstant::from_nanos(u64::MAX)
        );
    }
}
