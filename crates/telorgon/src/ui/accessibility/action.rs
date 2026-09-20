use std::fmt;
use std::sync::Arc;

use crate::ui::SemanticAction;

use crate::ui::accessibility::{
    SemanticNodeId, SemanticTreeGeneration, SemanticTreeRevision, SemanticTreeSnapshot,
};

/// Hard bound for replacement text carried by one assistive action.
pub const MAX_ACTION_TEXT_BYTES: usize = 64 * 1024;

/// Typed data associated with an assistive action.
#[derive(Clone, PartialEq)]
pub enum AssistiveActionData {
    None,
    Text(Arc<str>),
    Number(f64),
    /// UTF-8 byte offsets in the canonical accessible text exposed for the target node.
    TextSelection {
        anchor: u32,
        active: u32,
    },
}

impl fmt::Debug for AssistiveActionData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::Text(text) => formatter
                .debug_struct("Text")
                .field("byte_len", &text.len())
                .finish_non_exhaustive(),
            Self::Number(number) => formatter.debug_tuple("Number").field(number).finish(),
            Self::TextSelection { anchor, active } => formatter
                .debug_struct("TextSelection")
                .field("anchor", anchor)
                .field("active", active)
                .finish(),
        }
    }
}

/// One generation- and revision-citing assistive action after native convention conversion.
#[derive(Clone, PartialEq)]
pub struct AssistiveActionRequest {
    tree_generation: SemanticTreeGeneration,
    observed_revision: SemanticTreeRevision,
    target: SemanticNodeId,
    action: SemanticAction,
    data: AssistiveActionData,
}

impl AssistiveActionRequest {
    pub fn new(
        tree_generation: SemanticTreeGeneration,
        observed_revision: SemanticTreeRevision,
        target: SemanticNodeId,
        action: SemanticAction,
        data: AssistiveActionData,
    ) -> Result<Self, AssistiveActionError> {
        validate_data(action, &data)?;
        Ok(Self {
            tree_generation,
            observed_revision,
            target,
            action,
            data,
        })
    }

    pub const fn tree_generation(&self) -> SemanticTreeGeneration {
        self.tree_generation
    }

    pub const fn observed_revision(&self) -> SemanticTreeRevision {
        self.observed_revision
    }

    pub const fn target(&self) -> SemanticNodeId {
        self.target
    }

    pub const fn action(&self) -> SemanticAction {
        self.action
    }

    pub const fn data(&self) -> &AssistiveActionData {
        &self.data
    }

    /// Validates tree identity, revision, target identity, and advertised action capability.
    pub fn validate_against(
        &self,
        snapshot: &SemanticTreeSnapshot,
    ) -> Result<(), AssistiveActionError> {
        if self.tree_generation != snapshot.generation() {
            return Err(AssistiveActionError::StaleTreeGeneration {
                expected: snapshot.generation(),
                observed: self.tree_generation,
            });
        }
        if self.observed_revision != snapshot.revision() {
            return Err(AssistiveActionError::StaleTreeRevision {
                expected: snapshot.revision(),
                observed: self.observed_revision,
            });
        }
        let node = snapshot
            .node(self.target)
            .ok_or(AssistiveActionError::UnknownTarget(self.target))?;
        if !node.semantics().effective_actions().contains(self.action) {
            return Err(AssistiveActionError::ActionNotAdvertised {
                target: self.target,
                action: self.action,
            });
        }
        Ok(())
    }
}

impl fmt::Debug for AssistiveActionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AssistiveActionRequest")
            .field("tree_generation", &self.tree_generation)
            .field("observed_revision", &self.observed_revision)
            .field("target", &self.target)
            .field("action", &self.action)
            .field("data", &self.data)
            .finish_non_exhaustive()
    }
}

/// Invalid action payload or stale/unavailable semantic target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistiveActionError {
    MissingData {
        action: SemanticAction,
    },
    UnexpectedData {
        action: SemanticAction,
    },
    WrongDataKind {
        action: SemanticAction,
    },
    TextTooLarge,
    NonFiniteNumber,
    StaleTreeGeneration {
        expected: SemanticTreeGeneration,
        observed: SemanticTreeGeneration,
    },
    StaleTreeRevision {
        expected: SemanticTreeRevision,
        observed: SemanticTreeRevision,
    },
    UnknownTarget(SemanticNodeId),
    ActionNotAdvertised {
        target: SemanticNodeId,
        action: SemanticAction,
    },
}

impl fmt::Display for AssistiveActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingData { .. } => "assistive action is missing required data",
            Self::UnexpectedData { .. } => "assistive action carried unexpected data",
            Self::WrongDataKind { .. } => "assistive action carried the wrong data kind",
            Self::TextTooLarge => "assistive action text exceeds the neutral hard bound",
            Self::NonFiniteNumber => "assistive action number must be finite",
            Self::StaleTreeGeneration { .. } => "assistive action cites a stale tree generation",
            Self::StaleTreeRevision { .. } => "assistive action cites a stale tree revision",
            Self::UnknownTarget(_) => "assistive action target is unavailable",
            Self::ActionNotAdvertised { .. } => {
                "assistive action was not advertised by its current target"
            }
        })
    }
}

impl std::error::Error for AssistiveActionError {}

fn validate_data(
    action: SemanticAction,
    data: &AssistiveActionData,
) -> Result<(), AssistiveActionError> {
    match (action, data) {
        (SemanticAction::SetValue, AssistiveActionData::Text(text))
        | (SemanticAction::SetText, AssistiveActionData::Text(text)) => {
            if text.len() > MAX_ACTION_TEXT_BYTES {
                Err(AssistiveActionError::TextTooLarge)
            } else {
                Ok(())
            }
        }
        (SemanticAction::SetValue, AssistiveActionData::Number(number)) => {
            if number.is_finite() {
                Ok(())
            } else {
                Err(AssistiveActionError::NonFiniteNumber)
            }
        }
        (SemanticAction::SetSelection, AssistiveActionData::TextSelection { .. }) => Ok(()),
        (
            SemanticAction::SetValue | SemanticAction::SetText | SemanticAction::SetSelection,
            AssistiveActionData::None,
        ) => Err(AssistiveActionError::MissingData { action }),
        (SemanticAction::SetValue | SemanticAction::SetText | SemanticAction::SetSelection, _) => {
            Err(AssistiveActionError::WrongDataKind { action })
        }
        (_, AssistiveActionData::None) => Ok(()),
        (_, _) => Err(AssistiveActionError::UnexpectedData { action }),
    }
}
