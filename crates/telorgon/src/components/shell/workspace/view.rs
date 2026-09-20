//! Named workspace presentation over exact host membership and geometry.

use std::fmt;
use std::sync::Arc;

use crate::foundation::{PointF, RectF};
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    OutputId, OutputRevision, ShellGrantToken, ShellLayerKind, SurfaceId, WorkspaceId,
    WorkspaceRevision, WorkspaceSnapshot, WorkspaceSurface,
};
use crate::components::shell::primitives::{OutputViewRef, ShellLayerRef};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRelationship, SemanticRelationshipKind, SemanticRole,
    SemanticState, SizeRule, UiNodeId,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkspaceViewStyle {
    pub container: BoxStyle,
    pub content: BoxStyle,
    pub layout: LayoutStyle,
    pub content_layout: LayoutStyle,
}

impl Default for WorkspaceViewStyle {
    fn default() -> Self {
        let fill = BoxStyle {
            width: SizeRule::Fill(1.0),
            height: SizeRule::Fill(1.0),
            ..BoxStyle::default()
        };
        Self {
            container: fill,
            content: fill,
            layout: LayoutStyle::default(),
            content_layout: LayoutStyle::default(),
        }
    }
}

/// One immutable workspace revision. It filters only by the requested output and never reorders.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceView {
    snapshot: Arc<WorkspaceSnapshot>,
    style: WorkspaceViewStyle,
}

impl WorkspaceView {
    pub fn new(snapshot: WorkspaceSnapshot) -> Self {
        Self {
            snapshot: Arc::new(snapshot),
            style: WorkspaceViewStyle::default(),
        }
    }

    pub const fn style(mut self, style: WorkspaceViewStyle) -> Self {
        self.style = style;
        self
    }

    pub fn snapshot(&self) -> &WorkspaceSnapshot {
        &self.snapshot
    }

    pub fn workspace(&self) -> WorkspaceId {
        self.snapshot.id()
    }

    pub fn revision(&self) -> WorkspaceRevision {
        self.snapshot.revision()
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        layer: ShellLayerRef,
        output: OutputViewRef,
    ) -> Result<WorkspaceViewRef, WorkspaceViewMountError> {
        if layer.kind() != ShellLayerKind::Workspace {
            return Err(WorkspaceViewError::RequiresWorkspaceLayer.into());
        }
        if layer.output() != output.output() {
            return Err(WorkspaceViewError::OutputMismatch.into());
        }

        let root = ui
            .foundation()
            .layer_node_under(
                layer.content_node(),
                self.snapshot.active(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("workspace-view layer is stale"))?;
        let content = ui
            .foundation()
            .container_node_under(
                root.node,
                self.style.content,
                self.style.content_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("workspace-view content parent is stale"))?;
        ui.foundation()
            .semantic_node(
                content.node,
                SemanticNode {
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;
        let name = ui.foundation().intern(self.snapshot.name().as_str());
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Region,
                    name: SemanticName::Text(name),
                    state: SemanticState {
                        hidden: !self.snapshot.active(),
                        inert: !self.snapshot.active(),
                        ..SemanticState::default()
                    },
                    relationships: vec![SemanticRelationship {
                        kind: SemanticRelationshipKind::Owns,
                        target: content.node,
                    }],
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;

        let placements = self
            .snapshot
            .surfaces()
            .iter()
            .copied()
            .filter(|placement| placement.output() == output.output())
            .collect();
        let logical = output.snapshot().geometry().logical_bounds();
        Ok(WorkspaceViewRef {
            root,
            content,
            output: output.output(),
            output_revision: output.revision(),
            output_bounds: logical,
            output_origin: PointF {
                x: logical.x,
                y: logical.y,
            },
            grant: layer.authority().grant(),
            snapshot: Arc::clone(&self.snapshot),
            placements,
        })
    }
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid workspace-view semantics: {error:?}"))
}

#[derive(Clone, Debug)]
pub struct WorkspaceViewRef {
    root: ControlHandle,
    content: ControlHandle,
    output: OutputId,
    output_revision: OutputRevision,
    output_bounds: RectF,
    output_origin: PointF,
    grant: ShellGrantToken,
    snapshot: Arc<WorkspaceSnapshot>,
    placements: Vec<WorkspaceSurface>,
}

impl WorkspaceViewRef {
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub const fn content_node(&self) -> UiNodeId {
        self.content.node
    }

    pub const fn output(&self) -> OutputId {
        self.output
    }

    pub const fn output_revision(&self) -> OutputRevision {
        self.output_revision
    }

    pub const fn output_origin(&self) -> PointF {
        self.output_origin
    }

    pub const fn output_bounds(&self) -> RectF {
        self.output_bounds
    }

    pub const fn grant(&self) -> ShellGrantToken {
        self.grant
    }

    pub fn snapshot(&self) -> &WorkspaceSnapshot {
        &self.snapshot
    }

    pub fn workspace(&self) -> WorkspaceId {
        self.snapshot.id()
    }

    pub fn revision(&self) -> WorkspaceRevision {
        self.snapshot.revision()
    }

    pub fn active(&self) -> bool {
        self.snapshot.active()
    }

    /// Exact host placements for this output in retained back-to-front order.
    pub fn placements(&self) -> &[WorkspaceSurface] {
        &self.placements
    }

    pub fn placement(&self, surface: SurfaceId) -> Option<WorkspaceSurface> {
        self.placements
            .iter()
            .copied()
            .find(|placement| placement.surface() == surface)
    }

    /// Translates one retained global logical placement into this output's local coordinates.
    pub fn local_bounds(&self, surface: SurfaceId) -> Option<RectF> {
        self.placement(surface).map(|placement| {
            let bounds = placement.bounds();
            RectF {
                x: bounds.x - self.output_origin.x,
                y: bounds.y - self.output_origin.y,
                width: bounds.width,
                height: bounds.height,
            }
        })
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }

    pub const fn visible(&self) -> Property<bool> {
        self.root.visible
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceViewError {
    RequiresWorkspaceLayer,
    OutputMismatch,
}

impl fmt::Display for WorkspaceViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid workspace view: {self:?}")
    }
}

impl std::error::Error for WorkspaceViewError {}

#[derive(Debug)]
pub enum WorkspaceViewMountError {
    View(WorkspaceViewError),
    Runtime(RuntimeError),
}

impl fmt::Display for WorkspaceViewMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::View(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WorkspaceViewMountError {}

impl From<WorkspaceViewError> for WorkspaceViewMountError {
    fn from(value: WorkspaceViewError) -> Self {
        Self::View(value)
    }
}

impl From<RuntimeError> for WorkspaceViewMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
