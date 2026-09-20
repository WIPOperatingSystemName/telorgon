//! Application grid with caller-selected column addressing and exact host entries.

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
pub struct ApplicationGridStyle {
    pub container: BoxStyle,
    pub item: BoxStyle,
    pub layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for ApplicationGridStyle {
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
pub struct ApplicationGrid {
    label: String,
    catalog: ApplicationCatalog,
    columns: u32,
    style: ApplicationGridStyle,
}

impl ApplicationGrid {
    pub const MAX_COLUMNS: u32 = 64;

    pub fn new(
        label: impl Into<String>,
        applications: Vec<ApplicationEntry>,
        columns: u32,
    ) -> Result<Self, ApplicationGridError> {
        Self::from_catalog(label, ApplicationCatalog::new(applications)?, columns)
    }

    pub fn from_catalog(
        label: impl Into<String>,
        catalog: ApplicationCatalog,
        columns: u32,
    ) -> Result<Self, ApplicationGridError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ApplicationGridError::MissingAccessibleName);
        }
        if columns == 0 || columns > Self::MAX_COLUMNS {
            return Err(ApplicationGridError::InvalidColumnCount { columns });
        }
        Ok(Self {
            label,
            catalog,
            columns,
            style: ApplicationGridStyle::default(),
        })
    }

    pub fn style(mut self, style: ApplicationGridStyle) -> Result<Self, ApplicationGridError> {
        validate_style(style)?;
        self.style = style;
        Ok(self)
    }

    pub const fn columns(&self) -> u32 {
        self.columns
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
    ) -> Result<ApplicationGridRef, ApplicationGridMountError>
    where
        Action: 'static,
        Map: Fn(ApplicationActionIntent) -> Action + 'static,
    {
        validate_style(self.style)?;
        validate_launcher_layer(root, layer).map_err(ApplicationGridError::LauncherBoundary)?;
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
                container_role: SemanticRole::Grid,
                item_role: SemanticRole::Button,
            },
            root.grant()
                .permits(ShellCapabilities::INVOKE_SYSTEM_ACTION),
            map,
        )?;
        let columns = self.columns;
        let items = presentation
            .items
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                let index = u32::try_from(index)
                    .expect("application catalogs are bounded below the u32 limit");
                ApplicationGridItemRef {
                    item,
                    row: index / columns,
                    column: index % columns,
                }
            })
            .collect();
        Ok(ApplicationGridRef {
            container: presentation.container,
            catalog: self.catalog.clone(),
            columns,
            items,
        })
    }
}

fn validate_style(style: ApplicationGridStyle) -> Result<(), ApplicationGridError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(ApplicationGridError::InvalidLabelSize);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct ApplicationGridRef {
    container: ControlHandle,
    catalog: ApplicationCatalog,
    columns: u32,
    items: Vec<ApplicationGridItemRef>,
}

impl ApplicationGridRef {
    pub const fn node(&self) -> UiNodeId {
        self.container.node
    }

    pub const fn columns(&self) -> u32 {
        self.columns
    }

    pub fn catalog(&self) -> &ApplicationCatalog {
        &self.catalog
    }

    pub fn items(&self) -> &[ApplicationGridItemRef] {
        &self.items
    }

    pub fn item(&self, application: ApplicationId) -> Option<ApplicationGridItemRef> {
        self.items
            .iter()
            .copied()
            .find(|item| item.item.application() == application)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ApplicationGridItemRef {
    item: ApplicationItemRef,
    row: u32,
    column: u32,
}

impl ApplicationGridItemRef {
    pub const fn item(self) -> ApplicationItemRef {
        self.item
    }

    pub const fn node(self) -> UiNodeId {
        self.item.node()
    }

    pub const fn application(self) -> ApplicationId {
        self.item.application()
    }

    pub const fn row(self) -> u32 {
        self.row
    }

    pub const fn column(self) -> u32 {
        self.column
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationGridError {
    Catalog(ApplicationCatalogError),
    LauncherBoundary(LauncherError),
    MissingAccessibleName,
    InvalidColumnCount { columns: u32 },
    InvalidLabelSize,
}

impl fmt::Display for ApplicationGridError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid application grid: {self:?}")
    }
}

impl std::error::Error for ApplicationGridError {}

impl From<ApplicationCatalogError> for ApplicationGridError {
    fn from(value: ApplicationCatalogError) -> Self {
        Self::Catalog(value)
    }
}

#[derive(Debug)]
pub enum ApplicationGridMountError {
    Grid(ApplicationGridError),
    Runtime(RuntimeError),
}

impl fmt::Display for ApplicationGridMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Grid(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ApplicationGridMountError {}

impl From<ApplicationGridError> for ApplicationGridMountError {
    fn from(value: ApplicationGridError) -> Self {
        Self::Grid(value)
    }
}

impl From<RuntimeError> for ApplicationGridMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
