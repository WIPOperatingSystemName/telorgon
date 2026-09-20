//! Dense generational storage for Telorgon's mounted UI runtime.

mod arena;
mod sparse_set;

pub use arena::{DirtyFlags, NodeArena, NodeCore, NodeId, SubtreeRange};
pub use sparse_set::SparseSet;
