//! Opaque, nonzero identities for externally observable platform objects.

use std::fmt;
use std::num::{NonZeroU32, NonZeroU64};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct GenerationalIdentity {
    slot: NonZeroU32,
    generation: NonZeroU32,
}

impl GenerationalIdentity {
    const MIN: Self = Self {
        slot: NonZeroU32::MIN,
        generation: NonZeroU32::MIN,
    };

    const fn new(slot: NonZeroU32, generation: NonZeroU32) -> Self {
        Self { slot, generation }
    }

    const fn from_raw(slot: u32, generation: u32) -> Option<Self> {
        match (NonZeroU32::new(slot), NonZeroU32::new(generation)) {
            (Some(slot), Some(generation)) => Some(Self::new(slot, generation)),
            _ => None,
        }
    }

    const fn slot(self) -> u32 {
        self.slot.get()
    }

    const fn generation(self) -> u32 {
        self.generation.get()
    }
}

macro_rules! define_generational_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        ///
        /// The numeric slot is meaningful only to the owner that issued the identity. Reusing a
        /// released slot requires a different nonzero generation.
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(GenerationalIdentity);

        impl $name {
            /// Smallest valid identity, primarily useful in deterministic fixtures.
            pub const MIN: Self = Self(GenerationalIdentity::MIN);

            /// Creates an identity from an owner-issued nonzero slot and generation.
            pub const fn new(slot: NonZeroU32, generation: NonZeroU32) -> Self {
                Self(GenerationalIdentity::new(slot, generation))
            }

            /// Creates an identity from raw fixture or host-map values, rejecting either zero.
            pub const fn from_raw(slot: u32, generation: u32) -> Option<Self> {
                match GenerationalIdentity::from_raw(slot, generation) {
                    Some(identity) => Some(Self(identity)),
                    None => None,
                }
            }

            /// Returns the owner-local slot without converting it to a native object identity.
            pub const fn slot(self) -> u32 {
                self.0.slot()
            }

            /// Returns the generation that distinguishes reuse of the same owner-local slot.
            pub const fn generation(self) -> u32 {
                self.0.generation()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .field("slot", &self.slot())
                    .field("generation", &self.generation())
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}@{}", self.slot(), self.generation())
            }
        }
    };
}

define_generational_id!(
    ViewId,
    "Identity of one view generation within its issuing platform host."
);
define_generational_id!(
    DataOfferId,
    "Identity of one clipboard or drag data-offer generation within its issuing service."
);
define_generational_id!(
    CursorConstraintLeaseId,
    "Identity of one active cursor confinement or lock lease within its issuing service."
);
define_generational_id!(
    PowerInhibitionLeaseId,
    "Identity of one active idle or system-sleep inhibition lease within its issuing service."
);
define_generational_id!(
    RestorationSessionId,
    "Identity of one platform-restoration session generation within its issuing service."
);
define_generational_id!(
    DisplayId,
    "Identity of one connected display generation within its issuing service."
);

/// Nonzero sequence assigned when a platform request is admitted.
///
/// A request identity is not a view, native handle, protocol serial, or indication that the
/// requested operation succeeded. Terminal outcome semantics belong to the request package.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(NonZeroU64);

impl RequestId {
    /// Smallest valid request sequence, primarily useful in deterministic fixtures.
    pub const MIN: Self = Self(NonZeroU64::MIN);

    /// Wraps an owner-issued nonzero request sequence.
    pub const fn new(sequence: NonZeroU64) -> Self {
        Self(sequence)
    }

    /// Wraps a raw sequence, rejecting the reserved zero value.
    pub const fn from_raw(sequence: u64) -> Option<Self> {
        match NonZeroU64::new(sequence) {
            Some(sequence) => Some(Self(sequence)),
            None => None,
        }
    }

    /// Returns the opaque sequence for owner maps and deterministic traces.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl From<RequestId> for NonZeroU64 {
    fn from(value: RequestId) -> Self {
        value.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// Nonzero generation of native surface availability for one view.
///
/// The generation contains no native handle. A presenter or adapter must reject surface work that
/// cites an earlier generation after native surface replacement.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NativeSurfaceGeneration(NonZeroU64);

impl NativeSurfaceGeneration {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    /// Wraps an adapter-issued nonzero generation.
    pub const fn new(generation: NonZeroU64) -> Self {
        Self(generation)
    }

    /// Wraps a raw generation, rejecting the unavailable zero value.
    pub const fn from_raw(generation: u64) -> Option<Self> {
        match NonZeroU64::new(generation) {
            Some(generation) => Some(Self(generation)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl From<NativeSurfaceGeneration> for NonZeroU64 {
    fn from(value: NativeSurfaceGeneration) -> Self {
        value.0
    }
}

impl fmt::Display for NativeSurfaceGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::hash::Hash;
    use std::mem::size_of;

    use super::*;

    fn assert_wire_value<T: Copy + Eq + Ord + Hash + Send + Sync + 'static>() {}

    #[test]
    fn generational_identities_reject_zero_parts_and_preserve_owner_values() {
        for raw in [(0, 0), (0, 1), (1, 0)] {
            assert_eq!(ViewId::from_raw(raw.0, raw.1), None);
            assert_eq!(DataOfferId::from_raw(raw.0, raw.1), None);
            assert_eq!(DisplayId::from_raw(raw.0, raw.1), None);
        }

        let view = ViewId::from_raw(7, 3).unwrap();
        let offer = DataOfferId::from_raw(7, 3).unwrap();
        assert_eq!((view.slot(), view.generation()), (7, 3));
        assert_eq!((offer.slot(), offer.generation()), (7, 3));
        assert_eq!(view.to_string(), "7@3");
        assert_eq!(offer.to_string(), "7@3");
    }

    #[test]
    fn reused_slots_require_a_distinct_generation_and_do_not_match_stale_handles() {
        let stale = ViewId::from_raw(4, 1).unwrap();
        let replacement = ViewId::from_raw(4, 2).unwrap();
        assert_ne!(stale, replacement);
        assert_eq!(stale.slot(), replacement.slot());

        let live = BTreeSet::from([replacement]);
        assert!(!live.contains(&stale));
        assert!(live.contains(&replacement));
    }

    #[test]
    fn sequences_and_surface_generations_are_nonzero_and_semantically_distinct() {
        assert_eq!(RequestId::from_raw(0), None);
        assert_eq!(NativeSurfaceGeneration::from_raw(0), None);
        assert_eq!(RequestId::from_raw(11).unwrap().get(), 11);
        assert_eq!(NativeSurfaceGeneration::from_raw(11).unwrap().get(), 11);
        assert_eq!(RequestId::MIN.to_string(), "1");
        assert_eq!(NativeSurfaceGeneration::INITIAL.to_string(), "1");
    }

    #[test]
    fn identities_are_compact_thread_transferable_values_without_zero_sentinels() {
        assert_wire_value::<ViewId>();
        assert_wire_value::<DataOfferId>();
        assert_wire_value::<DisplayId>();
        assert_wire_value::<RequestId>();
        assert_wire_value::<NativeSurfaceGeneration>();

        assert_eq!(size_of::<ViewId>(), size_of::<u64>());
        assert_eq!(size_of::<Option<ViewId>>(), size_of::<ViewId>());
        assert_eq!(size_of::<DataOfferId>(), size_of::<u64>());
        assert_eq!(size_of::<Option<DataOfferId>>(), size_of::<DataOfferId>());
        assert_eq!(size_of::<DisplayId>(), size_of::<u64>());
        assert_eq!(size_of::<Option<DisplayId>>(), size_of::<DisplayId>());
        assert_eq!(size_of::<Option<RequestId>>(), size_of::<RequestId>());
        assert_eq!(
            size_of::<Option<NativeSurfaceGeneration>>(),
            size_of::<NativeSurfaceGeneration>()
        );
    }
}
