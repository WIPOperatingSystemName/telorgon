//! Workspace overview cards with stable preview hosts and controlled selection intents.

use std::fmt;
use std::rc::Rc;

use crate::foundation::{ColorRgba8, RectF};
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{OutputId, OutputRevision, ShellCapabilities, SurfaceId, WorkspaceId};
use crate::components::shell::primitives::{OutputViewRef, ShellLayerRef, ShellRootRef};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, SemanticActions, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRole, SemanticState, SizeRule, UiNodeId,
};

use super::{
    WorkspaceCatalog, WorkspaceCatalogError, WorkspaceSelectionIntent, WorkspaceSwitcherError,
    validate_overlay,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkspaceOverviewStyle {
    pub container: BoxStyle,
    pub item: BoxStyle,
    pub preview: BoxStyle,
    pub surface: BoxStyle,
    pub layout: LayoutStyle,
    pub preview_layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
    pub preview_scale: f32,
}

impl Default for WorkspaceOverviewStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            item: BoxStyle::default(),
            preview: BoxStyle::default(),
            surface: BoxStyle::default(),
            layout: LayoutStyle {
                flow: Flow::Horizontal,
                ..LayoutStyle::default()
            },
            preview_layout: LayoutStyle {
                flow: Flow::Overlay,
                ..LayoutStyle::default()
            },
            label_color: ColorRgba8::rgba(245, 247, 250, 255),
            label_size: 14.0,
            preview_scale: 0.16,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceOverview {
    label: String,
    catalog: WorkspaceCatalog,
    style: WorkspaceOverviewStyle,
}

impl WorkspaceOverview {
    pub fn new(
        label: impl Into<String>,
        workspaces: Vec<crate::shell::WorkspaceSnapshot>,
    ) -> Result<Self, WorkspaceOverviewError> {
        Self::from_catalog(label, WorkspaceCatalog::new(workspaces)?)
    }

    pub fn from_catalog(
        label: impl Into<String>,
        catalog: WorkspaceCatalog,
    ) -> Result<Self, WorkspaceOverviewError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(WorkspaceOverviewError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            catalog,
            style: WorkspaceOverviewStyle::default(),
        })
    }

    pub fn style(mut self, style: WorkspaceOverviewStyle) -> Result<Self, WorkspaceOverviewError> {
        validate_style(style)?;
        self.style = style;
        Ok(self)
    }

    pub fn catalog(&self) -> &WorkspaceCatalog {
        &self.catalog
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        root: ShellRootRef,
        layer: ShellLayerRef,
        output: OutputViewRef,
        map: Map,
    ) -> Result<WorkspaceOverviewRef, WorkspaceOverviewMountError>
    where
        Action: 'static,
        Map: Fn(WorkspaceSelectionIntent) -> Action + 'static,
    {
        validate_style(self.style)?;
        validate_overlay(root, layer).map_err(WorkspaceOverviewError::OverlayBoundary)?;
        if output.output() != layer.output() {
            return Err(WorkspaceOverviewError::OutputMismatch.into());
        }
        let authorized = root.grant().permits(ShellCapabilities::SELECT_WORKSPACE);
        let container = ui
            .foundation()
            .container_node_under(
                layer.content_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("workspace-overview overlay is stale"))?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                container.node,
                SemanticNode::named(SemanticRole::Region, name),
            )
            .map_err(semantic_runtime_error)?;

        let logical = output.snapshot().geometry().logical_bounds();
        let map = Rc::new(map);
        let mut items = Vec::with_capacity(self.catalog.workspaces().len());
        for workspace in self.catalog.workspaces() {
            let available = authorized && !workspace.active();
            let item = ui
                .foundation()
                .action_node_under(
                    container.node,
                    self.style.item,
                    authorized,
                    true,
                    |writer| {
                        writer.text(
                            workspace.name().as_str(),
                            self.style.label_color,
                            self.style.label_size,
                        );
                    },
                )
                .ok_or_else(|| RuntimeError::new("workspace-overview container is stale"))?;
            let mut preview_style = self.style.preview;
            preview_style.width = SizeRule::Logical(logical.width * self.style.preview_scale);
            preview_style.height = SizeRule::Logical(logical.height * self.style.preview_scale);
            let preview = ui
                .foundation()
                .container_node_under(item.node, preview_style, self.style.preview_layout, |_| {})
                .ok_or_else(|| RuntimeError::new("workspace-overview preview parent is stale"))?;
            ui.foundation()
                .semantic_node(
                    preview.node,
                    SemanticNode {
                        participation: SemanticParticipation::Exclude,
                        ..SemanticNode::default()
                    },
                )
                .map_err(semantic_runtime_error)?;

            let mut surfaces = Vec::new();
            for placement in workspace
                .surfaces()
                .iter()
                .copied()
                .filter(|placement| placement.output() == output.output())
            {
                let source = placement.bounds();
                let projected = RectF {
                    x: (source.x - logical.x) * self.style.preview_scale,
                    y: (source.y - logical.y) * self.style.preview_scale,
                    width: source.width * self.style.preview_scale,
                    height: source.height * self.style.preview_scale,
                };
                let mut surface_style = self.style.surface;
                surface_style.width = SizeRule::Logical(projected.width);
                surface_style.height = SizeRule::Logical(projected.height);
                surface_style.transform.translation.x = projected.x;
                surface_style.transform.translation.y = projected.y;
                let surface = ui
                    .foundation()
                    .container_node_under(
                        preview.node,
                        surface_style,
                        LayoutStyle::default(),
                        |_| {},
                    )
                    .ok_or_else(|| RuntimeError::new("workspace-overview preview is stale"))?;
                ui.foundation()
                    .semantic_node(
                        surface.node,
                        SemanticNode {
                            participation: SemanticParticipation::Exclude,
                            ..SemanticNode::default()
                        },
                    )
                    .map_err(semantic_runtime_error)?;
                surfaces.push(WorkspaceOverviewSurfaceRef {
                    control: surface,
                    surface: placement.surface(),
                    source_bounds: source,
                    preview_bounds: projected,
                });
            }

            let item_name = ui.foundation().intern(workspace.name().as_str());
            let mut actions = SemanticActions::NONE;
            if authorized {
                actions |= SemanticActions::FOCUS;
            }
            if available {
                actions |= SemanticActions::ACTIVATE;
            }
            ui.foundation()
                .semantic_node(
                    item.node,
                    SemanticNode {
                        role: SemanticRole::Button,
                        name: SemanticName::Text(item_name),
                        state: SemanticState {
                            disabled: !authorized,
                            focusable: authorized,
                            selected: Some(workspace.active()),
                            ..SemanticState::default()
                        },
                        actions,
                        ..SemanticNode::default()
                    },
                )
                .map_err(semantic_runtime_error)?;
            if available {
                let map = Rc::clone(&map);
                let workspace_id = workspace.id();
                let revision = workspace.revision();
                ui.route_activation(item.node, move |activation| {
                    map(WorkspaceSelectionIntent::new(
                        workspace_id,
                        revision,
                        activation,
                    ))
                })?;
            }
            items.push(WorkspaceOverviewItemRef {
                control: item,
                preview,
                workspace: workspace.id(),
                active: workspace.active(),
                available,
                surfaces,
            });
        }
        Ok(WorkspaceOverviewRef {
            container,
            catalog: self.catalog.clone(),
            output: output.output(),
            output_revision: output.revision(),
            items,
        })
    }
}

fn validate_style(style: WorkspaceOverviewStyle) -> Result<(), WorkspaceOverviewError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(WorkspaceOverviewError::InvalidLabelSize);
    }
    if !style.preview_scale.is_finite() || style.preview_scale <= 0.0 || style.preview_scale > 1.0 {
        return Err(WorkspaceOverviewError::InvalidPreviewScale);
    }
    Ok(())
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid workspace-overview semantics: {error:?}"))
}

#[derive(Clone, Debug)]
pub struct WorkspaceOverviewRef {
    container: ControlHandle,
    catalog: WorkspaceCatalog,
    output: OutputId,
    output_revision: OutputRevision,
    items: Vec<WorkspaceOverviewItemRef>,
}

impl WorkspaceOverviewRef {
    pub const fn node(&self) -> UiNodeId {
        self.container.node
    }

    pub fn catalog(&self) -> &WorkspaceCatalog {
        &self.catalog
    }

    pub const fn output(&self) -> OutputId {
        self.output
    }

    pub const fn output_revision(&self) -> OutputRevision {
        self.output_revision
    }

    pub fn items(&self) -> &[WorkspaceOverviewItemRef] {
        &self.items
    }

    pub fn item(&self, workspace: WorkspaceId) -> Option<&WorkspaceOverviewItemRef> {
        self.items.iter().find(|item| item.workspace == workspace)
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceOverviewItemRef {
    control: ControlHandle,
    preview: ControlHandle,
    workspace: WorkspaceId,
    active: bool,
    available: bool,
    surfaces: Vec<WorkspaceOverviewSurfaceRef>,
}

impl WorkspaceOverviewItemRef {
    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn preview_node(&self) -> UiNodeId {
        self.preview.node
    }

    pub const fn workspace(&self) -> WorkspaceId {
        self.workspace
    }

    pub const fn active(&self) -> bool {
        self.active
    }

    pub const fn available(&self) -> bool {
        self.available
    }

    pub fn surfaces(&self) -> &[WorkspaceOverviewSurfaceRef] {
        &self.surfaces
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WorkspaceOverviewSurfaceRef {
    control: ControlHandle,
    surface: SurfaceId,
    source_bounds: RectF,
    preview_bounds: RectF,
}

impl WorkspaceOverviewSurfaceRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    pub const fn source_bounds(self) -> RectF {
        self.source_bounds
    }

    pub const fn preview_bounds(self) -> RectF {
        self.preview_bounds
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceOverviewError {
    Catalog(WorkspaceCatalogError),
    OverlayBoundary(WorkspaceSwitcherError),
    MissingAccessibleName,
    InvalidLabelSize,
    InvalidPreviewScale,
    OutputMismatch,
}

impl fmt::Display for WorkspaceOverviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid workspace overview: {self:?}")
    }
}

impl std::error::Error for WorkspaceOverviewError {}

impl From<WorkspaceCatalogError> for WorkspaceOverviewError {
    fn from(value: WorkspaceCatalogError) -> Self {
        Self::Catalog(value)
    }
}

#[derive(Debug)]
pub enum WorkspaceOverviewMountError {
    Overview(WorkspaceOverviewError),
    Runtime(RuntimeError),
}

impl fmt::Display for WorkspaceOverviewMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overview(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WorkspaceOverviewMountError {}

impl From<WorkspaceOverviewError> for WorkspaceOverviewMountError {
    fn from(value: WorkspaceOverviewError) -> Self {
        Self::Overview(value)
    }
}

impl From<RuntimeError> for WorkspaceOverviewMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
