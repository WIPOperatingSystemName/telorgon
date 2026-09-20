//! Immutable view-scoped platform event envelopes.
//!
//! This module describes facts already produced by a platform adapter. It intentionally owns no
//! native event translation, queue, callback, clock, scheduler, coordinate conversion, coalescing
//! policy, or dispatch policy.

use std::num::NonZeroU64;

use crate::platform::contracts::{EventStamp, MetricsRevision, ViewId};

/// Explicit statement of whether an event's coordinates were converted using view metrics.
///
/// Events without converted coordinates use [`MetricsCitation::NOT_CONVERTED`]. An adapter that
/// converts any coordinate into a view-relative space must instead cite the exact retained metrics
/// revision through [`MetricsCitation::converted_using`]. The citation does not perform or validate
/// a coordinate conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MetricsCitation {
    revision: Option<MetricsRevision>,
}

impl MetricsCitation {
    /// Citation for an event whose payload contains no metrics-derived coordinate conversion.
    pub const NOT_CONVERTED: Self = Self { revision: None };

    /// Cites the exact metrics publication used to convert coordinates in the event payload.
    pub const fn converted_using(revision: MetricsRevision) -> Self {
        Self {
            revision: Some(revision),
        }
    }

    /// Returns the cited revision exactly when coordinate conversion occurred.
    pub const fn revision(self) -> Option<MetricsRevision> {
        self.revision
    }

    /// Reports whether the payload contains coordinates converted using the cited metrics.
    pub const fn conversion_occurred(self) -> bool {
        self.revision.is_some()
    }
}

impl Default for MetricsCitation {
    fn default() -> Self {
        Self::NOT_CONVERTED
    }
}

/// Nonzero number of older events represented by one retained newest event.
///
/// The count excludes the retained event itself. It is diagnostic metadata, not permission to
/// coalesce a particular payload or reorder it relative to other events.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CollapsedEventCount(NonZeroU64);

impl CollapsedEventCount {
    /// Smallest valid collapsed count.
    pub const ONE: Self = Self(NonZeroU64::MIN);

    /// Wraps a host-counted nonzero number of collapsed older events.
    pub const fn new(count: NonZeroU64) -> Self {
        Self(count)
    }

    /// Wraps a raw count, rejecting zero because zero means the event was not coalesced.
    pub const fn from_raw(count: u64) -> Option<Self> {
        match NonZeroU64::new(count) {
            Some(count) => Some(Self(count)),
            None => None,
        }
    }

    /// Returns the number of older events collapsed into the retained event.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl From<CollapsedEventCount> for NonZeroU64 {
    fn from(value: CollapsedEventCount) -> Self {
        value.0
    }
}

/// Ordering evidence attached to a single or coalesced platform event.
///
/// `newest_stamp` is always the stamp of the retained event, never the first event in a collapsed
/// run. A nonzero `collapsed_event_count` records how many older events the adapter replaced after
/// separately determining that coalescing was behavior-preserving.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CoalescingMetadata {
    newest_stamp: EventStamp,
    collapsed_event_count: Option<CollapsedEventCount>,
}

impl CoalescingMetadata {
    /// Describes one event that does not replace any older event.
    pub const fn single(newest_stamp: EventStamp) -> Self {
        Self {
            newest_stamp,
            collapsed_event_count: None,
        }
    }

    /// Describes a retained newest event that replaces the stated nonzero number of older events.
    pub const fn coalesced(
        newest_stamp: EventStamp,
        collapsed_event_count: CollapsedEventCount,
    ) -> Self {
        Self {
            newest_stamp,
            collapsed_event_count: Some(collapsed_event_count),
        }
    }

    /// Returns the retained newest event's complete source/receipt ordering stamp.
    pub const fn newest_stamp(self) -> EventStamp {
        self.newest_stamp
    }

    /// Returns the nonzero collapsed count, or `None` for a single uncoalesced event.
    pub const fn collapsed_event_count(self) -> Option<CollapsedEventCount> {
        self.collapsed_event_count
    }

    /// Returns the number of older events collapsed, including zero for an uncoalesced event.
    pub const fn collapsed_count(self) -> u64 {
        match self.collapsed_event_count {
            Some(count) => count.get(),
            None => 0,
        }
    }

    /// Reports whether this retained event represents any collapsed older events.
    pub const fn is_coalesced(self) -> bool {
        self.collapsed_event_count.is_some()
    }
}

/// Immutable view-scoped platform event carrying an application-selected typed payload.
///
/// `T` lets this neutral crate envelope lifecycle, input, text, accessibility, service, or test
/// payloads without depending on those domains. The host supplies already-issued ordering,
/// conversion, and coalescing facts; constructing this value performs no platform work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformEvent<T> {
    view: ViewId,
    coalescing: CoalescingMetadata,
    metrics: MetricsCitation,
    payload: T,
}

impl<T> PlatformEvent<T> {
    /// Envelopes one uncoalesced event.
    pub const fn new(
        view: ViewId,
        stamp: EventStamp,
        metrics: MetricsCitation,
        payload: T,
    ) -> Self {
        Self::from_coalescing(view, CoalescingMetadata::single(stamp), metrics, payload)
    }

    /// Envelopes an event with explicit adapter-produced coalescing metadata.
    ///
    /// This constructor records a coalescing result; it does not combine events. In particular it
    /// cannot compare or merge different views, payload semantic kinds, or metrics citations. The
    /// adapter's separately reviewed coalescing policy must establish that compatibility before it
    /// supplies this evidence.
    pub const fn from_coalescing(
        view: ViewId,
        coalescing: CoalescingMetadata,
        metrics: MetricsCitation,
        payload: T,
    ) -> Self {
        Self {
            view,
            coalescing,
            metrics,
            payload,
        }
    }

    /// Returns the generation-safe view that owns this event.
    pub const fn view(&self) -> ViewId {
        self.view
    }

    /// Returns the newest retained event stamp.
    pub const fn stamp(&self) -> EventStamp {
        self.coalescing.newest_stamp()
    }

    /// Returns the event's coordinate-conversion citation.
    pub const fn metrics_citation(&self) -> MetricsCitation {
        self.metrics
    }

    /// Returns the cited metrics revision exactly when coordinate conversion occurred.
    pub const fn metrics_revision(&self) -> Option<MetricsRevision> {
        self.metrics.revision()
    }

    /// Returns the adapter-supplied coalescing evidence.
    pub const fn coalescing(&self) -> CoalescingMetadata {
        self.coalescing
    }

    /// Borrows the typed payload without exposing envelope mutation.
    pub const fn payload(&self) -> &T {
        &self.payload
    }

    /// Consumes the immutable envelope and returns its payload.
    pub fn into_payload(self) -> T {
        self.payload
    }

    /// Changes only the payload type while preserving all platform evidence exactly.
    pub fn map_payload<U>(self, map: impl FnOnce(T) -> U) -> PlatformEvent<U> {
        PlatformEvent {
            view: self.view,
            coalescing: self.coalescing,
            metrics: self.metrics,
            payload: map(self.payload),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;

    use crate::platform::contracts::MonotonicInstant;

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct NeutralPayload {
        code: u16,
    }

    fn stamp(sequence: u64, received_at: u64, source_at: Option<u64>) -> EventStamp {
        EventStamp {
            sequence,
            received_at: MonotonicInstant::from_nanos(received_at),
            source_at: source_at.map(MonotonicInstant::from_nanos),
        }
    }

    fn view() -> ViewId {
        ViewId::from_raw(7, 3).unwrap()
    }

    fn assert_wire_value<T: Copy + Eq + Hash + Send + Sync + 'static>() {}

    #[test]
    fn generic_event_preserves_view_stamp_and_typed_payload() {
        let newest = stamp(9, 50, Some(45));
        let event = PlatformEvent::new(
            view(),
            newest,
            MetricsCitation::NOT_CONVERTED,
            NeutralPayload { code: 12 },
        );

        assert_eq!(event.view(), view());
        assert_eq!(event.stamp(), newest);
        assert_eq!(event.payload(), &NeutralPayload { code: 12 });
        assert_eq!(event.metrics_revision(), None);
        assert!(!event.metrics_citation().conversion_occurred());
        assert!(!event.coalescing().is_coalesced());
        assert_eq!(event.coalescing().collapsed_count(), 0);
    }

    #[test]
    fn coordinate_conversion_cites_the_exact_metrics_revision() {
        let revision = MetricsRevision::from_raw(41).unwrap();
        let citation = MetricsCitation::converted_using(revision);
        let event = PlatformEvent::new(view(), stamp(10, 60, None), citation, (3_i16, 8_i16));

        assert!(citation.conversion_occurred());
        assert_eq!(citation.revision(), Some(revision));
        assert_eq!(event.metrics_revision(), Some(revision));
    }

    #[test]
    fn coalescing_retains_the_newest_complete_stamp_and_nonzero_count() {
        assert_eq!(CollapsedEventCount::from_raw(0), None);
        let count = CollapsedEventCount::from_raw(4).unwrap();
        let newest = stamp(25, 800, Some(790));
        let metadata = CoalescingMetadata::coalesced(newest, count);
        let event = PlatformEvent::from_coalescing(
            view(),
            metadata,
            MetricsCitation::NOT_CONVERTED,
            "newest",
        );

        assert_eq!(event.stamp(), newest);
        assert_eq!(event.coalescing().newest_stamp(), newest);
        assert_eq!(event.coalescing().collapsed_event_count(), Some(count));
        assert_eq!(event.coalescing().collapsed_count(), 4);
        assert!(event.coalescing().is_coalesced());
        assert_eq!(count.get(), 4);
        assert_wire_value::<CollapsedEventCount>();
        assert_wire_value::<MetricsCitation>();
        assert_wire_value::<CoalescingMetadata>();
    }

    #[test]
    fn payload_mapping_preserves_every_platform_fact() {
        let revision = MetricsRevision::from_raw(6).unwrap();
        let metadata =
            CoalescingMetadata::coalesced(stamp(30, 900, Some(880)), CollapsedEventCount::ONE);
        let before = PlatformEvent::from_coalescing(
            view(),
            metadata,
            MetricsCitation::converted_using(revision),
            7_u8,
        );
        let after = before.map_payload(|value| u32::from(value) * 2);

        assert_eq!(after.view(), view());
        assert_eq!(after.coalescing(), metadata);
        assert_eq!(after.metrics_revision(), Some(revision));
        assert_eq!(after.into_payload(), 14);
    }
}
