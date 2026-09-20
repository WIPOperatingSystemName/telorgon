//! Atomic view snapshots and platform-neutral close protocol values.

use std::error::Error;
use std::fmt;
use std::num::NonZeroU64;

use crate::platform::contracts::{
    ActivityState, LifecycleError, NativeSurfaceGeneration, NativeSurfaceState, ViewId,
    ViewLifecycle, ViewLifetime, ViewMetrics, ViewMetricsError, ViewMetricsSnapshot,
    ViewMetricsState, VisibilityState,
};

/// Monotonic revision of one view's complete published snapshot.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewRevision(NonZeroU64);

impl ViewRevision {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    /// Wraps a host-owned nonzero revision.
    pub const fn new(revision: NonZeroU64) -> Self {
        Self(revision)
    }

    /// Wraps a raw revision, rejecting the reserved zero value.
    pub const fn from_raw(revision: u64) -> Option<Self> {
        match NonZeroU64::new(revision) {
            Some(revision) => Some(Self(revision)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub const fn checked_next(self) -> Option<Self> {
        match self.get().checked_add(1) {
            Some(next) => Self::from_raw(next),
            None => None,
        }
    }
}

impl From<ViewRevision> for NonZeroU64 {
    fn from(value: ViewRevision) -> Self {
        value.0
    }
}

impl fmt::Display for ViewRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// One immutable, coherent publication of currently modeled facts for a view.
///
/// Metrics, focus, and environment snapshots will be added by their focused packages. Private
/// fields keep that extension compatible and prevent consumers from constructing invented host
/// truth.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewSnapshot {
    view: ViewId,
    revision: ViewRevision,
    lifetime: ViewLifetime,
    activity: ActivityState,
    visibility: VisibilityState,
    surface: NativeSurfaceState,
    metrics: ViewMetricsSnapshot,
}

impl ViewSnapshot {
    fn capture(
        view: ViewId,
        revision: ViewRevision,
        lifecycle: &ViewLifecycle,
        metrics: &ViewMetricsState,
    ) -> Self {
        Self {
            view,
            revision,
            lifetime: lifecycle.lifetime(),
            activity: lifecycle.activity(),
            visibility: lifecycle.visibility(),
            surface: lifecycle.surface(),
            metrics: metrics.snapshot(),
        }
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn revision(&self) -> ViewRevision {
        self.revision
    }

    pub const fn lifetime(&self) -> ViewLifetime {
        self.lifetime
    }

    pub const fn activity(&self) -> ActivityState {
        self.activity
    }

    pub const fn visibility(&self) -> VisibilityState {
        self.visibility
    }

    pub const fn surface(&self) -> NativeSurfaceState {
        self.surface
    }

    pub const fn metrics(&self) -> &ViewMetricsSnapshot {
        &self.metrics
    }
}

/// One accepted lifecycle observation and its before/after atomic publications.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewUpdate {
    previous: ViewSnapshot,
    current: ViewSnapshot,
}

impl ViewUpdate {
    const fn new(previous: ViewSnapshot, current: ViewSnapshot) -> Self {
        Self { previous, current }
    }

    pub const fn previous(&self) -> &ViewSnapshot {
        &self.previous
    }

    pub const fn current(&self) -> &ViewSnapshot {
        &self.current
    }

    pub const fn is_changed(&self) -> bool {
        self.previous.revision.get() != self.current.revision.get()
    }
}

/// Failure to publish a lifecycle observation for one view.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewStateError {
    Lifecycle(LifecycleError),
    Metrics(ViewMetricsError),
    RevisionExhausted { revision: ViewRevision },
}

impl fmt::Display for ViewStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lifecycle(error) => error.fmt(formatter),
            Self::Metrics(error) => error.fmt(formatter),
            Self::RevisionExhausted { revision } => {
                write!(
                    formatter,
                    "view snapshot revision {revision} cannot advance"
                )
            }
        }
    }
}

impl Error for ViewStateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Lifecycle(error) => Some(error),
            Self::Metrics(error) => Some(error),
            Self::RevisionExhausted { .. } => None,
        }
    }
}

impl From<LifecycleError> for ViewStateError {
    fn from(error: LifecycleError) -> Self {
        Self::Lifecycle(error)
    }
}

impl From<ViewMetricsError> for ViewStateError {
    fn from(error: ViewMetricsError) -> Self {
        Self::Metrics(error)
    }
}

/// Single-view owner that publishes coherent lifecycle snapshots.
///
/// It owns neither a view registry nor any native object, presenter, callback, event loop, or exit
/// policy. Equal observations retain the current revision. A rejected observation or revision
/// exhaustion leaves the lifecycle and publication unchanged.
#[derive(Debug)]
pub struct ViewState {
    view: ViewId,
    revision: ViewRevision,
    lifecycle: ViewLifecycle,
    metrics: ViewMetricsState,
}

impl ViewState {
    pub fn new(view: ViewId) -> Self {
        Self::with_metrics(view, ViewMetrics::default())
    }

    pub fn with_metrics(view: ViewId, metrics: ViewMetrics) -> Self {
        Self {
            view,
            revision: ViewRevision::INITIAL,
            lifecycle: ViewLifecycle::new(),
            metrics: ViewMetricsState::new(metrics),
        }
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn revision(&self) -> ViewRevision {
        self.revision
    }

    pub fn snapshot(&self) -> ViewSnapshot {
        ViewSnapshot::capture(self.view, self.revision, &self.lifecycle, &self.metrics)
    }

    pub fn observe_lifetime(&mut self, next: ViewLifetime) -> Result<ViewUpdate, ViewStateError> {
        let changed = self.lifecycle.validate_lifetime(next)?;
        let next_revision = self.next_revision(changed)?;
        let previous = self.snapshot();
        let transition = self.lifecycle.observe_lifetime(next)?;
        debug_assert_eq!(transition.is_changed(), changed);
        Ok(self.publish(previous, next_revision))
    }

    pub fn observe_activity(&mut self, next: ActivityState) -> Result<ViewUpdate, ViewStateError> {
        let changed = self.lifecycle.validate_activity(next)?;
        let next_revision = self.next_revision(changed)?;
        let previous = self.snapshot();
        let transition = self.lifecycle.observe_activity(next)?;
        debug_assert_eq!(transition.is_changed(), changed);
        Ok(self.publish(previous, next_revision))
    }

    pub fn observe_visibility(
        &mut self,
        next: VisibilityState,
    ) -> Result<ViewUpdate, ViewStateError> {
        let changed = self.lifecycle.validate_visibility(next)?;
        let next_revision = self.next_revision(changed)?;
        let previous = self.snapshot();
        let transition = self.lifecycle.observe_visibility(next)?;
        debug_assert_eq!(transition.is_changed(), changed);
        Ok(self.publish(previous, next_revision))
    }

    pub fn observe_surface_available(
        &mut self,
        generation: NativeSurfaceGeneration,
    ) -> Result<ViewUpdate, ViewStateError> {
        let changed = self.lifecycle.validate_surface_available(generation)?;
        let next_revision = self.next_revision(changed)?;
        let previous = self.snapshot();
        let transition = self.lifecycle.observe_surface_available(generation)?;
        debug_assert_eq!(transition.is_changed(), changed);
        Ok(self.publish(previous, next_revision))
    }

    pub fn observe_surface_unavailable(&mut self) -> Result<ViewUpdate, ViewStateError> {
        let changed = self.lifecycle.validate_surface_unavailable()?;
        let next_revision = self.next_revision(changed)?;
        let previous = self.snapshot();
        let transition = self.lifecycle.observe_surface_unavailable()?;
        debug_assert_eq!(transition.is_changed(), changed);
        Ok(self.publish(previous, next_revision))
    }

    pub fn observe_metrics(&mut self, next: ViewMetrics) -> Result<ViewUpdate, ViewStateError> {
        let (changed, next_metrics_revision) = self.metrics.validate_update(&next)?;
        let next_revision = self.next_revision(changed)?;
        let previous = self.snapshot();
        let metrics_update = self
            .metrics
            .publish_validated(next, changed, next_metrics_revision);
        debug_assert_eq!(metrics_update.is_changed(), changed);
        Ok(self.publish(previous, next_revision))
    }

    fn next_revision(&self, changed: bool) -> Result<ViewRevision, ViewStateError> {
        if !changed {
            return Ok(self.revision);
        }
        self.revision
            .checked_next()
            .ok_or(ViewStateError::RevisionExhausted {
                revision: self.revision,
            })
    }

    fn publish(&mut self, previous: ViewSnapshot, next_revision: ViewRevision) -> ViewUpdate {
        self.revision = next_revision;
        ViewUpdate::new(previous, self.snapshot())
    }
}

/// Source of a cancellable close request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CloseRequestReason {
    User,
    System,
    Application,
}

/// Cancellable close request routed against an exact view publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CloseRequest {
    view: ViewId,
    observed_revision: ViewRevision,
    reason: CloseRequestReason,
}

impl CloseRequest {
    pub const fn from_snapshot(snapshot: &ViewSnapshot, reason: CloseRequestReason) -> Self {
        Self {
            view: snapshot.view,
            observed_revision: snapshot.revision,
            reason,
        }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn observed_revision(self) -> ViewRevision {
        self.observed_revision
    }

    pub const fn reason(self) -> CloseRequestReason {
        self.reason
    }
}

/// Application response to a cancellable [`CloseRequest`].
///
/// This value does not close a native object or mutate a view snapshot. The host adapter owns
/// execution and later observed truth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CloseRequestDecision {
    Accept,
    Reject,
    Defer,
}

/// Host-enforced native-destruction phase that cannot receive a close decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForcedDestructionPhase {
    Destroying,
    Destroyed,
}

/// Unanswerable destruction notification against an exact view publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ForcedDestruction {
    view: ViewId,
    observed_revision: ViewRevision,
    phase: ForcedDestructionPhase,
}

impl ForcedDestruction {
    pub const fn from_snapshot(snapshot: &ViewSnapshot, phase: ForcedDestructionPhase) -> Self {
        Self {
            view: snapshot.view,
            observed_revision: snapshot.revision,
            phase,
        }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn observed_revision(self) -> ViewRevision {
        self.observed_revision
    }

    pub const fn phase(self) -> ForcedDestructionPhase {
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;

    use crate::foundation::SizeF;

    use super::*;

    fn view() -> ViewId {
        ViewId::from_raw(7, 2).unwrap()
    }

    fn generation(value: u64) -> NativeSurfaceGeneration {
        NativeSurfaceGeneration::from_raw(value).unwrap()
    }

    fn assert_value<T: Copy + Eq + Hash + Send + Sync + 'static>() {}

    fn assert_snapshot<T: Clone + PartialEq + Send + Sync + 'static>() {}

    #[test]
    fn initial_snapshot_is_coherent_and_private_host_truth() {
        let state = ViewState::new(view());
        let snapshot = state.snapshot();
        assert_eq!(snapshot.view(), view());
        assert_eq!(snapshot.revision(), ViewRevision::INITIAL);
        assert_eq!(snapshot.lifetime(), ViewLifetime::Declared);
        assert_eq!(snapshot.activity(), ActivityState::Inactive);
        assert_eq!(snapshot.visibility(), VisibilityState::Hidden);
        assert_eq!(snapshot.surface(), NativeSurfaceState::Unavailable);
        assert!(!snapshot.metrics().is_renderable());
        assert_snapshot::<ViewSnapshot>();
        assert_snapshot::<ViewUpdate>();
        assert_value::<ViewRevision>();
    }

    #[test]
    fn accepted_changes_publish_one_revision_and_redundant_observations_publish_none() {
        let mut state = ViewState::new(view());
        let live = state.observe_lifetime(ViewLifetime::Live).unwrap();
        assert!(live.is_changed());
        assert_eq!(live.previous().revision().get(), 1);
        assert_eq!(live.current().revision().get(), 2);
        assert_eq!(live.current().lifetime(), ViewLifetime::Live);

        let redundant = state.observe_lifetime(ViewLifetime::Live).unwrap();
        assert!(!redundant.is_changed());
        assert_eq!(redundant.previous(), redundant.current());

        let activity = state.observe_activity(ActivityState::Active).unwrap();
        assert_eq!(activity.current().revision().get(), 3);
        assert_eq!(activity.current().activity(), ActivityState::Active);
        assert_eq!(activity.current().lifetime(), ViewLifetime::Live);
    }

    #[test]
    fn invalid_lifecycle_and_revision_exhaustion_are_atomic() {
        let mut state = ViewState::new(view());
        state.observe_lifetime(ViewLifetime::Live).unwrap();
        let before_invalid = state.snapshot();
        assert_eq!(
            state.observe_lifetime(ViewLifetime::Declared),
            Err(ViewStateError::Lifecycle(
                LifecycleError::InvalidLifetimeTransition {
                    from: ViewLifetime::Live,
                    to: ViewLifetime::Declared,
                }
            ))
        );
        assert_eq!(state.snapshot(), before_invalid);

        state.revision = ViewRevision::from_raw(u64::MAX).unwrap();
        let before_exhaustion = state.snapshot();
        assert_eq!(
            state.observe_visibility(VisibilityState::Visible),
            Err(ViewStateError::RevisionExhausted {
                revision: ViewRevision::from_raw(u64::MAX).unwrap(),
            })
        );
        assert_eq!(state.snapshot(), before_exhaustion);
        assert!(
            !state
                .observe_activity(ActivityState::Inactive)
                .unwrap()
                .is_changed()
        );
    }

    #[test]
    fn each_surface_fact_is_published_atomically_with_the_other_axes() {
        let mut state = ViewState::new(view());
        state.observe_lifetime(ViewLifetime::Live).unwrap();
        state.observe_activity(ActivityState::Suspended).unwrap();
        state.observe_visibility(VisibilityState::Hidden).unwrap();
        let available = state.observe_surface_available(generation(4)).unwrap();

        assert_eq!(available.current().activity(), ActivityState::Suspended);
        assert_eq!(available.current().visibility(), VisibilityState::Hidden);
        assert_eq!(
            available.current().surface(),
            NativeSurfaceState::Available {
                generation: generation(4),
            }
        );

        let retired = state.observe_surface_unavailable().unwrap();
        assert_eq!(retired.current().surface(), NativeSurfaceState::Unavailable);
        assert_eq!(
            retired.current().revision().get(),
            available.current().revision().get() + 1
        );
    }

    #[test]
    fn metrics_and_enclosing_view_revisions_publish_atomically() {
        let mut state = ViewState::new(view());
        let metrics = ViewMetrics::new(
            crate::platform::contracts::PhysicalExtent::new(800, 600),
            crate::platform::contracts::ScaleFactor::new(2.0).unwrap(),
            crate::platform::contracts::DisplayProperties::default(),
        )
        .unwrap();
        let update = state.observe_metrics(metrics.clone()).unwrap();
        assert_eq!(update.current().revision().get(), 2);
        assert_eq!(update.current().metrics().revision().get(), 2);
        assert_eq!(
            update.current().metrics().metrics().logical_extent(),
            SizeF {
                width: 400.0,
                height: 300.0,
            }
        );

        let redundant = state.observe_metrics(metrics).unwrap();
        assert!(!redundant.is_changed());
        assert_eq!(redundant.previous(), redundant.current());

        state.revision = ViewRevision::from_raw(u64::MAX).unwrap();
        let before_exhaustion = state.snapshot();
        assert_eq!(
            state.observe_metrics(ViewMetrics::default()),
            Err(ViewStateError::RevisionExhausted {
                revision: ViewRevision::from_raw(u64::MAX).unwrap(),
            })
        );
        assert_eq!(state.snapshot(), before_exhaustion);
    }

    #[test]
    fn cancellable_requests_and_forced_destruction_are_distinct_values() {
        let state = ViewState::new(view());
        let snapshot = state.snapshot();
        let request = CloseRequest::from_snapshot(&snapshot, CloseRequestReason::User);
        let destruction =
            ForcedDestruction::from_snapshot(&snapshot, ForcedDestructionPhase::Destroying);

        assert_eq!(request.view(), destruction.view());
        assert_eq!(request.observed_revision(), destruction.observed_revision());
        assert_eq!(request.reason(), CloseRequestReason::User);
        assert_eq!(destruction.phase(), ForcedDestructionPhase::Destroying);
        let decisions = [
            CloseRequestDecision::Accept,
            CloseRequestDecision::Reject,
            CloseRequestDecision::Defer,
        ];
        assert_eq!(decisions.len(), 3);
        assert_value::<CloseRequest>();
        assert_value::<ForcedDestruction>();
    }
}
