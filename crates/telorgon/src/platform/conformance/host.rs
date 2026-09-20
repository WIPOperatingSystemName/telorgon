//! Deterministic composition of fake time, canonical views, stamps, and bounded events.

use std::error::Error;
use std::fmt;
use std::num::NonZeroU16;

use crate::platform::contracts::{
    EventStamp, EventStampError, EventStampStream, MetricsCitation, MonotonicInstant,
    PlatformEvent, ViewId,
};

use crate::platform::conformance::{
    BoundedCapture, CaptureLimitError, FakeClock, ViewDriver, ViewDriverLimitError,
};

/// Invalid capacity supplied while constructing a deterministic host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostLimitError {
    Views(ViewDriverLimitError),
    Events(CaptureLimitError),
}

impl fmt::Display for HostLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Views(_) => "deterministic host view limits are invalid",
            Self::Events(_) => "deterministic host event limit is invalid",
        })
    }
}

impl Error for HostLimitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Views(error) => Some(error),
            Self::Events(error) => Some(error),
        }
    }
}

impl From<ViewDriverLimitError> for HostLimitError {
    fn from(error: ViewDriverLimitError) -> Self {
        Self::Views(error)
    }
}

impl From<CaptureLimitError> for HostLimitError {
    fn from(error: CaptureLimitError) -> Self {
        Self::Events(error)
    }
}

/// Payload-free classification of a rejected deterministic event emission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostEmitErrorKind {
    ViewUnavailable { view: ViewId },
    EventCaptureFull { maximum: NonZeroU16 },
    Stamp(EventStampError),
}

/// Rejected event emission that returns ownership of its application-selected payload.
pub struct HostEmitError<T> {
    kind: HostEmitErrorKind,
    payload: T,
}

impl<T> HostEmitError<T> {
    pub const fn kind(&self) -> HostEmitErrorKind {
        self.kind
    }

    pub fn into_payload(self) -> T {
        self.payload
    }

    pub fn into_parts(self) -> (HostEmitErrorKind, T) {
        (self.kind, self.payload)
    }
}

impl<T> fmt::Debug for HostEmitError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostEmitError")
            .field("kind", &self.kind)
            .field("payload_redacted", &true)
            .finish()
    }
}

impl<T> fmt::Display for HostEmitError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            HostEmitErrorKind::ViewUnavailable { .. } => {
                "deterministic event cites an unavailable view"
            }
            HostEmitErrorKind::EventCaptureFull { .. } => "deterministic event capture is full",
            HostEmitErrorKind::Stamp(_) => "deterministic event stamp was rejected",
        })
    }
}

impl<T> Error for HostEmitError<T> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self.kind {
            HostEmitErrorKind::Stamp(ref error) => Some(error),
            _ => None,
        }
    }
}

/// Native-free deterministic platform host for one application-selected event payload type.
///
/// Event capacity and view existence are checked before the stamp stream advances. Therefore a
/// rejected emission returns its payload and leaves the entire trace unchanged.
#[derive(Debug)]
pub struct DeterministicHost<T> {
    clock: FakeClock,
    stamps: EventStampStream,
    views: ViewDriver,
    events: BoundedCapture<PlatformEvent<T>>,
}

impl<T> DeterministicHost<T> {
    pub fn new(
        initial_time: MonotonicInstant,
        maximum_views: NonZeroU16,
        maximum_view_updates: NonZeroU16,
        maximum_events: NonZeroU16,
    ) -> Result<Self, HostLimitError> {
        Ok(Self {
            clock: FakeClock::new(initial_time),
            stamps: EventStampStream::new(),
            views: ViewDriver::new(maximum_views, maximum_view_updates)?,
            events: BoundedCapture::new(maximum_events)?,
        })
    }

    pub const fn clock(&self) -> &FakeClock {
        &self.clock
    }

    pub const fn clock_mut(&mut self) -> &mut FakeClock {
        &mut self.clock
    }

    pub const fn views(&self) -> &ViewDriver {
        &self.views
    }

    pub const fn views_mut(&mut self) -> &mut ViewDriver {
        &mut self.views
    }

    pub const fn events(&self) -> &BoundedCapture<PlatformEvent<T>> {
        &self.events
    }

    pub const fn events_mut(&mut self) -> &mut BoundedCapture<PlatformEvent<T>> {
        &mut self.events
    }

    pub const fn last_stamp(&self) -> Option<EventStamp> {
        self.stamps.last()
    }

    pub fn emit(
        &mut self,
        view: ViewId,
        metrics: MetricsCitation,
        source_at: Option<MonotonicInstant>,
        payload: T,
    ) -> Result<EventStamp, HostEmitError<T>> {
        if !self.views.contains(view) {
            return Err(HostEmitError {
                kind: HostEmitErrorKind::ViewUnavailable { view },
                payload,
            });
        }
        if self.events.is_full() {
            return Err(HostEmitError {
                kind: HostEmitErrorKind::EventCaptureFull {
                    maximum: self.events.capacity(),
                },
                payload,
            });
        }
        let stamp = match self.stamps.stamp(self.clock.current(), source_at) {
            Ok(stamp) => stamp,
            Err(error) => {
                return Err(HostEmitError {
                    kind: HostEmitErrorKind::Stamp(error),
                    payload,
                });
            }
        };
        self.events
            .push(PlatformEvent::new(view, stamp, metrics, payload))
            .expect("event capacity was checked before advancing the stamp stream");
        Ok(stamp)
    }
}
