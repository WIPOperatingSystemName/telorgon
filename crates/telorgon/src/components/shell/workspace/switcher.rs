//! Compact controlled workspace selection over an exact ordered host catalog.

use std::collections::HashSet;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use crate::foundation::ColorRgba8;
use crate::input::{Activation, ChangeSource};
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    InputSource, ShellCapabilities, ShellLayerKind, WorkspaceId, WorkspaceRequest,
    WorkspaceRevision, WorkspaceSnapshot,
};
use crate::components::shell::primitives::{ShellLayerRef, ShellRootRef};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, SemanticActions, SemanticName, SemanticNode,
    SemanticRole, SemanticState, UiNodeId,
};

/// Bounded host order shared by workspace switcher and overview presentations.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceCatalog(Arc<[WorkspaceSnapshot]>);

impl WorkspaceCatalog {
    pub const MAX_WORKSPACES: usize = 256;

    pub fn new(workspaces: Vec<WorkspaceSnapshot>) -> Result<Self, WorkspaceCatalogError> {
        if workspaces.is_empty() {
            return Err(WorkspaceCatalogError::Empty);
        }
        if workspaces.len() > Self::MAX_WORKSPACES {
            return Err(WorkspaceCatalogError::TooMany {
                count: workspaces.len(),
                max: Self::MAX_WORKSPACES,
            });
        }
        let mut seen = HashSet::with_capacity(workspaces.len());
        let mut active = None;
        for (index, workspace) in workspaces.iter().enumerate() {
            if !seen.insert(workspace.id()) {
                return Err(WorkspaceCatalogError::DuplicateWorkspace {
                    workspace: workspace.id(),
                });
            }
            if index > 0 && workspaces[index - 1].order() >= workspace.order() {
                return Err(WorkspaceCatalogError::NonCanonicalOrder { index });
            }
            if workspace.active()
                && let Some(first) = active.replace(workspace.id())
            {
                return Err(WorkspaceCatalogError::MultipleActive {
                    first,
                    second: workspace.id(),
                });
            }
        }
        Ok(Self(workspaces.into()))
    }

    pub fn workspaces(&self) -> &[WorkspaceSnapshot] {
        &self.0
    }

    pub fn workspace(&self, workspace: WorkspaceId) -> Option<&WorkspaceSnapshot> {
        self.0.iter().find(|candidate| candidate.id() == workspace)
    }

    pub fn active(&self) -> Option<&WorkspaceSnapshot> {
        self.0.iter().find(|workspace| workspace.active())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceCatalogError {
    Empty,
    TooMany {
        count: usize,
        max: usize,
    },
    DuplicateWorkspace {
        workspace: WorkspaceId,
    },
    NonCanonicalOrder {
        index: usize,
    },
    MultipleActive {
        first: WorkspaceId,
        second: WorkspaceId,
    },
}

impl fmt::Display for WorkspaceCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid workspace catalog: {self:?}")
    }
}

impl std::error::Error for WorkspaceCatalogError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkspaceSwitcherStyle {
    pub container: BoxStyle,
    pub item: BoxStyle,
    pub layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for WorkspaceSwitcherStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            item: BoxStyle::default(),
            layout: LayoutStyle {
                flow: Flow::Horizontal,
                ..LayoutStyle::default()
            },
            label_color: ColorRgba8::rgba(245, 247, 250, 255),
            label_size: 14.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceSwitcher {
    label: String,
    catalog: WorkspaceCatalog,
    style: WorkspaceSwitcherStyle,
}

impl WorkspaceSwitcher {
    pub fn new(
        label: impl Into<String>,
        workspaces: Vec<WorkspaceSnapshot>,
    ) -> Result<Self, WorkspaceSwitcherError> {
        Self::from_catalog(label, WorkspaceCatalog::new(workspaces)?)
    }

    pub fn from_catalog(
        label: impl Into<String>,
        catalog: WorkspaceCatalog,
    ) -> Result<Self, WorkspaceSwitcherError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(WorkspaceSwitcherError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            catalog,
            style: WorkspaceSwitcherStyle::default(),
        })
    }

    pub fn style(mut self, style: WorkspaceSwitcherStyle) -> Result<Self, WorkspaceSwitcherError> {
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
        map: Map,
    ) -> Result<WorkspaceSwitcherRef, WorkspaceSwitcherMountError>
    where
        Action: 'static,
        Map: Fn(WorkspaceSelectionIntent) -> Action + 'static,
    {
        validate_style(self.style)?;
        validate_overlay(root, layer)?;
        let authorized = root.grant().permits(ShellCapabilities::SELECT_WORKSPACE);
        let container = ui
            .foundation()
            .container_node_under(
                layer.content_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("workspace-switcher overlay is stale"))?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                container.node,
                SemanticNode::named(SemanticRole::Toolbar, name),
            )
            .map_err(semantic_runtime_error)?;

        let map = Rc::new(map);
        let mut items = Vec::with_capacity(self.catalog.workspaces().len());
        for workspace in self.catalog.workspaces() {
            let available = authorized && !workspace.active();
            let node = ui
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
                .ok_or_else(|| RuntimeError::new("workspace-switcher container is stale"))?;
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
                    node.node,
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
                ui.route_activation(node.node, move |activation| {
                    map(WorkspaceSelectionIntent::new(
                        workspace_id,
                        revision,
                        activation,
                    ))
                })?;
            }
            items.push(WorkspaceSwitcherItemRef {
                control: node,
                workspace: workspace.id(),
                revision: workspace.revision(),
                active: workspace.active(),
                available,
            });
        }
        Ok(WorkspaceSwitcherRef {
            container,
            catalog: self.catalog.clone(),
            items,
        })
    }
}

pub(crate) fn validate_overlay(
    root: ShellRootRef,
    layer: ShellLayerRef,
) -> Result<(), WorkspaceSwitcherError> {
    if layer.kind() != ShellLayerKind::Overlay {
        return Err(WorkspaceSwitcherError::RequiresOverlayLayer);
    }
    if root.output() != layer.output() {
        return Err(WorkspaceSwitcherError::OutputMismatch);
    }
    if root.grant().token() != layer.authority().grant() {
        return Err(WorkspaceSwitcherError::GrantMismatch);
    }
    Ok(())
}

fn validate_style(style: WorkspaceSwitcherStyle) -> Result<(), WorkspaceSwitcherError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(WorkspaceSwitcherError::InvalidLabelSize);
    }
    Ok(())
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid workspace-switcher semantics: {error:?}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkspaceSelectionIntent {
    workspace: WorkspaceId,
    revision: WorkspaceRevision,
    activation: Activation,
}

impl WorkspaceSelectionIntent {
    pub(crate) const fn new(
        workspace: WorkspaceId,
        revision: WorkspaceRevision,
        activation: Activation,
    ) -> Self {
        Self {
            workspace,
            revision,
            activation,
        }
    }

    pub const fn workspace(self) -> WorkspaceId {
        self.workspace
    }

    pub const fn revision(self) -> WorkspaceRevision {
        self.revision
    }

    pub const fn activation(self) -> Activation {
        self.activation
    }

    pub const fn inferred_source(self) -> Option<InputSource> {
        match self.activation.source {
            ChangeSource::Pointer => None,
            ChangeSource::Keyboard | ChangeSource::Directional => Some(InputSource::Keyboard),
            ChangeSource::Accessibility => Some(InputSource::Accessibility),
            ChangeSource::Programmatic => Some(InputSource::Programmatic),
        }
    }

    pub fn inferred_request(self) -> Option<WorkspaceRequest> {
        self.inferred_source()
            .map(|source| WorkspaceRequest::Select {
                workspace: self.workspace,
                revision: self.revision,
                source,
            })
    }

    pub fn request(
        self,
        source: InputSource,
    ) -> Result<WorkspaceRequest, WorkspaceSelectionSourceError> {
        let matches = match self.activation.source {
            ChangeSource::Pointer => source.is_contact(),
            ChangeSource::Keyboard | ChangeSource::Directional => source == InputSource::Keyboard,
            ChangeSource::Accessibility => source == InputSource::Accessibility,
            ChangeSource::Programmatic => source == InputSource::Programmatic,
        };
        if !matches {
            return Err(WorkspaceSelectionSourceError::SourceMismatch);
        }
        Ok(WorkspaceRequest::Select {
            workspace: self.workspace,
            revision: self.revision,
            source,
        })
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceSwitcherRef {
    container: ControlHandle,
    catalog: WorkspaceCatalog,
    items: Vec<WorkspaceSwitcherItemRef>,
}

impl WorkspaceSwitcherRef {
    pub const fn node(&self) -> UiNodeId {
        self.container.node
    }

    pub fn catalog(&self) -> &WorkspaceCatalog {
        &self.catalog
    }

    pub fn items(&self) -> &[WorkspaceSwitcherItemRef] {
        &self.items
    }

    pub fn item(&self, workspace: WorkspaceId) -> Option<WorkspaceSwitcherItemRef> {
        self.items
            .iter()
            .copied()
            .find(|item| item.workspace == workspace)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WorkspaceSwitcherItemRef {
    control: ControlHandle,
    workspace: WorkspaceId,
    revision: WorkspaceRevision,
    active: bool,
    available: bool,
}

impl WorkspaceSwitcherItemRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn workspace(self) -> WorkspaceId {
        self.workspace
    }

    pub const fn revision(self) -> WorkspaceRevision {
        self.revision
    }

    pub const fn active(self) -> bool {
        self.active
    }

    pub const fn available(self) -> bool {
        self.available
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceSelectionSourceError {
    SourceMismatch,
}

impl fmt::Display for WorkspaceSelectionSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("workspace selection activation and shell input source do not match")
    }
}

impl std::error::Error for WorkspaceSelectionSourceError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceSwitcherError {
    Catalog(WorkspaceCatalogError),
    MissingAccessibleName,
    InvalidLabelSize,
    RequiresOverlayLayer,
    OutputMismatch,
    GrantMismatch,
}

impl fmt::Display for WorkspaceSwitcherError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid workspace switcher: {self:?}")
    }
}

impl std::error::Error for WorkspaceSwitcherError {}

impl From<WorkspaceCatalogError> for WorkspaceSwitcherError {
    fn from(value: WorkspaceCatalogError) -> Self {
        Self::Catalog(value)
    }
}

#[derive(Debug)]
pub enum WorkspaceSwitcherMountError {
    Switcher(WorkspaceSwitcherError),
    Runtime(RuntimeError),
}

impl fmt::Display for WorkspaceSwitcherMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Switcher(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WorkspaceSwitcherMountError {}

impl From<WorkspaceSwitcherError> for WorkspaceSwitcherMountError {
    fn from(value: WorkspaceSwitcherError) -> Self {
        Self::Switcher(value)
    }
}

impl From<RuntimeError> for WorkspaceSwitcherMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
