//! Taskbar presentation mounted inside an exact authorized panel.

use std::fmt;

use crate::foundation::ColorRgba8;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{ApplicationEntry, ApplicationId, ShellCapabilities};
use crate::ui::{BoxStyle, ControlHandle, Flow, LayoutStyle, SemanticRole, UiNodeId};

use crate::components::shell::launcher::{
    ApplicationActionIntent, ApplicationCatalog, ApplicationCatalogError, ApplicationItemRef,
    ApplicationPresentationStyle, mount_application_catalog,
};

use super::PanelRef;
use crate::components::shell::primitives::ShellRootRef;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskbarStyle {
    pub container: BoxStyle,
    pub item: BoxStyle,
    pub layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for TaskbarStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            item: BoxStyle::default(),
            layout: LayoutStyle {
                flow: Flow::Horizontal,
                ..LayoutStyle::default()
            },
            label_color: ColorRgba8::rgba(245, 247, 250, 255),
            label_size: 13.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Taskbar {
    label: String,
    catalog: ApplicationCatalog,
    style: TaskbarStyle,
}

impl Taskbar {
    pub fn new(
        label: impl Into<String>,
        applications: Vec<ApplicationEntry>,
    ) -> Result<Self, TaskbarError> {
        Self::from_catalog(label, ApplicationCatalog::new(applications)?)
    }

    pub fn from_catalog(
        label: impl Into<String>,
        catalog: ApplicationCatalog,
    ) -> Result<Self, TaskbarError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TaskbarError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            catalog,
            style: TaskbarStyle::default(),
        })
    }

    pub fn style(mut self, style: TaskbarStyle) -> Result<Self, TaskbarError> {
        validate_style(style)?;
        self.style = style;
        Ok(self)
    }

    pub fn catalog(&self) -> &ApplicationCatalog {
        &self.catalog
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        root: ShellRootRef,
        panel: PanelRef,
        map: Map,
    ) -> Result<TaskbarRef, TaskbarMountError>
    where
        Action: 'static,
        Map: Fn(ApplicationActionIntent) -> Action + 'static,
    {
        validate_style(self.style)?;
        validate_panel(root, panel)?;
        let presentation = mount_application_catalog(
            ui,
            panel.content_node(),
            &self.label,
            &self.catalog,
            ApplicationPresentationStyle {
                container: self.style.container,
                item: self.style.item,
                layout: self.style.layout,
                label_color: self.style.label_color,
                label_size: self.style.label_size,
                container_role: SemanticRole::Toolbar,
                item_role: SemanticRole::Button,
            },
            root.grant()
                .permits(ShellCapabilities::INVOKE_SYSTEM_ACTION),
            map,
        )?;
        Ok(TaskbarRef {
            container: presentation.container,
            catalog: self.catalog.clone(),
            items: presentation.items,
            panel,
        })
    }
}

fn validate_style(style: TaskbarStyle) -> Result<(), TaskbarError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(TaskbarError::InvalidLabelSize);
    }
    Ok(())
}

pub(crate) fn validate_panel(root: ShellRootRef, panel: PanelRef) -> Result<(), TaskbarError> {
    if root.output() != panel.output() {
        return Err(TaskbarError::OutputMismatch);
    }
    if root.grant().token() != panel.reservation().grant() {
        return Err(TaskbarError::GrantMismatch);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct TaskbarRef {
    container: ControlHandle,
    catalog: ApplicationCatalog,
    items: Vec<ApplicationItemRef>,
    panel: PanelRef,
}

impl TaskbarRef {
    pub const fn node(&self) -> UiNodeId {
        self.container.node
    }

    pub const fn panel(&self) -> PanelRef {
        self.panel
    }

    pub fn catalog(&self) -> &ApplicationCatalog {
        &self.catalog
    }

    pub fn items(&self) -> &[ApplicationItemRef] {
        &self.items
    }

    pub fn item(&self, application: ApplicationId) -> Option<ApplicationItemRef> {
        self.items
            .iter()
            .copied()
            .find(|item| item.application() == application)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskbarError {
    Catalog(ApplicationCatalogError),
    MissingAccessibleName,
    InvalidLabelSize,
    OutputMismatch,
    GrantMismatch,
}

impl fmt::Display for TaskbarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid taskbar: {self:?}")
    }
}

impl std::error::Error for TaskbarError {}

impl From<ApplicationCatalogError> for TaskbarError {
    fn from(value: ApplicationCatalogError) -> Self {
        Self::Catalog(value)
    }
}

#[derive(Debug)]
pub enum TaskbarMountError {
    Taskbar(TaskbarError),
    Runtime(RuntimeError),
}

impl fmt::Display for TaskbarMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Taskbar(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TaskbarMountError {}

impl From<TaskbarError> for TaskbarMountError {
    fn from(value: TaskbarError) -> Self {
        Self::Taskbar(value)
    }
}

impl From<RuntimeError> for TaskbarMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
