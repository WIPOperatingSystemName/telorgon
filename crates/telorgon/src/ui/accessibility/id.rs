use std::fmt;
use std::num::NonZeroU64;

/// Generation of one live view's semantic tree.
///
/// Replacing or closing the view retires this value. Reusing mounted node IDs under a new tree
/// generation therefore cannot make an old assistive action current again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticTreeGeneration(NonZeroU64);

impl SemanticTreeGeneration {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    pub const fn from_raw(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub const fn checked_next(self) -> Option<Self> {
        match self.0.get().checked_add(1) {
            Some(next) => Self::from_raw(next),
            None => None,
        }
    }
}

impl fmt::Display for SemanticTreeGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Monotonic revision inside one [`SemanticTreeGeneration`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticTreeRevision(NonZeroU64);

impl SemanticTreeRevision {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    pub const fn from_raw(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub const fn checked_next(self) -> Option<Self> {
        match self.0.get().checked_add(1) {
            Some(next) => Self::from_raw(next),
            None => None,
        }
    }
}

impl fmt::Display for SemanticTreeRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
