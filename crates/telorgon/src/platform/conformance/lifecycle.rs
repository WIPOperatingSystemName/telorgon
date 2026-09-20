//! Bounded multi-view lifecycle driver over canonical `telorgon-platform` state owners.

use std::error::Error;
use std::fmt;
use std::num::NonZeroU16;

use crate::platform::contracts::{
    ActivityState, NativeSurfaceGeneration, ViewId, ViewLifetime, ViewMetrics, ViewSnapshot,
    ViewState, ViewStateError, ViewUpdate, VisibilityState,
};

use crate::platform::conformance::{BoundedCapture, CaptureLimitError};

/// Neutral hard bound on views retained by one deterministic driver.
pub const MAX_CONFORMANCE_VIEWS: u16 = 64;

/// One explicit host observation applied to a canonical view-state owner.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewObservation {
    Lifetime(ViewLifetime),
    Activity(ActivityState),
    Visibility(VisibilityState),
    SurfaceAvailable(NativeSurfaceGeneration),
    SurfaceUnavailable,
    Metrics(ViewMetrics),
}

/// Invalid construction limits for a lifecycle driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewDriverLimitError {
    ViewLimitTooLarge { requested: u16, maximum: u16 },
    UpdateCapture(CaptureLimitError),
}

impl fmt::Display for ViewDriverLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ViewLimitTooLarge { .. } => "conformance view capacity exceeds the hard bound",
            Self::UpdateCapture(_) => "conformance view-update capture limit is invalid",
        })
    }
}

impl Error for ViewDriverLimitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ViewLimitTooLarge { .. } => None,
            Self::UpdateCapture(error) => Some(error),
        }
    }
}

impl From<CaptureLimitError> for ViewDriverLimitError {
    fn from(error: CaptureLimitError) -> Self {
        Self::UpdateCapture(error)
    }
}

/// Failure to mutate a deterministic view registry and its bounded trace atomically.
#[derive(Debug, PartialEq)]
pub enum ViewDriverError {
    DuplicateView { view: ViewId },
    ViewCapacityReached { maximum: NonZeroU16 },
    ViewUnavailable { view: ViewId },
    UpdateCaptureFull { maximum: NonZeroU16 },
    State { view: ViewId, error: ViewStateError },
}

impl fmt::Display for ViewDriverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DuplicateView { .. } => "deterministic host already contains the view",
            Self::ViewCapacityReached { .. } => "deterministic host view capacity was reached",
            Self::ViewUnavailable { .. } => "deterministic host view is unavailable",
            Self::UpdateCaptureFull { .. } => "deterministic view-update capture is full",
            Self::State { .. } => "canonical view-state observation was rejected",
        })
    }
}

impl Error for ViewDriverError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::State { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Bounded owner of independent canonical view states and their ordered observation results.
///
/// Capture capacity is checked before applying an observation, so a full trace leaves canonical
/// view state unchanged. Redundant observations are retained as unchanged `ViewUpdate` records.
#[derive(Debug)]
pub struct ViewDriver {
    maximum_views: NonZeroU16,
    views: Vec<ViewState>,
    updates: BoundedCapture<ViewUpdate>,
}

impl ViewDriver {
    pub fn new(
        maximum_views: NonZeroU16,
        maximum_updates: NonZeroU16,
    ) -> Result<Self, ViewDriverLimitError> {
        if maximum_views.get() > MAX_CONFORMANCE_VIEWS {
            return Err(ViewDriverLimitError::ViewLimitTooLarge {
                requested: maximum_views.get(),
                maximum: MAX_CONFORMANCE_VIEWS,
            });
        }
        Ok(Self {
            maximum_views,
            views: Vec::with_capacity(maximum_views.get() as usize),
            updates: BoundedCapture::new(maximum_updates)?,
        })
    }

    pub const fn maximum_views(&self) -> NonZeroU16 {
        self.maximum_views
    }

    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    pub fn contains(&self, view: ViewId) -> bool {
        self.views.iter().any(|state| state.view() == view)
    }

    pub fn add_view(&mut self, view: ViewId, metrics: ViewMetrics) -> Result<(), ViewDriverError> {
        if self.contains(view) {
            return Err(ViewDriverError::DuplicateView { view });
        }
        if self.views.len() == self.maximum_views.get() as usize {
            return Err(ViewDriverError::ViewCapacityReached {
                maximum: self.maximum_views,
            });
        }
        self.views.push(ViewState::with_metrics(view, metrics));
        Ok(())
    }

    pub fn snapshot(&self, view: ViewId) -> Option<ViewSnapshot> {
        self.views
            .iter()
            .find(|state| state.view() == view)
            .map(ViewState::snapshot)
    }

    pub fn snapshots(&self) -> impl ExactSizeIterator<Item = ViewSnapshot> + '_ {
        self.views.iter().map(ViewState::snapshot)
    }

    pub const fn updates(&self) -> &BoundedCapture<ViewUpdate> {
        &self.updates
    }

    pub const fn updates_mut(&mut self) -> &mut BoundedCapture<ViewUpdate> {
        &mut self.updates
    }

    pub fn observe(
        &mut self,
        view: ViewId,
        observation: ViewObservation,
    ) -> Result<&ViewUpdate, ViewDriverError> {
        if self.updates.is_full() {
            return Err(ViewDriverError::UpdateCaptureFull {
                maximum: self.updates.capacity(),
            });
        }
        let state = self
            .views
            .iter_mut()
            .find(|state| state.view() == view)
            .ok_or(ViewDriverError::ViewUnavailable { view })?;
        let update = match observation {
            ViewObservation::Lifetime(next) => state.observe_lifetime(next),
            ViewObservation::Activity(next) => state.observe_activity(next),
            ViewObservation::Visibility(next) => state.observe_visibility(next),
            ViewObservation::SurfaceAvailable(generation) => {
                state.observe_surface_available(generation)
            }
            ViewObservation::SurfaceUnavailable => state.observe_surface_unavailable(),
            ViewObservation::Metrics(metrics) => state.observe_metrics(metrics),
        }
        .map_err(|error| ViewDriverError::State { view, error })?;
        self.updates
            .push(update)
            .expect("capture capacity was checked before applying the view observation");
        Ok(self
            .updates
            .back()
            .expect("the accepted view observation was captured"))
    }
}
