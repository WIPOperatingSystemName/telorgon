//! Stable, protocol-neutral identities supplied by a shell policy host.

use std::fmt;
use std::num::NonZeroU64;

macro_rules! define_shell_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU64);

        impl $name {
            /// Smallest valid identity, primarily useful in deterministic fixtures.
            pub const MIN: Self = Self(NonZeroU64::MIN);

            /// Wraps a caller-supplied nonzero host identity.
            pub const fn new(value: NonZeroU64) -> Self {
                Self(value)
            }

            /// Wraps a raw identity, rejecting the reserved zero value.
            pub const fn from_raw(value: u64) -> Option<Self> {
                match NonZeroU64::new(value) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            /// Returns the opaque numeric value for host maps and trace fixtures.
            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }

        impl From<$name> for NonZeroU64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.get().fmt(formatter)
            }
        }
    };
}

define_shell_id!(OutputId, "Stable identity of one host-owned output.");
define_shell_id!(
    SurfaceId,
    "Stable identity of one host-owned client surface."
);
define_shell_id!(WorkspaceId, "Stable identity of one host-owned workspace.");
define_shell_id!(
    ApplicationId,
    "Stable identity of one host-described application or launcher entry."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shell_identity_rejects_zero_and_preserves_the_host_value() {
        assert_eq!(OutputId::from_raw(0), None);
        assert_eq!(SurfaceId::from_raw(0), None);
        assert_eq!(WorkspaceId::from_raw(0), None);
        assert_eq!(ApplicationId::from_raw(0), None);

        assert_eq!(OutputId::from_raw(11).unwrap().get(), 11);
        assert_eq!(SurfaceId::from_raw(12).unwrap().get(), 12);
        assert_eq!(WorkspaceId::from_raw(13).unwrap().get(), 13);
        assert_eq!(ApplicationId::from_raw(14).unwrap().get(), 14);
    }

    #[test]
    fn option_uses_the_nonzero_niche_without_a_sentinel_identity() {
        assert_eq!(
            std::mem::size_of::<Option<OutputId>>(),
            std::mem::size_of::<u64>()
        );
        assert_eq!(OutputId::MIN.to_string(), "1");
    }
}

/// Identity of a desktop policy window, independent of its current client surface.
/// Slots may be recycled only with a new generation. Identities are scoped to one
/// desktop host; neither field is an XID or a Wayland protocol object ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId {
    slot: std::num::NonZeroU32,
    generation: std::num::NonZeroU32,
}
impl WindowId {
    pub const fn new(slot: std::num::NonZeroU32, generation: std::num::NonZeroU32) -> Self {
        Self { slot, generation }
    }
    pub const fn slot(self) -> u32 {
        self.slot.get()
    }
    pub const fn generation(self) -> u32 {
        self.generation.get()
    }
}
