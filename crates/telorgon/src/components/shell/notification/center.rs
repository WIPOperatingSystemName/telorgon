//! Persistent notification-center view over the exact host catalog.

use std::fmt;

use crate::foundation::ColorRgba8;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{NotificationId, NotificationSnapshot, ShellCapabilities};
use crate::components::shell::primitives::{ShellLayerRef, ShellRootRef};
use crate::ui::{BoxStyle, ControlHandle, Flow, LayoutStyle, UiNodeId};

use super::{
    NotificationActionIntent, NotificationCatalog, NotificationCatalogError, NotificationHostError,
    NotificationHostStyle, NotificationPresentationMode, NotificationRef,
    mount_notification_catalog, validate_overlay,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NotificationCenterStyle {
    pub container: BoxStyle,
    pub notification: BoxStyle,
    pub action: BoxStyle,
    pub layout: LayoutStyle,
    pub notification_layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for NotificationCenterStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            notification: BoxStyle::default(),
            action: BoxStyle::default(),
            layout: LayoutStyle {
                flow: Flow::Vertical,
                ..LayoutStyle::default()
            },
            notification_layout: LayoutStyle {
                flow: Flow::Vertical,
                ..LayoutStyle::default()
            },
            label_color: ColorRgba8::rgba(245, 247, 250, 255),
            label_size: 14.0,
        }
    }
}

impl From<NotificationCenterStyle> for NotificationHostStyle {
    fn from(value: NotificationCenterStyle) -> Self {
        Self {
            container: value.container,
            notification: value.notification,
            action: value.action,
            layout: value.layout,
            notification_layout: value.notification_layout,
            label_color: value.label_color,
            label_size: value.label_size,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NotificationCenter {
    label: String,
    catalog: NotificationCatalog,
    style: NotificationCenterStyle,
}

impl NotificationCenter {
    pub fn new(
        label: impl Into<String>,
        notifications: Vec<NotificationSnapshot>,
    ) -> Result<Self, NotificationCenterError> {
        Self::from_catalog(label, NotificationCatalog::new(notifications)?)
    }

    pub fn from_catalog(
        label: impl Into<String>,
        catalog: NotificationCatalog,
    ) -> Result<Self, NotificationCenterError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(NotificationCenterError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            catalog,
            style: NotificationCenterStyle::default(),
        })
    }

    pub fn style(
        mut self,
        style: NotificationCenterStyle,
    ) -> Result<Self, NotificationCenterError> {
        validate_style(style)?;
        self.style = style;
        Ok(self)
    }

    pub fn catalog(&self) -> &NotificationCatalog {
        &self.catalog
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        root: ShellRootRef,
        layer: ShellLayerRef,
        map: Map,
    ) -> Result<NotificationCenterRef, NotificationCenterMountError>
    where
        Action: 'static,
        Map: Fn(NotificationActionIntent) -> Action + 'static,
    {
        validate_style(self.style)?;
        validate_overlay(root, layer).map_err(NotificationCenterError::OverlayBoundary)?;
        let presentation = mount_notification_catalog(
            ui,
            layer.content_node(),
            &self.label,
            &self.catalog,
            NotificationPresentationMode::Center,
            self.style.into(),
            root.grant()
                .permits(ShellCapabilities::INVOKE_SYSTEM_ACTION),
            map,
        )?;
        Ok(NotificationCenterRef {
            container: presentation.container,
            catalog: self.catalog.clone(),
            notifications: presentation.notifications,
        })
    }
}

fn validate_style(style: NotificationCenterStyle) -> Result<(), NotificationCenterError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(NotificationCenterError::InvalidLabelSize);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct NotificationCenterRef {
    container: ControlHandle,
    catalog: NotificationCatalog,
    notifications: Vec<NotificationRef>,
}

impl NotificationCenterRef {
    pub const fn node(&self) -> UiNodeId {
        self.container.node
    }

    pub fn catalog(&self) -> &NotificationCatalog {
        &self.catalog
    }

    pub fn notifications(&self) -> &[NotificationRef] {
        &self.notifications
    }

    pub fn notification(&self, notification: NotificationId) -> Option<&NotificationRef> {
        self.notifications
            .iter()
            .find(|candidate| candidate.snapshot().id() == notification)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationCenterError {
    Catalog(NotificationCatalogError),
    OverlayBoundary(NotificationHostError),
    MissingAccessibleName,
    InvalidLabelSize,
}

impl fmt::Display for NotificationCenterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid notification center: {self:?}")
    }
}

impl std::error::Error for NotificationCenterError {}

impl From<NotificationCatalogError> for NotificationCenterError {
    fn from(value: NotificationCatalogError) -> Self {
        Self::Catalog(value)
    }
}

#[derive(Debug)]
pub enum NotificationCenterMountError {
    Center(NotificationCenterError),
    Runtime(RuntimeError),
}

impl fmt::Display for NotificationCenterMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Center(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for NotificationCenterMountError {}

impl From<NotificationCenterError> for NotificationCenterMountError {
    fn from(value: NotificationCenterError) -> Self {
        Self::Center(value)
    }
}

impl From<RuntimeError> for NotificationCenterMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
