//! Start-menu presentation over caller-ordered host application entries.

use std::fmt;

use crate::foundation::ColorRgba8;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{ApplicationEntry, ApplicationId, ShellCapabilities};
use crate::components::shell::primitives::{ShellLayerRef, ShellRootRef};
use crate::ui::{BoxStyle, ControlHandle, Flow, LayoutStyle, SemanticRole, UiNodeId};

use super::{
    ApplicationActionIntent, ApplicationCatalog, ApplicationCatalogError, ApplicationItemRef,
    ApplicationPresentationStyle, LauncherError, mount_application_catalog,
    validate_launcher_layer,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartMenuStyle {
    pub container: BoxStyle,
    pub item: BoxStyle,
    pub layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for StartMenuStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            item: BoxStyle::default(),
            layout: LayoutStyle {
                flow: Flow::Vertical,
                ..LayoutStyle::default()
            },
            label_color: ColorRgba8::rgba(245, 247, 250, 255),
            label_size: 14.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StartMenu {
    label: String,
    catalog: ApplicationCatalog,
    style: StartMenuStyle,
}

impl StartMenu {
    pub fn new(
        label: impl Into<String>,
        applications: Vec<ApplicationEntry>,
    ) -> Result<Self, StartMenuError> {
        Self::from_catalog(label, ApplicationCatalog::new(applications)?)
    }

    pub fn from_catalog(
        label: impl Into<String>,
        catalog: ApplicationCatalog,
    ) -> Result<Self, StartMenuError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(StartMenuError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            catalog,
            style: StartMenuStyle::default(),
        })
    }

    pub fn style(mut self, style: StartMenuStyle) -> Result<Self, StartMenuError> {
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
        layer: ShellLayerRef,
        map: Map,
    ) -> Result<StartMenuRef, StartMenuMountError>
    where
        Action: 'static,
        Map: Fn(ApplicationActionIntent) -> Action + 'static,
    {
        validate_style(self.style)?;
        validate_launcher_layer(root, layer).map_err(StartMenuError::LauncherBoundary)?;
        let presentation = mount_application_catalog(
            ui,
            layer.content_node(),
            &self.label,
            &self.catalog,
            ApplicationPresentationStyle {
                container: self.style.container,
                item: self.style.item,
                layout: self.style.layout,
                label_color: self.style.label_color,
                label_size: self.style.label_size,
                container_role: SemanticRole::Menu,
                item_role: SemanticRole::MenuItem,
            },
            root.grant()
                .permits(ShellCapabilities::INVOKE_SYSTEM_ACTION),
            map,
        )?;
        Ok(StartMenuRef {
            container: presentation.container,
            catalog: self.catalog.clone(),
            items: presentation.items,
        })
    }
}

fn validate_style(style: StartMenuStyle) -> Result<(), StartMenuError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(StartMenuError::InvalidLabelSize);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct StartMenuRef {
    container: ControlHandle,
    catalog: ApplicationCatalog,
    items: Vec<ApplicationItemRef>,
}

impl StartMenuRef {
    pub const fn node(&self) -> UiNodeId {
        self.container.node
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
pub enum StartMenuError {
    Catalog(ApplicationCatalogError),
    LauncherBoundary(LauncherError),
    MissingAccessibleName,
    InvalidLabelSize,
}

impl fmt::Display for StartMenuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid start menu: {self:?}")
    }
}

impl std::error::Error for StartMenuError {}

impl From<ApplicationCatalogError> for StartMenuError {
    fn from(value: ApplicationCatalogError) -> Self {
        Self::Catalog(value)
    }
}

#[derive(Debug)]
pub enum StartMenuMountError {
    Menu(StartMenuError),
    Runtime(RuntimeError),
}

impl fmt::Display for StartMenuMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Menu(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for StartMenuMountError {}

impl From<StartMenuError> for StartMenuMountError {
    fn from(value: StartMenuError) -> Self {
        Self::Menu(value)
    }
}

impl From<RuntimeError> for StartMenuMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
