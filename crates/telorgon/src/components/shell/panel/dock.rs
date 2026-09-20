//! Dock presentation mounted inside an exact authorized panel.

use std::fmt;

use crate::foundation::ColorRgba8;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{ApplicationEntry, ApplicationId, ShellCapabilities};
use crate::components::shell::primitives::ShellRootRef;
use crate::ui::{BoxStyle, ControlHandle, Flow, LayoutStyle, SemanticRole, UiNodeId};

use crate::components::shell::launcher::{
    ApplicationActionIntent, ApplicationCatalog, ApplicationCatalogError, ApplicationItemRef,
    ApplicationPresentationStyle, mount_application_catalog,
};

use super::{PanelRef, TaskbarError, validate_panel};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockStyle {
    pub container: BoxStyle,
    pub item: BoxStyle,
    pub layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for DockStyle {
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
pub struct Dock {
    label: String,
    catalog: ApplicationCatalog,
    style: DockStyle,
}

impl Dock {
    pub fn new(
        label: impl Into<String>,
        applications: Vec<ApplicationEntry>,
    ) -> Result<Self, DockError> {
        Self::from_catalog(label, ApplicationCatalog::new(applications)?)
    }

    pub fn from_catalog(
        label: impl Into<String>,
        catalog: ApplicationCatalog,
    ) -> Result<Self, DockError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(DockError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            catalog,
            style: DockStyle::default(),
        })
    }

    pub fn style(mut self, style: DockStyle) -> Result<Self, DockError> {
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
    ) -> Result<DockRef, DockMountError>
    where
        Action: 'static,
        Map: Fn(ApplicationActionIntent) -> Action + 'static,
    {
        validate_style(self.style)?;
        validate_panel(root, panel).map_err(DockError::PanelBoundary)?;
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
                container_role: SemanticRole::Navigation,
                item_role: SemanticRole::Button,
            },
            root.grant()
                .permits(ShellCapabilities::INVOKE_SYSTEM_ACTION),
            map,
        )?;
        Ok(DockRef {
            container: presentation.container,
            catalog: self.catalog.clone(),
            items: presentation.items,
            panel,
        })
    }
}

fn validate_style(style: DockStyle) -> Result<(), DockError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(DockError::InvalidLabelSize);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct DockRef {
    container: ControlHandle,
    catalog: ApplicationCatalog,
    items: Vec<ApplicationItemRef>,
    panel: PanelRef,
}

impl DockRef {
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
pub enum DockError {
    Catalog(ApplicationCatalogError),
    PanelBoundary(TaskbarError),
    MissingAccessibleName,
    InvalidLabelSize,
}

impl fmt::Display for DockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid dock: {self:?}")
    }
}

impl std::error::Error for DockError {}

impl From<ApplicationCatalogError> for DockError {
    fn from(value: ApplicationCatalogError) -> Self {
        Self::Catalog(value)
    }
}

#[derive(Debug)]
pub enum DockMountError {
    Dock(DockError),
    Runtime(RuntimeError),
}

impl fmt::Display for DockMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dock(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DockMountError {}

impl From<DockError> for DockMountError {
    fn from(value: DockError) -> Self {
        Self::Dock(value)
    }
}

impl From<RuntimeError> for DockMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
