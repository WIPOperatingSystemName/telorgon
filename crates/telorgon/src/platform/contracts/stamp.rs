//! Monotonic platform-event stamps and per-host-stream sequencing.

use std::error::Error;
use std::fmt;

pub use crate::foundation::MonotonicInstant;

/// Ordering and timing attached to one platform-to-runtime event.
///
/// `sequence` is meaningful only within the [`EventStampStream`] that issued it. `received_at`
/// and `source_at`, when present, belong to the same host-selected monotonic clock domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EventStamp {
    pub sequence: u64,
    pub received_at: MonotonicInstant,
    pub source_at: Option<MonotonicInstant>,
}

/// Failure to issue a valid event stamp without violating stream ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventStampError {
    /// Every sequence value in the stream has already been issued.
    SequenceExhausted,
    /// The injected receipt time moved backward within one host stream.
    ReceiptTimeRegressed {
        previous: MonotonicInstant,
        received: MonotonicInstant,
    },
    /// A purported source timestamp occurs after the host receipt timestamp.
    SourceTimeAfterReceipt {
        source: MonotonicInstant,
        received: MonotonicInstant,
    },
}

impl fmt::Display for EventStampError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::SequenceExhausted => formatter.write_str("platform event sequence exhausted"),
            Self::ReceiptTimeRegressed { previous, received } => write!(
                formatter,
                "platform event receipt time regressed from {}ns to {}ns",
                previous.as_nanos(),
                received.as_nanos()
            ),
            Self::SourceTimeAfterReceipt { source, received } => write!(
                formatter,
                "platform event source time {}ns is after receipt time {}ns",
                source.as_nanos(),
                received.as_nanos()
            ),
        }
    }
}

impl Error for EventStampError {}

/// Strict sequence owner for one platform host event stream.
///
/// The stream owns no clock, queue, timer, event loop, or native timestamp conversion. Callers
/// inject already-mapped monotonic instants. One stream must not be shared by unrelated host event
/// streams, and it is intentionally not cloneable because cloning could issue duplicate sequence
/// numbers.
#[derive(Debug, Default)]
pub struct EventStampStream {
    last: Option<EventStamp>,
}

impl EventStampStream {
    /// Creates an empty stream. Its first accepted event receives sequence `1`.
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// Returns the last accepted stamp without advancing the stream.
    pub const fn last(&self) -> Option<EventStamp> {
        self.last
    }

    /// Issues the next stamp using host-injected monotonic times.
    ///
    /// Equal consecutive receipt times are valid because sequence, rather than timestamp
    /// resolution, supplies strict ordering. A failed call leaves the stream unchanged.
    pub fn stamp(
        &mut self,
        received_at: MonotonicInstant,
        source_at: Option<MonotonicInstant>,
    ) -> Result<EventStamp, EventStampError> {
        if let Some(source) = source_at
            && source > received_at
        {
            return Err(EventStampError::SourceTimeAfterReceipt {
                source,
                received: received_at,
            });
        }

        let sequence = match self.last {
            Some(last) => {
                if received_at < last.received_at {
                    return Err(EventStampError::ReceiptTimeRegressed {
                        previous: last.received_at,
                        received: received_at,
                    });
                }
                last.sequence
                    .checked_add(1)
                    .ok_or(EventStampError::SequenceExhausted)?
            }
            None => 1,
        };

        let stamp = EventStamp {
            sequence,
            received_at,
            source_at,
        };
        self.last = Some(stamp);
        Ok(stamp)
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;

    use super::*;

    fn at(nanos: u64) -> MonotonicInstant {
        MonotonicInstant::from_nanos(nanos)
    }

    fn assert_wire_value<T: Copy + Eq + Hash + Send + Sync + 'static>() {}

    #[test]
    fn stream_assigns_strict_sequences_even_when_clock_resolution_ties() {
        let mut stream = EventStampStream::new();
        let first = stream.stamp(at(10), None).unwrap();
        let second = stream.stamp(at(10), Some(at(9))).unwrap();
        let third = stream.stamp(at(11), Some(at(11))).unwrap();

        assert_eq!([first.sequence, second.sequence, third.sequence], [1, 2, 3]);
        assert_eq!(first.source_at, None);
        assert_eq!(second.source_at, Some(at(9)));
        assert_eq!(stream.last(), Some(third));
        assert_wire_value::<EventStamp>();
    }

    #[test]
    fn invalid_injected_times_are_rejected_without_advancing_the_stream() {
        let mut stream = EventStampStream::new();
        let first = stream.stamp(at(10), Some(at(8))).unwrap();

        assert_eq!(
            stream.stamp(at(9), None),
            Err(EventStampError::ReceiptTimeRegressed {
                previous: at(10),
                received: at(9),
            })
        );
        assert_eq!(stream.last(), Some(first));

        assert_eq!(
            stream.stamp(at(11), Some(at(12))),
            Err(EventStampError::SourceTimeAfterReceipt {
                source: at(12),
                received: at(11),
            })
        );
        assert_eq!(stream.last(), Some(first));

        assert_eq!(stream.stamp(at(11), None).unwrap().sequence, 2);
    }

    #[test]
    fn sequence_exhaustion_is_explicit_and_does_not_wrap() {
        let exhausted = EventStamp {
            sequence: u64::MAX,
            received_at: at(20),
            source_at: None,
        };
        let mut stream = EventStampStream {
            last: Some(exhausted),
        };

        assert_eq!(
            stream.stamp(at(21), None),
            Err(EventStampError::SequenceExhausted)
        );
        assert_eq!(stream.last(), Some(exhausted));
    }
}
