//! Bounded bidirectional ownership of Winit window and neutral view identities.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::num::{NonZeroU16, NonZeroU32};

use crate::platform::contracts::ViewId;
use winit::window::WindowId;

/// Hard upper bound for simultaneously registered Winit views.
pub const MAX_WINIT_VIEWS: u16 = 1_024;

/// Error returned before constructing an over-wide registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewRegistryLimitError {
    /// Requested simultaneous-view capacity.
    pub requested: u16,
    /// Adapter hard bound.
    pub maximum: u16,
}

impl fmt::Display for ViewRegistryLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "requested Winit view capacity {} exceeds maximum {}",
            self.requested, self.maximum
        )
    }
}

impl Error for ViewRegistryLimitError {}

/// One newly registered logical view and its current native window identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewRegistration {
    /// Newly issued generation-aware logical identity.
    pub view: ViewId,
    /// Winit identity currently backing the logical view.
    pub window: WindowId,
}

/// An atomic change of native window identity for an existing logical view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowReplacement {
    /// Logical identity preserved across the replacement.
    pub view: ViewId,
    /// Native identity removed from the registry.
    pub previous_window: WindowId,
    /// Native identity installed in the registry.
    pub current_window: WindowId,
}

/// One retired logical view generation and its final native window identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetiredView {
    /// Logical identity that can no longer resolve.
    pub view: ViewId,
    /// Native identity removed from the registry.
    pub window: WindowId,
}

/// Typed rejection from a view-registry mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewRegistryError {
    /// The configured simultaneous-view capacity is full.
    CapacityReached {
        /// Configured simultaneous-view capacity.
        maximum: NonZeroU16,
    },
    /// Every owner-local slot has permanently exhausted its nonzero generation space.
    IdentitySpaceExhausted {
        /// Configured slot bound.
        maximum: NonZeroU16,
    },
    /// The Winit identity already belongs to a registered view.
    WindowAlreadyRegistered {
        /// Duplicate Winit identity.
        window: WindowId,
        /// View currently owning the identity.
        view: ViewId,
    },
    /// The cited view generation is stale, retired, or unknown.
    ViewUnavailable {
        /// Unresolvable logical view identity.
        view: ViewId,
    },
    /// The cited Winit identity is not the one currently backing the view.
    WindowMismatch {
        /// Logical view whose current window was checked.
        view: ViewId,
        /// Window identity supplied by the caller.
        expected_window: WindowId,
        /// Window identity currently registered for the view.
        registered_window: WindowId,
    },
    /// Replacement requires a distinct native identity.
    ReplacementUnchanged {
        /// Logical view whose replacement was requested.
        view: ViewId,
        /// Existing and proposed Winit identity.
        window: WindowId,
    },
}

impl fmt::Display for ViewRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityReached { maximum } => {
                write!(formatter, "Winit view capacity {maximum} is full")
            }
            Self::IdentitySpaceExhausted { maximum } => write!(
                formatter,
                "all {maximum} Winit view slots exhausted their generation space"
            ),
            Self::WindowAlreadyRegistered { window, view } => {
                write!(
                    formatter,
                    "Winit window {window:?} is already registered to {view}"
                )
            }
            Self::ViewUnavailable { view } => {
                write!(formatter, "Winit view {view} is stale, retired, or unknown")
            }
            Self::WindowMismatch {
                view,
                expected_window,
                registered_window,
            } => write!(
                formatter,
                "Winit view {view} is registered to {registered_window:?}, not {expected_window:?}"
            ),
            Self::ReplacementUnchanged { view, window } => write!(
                formatter,
                "Winit view {view} is already registered to replacement window {window:?}"
            ),
        }
    }
}

impl Error for ViewRegistryError {}

#[derive(Clone, Copy, Debug)]
struct ViewSlot {
    generation: NonZeroU32,
    window: Option<WindowId>,
}

/// Bounded bidirectional `WindowId`/`ViewId` registry owned by one Winit host.
///
/// Native-window replacement preserves a logical `ViewId`. Full retirement ends that view
/// generation; if its owner-local slot is reused, the registry issues a strictly newer generation.
/// Every mutating operation validates all cited identities before changing either direction of the
/// mapping.
#[derive(Debug)]
pub struct ViewRegistry {
    maximum_views: NonZeroU16,
    active_views: usize,
    slots: Vec<ViewSlot>,
    views_by_window: HashMap<WindowId, ViewId>,
}

impl ViewRegistry {
    /// Creates an empty registry with an explicit bounded simultaneous-view capacity.
    pub fn new(maximum_views: NonZeroU16) -> Result<Self, ViewRegistryLimitError> {
        if maximum_views.get() > MAX_WINIT_VIEWS {
            return Err(ViewRegistryLimitError {
                requested: maximum_views.get(),
                maximum: MAX_WINIT_VIEWS,
            });
        }

        let capacity = usize::from(maximum_views.get());
        Ok(Self {
            maximum_views,
            active_views: 0,
            slots: Vec::with_capacity(capacity),
            views_by_window: HashMap::with_capacity(capacity),
        })
    }

    /// Returns the configured simultaneous-view capacity.
    pub const fn maximum_views(&self) -> NonZeroU16 {
        self.maximum_views
    }

    /// Returns the number of currently registered views.
    pub const fn len(&self) -> usize {
        self.active_views
    }

    /// Returns whether the registry contains no active views.
    pub const fn is_empty(&self) -> bool {
        self.active_views == 0
    }

    /// Looks up the current logical view for a Winit identity.
    pub fn view_for_window(&self, window: WindowId) -> Option<ViewId> {
        self.views_by_window.get(&window).copied()
    }

    /// Looks up the current Winit identity for an exact logical view generation.
    pub fn window_for_view(&self, view: ViewId) -> Option<WindowId> {
        let slot = self.slots.get(slot_index(view)?)?;
        (slot.generation.get() == view.generation())
            .then_some(slot.window)
            .flatten()
    }

    /// Iterates active mappings in ascending owner-local view-slot order.
    pub fn iter(&self) -> impl Iterator<Item = (ViewId, WindowId)> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            let window = slot.window?;
            let view = ViewId::new(slot_number(index), slot.generation);
            Some((view, window))
        })
    }

    /// Registers a distinct Winit identity and issues a fresh logical view generation.
    pub fn register(&mut self, window: WindowId) -> Result<ViewRegistration, ViewRegistryError> {
        if let Some(view) = self.view_for_window(window) {
            return Err(ViewRegistryError::WindowAlreadyRegistered { window, view });
        }
        if self.active_views == usize::from(self.maximum_views.get()) {
            return Err(ViewRegistryError::CapacityReached {
                maximum: self.maximum_views,
            });
        }

        let reusable = self
            .slots
            .iter()
            .position(|slot| slot.window.is_none() && slot.generation.get() < u32::MAX);
        let (index, generation) = match reusable {
            Some(index) => {
                let next = self.slots[index]
                    .generation
                    .get()
                    .checked_add(1)
                    .and_then(NonZeroU32::new)
                    .expect("reusable slots have a non-exhausted generation");
                (index, next)
            }
            None if self.slots.len() < usize::from(self.maximum_views.get()) => {
                (self.slots.len(), NonZeroU32::MIN)
            }
            None => {
                return Err(ViewRegistryError::IdentitySpaceExhausted {
                    maximum: self.maximum_views,
                });
            }
        };

        let view = ViewId::new(slot_number(index), generation);
        if index == self.slots.len() {
            self.slots.push(ViewSlot {
                generation,
                window: Some(window),
            });
        } else {
            self.slots[index] = ViewSlot {
                generation,
                window: Some(window),
            };
        }
        let displaced = self.views_by_window.insert(window, view);
        debug_assert!(displaced.is_none());
        self.active_views += 1;

        Ok(ViewRegistration { view, window })
    }

    /// Atomically replaces the native identity backing an exact registered view.
    pub fn replace_window(
        &mut self,
        view: ViewId,
        expected_window: WindowId,
        replacement_window: WindowId,
    ) -> Result<WindowReplacement, ViewRegistryError> {
        let registered_window = self.registered_window(view)?;
        if registered_window != expected_window {
            return Err(ViewRegistryError::WindowMismatch {
                view,
                expected_window,
                registered_window,
            });
        }
        if replacement_window == registered_window {
            return Err(ViewRegistryError::ReplacementUnchanged {
                view,
                window: registered_window,
            });
        }
        if let Some(owner) = self.view_for_window(replacement_window) {
            return Err(ViewRegistryError::WindowAlreadyRegistered {
                window: replacement_window,
                view: owner,
            });
        }

        let index = slot_index(view).expect("registered views always have a nonzero slot");
        let removed = self.views_by_window.remove(&registered_window);
        debug_assert_eq!(removed, Some(view));
        let displaced = self.views_by_window.insert(replacement_window, view);
        debug_assert!(displaced.is_none());
        self.slots[index].window = Some(replacement_window);

        Ok(WindowReplacement {
            view,
            previous_window: registered_window,
            current_window: replacement_window,
        })
    }

    /// Retires an exact logical generation only if its cited Winit identity is still current.
    pub fn retire(
        &mut self,
        view: ViewId,
        expected_window: WindowId,
    ) -> Result<RetiredView, ViewRegistryError> {
        let registered_window = self.registered_window(view)?;
        if registered_window != expected_window {
            return Err(ViewRegistryError::WindowMismatch {
                view,
                expected_window,
                registered_window,
            });
        }

        let index = slot_index(view).expect("registered views always have a nonzero slot");
        let removed = self.views_by_window.remove(&registered_window);
        debug_assert_eq!(removed, Some(view));
        self.slots[index].window = None;
        self.active_views -= 1;

        Ok(RetiredView {
            view,
            window: registered_window,
        })
    }

    fn registered_window(&self, view: ViewId) -> Result<WindowId, ViewRegistryError> {
        self.window_for_view(view)
            .ok_or(ViewRegistryError::ViewUnavailable { view })
    }
}

fn slot_index(view: ViewId) -> Option<usize> {
    usize::try_from(view.slot()).ok()?.checked_sub(1)
}

fn slot_number(index: usize) -> NonZeroU32 {
    u32::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(1))
        .and_then(NonZeroU32::new)
        .expect("the Winit registry hard bound fits nonzero u32 view slots")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maximum(value: u16) -> NonZeroU16 {
        NonZeroU16::new(value).unwrap()
    }

    fn window(value: u64) -> WindowId {
        WindowId::from(value)
    }

    #[test]
    fn generation_exhaustion_skips_a_retired_slot_then_reports_terminal_exhaustion() {
        let mut registry = ViewRegistry::new(maximum(2)).unwrap();
        registry.slots.push(ViewSlot {
            generation: NonZeroU32::MAX,
            window: None,
        });

        let registration = registry.register(window(2)).unwrap();
        assert_eq!(
            (registration.view.slot(), registration.view.generation()),
            (2, 1)
        );
        registry.retire(registration.view, window(2)).unwrap();
        registry.slots[1].generation = NonZeroU32::MAX;

        assert_eq!(
            registry.register(window(3)),
            Err(ViewRegistryError::IdentitySpaceExhausted {
                maximum: maximum(2),
            })
        );
        assert!(registry.is_empty());
    }
}
