//! Host-classified floating workspace region with overlap-preserving painter order.

use std::fmt;
use std::sync::Arc;

use crate::foundation::{PointF, RectF};
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    OutputId, OutputRevision, ShellGrantToken, SurfaceId, WorkspaceId, WorkspaceRevision,
    WorkspaceSnapshot, WorkspaceSurface,
};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, Property, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRelationship, SemanticRelationshipKind, SemanticRole, SizeRule,
    UiNodeId,
};

use super::stack::{WindowStackHost, sealed};
use super::tiling_region::{
    RegionSelectionError, contains_rect, select_placements, valid_positive_rect,
};
use crate::components::shell::WorkspaceViewRef;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatingRegionStyle {
    pub container: BoxStyle,
    pub content: BoxStyle,
    pub layout: LayoutStyle,
    pub content_layout: LayoutStyle,
}

impl Default for FloatingRegionStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            content: BoxStyle {
                width: SizeRule::Fill(1.0),
                height: SizeRule::Fill(1.0),
                ..BoxStyle::default()
            },
            layout: LayoutStyle {
                flow: Flow::Overlay,
                ..LayoutStyle::default()
            },
            content_layout: LayoutStyle {
                flow: Flow::Overlay,
                ..LayoutStyle::default()
            },
        }
    }
}

/// Exact floating membership and geometry supplied by the policy host.
#[derive(Clone, Debug, PartialEq)]
pub struct FloatingRegion {
    label: String,
    workspace: Arc<WorkspaceSnapshot>,
    output: OutputId,
    bounds: RectF,
    placements: Arc<[WorkspaceSurface]>,
    style: FloatingRegionStyle,
}

impl FloatingRegion {
    pub fn new(
        label: impl Into<String>,
        workspace: WorkspaceSnapshot,
        output: OutputId,
        bounds: RectF,
        surfaces: Vec<SurfaceId>,
    ) -> Result<Self, FloatingRegionError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(FloatingRegionError::MissingAccessibleName);
        }
        if !valid_positive_rect(bounds) {
            return Err(FloatingRegionError::InvalidBounds);
        }
        let placements =
            select_placements(&workspace, output, &surfaces).map_err(|error| match error {
                RegionSelectionError::DuplicateSurface { surface } => {
                    FloatingRegionError::DuplicateSurface { surface }
                }
                RegionSelectionError::SurfaceNotOnOutput { surface } => {
                    FloatingRegionError::SurfaceNotOnOutput { surface }
                }
            })?;
        for placement in &placements {
            if !contains_rect(bounds, placement.bounds()) {
                return Err(FloatingRegionError::SurfaceOutsideRegion {
                    surface: placement.surface(),
                });
            }
        }
        Ok(Self {
            label,
            workspace: Arc::new(workspace),
            output,
            bounds,
            placements: placements.into(),
            style: FloatingRegionStyle::default(),
        })
    }

    pub const fn style(mut self, style: FloatingRegionStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn bounds(&self) -> RectF {
        self.bounds
    }

    pub fn placements(&self) -> &[WorkspaceSurface] {
        &self.placements
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        workspace: &WorkspaceViewRef,
    ) -> Result<FloatingRegionRef, FloatingRegionMountError> {
        if workspace.workspace() != self.workspace.id()
            || workspace.revision() != self.workspace.revision()
        {
            return Err(FloatingRegionError::WorkspaceSnapshotMismatch.into());
        }
        if workspace.output() != self.output {
            return Err(FloatingRegionError::OutputMismatch.into());
        }
        if !contains_rect(workspace.output_bounds(), self.bounds) {
            return Err(FloatingRegionError::RegionOutsideOutput.into());
        }

        let mut style = self.style.container;
        style.width = SizeRule::Logical(self.bounds.width);
        style.height = SizeRule::Logical(self.bounds.height);
        style.transform.translation.x = self.bounds.x - workspace.output_origin().x;
        style.transform.translation.y = self.bounds.y - workspace.output_origin().y;
        let root = ui
            .foundation()
            .container_node_under(workspace.content_node(), style, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("floating-region workspace is stale"))?;
        let content = ui
            .foundation()
            .container_node_under(
                root.node,
                self.style.content,
                self.style.content_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("floating-region content parent is stale"))?;
        ui.foundation()
            .semantic_node(
                content.node,
                SemanticNode {
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Region,
                    name: SemanticName::Text(name),
                    relationships: vec![SemanticRelationship {
                        kind: SemanticRelationshipKind::Owns,
                        target: content.node,
                    }],
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;
        Ok(FloatingRegionRef {
            root,
            content,
            workspace: Arc::clone(&self.workspace),
            output: self.output,
            output_revision: workspace.output_revision(),
            grant: workspace.grant(),
            bounds: self.bounds,
            placements: Arc::clone(&self.placements),
        })
    }
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid floating-region semantics: {error:?}"))
}

#[derive(Clone, Debug)]
pub struct FloatingRegionRef {
    root: ControlHandle,
    content: ControlHandle,
    workspace: Arc<WorkspaceSnapshot>,
    output: OutputId,
    output_revision: OutputRevision,
    grant: ShellGrantToken,
    bounds: RectF,
    placements: Arc<[WorkspaceSurface]>,
}

impl FloatingRegionRef {
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub const fn content_node(&self) -> UiNodeId {
        self.content.node
    }

    pub const fn bounds(&self) -> RectF {
        self.bounds
    }

    pub fn placements(&self) -> &[WorkspaceSurface] {
        &self.placements
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

impl sealed::Sealed for FloatingRegionRef {}

impl WindowStackHost for FloatingRegionRef {
    fn content_node(&self) -> UiNodeId {
        self.content.node
    }

    fn workspace(&self) -> WorkspaceId {
        self.workspace.id()
    }

    fn workspace_revision(&self) -> WorkspaceRevision {
        self.workspace.revision()
    }

    fn output(&self) -> OutputId {
        self.output
    }

    fn output_revision(&self) -> OutputRevision {
        self.output_revision
    }

    fn grant(&self) -> ShellGrantToken {
        self.grant
    }

    fn global_origin(&self) -> PointF {
        PointF {
            x: self.bounds.x,
            y: self.bounds.y,
        }
    }

    fn placements(&self) -> &[WorkspaceSurface] {
        &self.placements
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FloatingRegionError {
    MissingAccessibleName,
    InvalidBounds,
    DuplicateSurface { surface: SurfaceId },
    SurfaceNotOnOutput { surface: SurfaceId },
    SurfaceOutsideRegion { surface: SurfaceId },
    WorkspaceSnapshotMismatch,
    OutputMismatch,
    RegionOutsideOutput,
}

impl fmt::Display for FloatingRegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid floating region: {self:?}")
    }
}

impl std::error::Error for FloatingRegionError {}

#[derive(Debug)]
pub enum FloatingRegionMountError {
    Region(FloatingRegionError),
    Runtime(RuntimeError),
}

impl fmt::Display for FloatingRegionMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Region(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FloatingRegionMountError {}

impl From<FloatingRegionError> for FloatingRegionMountError {
    fn from(value: FloatingRegionError) -> Self {
        Self::Region(value)
    }
}

impl From<RuntimeError> for FloatingRegionMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
