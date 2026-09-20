use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// A key is local to one immediate element parent.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Integer(u64),
    Text(Arc<str>),
}

impl From<u64> for Key {
    fn from(value: u64) -> Self {
        Self::Integer(value)
    }
}

impl From<u32> for Key {
    fn from(value: u32) -> Self {
        Self::Integer(u64::from(value))
    }
}

impl From<usize> for Key {
    fn from(value: usize) -> Self {
        Self::Integer(value as u64)
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Self::Text(value.into())
    }
}

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}

/// Stable hash helper for custom keys that should not expose their original value.
pub fn hashed_key(value: impl Hash) -> Key {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    Key::Integer(hasher.finish())
}
