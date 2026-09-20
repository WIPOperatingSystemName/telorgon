//! Immutable post-turn scheduling facts.
//!
//! A runtime produces one [`PostTurnSchedule`] after a bounded owner-thread turn. Managed and
//! embedded hosts interpret the same facts using their own event-loop or frame-scheduler policy.
//! This module does not read a clock, choose native control flow, request redraw, sleep, poll,
//! spawn work, invoke callbacks, or own an event or completion queue.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::platform::contracts::{MonotonicInstant, ViewId};

/// Maximum number of redraw-view identities retained in one scheduling decision.
///
/// This bounds allocation controlled by a host or runtime publication. A caller with more live
/// dirty views must publish the work across later bounded turns rather than construct an
/// unbounded decision.
pub const MAX_REDRAW_VIEWS: usize = 1_024;

/// Independent runtime pipeline stages that still have work after the current turn.
///
/// These are observations, not permission for this package to run any stage. Keeping the stages
/// separate lets an adapter preserve diagnostics and lets an embedding host decide how to arrange
/// its next owner-thread turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RemainingWork {
    update: bool,
    layout: bool,
    semantics: bool,
    scene: bool,
}

impl RemainingWork {
    /// No runtime pipeline stage reports remaining work.
    pub const NONE: Self = Self::new(false, false, false, false);

    /// Records the four independent remaining-work facts.
    pub const fn new(update: bool, layout: bool, semantics: bool, scene: bool) -> Self {
        Self {
            update,
            layout,
            semantics,
            scene,
        }
    }

    pub const fn update(self) -> bool {
        self.update
    }

    pub const fn layout(self) -> bool {
        self.layout
    }

    pub const fn semantics(self) -> bool {
        self.semantics
    }

    pub const fn scene(self) -> bool {
        self.scene
    }

    /// Reports whether at least one runtime pipeline stage still has work.
    pub const fn any(self) -> bool {
        self.update || self.layout || self.semantics || self.scene
    }

    /// Forms the policy-free union of two sets of remaining-work facts.
    pub const fn merged(self, other: Self) -> Self {
        Self::new(
            self.update || other.update,
            self.layout || other.layout,
            self.semantics || other.semantics,
            self.scene || other.scene,
        )
    }
}

/// Host-side progress already pending when a runtime turn ends.
///
/// A pending wake and a pending service completion are deliberately distinct facts. They are
/// represented as presence rather than counts because a scheduling decision only needs to retain
/// whether each class requires service; queue depth and wake coalescing policy belong to the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PendingHostFacts {
    wake: bool,
    service_completions: bool,
}

impl PendingHostFacts {
    /// Neither host-side progress class is currently pending.
    pub const NONE: Self = Self::new(false, false);

    /// Records independent host-wake and service-completion presence.
    pub const fn new(wake: bool, service_completions: bool) -> Self {
        Self {
            wake,
            service_completions,
        }
    }

    pub const fn wake_pending(self) -> bool {
        self.wake
    }

    pub const fn service_completions_pending(self) -> bool {
        self.service_completions
    }

    /// Reports whether either class of host-side progress is pending.
    pub const fn any(self) -> bool {
        self.wake || self.service_completions
    }

    /// Forms the policy-free union of two host-progress observations.
    pub const fn merged(self, other: Self) -> Self {
        Self::new(
            self.wake || other.wake,
            self.service_completions || other.service_completions,
        )
    }
}

/// Validation failure while constructing or combining a post-turn decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScheduleError {
    /// A publication would retain more redraw identities than the neutral boundary permits.
    TooManyRedrawViews { count: usize, maximum: usize },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRedrawViews { count, maximum } => write!(
                formatter,
                "post-turn schedule contains {count} redraw views; maximum is {maximum}"
            ),
        }
    }
}

impl Error for ScheduleError {}

/// Immutable normalized scheduling decision returned after one runtime turn.
///
/// Redraw identities are generation-safe, sorted, unique, and bounded. The optional deadline is
/// an already-computed instant in the host-selected monotonic domain; constructing or inspecting
/// this value never samples that domain. A host remains responsible for mapping these facts into
/// native redraw and wait behavior.
#[must_use = "a post-turn schedule must be interpreted by its managed or embedded host"]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PostTurnSchedule {
    remaining_work: RemainingWork,
    redraw_views: Arc<[ViewId]>,
    next_deadline: Option<MonotonicInstant>,
    pending_host: PendingHostFacts,
}

impl PostTurnSchedule {
    /// Validates and normalizes one scheduling decision.
    ///
    /// The input slice is bounded before this function copies or sorts it. Repeated identities are
    /// then collapsed, and the retained identities are sorted by slot and generation so equivalent
    /// publications compare and iterate deterministically regardless of discovery order.
    pub fn new(
        remaining_work: RemainingWork,
        redraw_views: &[ViewId],
        next_deadline: Option<MonotonicInstant>,
        pending_host: PendingHostFacts,
    ) -> Result<Self, ScheduleError> {
        validate_redraw_count(redraw_views.len())?;

        let mut redraw_views = redraw_views.to_vec();
        redraw_views.sort_unstable();
        redraw_views.dedup();

        Ok(Self {
            remaining_work,
            redraw_views: redraw_views.into(),
            next_deadline,
            pending_host,
        })
    }

    /// Returns the four independent remaining-runtime-work facts.
    pub const fn remaining_work(&self) -> RemainingWork {
        self.remaining_work
    }

    /// Returns the deterministic sorted set of generation-safe redraw-view identities.
    pub fn redraw_views(&self) -> &[ViewId] {
        &self.redraw_views
    }

    /// Reports whether the normalized decision names this exact view generation for redraw.
    pub fn redraw_pending_for(&self, view: ViewId) -> bool {
        self.redraw_views.binary_search(&view).is_ok()
    }

    /// Returns the already-computed next monotonic deadline, if one exists.
    pub const fn next_deadline(&self) -> Option<MonotonicInstant> {
        self.next_deadline
    }

    /// Returns the independent pending host-side progress facts.
    pub const fn pending_host(&self) -> PendingHostFacts {
        self.pending_host
    }

    /// Forms the deterministic policy-free union of two decisions.
    ///
    /// Remaining-work and pending-host facts are ORed, redraw views form a sorted set union, and
    /// the earliest present deadline is retained. The merge fails rather than truncating if its
    /// unique redraw union exceeds [`MAX_REDRAW_VIEWS`].
    pub fn merged(&self, other: &Self) -> Result<Self, ScheduleError> {
        let redraw_views = merge_redraw_views(&self.redraw_views, &other.redraw_views)?;

        Ok(Self {
            remaining_work: self.remaining_work.merged(other.remaining_work),
            redraw_views: redraw_views.into(),
            next_deadline: earliest_deadline(self.next_deadline, other.next_deadline),
            pending_host: self.pending_host.merged(other.pending_host),
        })
    }
}

fn validate_redraw_count(count: usize) -> Result<(), ScheduleError> {
    if count > MAX_REDRAW_VIEWS {
        return Err(ScheduleError::TooManyRedrawViews {
            count,
            maximum: MAX_REDRAW_VIEWS,
        });
    }
    Ok(())
}

fn merge_redraw_views(left: &[ViewId], right: &[ViewId]) -> Result<Vec<ViewId>, ScheduleError> {
    let mut merged = Vec::with_capacity((left.len() + right.len()).min(MAX_REDRAW_VIEWS));
    let (mut left_index, mut right_index) = (0, 0);

    while left_index < left.len() || right_index < right.len() {
        let next = match (left.get(left_index), right.get(right_index)) {
            (Some(left), Some(right)) if left < right => {
                left_index += 1;
                *left
            }
            (Some(left), Some(right)) if right < left => {
                right_index += 1;
                *right
            }
            (Some(left), Some(_)) => {
                left_index += 1;
                right_index += 1;
                *left
            }
            (Some(left), None) => {
                left_index += 1;
                *left
            }
            (None, Some(right)) => {
                right_index += 1;
                *right
            }
            (None, None) => break,
        };

        if merged.len() == MAX_REDRAW_VIEWS {
            return Err(ScheduleError::TooManyRedrawViews {
                count: MAX_REDRAW_VIEWS + 1,
                maximum: MAX_REDRAW_VIEWS,
            });
        }
        merged.push(next);
    }

    Ok(merged)
}

const fn earliest_deadline(
    left: Option<MonotonicInstant>,
    right: Option<MonotonicInstant>,
) -> Option<MonotonicInstant> {
    match (left, right) {
        (Some(left), Some(right)) if left.as_nanos() <= right.as_nanos() => Some(left),
        (Some(_), Some(right)) => Some(right),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;

    use super::*;

    fn view(slot: u32, generation: u32) -> ViewId {
        ViewId::from_raw(slot, generation).unwrap()
    }

    #[test]
    fn decision_preserves_independent_facts_and_normalizes_redraw_views() {
        let work = RemainingWork::new(true, false, true, false);
        let host = PendingHostFacts::new(false, true);
        let schedule = PostTurnSchedule::new(
            work,
            &[view(9, 1), view(2, 3), view(9, 1), view(2, 1)],
            Some(MonotonicInstant::from_nanos(55)),
            host,
        )
        .unwrap();

        assert!(schedule.remaining_work().update());
        assert!(!schedule.remaining_work().layout());
        assert!(schedule.remaining_work().semantics());
        assert!(!schedule.remaining_work().scene());
        assert_eq!(
            schedule.redraw_views(),
            &[view(2, 1), view(2, 3), view(9, 1)]
        );
        assert!(schedule.redraw_pending_for(view(2, 3)));
        assert!(!schedule.redraw_pending_for(view(2, 2)));
        assert_eq!(
            schedule.next_deadline(),
            Some(MonotonicInstant::from_nanos(55))
        );
        assert!(!schedule.pending_host().wake_pending());
        assert!(schedule.pending_host().service_completions_pending());
    }

    #[test]
    fn remaining_and_host_facts_form_policy_free_unions() {
        let left_work = RemainingWork::new(true, false, false, true);
        let right_work = RemainingWork::new(false, true, true, false);
        assert_eq!(
            left_work.merged(right_work),
            RemainingWork::new(true, true, true, true)
        );
        assert!(left_work.any());
        assert!(!RemainingWork::NONE.any());

        let left_host = PendingHostFacts::new(true, false);
        let right_host = PendingHostFacts::new(false, true);
        assert_eq!(
            left_host.merged(right_host),
            PendingHostFacts::new(true, true)
        );
        assert!(left_host.any());
        assert!(!PendingHostFacts::NONE.any());
    }

    #[test]
    fn equivalent_discovery_orders_produce_equal_decisions() {
        let left = PostTurnSchedule::new(
            RemainingWork::NONE,
            &[view(3, 1), view(1, 1), view(2, 1)],
            None,
            PendingHostFacts::NONE,
        )
        .unwrap();
        let right = PostTurnSchedule::new(
            RemainingWork::NONE,
            &[view(2, 1), view(3, 1), view(1, 1)],
            None,
            PendingHostFacts::NONE,
        )
        .unwrap();

        assert_eq!(left, right);
    }

    #[test]
    fn merge_unions_views_and_facts_while_retaining_the_earliest_deadline() {
        let left = PostTurnSchedule::new(
            RemainingWork::new(true, false, false, false),
            &[view(3, 1), view(1, 1)],
            Some(MonotonicInstant::from_nanos(30)),
            PendingHostFacts::new(true, false),
        )
        .unwrap();
        let right = PostTurnSchedule::new(
            RemainingWork::new(false, false, false, true),
            &[view(2, 1), view(3, 1)],
            Some(MonotonicInstant::from_nanos(20)),
            PendingHostFacts::new(false, true),
        )
        .unwrap();

        let merged = left.merged(&right).unwrap();
        assert_eq!(
            merged.remaining_work(),
            RemainingWork::new(true, false, false, true)
        );
        assert_eq!(merged.redraw_views(), &[view(1, 1), view(2, 1), view(3, 1)]);
        assert_eq!(
            merged.next_deadline(),
            Some(MonotonicInstant::from_nanos(20))
        );
        assert_eq!(merged.pending_host(), PendingHostFacts::new(true, true));
    }

    #[test]
    fn absent_deadline_never_hides_a_present_deadline_during_merge() {
        let without = PostTurnSchedule::default();
        let with = PostTurnSchedule::new(
            RemainingWork::NONE,
            &[],
            Some(MonotonicInstant::from_nanos(8)),
            PendingHostFacts::NONE,
        )
        .unwrap();

        assert_eq!(
            without.merged(&with).unwrap().next_deadline(),
            Some(MonotonicInstant::from_nanos(8))
        );
        assert_eq!(
            with.merged(&without).unwrap().next_deadline(),
            Some(MonotonicInstant::from_nanos(8))
        );
    }

    #[test]
    fn construction_rejects_an_oversized_host_slice_before_retention() {
        let redraw_views = vec![view(1, 1); MAX_REDRAW_VIEWS + 1];
        let error = PostTurnSchedule::new(
            RemainingWork::NONE,
            &redraw_views,
            None,
            PendingHostFacts::NONE,
        )
        .unwrap_err();

        assert_eq!(
            error,
            ScheduleError::TooManyRedrawViews {
                count: MAX_REDRAW_VIEWS + 1,
                maximum: MAX_REDRAW_VIEWS,
            }
        );
        assert!(error.to_string().contains("maximum is 1024"));
    }

    #[test]
    fn merge_rejects_a_unique_union_beyond_the_bound_without_truncation() {
        let left_views: Vec<_> = (1..=MAX_REDRAW_VIEWS as u32)
            .map(|slot| view(slot, 1))
            .collect();
        let left = PostTurnSchedule::new(
            RemainingWork::NONE,
            &left_views,
            None,
            PendingHostFacts::NONE,
        )
        .unwrap();
        let right = PostTurnSchedule::new(
            RemainingWork::NONE,
            &[view(MAX_REDRAW_VIEWS as u32 + 1, 1)],
            None,
            PendingHostFacts::NONE,
        )
        .unwrap();

        assert_eq!(
            left.merged(&right),
            Err(ScheduleError::TooManyRedrawViews {
                count: MAX_REDRAW_VIEWS + 1,
                maximum: MAX_REDRAW_VIEWS,
            })
        );
    }

    #[test]
    fn empty_decision_is_a_cloneable_thread_transferable_value() {
        fn assert_value<T: Clone + Eq + Send + Sync + 'static>() {}
        fn assert_fact<T: Copy + Eq + Hash + Send + Sync + 'static>() {}

        assert_value::<PostTurnSchedule>();
        assert_fact::<RemainingWork>();
        assert_fact::<PendingHostFacts>();
        assert_fact::<ScheduleError>();

        let empty = PostTurnSchedule::default();
        assert_eq!(empty.remaining_work(), RemainingWork::NONE);
        assert!(empty.redraw_views().is_empty());
        assert_eq!(empty.next_deadline(), None);
        assert_eq!(empty.pending_host(), PendingHostFacts::NONE);
    }
}
