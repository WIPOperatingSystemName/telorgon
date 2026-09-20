//! Per-view semantic-tree publication and assistive-action admission boundary.
//!
//! [`crate::ui::accessibility`] remains the sole owner of semantic tree generations, revisions,
//! topology, resolved strings, geometry, deltas, and action validation. This module binds those
//! canonical records to a platform [`ViewId`], describes host capability, and exposes asynchronous
//! publication admission. Native action data must be converted into a canonical
//! [`AssistiveActionRequest`] and admitted against the current immutable snapshot before an event
//! can enter the portable runtime.
//!
//! No semantic data is reconstructed from pixels. No native accessibility API, adapter object,
//! callback, queue, executor, event loop, runtime dispatch, or semantic source of truth is owned
//! here.

use std::fmt;
use std::num::NonZeroU32;
use std::rc::Rc;

use crate::ui::accessibility::{
    AssistiveActionError, AssistiveActionRequest, MAX_SEMANTIC_NODES,
    MAX_SEMANTIC_TREE_STRING_BYTES, SemanticNodeId, SemanticTreeGeneration,
    SemanticTreePublication, SemanticTreePublicationKind, SemanticTreeRevision,
    SemanticTreeSnapshot,
};

use crate::platform::contracts::services::{ServiceKey, ServiceUnavailable};
use crate::platform::contracts::{CapabilityDescriptor, RequestAdmission, Support, ViewId};

/// Independently discoverable accessibility service operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AccessibilityOperations {
    tree_publication: bool,
    assistive_actions: bool,
}

impl AccessibilityOperations {
    pub const fn new(tree_publication: bool, assistive_actions: bool) -> Self {
        Self {
            tree_publication,
            assistive_actions,
        }
    }

    pub const fn supports_tree_publication(self) -> bool {
        self.tree_publication
    }

    pub const fn supports_assistive_actions(self) -> bool {
        self.assistive_actions
    }
}

/// Host-advertised bounds for one view's accessibility service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccessibilityLimits {
    maximum_nodes: NonZeroU32,
    maximum_string_bytes: NonZeroU32,
}

impl AccessibilityLimits {
    pub const fn new(
        maximum_nodes: NonZeroU32,
        maximum_string_bytes: NonZeroU32,
    ) -> Result<Self, AccessibilityLimitError> {
        if maximum_nodes.get() as usize > MAX_SEMANTIC_NODES {
            return Err(AccessibilityLimitError::NodeLimitTooLarge);
        }
        if maximum_string_bytes.get() as usize > MAX_SEMANTIC_TREE_STRING_BYTES {
            return Err(AccessibilityLimitError::StringBytesLimitTooLarge);
        }
        Ok(Self {
            maximum_nodes,
            maximum_string_bytes,
        })
    }

    pub const fn maximum_nodes(self) -> NonZeroU32 {
        self.maximum_nodes
    }

    pub const fn maximum_string_bytes(self) -> NonZeroU32 {
        self.maximum_string_bytes
    }
}

impl Default for AccessibilityLimits {
    fn default() -> Self {
        Self {
            maximum_nodes: NonZeroU32::new(MAX_SEMANTIC_NODES as u32)
                .expect("semantic node hard bound is nonzero"),
            maximum_string_bytes: NonZeroU32::new(MAX_SEMANTIC_TREE_STRING_BYTES as u32)
                .expect("semantic string-byte hard bound is nonzero"),
        }
    }
}

/// Invalid host-advertised accessibility limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessibilityLimitError {
    NodeLimitTooLarge,
    StringBytesLimitTooLarge,
}

impl fmt::Display for AccessibilityLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NodeLimitTooLarge => "accessibility node limit exceeds the neutral hard bound",
            Self::StringBytesLimitTooLarge => {
                "accessibility string-byte limit exceeds the neutral hard bound"
            }
        })
    }
}

impl std::error::Error for AccessibilityLimitError {}

/// Complete accessibility capability returned for one live view.
pub type AccessibilityCapability =
    CapabilityDescriptor<AccessibilityOperations, AccessibilityLimits>;

/// Scope for one accessibility capability query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccessibilityCapabilityQuery {
    view: ViewId,
}

impl AccessibilityCapabilityQuery {
    pub const fn new(view: ViewId) -> Self {
        Self { view }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }
}

/// One view-scoped canonical semantic-tree publication.
#[derive(Clone, PartialEq)]
pub struct AccessibilityPublicationRequest {
    view: ViewId,
    publication: SemanticTreePublication,
}

impl AccessibilityPublicationRequest {
    pub const fn new(view: ViewId, publication: SemanticTreePublication) -> Self {
        Self { view, publication }
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn kind(&self) -> SemanticTreePublicationKind {
        self.publication.kind()
    }

    pub const fn generation(&self) -> SemanticTreeGeneration {
        self.publication.generation()
    }

    pub const fn revision(&self) -> SemanticTreeRevision {
        self.publication.revision()
    }

    pub const fn canonical_publication(&self) -> &SemanticTreePublication {
        &self.publication
    }

    pub fn into_canonical_publication(self) -> SemanticTreePublication {
        self.publication
    }
}

impl fmt::Debug for AccessibilityPublicationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibilityPublicationRequest")
            .field("view", &self.view)
            .field("kind", &self.kind())
            .field("generation", &self.generation())
            .field("revision", &self.revision())
            .finish_non_exhaustive()
    }
}

/// Metadata returned when one canonical tree publication applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccessibilityApplied {
    view: ViewId,
    kind: SemanticTreePublicationKind,
    generation: SemanticTreeGeneration,
    revision: SemanticTreeRevision,
}

impl AccessibilityApplied {
    pub const fn from_request(request: &AccessibilityPublicationRequest) -> Self {
        Self {
            view: request.view(),
            kind: request.kind(),
            generation: request.generation(),
            revision: request.revision(),
        }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn kind(self) -> SemanticTreePublicationKind {
        self.kind
    }

    pub const fn generation(self) -> SemanticTreeGeneration {
        self.generation
    }

    pub const fn revision(self) -> SemanticTreeRevision {
        self.revision
    }
}

/// Immediate rejection before a semantic-tree publication is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityAdmissionError {
    ServiceUnavailable(ServiceUnavailable),
    ViewUnavailable {
        view: ViewId,
    },
    TreeUnavailable {
        view: ViewId,
        generation: SemanticTreeGeneration,
    },
    StaleTreeRevision {
        view: ViewId,
        expected: SemanticTreeRevision,
        observed: SemanticTreeRevision,
    },
    Unsupported,
    Denied,
    CapacityExceeded,
}

impl fmt::Display for AccessibilityAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServiceUnavailable(reason) => {
                write!(
                    formatter,
                    "accessibility service is unavailable: {reason:?}"
                )
            }
            Self::ViewUnavailable { view } => {
                write!(formatter, "accessibility view {view} is unavailable")
            }
            Self::TreeUnavailable { view, generation } => write!(
                formatter,
                "accessibility tree generation {generation} is unavailable for view {view}"
            ),
            Self::StaleTreeRevision {
                view,
                expected,
                observed,
            } => write!(
                formatter,
                "accessibility view {view} expected tree revision {expected}, observed {observed}"
            ),
            Self::Unsupported => formatter.write_str("accessibility operation is unsupported"),
            Self::Denied => formatter.write_str("accessibility operation was denied"),
            Self::CapacityExceeded => {
                formatter.write_str("accessibility admission capacity was exceeded")
            }
        }
    }
}

impl std::error::Error for AccessibilityAdmissionError {}

/// Linear asynchronous admission for one tree publication.
pub type AccessibilityPublicationAdmission =
    RequestAdmission<AccessibilityApplied, AccessibilityAdmissionError>;

/// A view-scoped action event admitted against one exact current semantic snapshot.
///
/// Construction is private except through [`Self::admit`], preventing stale generations,
/// revisions, node identities, or unadvertised operations from entering as valid events.
#[derive(Clone, PartialEq)]
pub struct AccessibilityActionEvent {
    view: ViewId,
    action: AssistiveActionRequest,
}

impl AccessibilityActionEvent {
    pub fn admit(
        view: ViewId,
        current: &SemanticTreeSnapshot,
        action: AssistiveActionRequest,
    ) -> AccessibilityActionAdmission {
        action
            .validate_against(current)
            .map_err(AccessibilityActionAdmissionError::InvalidAction)?;
        Ok(Self { view, action })
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn tree_generation(&self) -> SemanticTreeGeneration {
        self.action.tree_generation()
    }

    pub const fn observed_revision(&self) -> SemanticTreeRevision {
        self.action.observed_revision()
    }

    pub const fn target(&self) -> SemanticNodeId {
        self.action.target()
    }

    pub const fn canonical_action(&self) -> &AssistiveActionRequest {
        &self.action
    }

    pub fn into_canonical_action(self) -> AssistiveActionRequest {
        self.action
    }
}

impl fmt::Debug for AccessibilityActionEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibilityActionEvent")
            .field("view", &self.view)
            .field("tree_generation", &self.tree_generation())
            .field("observed_revision", &self.observed_revision())
            .field("target", &self.target())
            .field("action", &self.action.action())
            .field("data", self.action.data())
            .finish_non_exhaustive()
    }
}

/// Rejection before one native assistive action can become a portable platform event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityActionAdmissionError {
    InvalidAction(AssistiveActionError),
}

impl fmt::Display for AccessibilityActionAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAction(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AccessibilityActionAdmissionError {}

/// Immediate validated result of admitting one assistive action event.
pub type AccessibilityActionAdmission =
    Result<AccessibilityActionEvent, AccessibilityActionAdmissionError>;

/// Narrow service surface for per-view capability and semantic-tree publication.
pub trait AccessibilityService {
    fn capability(&self, query: AccessibilityCapabilityQuery) -> Support<AccessibilityCapability>;

    fn publish(
        &self,
        request: AccessibilityPublicationRequest,
    ) -> AccessibilityPublicationAdmission;
}

/// Type-level registry key for an owner-local accessibility service handle.
pub enum AccessibilityServiceKey {}

impl ServiceKey for AccessibilityServiceKey {
    type Handle = Rc<dyn AccessibilityService>;
}
