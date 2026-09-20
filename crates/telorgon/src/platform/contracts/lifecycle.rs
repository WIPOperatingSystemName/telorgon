//! Independent view lifetime, activity, visibility, and native-surface transitions.

use std::error::Error;
use std::fmt;

use crate::platform::contracts::NativeSurfaceGeneration;

/// Host-observed lifetime of one view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ViewLifetime {
    #[default]
    Declared,
    Live,
    Closing,
    Closed,
}

/// Host-observed application activity affecting one view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ActivityState {
    Active,
    #[default]
    Inactive,
    Background,
    Suspended,
}

/// Host-observed visibility of one view, independent of activity and surface availability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VisibilityState {
    Visible,
    #[default]
    Hidden,
    Occluded,
}

/// Availability of a native presentation surface for one view.
///
/// This value carries continuity only. It never contains or resolves a native handle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NativeSurfaceState {
    #[default]
    Unavailable,
    Available {
        generation: NativeSurfaceGeneration,
    },
}

impl NativeSurfaceState {
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available { .. })
    }

    pub const fn generation(self) -> Option<NativeSurfaceGeneration> {
        match self {
            Self::Unavailable => None,
            Self::Available { generation } => Some(generation),
        }
    }
}

/// One accepted or redundant lifecycle observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LifecycleTransition<T> {
    previous: T,
    current: T,
}

impl<T> LifecycleTransition<T> {
    const fn new(previous: T, current: T) -> Self {
        Self { previous, current }
    }

    pub const fn previous(&self) -> &T {
        &self.previous
    }

    pub const fn current(&self) -> &T {
        &self.current
    }

    pub fn into_current(self) -> T {
        self.current
    }
}

impl<T: PartialEq> LifecycleTransition<T> {
    pub fn is_changed(&self) -> bool {
        self.previous != self.current
    }
}

/// Independent lifecycle axis cited by a rejected post-close observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LifecycleAxis {
    Activity,
    Visibility,
    NativeSurface,
}

/// Rejected lifecycle observation that would violate view or surface continuity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleError {
    InvalidLifetimeTransition {
        from: ViewLifetime,
        to: ViewLifetime,
    },
    ViewClosed {
        axis: LifecycleAxis,
    },
    SurfaceRequiresLiveView {
        lifetime: ViewLifetime,
    },
    SurfaceAvailableAtClose {
        generation: NativeSurfaceGeneration,
    },
    SurfaceGenerationDidNotAdvance {
        previous: NativeSurfaceGeneration,
        observed: NativeSurfaceGeneration,
    },
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidLifetimeTransition { from, to } => {
                write!(
                    formatter,
                    "invalid view lifetime transition from {from:?} to {to:?}"
                )
            }
            Self::ViewClosed { axis } => {
                write!(
                    formatter,
                    "closed view rejected a change to its {axis:?} axis"
                )
            }
            Self::SurfaceRequiresLiveView { lifetime } => write!(
                formatter,
                "native surface availability requires a live view, observed {lifetime:?}"
            ),
            Self::SurfaceAvailableAtClose { generation } => write!(
                formatter,
                "native surface generation {generation} must be retired before the view closes"
            ),
            Self::SurfaceGenerationDidNotAdvance { previous, observed } => write!(
                formatter,
                "native surface generation did not advance beyond {previous}; observed {observed}"
            ),
        }
    }
}

impl Error for LifecycleError {}

/// Pure transition owner for one view's independent lifecycle axes.
///
/// The owner is intentionally not cloneable. It stores no view registry, native handle, presenter
/// state, event-loop object, callback, or application exit policy. Adapters observe native facts,
/// apply them here, and later publish a revisioned view snapshot through the view package.
#[derive(Debug)]
pub struct ViewLifecycle {
    lifetime: ViewLifetime,
    activity: ActivityState,
    visibility: VisibilityState,
    surface: NativeSurfaceState,
    last_surface_generation: Option<NativeSurfaceGeneration>,
}

impl Default for ViewLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewLifecycle {
    /// Creates a declared, inactive, hidden view with no native surface.
    pub const fn new() -> Self {
        Self {
            lifetime: ViewLifetime::Declared,
            activity: ActivityState::Inactive,
            visibility: VisibilityState::Hidden,
            surface: NativeSurfaceState::Unavailable,
            last_surface_generation: None,
        }
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

    /// Returns the newest surface generation ever accepted, including after it was retired.
    pub const fn last_surface_generation(&self) -> Option<NativeSurfaceGeneration> {
        self.last_surface_generation
    }

    /// Applies an observed lifetime fact. Equal observations are idempotent.
    pub fn observe_lifetime(
        &mut self,
        next: ViewLifetime,
    ) -> Result<LifecycleTransition<ViewLifetime>, LifecycleError> {
        let previous = self.lifetime;
        if !self.validate_lifetime(next)? {
            return Ok(LifecycleTransition::new(previous, next));
        }

        self.lifetime = next;
        Ok(LifecycleTransition::new(previous, next))
    }

    pub(crate) fn validate_lifetime(&self, next: ViewLifetime) -> Result<bool, LifecycleError> {
        let previous = self.lifetime;
        if previous == next {
            return Ok(false);
        }

        let valid = matches!(
            (previous, next),
            (
                ViewLifetime::Declared,
                ViewLifetime::Live | ViewLifetime::Closing | ViewLifetime::Closed
            ) | (
                ViewLifetime::Live,
                ViewLifetime::Closing | ViewLifetime::Closed
            ) | (ViewLifetime::Closing, ViewLifetime::Closed)
        );
        if !valid {
            return Err(LifecycleError::InvalidLifetimeTransition {
                from: previous,
                to: next,
            });
        }

        if next == ViewLifetime::Closed
            && let NativeSurfaceState::Available { generation } = self.surface
        {
            return Err(LifecycleError::SurfaceAvailableAtClose { generation });
        }

        Ok(true)
    }

    /// Applies activity independently of visibility and native-surface availability.
    pub fn observe_activity(
        &mut self,
        next: ActivityState,
    ) -> Result<LifecycleTransition<ActivityState>, LifecycleError> {
        let previous = self.activity;
        if !self.validate_activity(next)? {
            return Ok(LifecycleTransition::new(previous, next));
        }
        self.activity = next;
        Ok(LifecycleTransition::new(previous, next))
    }

    pub(crate) fn validate_activity(&self, next: ActivityState) -> Result<bool, LifecycleError> {
        if self.activity == next {
            return Ok(false);
        }
        self.ensure_open(LifecycleAxis::Activity)?;
        Ok(true)
    }

    /// Applies visibility independently of activity and native-surface availability.
    pub fn observe_visibility(
        &mut self,
        next: VisibilityState,
    ) -> Result<LifecycleTransition<VisibilityState>, LifecycleError> {
        let previous = self.visibility;
        if !self.validate_visibility(next)? {
            return Ok(LifecycleTransition::new(previous, next));
        }
        self.visibility = next;
        Ok(LifecycleTransition::new(previous, next))
    }

    pub(crate) fn validate_visibility(
        &self,
        next: VisibilityState,
    ) -> Result<bool, LifecycleError> {
        if self.visibility == next {
            return Ok(false);
        }
        self.ensure_open(LifecycleAxis::Visibility)?;
        Ok(true)
    }

    /// Accepts a current native-surface generation for a live view.
    ///
    /// Repeating the current generation is idempotent. A generation retired by an earlier
    /// unavailable observation cannot become current again.
    pub fn observe_surface_available(
        &mut self,
        generation: NativeSurfaceGeneration,
    ) -> Result<LifecycleTransition<NativeSurfaceState>, LifecycleError> {
        let next = NativeSurfaceState::Available { generation };
        let previous = self.surface;
        if !self.validate_surface_available(generation)? {
            return Ok(LifecycleTransition::new(previous, next));
        }

        self.surface = next;
        self.last_surface_generation = Some(generation);
        Ok(LifecycleTransition::new(previous, next))
    }

    pub(crate) fn validate_surface_available(
        &self,
        generation: NativeSurfaceGeneration,
    ) -> Result<bool, LifecycleError> {
        let next = NativeSurfaceState::Available { generation };
        if self.surface == next {
            return Ok(false);
        }
        self.ensure_open(LifecycleAxis::NativeSurface)?;
        if self.lifetime != ViewLifetime::Live {
            return Err(LifecycleError::SurfaceRequiresLiveView {
                lifetime: self.lifetime,
            });
        }
        if let Some(last) = self.last_surface_generation
            && generation <= last
        {
            return Err(LifecycleError::SurfaceGenerationDidNotAdvance {
                previous: last,
                observed: generation,
            });
        }

        Ok(true)
    }

    /// Retires the current native surface while preserving its last generation against reuse.
    pub fn observe_surface_unavailable(
        &mut self,
    ) -> Result<LifecycleTransition<NativeSurfaceState>, LifecycleError> {
        let previous = self.surface;
        let next = NativeSurfaceState::Unavailable;
        if !self.validate_surface_unavailable()? {
            return Ok(LifecycleTransition::new(previous, next));
        }
        self.surface = next;
        Ok(LifecycleTransition::new(previous, next))
    }

    pub(crate) fn validate_surface_unavailable(&self) -> Result<bool, LifecycleError> {
        if self.surface == NativeSurfaceState::Unavailable {
            return Ok(false);
        }
        self.ensure_open(LifecycleAxis::NativeSurface)?;
        Ok(true)
    }

    fn ensure_open(&self, axis: LifecycleAxis) -> Result<(), LifecycleError> {
        if self.lifetime == ViewLifetime::Closed {
            Err(LifecycleError::ViewClosed { axis })
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generation(value: u64) -> NativeSurfaceGeneration {
        NativeSurfaceGeneration::from_raw(value).unwrap()
    }

    fn assert_owner<T: Send + Sync + 'static>() {}

    #[test]
    fn new_view_has_explicit_nonrenderable_axes() {
        let lifecycle = ViewLifecycle::new();
        assert_eq!(lifecycle.lifetime(), ViewLifetime::Declared);
        assert_eq!(lifecycle.activity(), ActivityState::Inactive);
        assert_eq!(lifecycle.visibility(), VisibilityState::Hidden);
        assert_eq!(lifecycle.surface(), NativeSurfaceState::Unavailable);
        assert_eq!(lifecycle.last_surface_generation(), None);
        assert_owner::<ViewLifecycle>();
    }

    #[test]
    fn redundant_and_forward_lifetime_observations_are_atomic() {
        let mut lifecycle = ViewLifecycle::new();
        assert!(
            !lifecycle
                .observe_lifetime(ViewLifetime::Declared)
                .unwrap()
                .is_changed()
        );
        assert!(
            lifecycle
                .observe_lifetime(ViewLifetime::Live)
                .unwrap()
                .is_changed()
        );
        assert_eq!(
            lifecycle.observe_lifetime(ViewLifetime::Declared),
            Err(LifecycleError::InvalidLifetimeTransition {
                from: ViewLifetime::Live,
                to: ViewLifetime::Declared,
            })
        );
        assert_eq!(lifecycle.lifetime(), ViewLifetime::Live);
    }

    #[test]
    fn activity_visibility_and_surface_availability_remain_independent() {
        let mut lifecycle = ViewLifecycle::new();
        lifecycle.observe_lifetime(ViewLifetime::Live).unwrap();
        lifecycle.observe_surface_available(generation(1)).unwrap();
        lifecycle
            .observe_visibility(VisibilityState::Occluded)
            .unwrap();
        lifecycle
            .observe_activity(ActivityState::Suspended)
            .unwrap();

        assert_eq!(lifecycle.activity(), ActivityState::Suspended);
        assert_eq!(lifecycle.visibility(), VisibilityState::Occluded);
        assert_eq!(
            lifecycle.surface(),
            NativeSurfaceState::Available {
                generation: generation(1),
            }
        );
    }

    #[test]
    fn retired_surface_generations_cannot_reappear_or_move_backward() {
        let mut lifecycle = ViewLifecycle::new();
        lifecycle.observe_lifetime(ViewLifetime::Live).unwrap();
        lifecycle.observe_surface_available(generation(2)).unwrap();
        lifecycle.observe_surface_unavailable().unwrap();

        for observed in [1, 2] {
            assert_eq!(
                lifecycle.observe_surface_available(generation(observed)),
                Err(LifecycleError::SurfaceGenerationDidNotAdvance {
                    previous: generation(2),
                    observed: generation(observed),
                })
            );
            assert_eq!(lifecycle.surface(), NativeSurfaceState::Unavailable);
        }

        assert!(
            lifecycle
                .observe_surface_available(generation(3))
                .unwrap()
                .is_changed()
        );
        assert!(
            !lifecycle
                .observe_surface_available(generation(3))
                .unwrap()
                .is_changed()
        );
    }

    #[test]
    fn close_requires_surface_retirement_and_closed_is_terminal() {
        let mut lifecycle = ViewLifecycle::new();
        lifecycle.observe_lifetime(ViewLifetime::Live).unwrap();
        lifecycle.observe_surface_available(generation(1)).unwrap();
        assert_eq!(
            lifecycle.observe_lifetime(ViewLifetime::Closed),
            Err(LifecycleError::SurfaceAvailableAtClose {
                generation: generation(1),
            })
        );
        assert_eq!(lifecycle.lifetime(), ViewLifetime::Live);

        lifecycle.observe_surface_unavailable().unwrap();
        lifecycle.observe_lifetime(ViewLifetime::Closing).unwrap();
        lifecycle.observe_lifetime(ViewLifetime::Closed).unwrap();
        assert!(
            !lifecycle
                .observe_lifetime(ViewLifetime::Closed)
                .unwrap()
                .is_changed()
        );
        assert!(
            !lifecycle
                .observe_surface_unavailable()
                .unwrap()
                .is_changed()
        );
        assert_eq!(
            lifecycle.observe_activity(ActivityState::Background),
            Err(LifecycleError::ViewClosed {
                axis: LifecycleAxis::Activity,
            })
        );
        assert_eq!(lifecycle.activity(), ActivityState::Inactive);
    }

    #[test]
    fn surface_availability_requires_live_lifetime() {
        let mut lifecycle = ViewLifecycle::new();
        assert_eq!(
            lifecycle.observe_surface_available(generation(1)),
            Err(LifecycleError::SurfaceRequiresLiveView {
                lifetime: ViewLifetime::Declared,
            })
        );
        assert_eq!(lifecycle.last_surface_generation(), None);
        lifecycle.observe_lifetime(ViewLifetime::Closed).unwrap();
        assert_eq!(
            lifecycle.observe_surface_available(generation(1)),
            Err(LifecycleError::ViewClosed {
                axis: LifecycleAxis::NativeSurface,
            })
        );
    }
}
