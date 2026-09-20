use std::sync::Arc;

use crate::ui::StringId;

use crate::ui::accessibility::{
    MAX_SEMANTIC_NODES, MAX_SEMANTIC_STRINGS, ResolvedSemanticString, SemanticNodeId,
    SemanticTreeError, SemanticTreeGeneration, SemanticTreeNode, SemanticTreeRevision,
    SemanticTreeSnapshot,
};

/// Tri-state focus mutation carried by a tree delta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticFocusUpdate {
    Unchanged,
    Set(Option<SemanticNodeId>),
}

/// Validated structural/value delta relative to one exact tree revision.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticTreeDelta {
    generation: SemanticTreeGeneration,
    base_revision: SemanticTreeRevision,
    revision: SemanticTreeRevision,
    upserted_nodes: Arc<[SemanticTreeNode]>,
    removed_nodes: Arc<[SemanticNodeId]>,
    upserted_strings: Arc<[ResolvedSemanticString]>,
    removed_strings: Arc<[StringId]>,
    keyboard_focus: SemanticFocusUpdate,
    assistive_focus: SemanticFocusUpdate,
}

impl SemanticTreeDelta {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        generation: SemanticTreeGeneration,
        base_revision: SemanticTreeRevision,
        revision: SemanticTreeRevision,
        mut upserted_nodes: Vec<SemanticTreeNode>,
        mut removed_nodes: Vec<SemanticNodeId>,
        mut upserted_strings: Vec<ResolvedSemanticString>,
        mut removed_strings: Vec<StringId>,
        keyboard_focus: SemanticFocusUpdate,
        assistive_focus: SemanticFocusUpdate,
    ) -> Result<Self, SemanticTreeError> {
        if revision <= base_revision {
            return Err(SemanticTreeError::NonIncreasingRevision);
        }
        if upserted_nodes.len() > MAX_SEMANTIC_NODES || removed_nodes.len() > MAX_SEMANTIC_NODES {
            return Err(SemanticTreeError::NodeLimitExceeded);
        }
        if upserted_strings.len() > MAX_SEMANTIC_STRINGS
            || removed_strings.len() > MAX_SEMANTIC_STRINGS
        {
            return Err(SemanticTreeError::StringLimitExceeded);
        }

        upserted_nodes.sort_unstable_by_key(SemanticTreeNode::id);
        for pair in upserted_nodes.windows(2) {
            if pair[0].id() == pair[1].id() {
                return Err(SemanticTreeError::DuplicateDeltaNode(pair[0].id()));
            }
        }
        removed_nodes.sort_unstable();
        for pair in removed_nodes.windows(2) {
            if pair[0] == pair[1] {
                return Err(SemanticTreeError::DuplicateRemovedNode(pair[0]));
            }
        }
        if let Some(node) = upserted_nodes
            .iter()
            .map(SemanticTreeNode::id)
            .find(|node| removed_nodes.binary_search(node).is_ok())
        {
            return Err(SemanticTreeError::DeltaNodeAlsoRemoved(node));
        }

        upserted_strings.sort_unstable_by_key(ResolvedSemanticString::id);
        for pair in upserted_strings.windows(2) {
            if pair[0].id() == pair[1].id() {
                return Err(SemanticTreeError::DuplicateDeltaString(pair[0].id()));
            }
        }
        removed_strings.sort_unstable();
        for pair in removed_strings.windows(2) {
            if pair[0] == pair[1] {
                return Err(SemanticTreeError::DuplicateRemovedString(pair[0]));
            }
        }
        if let Some(string) = upserted_strings
            .iter()
            .map(ResolvedSemanticString::id)
            .find(|string| removed_strings.binary_search(string).is_ok())
        {
            return Err(SemanticTreeError::DeltaStringAlsoRemoved(string));
        }

        Ok(Self {
            generation,
            base_revision,
            revision,
            upserted_nodes: upserted_nodes.into(),
            removed_nodes: removed_nodes.into(),
            upserted_strings: upserted_strings.into(),
            removed_strings: removed_strings.into(),
            keyboard_focus,
            assistive_focus,
        })
    }

    pub const fn generation(&self) -> SemanticTreeGeneration {
        self.generation
    }

    pub const fn base_revision(&self) -> SemanticTreeRevision {
        self.base_revision
    }

    pub const fn revision(&self) -> SemanticTreeRevision {
        self.revision
    }

    pub fn upserted_nodes(&self) -> &[SemanticTreeNode] {
        &self.upserted_nodes
    }

    pub fn removed_nodes(&self) -> &[SemanticNodeId] {
        &self.removed_nodes
    }

    pub fn upserted_strings(&self) -> &[ResolvedSemanticString] {
        &self.upserted_strings
    }

    pub fn removed_strings(&self) -> &[StringId] {
        &self.removed_strings
    }

    pub const fn keyboard_focus(&self) -> SemanticFocusUpdate {
        self.keyboard_focus
    }

    pub const fn assistive_focus(&self) -> SemanticFocusUpdate {
        self.assistive_focus
    }
}

impl SemanticTreeSnapshot {
    /// Applies one exact-base delta and revalidates the complete resulting tree atomically.
    pub fn apply_delta(&self, delta: &SemanticTreeDelta) -> Result<Self, SemanticTreeError> {
        if delta.generation != self.generation() {
            return Err(SemanticTreeError::DeltaGenerationMismatch);
        }
        if delta.base_revision != self.revision() {
            return Err(SemanticTreeError::DeltaBaseRevisionMismatch);
        }
        if let Some(node) = delta
            .removed_nodes
            .iter()
            .copied()
            .find(|node| self.node(*node).is_none())
        {
            return Err(SemanticTreeError::UnknownRemovedNode(node));
        }
        if let Some(string) = delta
            .removed_strings
            .iter()
            .copied()
            .find(|string| self.resolved_string(*string).is_none())
        {
            return Err(SemanticTreeError::UnknownRemovedString(string));
        }

        let mut nodes = self.nodes().to_vec();
        nodes.retain(|node| delta.removed_nodes.binary_search(&node.id()).is_err());
        for replacement in delta.upserted_nodes.iter().cloned() {
            match nodes.binary_search_by_key(&replacement.id(), SemanticTreeNode::id) {
                Ok(index) => nodes[index] = replacement,
                Err(index) => nodes.insert(index, replacement),
            }
        }

        let mut strings = self.strings().to_vec();
        strings.retain(|string| delta.removed_strings.binary_search(&string.id()).is_err());
        for replacement in delta.upserted_strings.iter().cloned() {
            match strings.binary_search_by_key(&replacement.id(), ResolvedSemanticString::id) {
                Ok(index) => strings[index] = replacement,
                Err(index) => strings.insert(index, replacement),
            }
        }

        let keyboard_focus = match delta.keyboard_focus {
            SemanticFocusUpdate::Unchanged => self.keyboard_focus(),
            SemanticFocusUpdate::Set(focus) => focus,
        };
        let assistive_focus = match delta.assistive_focus {
            SemanticFocusUpdate::Unchanged => self.assistive_focus(),
            SemanticFocusUpdate::Set(focus) => focus,
        };

        Self::new(
            self.generation(),
            delta.revision,
            self.root(),
            nodes,
            strings,
            keyboard_focus,
            assistive_focus,
        )
    }
}

/// Exact tree state cited when retiring one live semantic generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SemanticTreeRetirement {
    generation: SemanticTreeGeneration,
    observed_revision: SemanticTreeRevision,
}

impl SemanticTreeRetirement {
    pub const fn new(
        generation: SemanticTreeGeneration,
        observed_revision: SemanticTreeRevision,
    ) -> Self {
        Self {
            generation,
            observed_revision,
        }
    }

    pub const fn generation(self) -> SemanticTreeGeneration {
        self.generation
    }

    pub const fn observed_revision(self) -> SemanticTreeRevision {
        self.observed_revision
    }
}

/// Publication class used for capability and completion metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SemanticTreePublicationKind {
    Activate,
    Update,
    Deactivate,
}

/// One canonical complete activation, revisioned delta, or generation retirement.
#[derive(Clone, Debug, PartialEq)]
pub enum SemanticTreePublication {
    Activate(SemanticTreeSnapshot),
    Update(SemanticTreeDelta),
    Deactivate(SemanticTreeRetirement),
}

impl SemanticTreePublication {
    pub const fn kind(&self) -> SemanticTreePublicationKind {
        match self {
            Self::Activate(_) => SemanticTreePublicationKind::Activate,
            Self::Update(_) => SemanticTreePublicationKind::Update,
            Self::Deactivate(_) => SemanticTreePublicationKind::Deactivate,
        }
    }

    pub const fn generation(&self) -> SemanticTreeGeneration {
        match self {
            Self::Activate(snapshot) => snapshot.generation(),
            Self::Update(delta) => delta.generation(),
            Self::Deactivate(retirement) => retirement.generation(),
        }
    }

    pub const fn revision(&self) -> SemanticTreeRevision {
        match self {
            Self::Activate(snapshot) => snapshot.revision(),
            Self::Update(delta) => delta.revision(),
            Self::Deactivate(retirement) => retirement.observed_revision(),
        }
    }
}
