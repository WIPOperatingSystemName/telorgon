//! Revisioned renderer- and platform-neutral accessibility trees and actions.
//!
//! [`crate::ui`] owns component-authored semantic inputs. This crate combines those inputs with
//! stable mounted identity, resolved strings, and layout geometry to form immutable per-view tree
//! snapshots and validated deltas. Platform adapters translate these records to native APIs; they
//! do not become a second semantic source of truth.
//!
//! This crate owns no pixels, renderer resources, native accessibility objects, callbacks, queue,
//! executor, event loop, or platform identity.

mod action;
mod id;
mod node;
mod tree;
mod update;

pub use crate::graphics::scene::NodeId as SemanticNodeId;
pub use crate::ui::{
    SemanticAction, SemanticActions, SemanticCheckState, SemanticCollection, SemanticError,
    SemanticName, SemanticNode, SemanticParticipation, SemanticRelationship,
    SemanticRelationshipKind, SemanticRole, SemanticState, SemanticValue, StringId,
};
pub use action::{
    AssistiveActionData, AssistiveActionError, AssistiveActionRequest, MAX_ACTION_TEXT_BYTES,
};
pub use id::{SemanticTreeGeneration, SemanticTreeRevision};
pub use node::{
    MAX_SEMANTIC_CHILDREN_PER_NODE, MAX_SEMANTIC_RELATIONSHIPS_PER_NODE, MAX_SEMANTIC_STRING_BYTES,
    ResolvedSemanticString, SemanticCoordinateSpace, SemanticNodeGeometry, SemanticTreeNode,
};
pub use tree::{
    MAX_SEMANTIC_NODES, MAX_SEMANTIC_STRINGS, MAX_SEMANTIC_TREE_STRING_BYTES, SemanticTreeError,
    SemanticTreeSnapshot,
};
pub use update::{
    SemanticFocusUpdate, SemanticTreeDelta, SemanticTreePublication, SemanticTreePublicationKind,
    SemanticTreeRetirement,
};
