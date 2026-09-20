//! Host-classified, nonoverlapping tiled workspace region.

use std::collections::HashSet;
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
use crate::components::shell::WorkspaceViewRef;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TilingRegionStyle {
    pub container: BoxStyle,
    pub content: BoxStyle,
    pub layout: LayoutStyle,
    pub content_layout: LayoutStyle,
}

impl Default for TilingRegionStyle {
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

/// Exact region and membership supplied by a policy host; no tiling algorithm runs here.
#[derive(Clone, Debug, PartialEq)]
pub struct TilingRegion {
    label: String,
    workspace: Arc<WorkspaceSnapshot>,
    output: OutputId,
    bounds: RectF,
    placements: Arc<[WorkspaceSurface]>,
    style: TilingRegionStyle,
}

impl TilingRegion {
    pub fn new(
        label: impl Into<String>,
        workspace: WorkspaceSnapshot,
        output: OutputId,
        bounds: RectF,
        surfaces: Vec<SurfaceId>,
    ) -> Result<Self, TilingRegionError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TilingRegionError::MissingAccessibleName);
        }
        if !valid_positive_rect(bounds) {
            return Err(TilingRegionError::InvalidBounds);
        }
        let placements =
            select_placements(&workspace, output, &surfaces).map_err(|error| match error {
                RegionSelectionError::DuplicateSurface { surface } => {
                    TilingRegionError::DuplicateSurface { surface }
                }
                RegionSelectionError::SurfaceNotOnOutput { surface } => {
                    TilingRegionError::SurfaceNotOnOutput { surface }
                }
            })?;
        for placement in &placements {
            if !contains_rect(bounds, placement.bounds()) {
                return Err(TilingRegionError::SurfaceOutsideRegion {
                    surface: placement.surface(),
                });
            }
        }
        for left in 0..placements.len() {
            for right in (left + 1)..placements.len() {
                if overlaps(placements[left].bounds(), placements[right].bounds()) {
                    return Err(TilingRegionError::OverlappingSurfaces {
                        first: placements[left].surface(),
                        second: placements[right].surface(),
                    });
                }
            }
        }
        Ok(Self {
            label,
            workspace: Arc::new(workspace),
            output,
            bounds,
            placements: placements.into(),
            style: TilingRegionStyle::default(),
        })
    }

    pub const fn style(mut self, style: TilingRegionStyle) -> Self {
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
    ) -> Result<TilingRegionRef, TilingRegionMountError> {
        if workspace.workspace() != self.workspace.id()
            || workspace.revision() != self.workspace.revision()
        {
            return Err(TilingRegionError::WorkspaceSnapshotMismatch.into());
        }
        if workspace.output() != self.output {
            return Err(TilingRegionError::OutputMismatch.into());
        }
        if !contains_rect(workspace.output_bounds(), self.bounds) {
            return Err(TilingRegionError::RegionOutsideOutput.into());
        }

        let mut style = self.style.container;
        style.width = SizeRule::Logical(self.bounds.width);
        style.height = SizeRule::Logical(self.bounds.height);
        style.transform.translation.x = self.bounds.x - workspace.output_origin().x;
        style.transform.translation.y = self.bounds.y - workspace.output_origin().y;
        let root = ui
            .foundation()
            .container_node_under(workspace.content_node(), style, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("tiling-region workspace is stale"))?;
        let content = ui
            .foundation()
            .container_node_under(
                root.node,
                self.style.content,
                self.style.content_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("tiling-region content parent is stale"))?;
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
        Ok(TilingRegionRef {
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
    RuntimeError::new(format!("invalid tiling-region semantics: {error:?}"))
}

#[derive(Clone, Debug)]
pub struct TilingRegionRef {
    root: ControlHandle,
    content: ControlHandle,
    workspace: Arc<WorkspaceSnapshot>,
    output: OutputId,
    output_revision: OutputRevision,
    grant: ShellGrantToken,
    bounds: RectF,
    placements: Arc<[WorkspaceSurface]>,
}

impl TilingRegionRef {
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

impl sealed::Sealed for TilingRegionRef {}

impl WindowStackHost for TilingRegionRef {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegionSelectionError {
    DuplicateSurface { surface: SurfaceId },
    SurfaceNotOnOutput { surface: SurfaceId },
}

pub(crate) fn select_placements(
    workspace: &WorkspaceSnapshot,
    output: OutputId,
    surfaces: &[SurfaceId],
) -> Result<Vec<WorkspaceSurface>, RegionSelectionError> {
    let mut selected = HashSet::with_capacity(surfaces.len());
    for surface in surfaces {
        if !selected.insert(*surface) {
            return Err(RegionSelectionError::DuplicateSurface { surface: *surface });
        }
        if workspace
            .surface(*surface)
            .is_none_or(|placement| placement.output() != output)
        {
            return Err(RegionSelectionError::SurfaceNotOnOutput { surface: *surface });
        }
    }
    Ok(workspace
        .surfaces()
        .iter()
        .copied()
        .filter(|placement| placement.output() == output && selected.contains(&placement.surface()))
        .collect())
}

pub(crate) fn valid_positive_rect(rect: RectF) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.width > 0.0
        && rect.height.is_finite()
        && rect.height > 0.0
        && rect.right().is_finite()
        && rect.bottom().is_finite()
}

pub(crate) fn contains_rect(outer: RectF, inner: RectF) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

fn overlaps(left: RectF, right: RectF) -> bool {
    left.x < right.right()
        && right.x < left.right()
        && left.y < right.bottom()
        && right.y < left.bottom()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TilingRegionError {
    MissingAccessibleName,
    InvalidBounds,
    DuplicateSurface { surface: SurfaceId },
    SurfaceNotOnOutput { surface: SurfaceId },
    SurfaceOutsideRegion { surface: SurfaceId },
    OverlappingSurfaces { first: SurfaceId, second: SurfaceId },
    WorkspaceSnapshotMismatch,
    OutputMismatch,
    RegionOutsideOutput,
}

impl fmt::Display for TilingRegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid tiling region: {self:?}")
    }
}

impl std::error::Error for TilingRegionError {}

#[derive(Debug)]
pub enum TilingRegionMountError {
    Region(TilingRegionError),
    Runtime(RuntimeError),
}

impl fmt::Display for TilingRegionMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Region(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TilingRegionMountError {}

impl From<TilingRegionError> for TilingRegionMountError {
    fn from(value: TilingRegionError) -> Self {
        Self::Region(value)
    }
}

impl From<RuntimeError> for TilingRegionMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
