//! Host-authorized retained client-surface visual metadata.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    ClientSurfaceSnapshot, OutputId, ShellCapabilities, ShellCapabilityGrant, ShellGrantToken,
    ShellLayerKind, SurfaceId, SurfaceProtection, SurfaceRevision,
};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticNode, SemanticParticipation,
    SemanticRole, UiNodeId,
};

use crate::components::shell::primitives::ShellLayerRef;

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceSnapshotToken(NonZeroU64);

impl SurfaceSnapshotToken {
    pub const fn from_raw(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Debug for SurfaceSnapshotToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SurfaceSnapshotToken(..)")
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceSnapshotRevision(NonZeroU64);

impl SurfaceSnapshotRevision {
    pub const fn from_raw(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SurfaceSnapshotPolicy {
    #[default]
    UnprotectedOnly,
    AllowProtected,
}

/// Opaque authorization issued by the host against one source revision and output grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceSnapshotAuthorization {
    token: SurfaceSnapshotToken,
    grant: ShellGrantToken,
    output: OutputId,
    surface: SurfaceId,
    source_revision: SurfaceRevision,
    revision: SurfaceSnapshotRevision,
    policy: SurfaceSnapshotPolicy,
}

impl SurfaceSnapshotAuthorization {
    #[allow(clippy::too_many_arguments)]
    pub fn from_host(
        token: SurfaceSnapshotToken,
        grant: ShellCapabilityGrant,
        surface: SurfaceId,
        source_revision: SurfaceRevision,
        revision: SurfaceSnapshotRevision,
        policy: SurfaceSnapshotPolicy,
    ) -> Result<Self, SurfaceSnapshotAuthorizationError> {
        if !grant.permits(ShellCapabilities::RETAIN_SURFACE_SNAPSHOT) {
            return Err(SurfaceSnapshotAuthorizationError::MissingCapability);
        }
        Ok(Self {
            token,
            grant: grant.token(),
            output: grant.output(),
            surface,
            source_revision,
            revision,
            policy,
        })
    }

    pub const fn token(self) -> SurfaceSnapshotToken {
        self.token
    }

    pub const fn grant(self) -> ShellGrantToken {
        self.grant
    }

    pub const fn output(self) -> OutputId {
        self.output
    }

    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    pub const fn source_revision(self) -> SurfaceRevision {
        self.source_revision
    }

    pub const fn revision(self) -> SurfaceSnapshotRevision {
        self.revision
    }

    pub const fn policy(self) -> SurfaceSnapshotPolicy {
        self.policy
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceSnapshotAuthorizationError {
    MissingCapability,
}

impl fmt::Display for SurfaceSnapshotAuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("host grant does not permit retained surface snapshots")
    }
}

impl std::error::Error for SurfaceSnapshotAuthorizationError {}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceSnapshotStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceSnapshot {
    source: Arc<ClientSurfaceSnapshot>,
    authorization: SurfaceSnapshotAuthorization,
    style: SurfaceSnapshotStyle,
}

impl SurfaceSnapshot {
    pub fn new(
        source: ClientSurfaceSnapshot,
        authorization: SurfaceSnapshotAuthorization,
    ) -> Result<Self, SurfaceSnapshotError> {
        if source.id() != authorization.surface()
            || source.revision() != authorization.source_revision()
        {
            return Err(SurfaceSnapshotError::SourceMismatch);
        }
        if source.content().protection() == SurfaceProtection::Protected
            && authorization.policy() != SurfaceSnapshotPolicy::AllowProtected
        {
            return Err(SurfaceSnapshotError::ProtectedContentDenied);
        }
        Ok(Self {
            source: Arc::new(source),
            authorization,
            style: SurfaceSnapshotStyle::default(),
        })
    }

    pub const fn style(mut self, style: SurfaceSnapshotStyle) -> Self {
        self.style = style;
        self
    }

    pub fn source(&self) -> &ClientSurfaceSnapshot {
        &self.source
    }

    pub const fn authorization(&self) -> SurfaceSnapshotAuthorization {
        self.authorization
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        layer: ShellLayerRef,
    ) -> Result<SurfaceSnapshotRef, SurfaceSnapshotMountError> {
        if layer.kind() != ShellLayerKind::Workspace {
            return Err(SurfaceSnapshotError::RequiresWorkspaceLayer.into());
        }
        if layer.output() != self.authorization.output() {
            return Err(SurfaceSnapshotError::OutputMismatch.into());
        }
        if layer.authority().grant() != self.authorization.grant() {
            return Err(SurfaceSnapshotError::GrantMismatch.into());
        }
        let control = ui
            .foundation()
            .container_node_under(
                layer.content_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("surface-snapshot layer is stale"))?;
        ui.foundation()
            .semantic_node(
                control.node,
                SemanticNode {
                    role: SemanticRole::Image,
                    participation: SemanticParticipation::Exclude,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid surface-snapshot semantics: {error:?}"))
            })?;
        Ok(SurfaceSnapshotRef {
            control,
            source: self.source.clone(),
            authorization: self.authorization,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SurfaceSnapshotRef {
    control: ControlHandle,
    source: Arc<ClientSurfaceSnapshot>,
    authorization: SurfaceSnapshotAuthorization,
}

impl SurfaceSnapshotRef {
    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.control.style
    }

    pub fn source(&self) -> &ClientSurfaceSnapshot {
        &self.source
    }

    pub const fn authorization(&self) -> SurfaceSnapshotAuthorization {
        self.authorization
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceSnapshotError {
    SourceMismatch,
    ProtectedContentDenied,
    RequiresWorkspaceLayer,
    OutputMismatch,
    GrantMismatch,
}

impl fmt::Display for SurfaceSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SourceMismatch => "snapshot authorization does not match the source revision",
            Self::ProtectedContentDenied => "snapshot authorization excludes protected content",
            Self::RequiresWorkspaceLayer => {
                "retained surface snapshots require an authorized workspace layer"
            }
            Self::OutputMismatch => "snapshot authorization output does not match its layer",
            Self::GrantMismatch => "snapshot authorization grant does not match its layer",
        })
    }
}

impl std::error::Error for SurfaceSnapshotError {}

#[derive(Debug)]
pub enum SurfaceSnapshotMountError {
    Snapshot(SurfaceSnapshotError),
    Runtime(RuntimeError),
}

impl fmt::Display for SurfaceSnapshotMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SurfaceSnapshotMountError {}

impl From<SurfaceSnapshotError> for SurfaceSnapshotMountError {
    fn from(value: SurfaceSnapshotError) -> Self {
        Self::Snapshot(value)
    }
}

impl From<RuntimeError> for SurfaceSnapshotMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
