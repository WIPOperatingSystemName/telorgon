//! Immutable client-surface revisions mounted as renderer-neutral external-content metadata.

use std::fmt;
use std::sync::Arc;

use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    ClientSurfaceSnapshot, OutputId, ShellGrantToken, ShellLayerKind, SurfaceContent,
    SurfaceDamage, SurfaceGeometry, SurfaceId, SurfaceRegions, SurfaceRevision,
};
use crate::ui::{
    BoxStyle, ControlHandle, ImageId, LayoutStyle, Property, SemanticNode, SemanticParticipation,
    SemanticRole, UiNodeId,
};

use crate::components::shell::primitives::ShellLayerRef;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClientSurfaceStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

/// One exact immutable host surface revision. This value does not import or render its content.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientSurface {
    snapshot: Arc<ClientSurfaceSnapshot>,
    style: ClientSurfaceStyle,
}

impl ClientSurface {
    pub fn new(snapshot: ClientSurfaceSnapshot) -> Self {
        Self {
            snapshot: Arc::new(snapshot),
            style: ClientSurfaceStyle::default(),
        }
    }

    pub(crate) fn from_shared(snapshot: Arc<ClientSurfaceSnapshot>) -> Self {
        Self {
            snapshot,
            style: ClientSurfaceStyle::default(),
        }
    }

    pub const fn style(mut self, style: ClientSurfaceStyle) -> Self {
        self.style = style;
        self
    }

    pub fn snapshot(&self) -> &ClientSurfaceSnapshot {
        &self.snapshot
    }

    pub fn surface(&self) -> SurfaceId {
        self.snapshot.id()
    }

    pub fn revision(&self) -> SurfaceRevision {
        self.snapshot.revision()
    }

    pub fn geometry(&self) -> SurfaceGeometry {
        self.snapshot.geometry()
    }

    pub fn regions(&self) -> &SurfaceRegions {
        self.snapshot.regions()
    }

    pub fn damage(&self) -> &SurfaceDamage {
        self.snapshot.damage()
    }

    pub fn content(&self) -> SurfaceContent {
        self.snapshot.content()
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        layer: ShellLayerRef,
    ) -> Result<ClientSurfaceRef, ClientSurfaceMountError> {
        if layer.kind() != ShellLayerKind::Workspace {
            return Err(ClientSurfacePrimitiveError::RequiresWorkspaceLayer.into());
        }
        self.mount_under(
            ui,
            layer.content_node(),
            layer.output(),
            layer.authority().grant(),
            None,
        )
        .map_err(Into::into)
    }

    /// Mounts this exact surface revision as a Telorgon image after the host imports its content.
    pub fn mount_image<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        layer: ShellLayerRef,
        image: ImageId,
    ) -> Result<ClientSurfaceRef, ClientSurfaceMountError> {
        if layer.kind() != ShellLayerKind::Workspace {
            return Err(ClientSurfacePrimitiveError::RequiresWorkspaceLayer.into());
        }
        self.mount_under(
            ui,
            layer.content_node(),
            layer.output(),
            layer.authority().grant(),
            Some(image),
        )
        .map_err(Into::into)
    }

    pub(crate) fn mount_under<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        parent: UiNodeId,
        output: OutputId,
        grant: ShellGrantToken,
        image: Option<ImageId>,
    ) -> Result<ClientSurfaceRef, RuntimeError> {
        let control = match image {
            Some(image) => ui
                .foundation()
                .image_node_under(
                    parent,
                    image,
                    self.snapshot.content().revision().get(),
                    self.style.container,
                    self.style.layout,
                )
                .ok_or_else(|| RuntimeError::new("client-surface parent is stale"))?,
            None => ui
                .foundation()
                .container_node_under(parent, self.style.container, self.style.layout, |_| {})
                .ok_or_else(|| RuntimeError::new("client-surface parent is stale"))?,
        };
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
                RuntimeError::new(format!("invalid client-surface semantics: {error:?}"))
            })?;
        Ok(ClientSurfaceRef {
            control,
            output,
            grant,
            snapshot: self.snapshot.clone(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ClientSurfaceRef {
    control: ControlHandle,
    output: OutputId,
    grant: ShellGrantToken,
    snapshot: Arc<ClientSurfaceSnapshot>,
}

impl ClientSurfaceRef {
    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn output(&self) -> OutputId {
        self.output
    }

    pub const fn grant(&self) -> ShellGrantToken {
        self.grant
    }

    pub fn snapshot(&self) -> &ClientSurfaceSnapshot {
        &self.snapshot
    }

    pub fn surface(&self) -> SurfaceId {
        self.snapshot.id()
    }

    pub fn revision(&self) -> SurfaceRevision {
        self.snapshot.revision()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientSurfacePrimitiveError {
    RequiresWorkspaceLayer,
}

impl fmt::Display for ClientSurfacePrimitiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("client surfaces require an authorized workspace layer")
    }
}

impl std::error::Error for ClientSurfacePrimitiveError {}

#[derive(Debug)]
pub enum ClientSurfaceMountError {
    Primitive(ClientSurfacePrimitiveError),
    Runtime(RuntimeError),
}

impl fmt::Display for ClientSurfaceMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primitive(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ClientSurfaceMountError {}

impl From<ClientSurfacePrimitiveError> for ClientSurfaceMountError {
    fn from(value: ClientSurfacePrimitiveError) -> Self {
        Self::Primitive(value)
    }
}

impl From<RuntimeError> for ClientSurfaceMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
