use std::num::NonZeroU32;

macro_rules! define_id {
    ($name:ident) => {
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU32);

        impl $name {
            pub const fn new(value: NonZeroU32) -> Self {
                Self(value)
            }

            pub const fn from_raw(value: u32) -> Option<Self> {
                match NonZeroU32::new(value) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            pub const fn get(self) -> u32 {
                self.0.get()
            }
        }
    };
}

define_id!(ClientId);
define_id!(ProtocolObjectId);
define_id!(WaylandBufferId);
define_id!(WaylandSurfaceId);
