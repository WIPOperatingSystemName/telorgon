use std::fmt;
use std::sync::Arc;

use crate::ui::{SemanticError, StringId};

use crate::ui::accessibility::{
    ResolvedSemanticString, SemanticNodeId, SemanticTreeGeneration, SemanticTreeNode,
    SemanticTreeRevision,
};

/// Hard bound for nodes in one live view semantic tree.
pub const MAX_SEMANTIC_NODES: usize = 65_536;
/// Hard bound for resolved strings in one live view semantic tree.
pub const MAX_SEMANTIC_STRINGS: usize = 65_536;
/// Hard bound for all resolved semantic string bytes in one live view tree.
pub const MAX_SEMANTIC_TREE_STRING_BYTES: usize = 4 * 1024 * 1024;

/// One complete immutable semantic tree publication for a live view generation.
#[derive(Clone, PartialEq)]
pub struct SemanticTreeSnapshot {
    generation: SemanticTreeGeneration,
    revision: SemanticTreeRevision,
    root: SemanticNodeId,
    nodes: Arc<[SemanticTreeNode]>,
    strings: Arc<[ResolvedSemanticString]>,
    keyboard_focus: Option<SemanticNodeId>,
    assistive_focus: Option<SemanticNodeId>,
}

impl SemanticTreeSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        generation: SemanticTreeGeneration,
        revision: SemanticTreeRevision,
        root: SemanticNodeId,
        mut nodes: Vec<SemanticTreeNode>,
        mut strings: Vec<ResolvedSemanticString>,
        keyboard_focus: Option<SemanticNodeId>,
        assistive_focus: Option<SemanticNodeId>,
    ) -> Result<Self, SemanticTreeError> {
        if nodes.is_empty() {
            return Err(SemanticTreeError::EmptyTree);
        }
        if nodes.len() > MAX_SEMANTIC_NODES {
            return Err(SemanticTreeError::NodeLimitExceeded);
        }
        if strings.len() > MAX_SEMANTIC_STRINGS {
            return Err(SemanticTreeError::StringLimitExceeded);
        }

        nodes.sort_unstable_by_key(SemanticTreeNode::id);
        for pair in nodes.windows(2) {
            if pair[0].id() == pair[1].id() {
                return Err(SemanticTreeError::DuplicateNode(pair[0].id()));
            }
        }
        strings.sort_unstable_by_key(ResolvedSemanticString::id);
        for pair in strings.windows(2) {
            if pair[0].id() == pair[1].id() {
                return Err(SemanticTreeError::DuplicateString(pair[0].id()));
            }
        }
        let total_string_bytes = strings
            .iter()
            .try_fold(0_usize, |total, string| {
                total.checked_add(string.byte_len())
            })
            .ok_or(SemanticTreeError::StringBytesLimitExceeded)?;
        if total_string_bytes > MAX_SEMANTIC_TREE_STRING_BYTES {
            return Err(SemanticTreeError::StringBytesLimitExceeded);
        }

        let snapshot = Self {
            generation,
            revision,
            root,
            nodes: nodes.into(),
            strings: strings.into(),
            keyboard_focus,
            assistive_focus,
        };
        snapshot.validate_complete()?;
        Ok(snapshot)
    }

    pub const fn generation(&self) -> SemanticTreeGeneration {
        self.generation
    }

    pub const fn revision(&self) -> SemanticTreeRevision {
        self.revision
    }

    pub const fn root(&self) -> SemanticNodeId {
        self.root
    }

    pub fn nodes(&self) -> &[SemanticTreeNode] {
        &self.nodes
    }

    pub fn strings(&self) -> &[ResolvedSemanticString] {
        &self.strings
    }

    pub const fn keyboard_focus(&self) -> Option<SemanticNodeId> {
        self.keyboard_focus
    }

    pub const fn assistive_focus(&self) -> Option<SemanticNodeId> {
        self.assistive_focus
    }

    pub fn node(&self, id: SemanticNodeId) -> Option<&SemanticTreeNode> {
        self.nodes
            .binary_search_by_key(&id, SemanticTreeNode::id)
            .ok()
            .map(|index| &self.nodes[index])
    }

    pub fn resolved_string(&self, id: StringId) -> Option<&str> {
        self.strings
            .binary_search_by_key(&id, ResolvedSemanticString::id)
            .ok()
            .map(|index| self.strings[index].value())
    }

    pub fn total_string_bytes(&self) -> usize {
        self.strings
            .iter()
            .map(ResolvedSemanticString::byte_len)
            .sum()
    }

    fn validate_complete(&self) -> Result<(), SemanticTreeError> {
        let root = self
            .node(self.root)
            .ok_or(SemanticTreeError::UnknownRoot(self.root))?;
        if root.parent().is_some() {
            return Err(SemanticTreeError::RootHasParent(self.root));
        }

        let mut referenced_strings = Vec::with_capacity(self.nodes.len().saturating_mul(3));
        for node in self.nodes.iter() {
            if node.id() != self.root {
                let parent_id = node
                    .parent()
                    .ok_or(SemanticTreeError::MissingParent(node.id()))?;
                let parent = self
                    .node(parent_id)
                    .ok_or(SemanticTreeError::UnknownParent {
                        node: node.id(),
                        parent: parent_id,
                    })?;
                if !parent.children().contains(&node.id()) {
                    return Err(SemanticTreeError::ParentMissingChild {
                        parent: parent_id,
                        child: node.id(),
                    });
                }
            }

            for child_id in node.children().iter().copied() {
                let child = self.node(child_id).ok_or(SemanticTreeError::UnknownChild {
                    parent: node.id(),
                    child: child_id,
                })?;
                if child.parent() != Some(node.id()) {
                    return Err(SemanticTreeError::ChildParentMismatch {
                        parent: node.id(),
                        child: child_id,
                    });
                }
            }

            for relationship in &node.semantics().relationships {
                if self.node(relationship.target).is_none() {
                    return Err(SemanticTreeError::UnknownRelationshipTarget {
                        node: node.id(),
                        target: relationship.target,
                    });
                }
            }
            for string in node.semantics().referenced_strings() {
                if self.resolved_string(string).is_none() {
                    return Err(SemanticTreeError::UnknownString {
                        node: node.id(),
                        string,
                    });
                }
                referenced_strings.push(string);
            }
            self.validate_reaches_root(node.id())?;
        }

        referenced_strings.sort_unstable();
        referenced_strings.dedup();
        if let Some(string) = self
            .strings
            .iter()
            .map(ResolvedSemanticString::id)
            .find(|string| referenced_strings.binary_search(string).is_err())
        {
            return Err(SemanticTreeError::UnreferencedString(string));
        }

        for focus in [self.keyboard_focus, self.assistive_focus]
            .into_iter()
            .flatten()
        {
            if self.node(focus).is_none() {
                return Err(SemanticTreeError::UnknownFocus(focus));
            }
        }
        Ok(())
    }

    fn validate_reaches_root(&self, start: SemanticNodeId) -> Result<(), SemanticTreeError> {
        let mut current = start;
        for _ in 0..self.nodes.len() {
            if current == self.root {
                return Ok(());
            }
            let node = self
                .node(current)
                .ok_or(SemanticTreeError::DisconnectedNode(start))?;
            current = node
                .parent()
                .ok_or(SemanticTreeError::DisconnectedNode(start))?;
        }
        Err(SemanticTreeError::DisconnectedNode(start))
    }
}

impl fmt::Debug for SemanticTreeSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SemanticTreeSnapshot")
            .field("generation", &self.generation)
            .field("revision", &self.revision)
            .field("root", &self.root)
            .field("node_count", &self.nodes.len())
            .field("string_count", &self.strings.len())
            .field("string_bytes", &self.total_string_bytes())
            .field("keyboard_focus", &self.keyboard_focus)
            .field("assistive_focus", &self.assistive_focus)
            .finish_non_exhaustive()
    }
}

/// Invalid complete tree, node, geometry, resolved string set, or delta application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticTreeError {
    EmptyTree,
    NodeLimitExceeded,
    StringLimitExceeded,
    StringTooLarge {
        id: StringId,
    },
    StringBytesLimitExceeded,
    ChildLimitExceeded {
        node: SemanticNodeId,
    },
    RelationshipLimitExceeded {
        node: SemanticNodeId,
    },
    NonFiniteGeometry,
    NegativeGeometryExtent,
    NonFiniteTransform,
    InvalidSemanticInput(SemanticError),
    UnresolvedParticipation {
        node: SemanticNodeId,
    },
    SelfParent {
        node: SemanticNodeId,
    },
    SelfChild {
        node: SemanticNodeId,
    },
    DuplicateChild {
        parent: SemanticNodeId,
        child: SemanticNodeId,
    },
    DuplicateNode(SemanticNodeId),
    DuplicateString(StringId),
    UnknownRoot(SemanticNodeId),
    RootHasParent(SemanticNodeId),
    MissingParent(SemanticNodeId),
    UnknownParent {
        node: SemanticNodeId,
        parent: SemanticNodeId,
    },
    ParentMissingChild {
        parent: SemanticNodeId,
        child: SemanticNodeId,
    },
    UnknownChild {
        parent: SemanticNodeId,
        child: SemanticNodeId,
    },
    ChildParentMismatch {
        parent: SemanticNodeId,
        child: SemanticNodeId,
    },
    DisconnectedNode(SemanticNodeId),
    UnknownRelationshipTarget {
        node: SemanticNodeId,
        target: SemanticNodeId,
    },
    UnknownString {
        node: SemanticNodeId,
        string: StringId,
    },
    UnreferencedString(StringId),
    UnknownFocus(SemanticNodeId),
    NonIncreasingRevision,
    DeltaGenerationMismatch,
    DeltaBaseRevisionMismatch,
    DuplicateDeltaNode(SemanticNodeId),
    DuplicateRemovedNode(SemanticNodeId),
    UnknownRemovedNode(SemanticNodeId),
    DeltaNodeAlsoRemoved(SemanticNodeId),
    DuplicateDeltaString(StringId),
    DuplicateRemovedString(StringId),
    UnknownRemovedString(StringId),
    DeltaStringAlsoRemoved(StringId),
}

impl fmt::Display for SemanticTreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyTree => "semantic tree must contain a root node",
            Self::NodeLimitExceeded => "semantic tree node count exceeds the neutral hard bound",
            Self::StringLimitExceeded => {
                "semantic tree string count exceeds the neutral hard bound"
            }
            Self::StringTooLarge { .. } => {
                "resolved semantic string exceeds the neutral hard bound"
            }
            Self::StringBytesLimitExceeded => {
                "semantic tree string bytes exceed the neutral hard bound"
            }
            Self::ChildLimitExceeded { .. } => {
                "semantic node child count exceeds the neutral hard bound"
            }
            Self::RelationshipLimitExceeded { .. } => {
                "semantic relationship count exceeds the neutral hard bound"
            }
            Self::NonFiniteGeometry => "semantic geometry must be finite",
            Self::NegativeGeometryExtent => "semantic geometry extents must be nonnegative",
            Self::NonFiniteTransform => "semantic transform must be finite",
            Self::InvalidSemanticInput(_) => "component semantic input is invalid",
            Self::UnresolvedParticipation { .. } => {
                "merge/exclude participation must be resolved before tree publication"
            }
            Self::SelfParent { .. } => "semantic node cannot parent itself",
            Self::SelfChild { .. } => "semantic node cannot contain itself",
            Self::DuplicateChild { .. } => "semantic node contains a duplicate child",
            Self::DuplicateNode(_) => "semantic tree contains a duplicate node identity",
            Self::DuplicateString(_) => "semantic tree contains a duplicate string identity",
            Self::UnknownRoot(_) => "semantic tree root is unavailable",
            Self::RootHasParent(_) => "semantic tree root must not have a parent",
            Self::MissingParent(_) => "non-root semantic node has no parent",
            Self::UnknownParent { .. } => "semantic node parent is unavailable",
            Self::ParentMissingChild { .. } => "semantic parent does not list its child",
            Self::UnknownChild { .. } => "semantic node child is unavailable",
            Self::ChildParentMismatch { .. } => "semantic child cites another parent",
            Self::DisconnectedNode(_) => "semantic node does not reach the tree root",
            Self::UnknownRelationshipTarget { .. } => "semantic relationship target is unavailable",
            Self::UnknownString { .. } => "semantic string reference is unresolved",
            Self::UnreferencedString(_) => {
                "semantic publication contains an unreferenced resolved string"
            }
            Self::UnknownFocus(_) => "semantic focus target is unavailable",
            Self::NonIncreasingRevision => "semantic delta revision must increase",
            Self::DeltaGenerationMismatch => "semantic delta generation is stale",
            Self::DeltaBaseRevisionMismatch => "semantic delta base revision is stale",
            Self::DuplicateDeltaNode(_) => "semantic delta contains a duplicate node update",
            Self::DuplicateRemovedNode(_) => "semantic delta removes a node more than once",
            Self::UnknownRemovedNode(_) => "semantic delta removes an unavailable node",
            Self::DeltaNodeAlsoRemoved(_) => "semantic delta both updates and removes a node",
            Self::DuplicateDeltaString(_) => "semantic delta contains a duplicate string update",
            Self::DuplicateRemovedString(_) => "semantic delta removes a string more than once",
            Self::UnknownRemovedString(_) => "semantic delta removes an unavailable string",
            Self::DeltaStringAlsoRemoved(_) => "semantic delta both updates and removes a string",
        })
    }
}

impl std::error::Error for SemanticTreeError {}
