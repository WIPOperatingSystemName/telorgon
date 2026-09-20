//! Controlled noninteractive on-screen display over one host notification snapshot.

use std::fmt;

use crate::foundation::ColorRgba8;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{NotificationPrivacy, NotificationSnapshot};
use crate::components::shell::primitives::{ShellLayerRef, ShellRootRef};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, SemanticName, SemanticNode, SemanticParticipation,
    SemanticRole, SemanticState, UiNodeId,
};

use super::{NotificationHostError, notification_semantic_error, validate_overlay};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnScreenDisplayStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for OnScreenDisplayStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            layout: LayoutStyle::default(),
            label_color: ColorRgba8::rgba(245, 247, 250, 255),
            label_size: 16.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OnScreenDisplay {
    label: String,
    notification: NotificationSnapshot,
    visible: bool,
    style: OnScreenDisplayStyle,
}

impl OnScreenDisplay {
    pub fn new(
        label: impl Into<String>,
        notification: NotificationSnapshot,
        visible: bool,
    ) -> Result<Self, OnScreenDisplayError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(OnScreenDisplayError::MissingAccessibleName);
        }
        if !notification.actions().is_empty() {
            return Err(OnScreenDisplayError::InteractiveNotification);
        }
        Ok(Self {
            label,
            notification,
            visible,
            style: OnScreenDisplayStyle::default(),
        })
    }

    pub fn style(mut self, style: OnScreenDisplayStyle) -> Result<Self, OnScreenDisplayError> {
        validate_style(style)?;
        self.style = style;
        Ok(self)
    }

    pub const fn notification(&self) -> &NotificationSnapshot {
        &self.notification
    }

    pub const fn visible(&self) -> bool {
        self.visible
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        root: ShellRootRef,
        layer: ShellLayerRef,
    ) -> Result<OnScreenDisplayRef, OnScreenDisplayMountError> {
        validate_style(self.style)?;
        validate_overlay(root, layer).map_err(OnScreenDisplayError::OverlayBoundary)?;
        let redacted = self.notification.privacy() == NotificationPrivacy::Secret;
        let control = ui
            .foundation()
            .layer_node_under(
                layer.content_node(),
                self.visible,
                self.style.container,
                self.style.layout,
                |writer| {
                    if !redacted {
                        writer.text(
                            self.notification.title().as_str(),
                            self.style.label_color,
                            self.style.label_size,
                        );
                        if self.notification.privacy() == NotificationPrivacy::Public
                            && let Some(body) = self.notification.body()
                        {
                            writer.text(
                                body.as_str(),
                                self.style.label_color,
                                self.style.label_size,
                            );
                        }
                    }
                },
            )
            .ok_or_else(|| RuntimeError::new("OSD overlay is stale"))?;
        if redacted {
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        participation: SemanticParticipation::Exclude,
                        ..SemanticNode::default()
                    },
                )
                .map_err(notification_semantic_error)?;
        } else {
            let name = ui.foundation().intern(&self.label);
            let description = if self.notification.privacy() == NotificationPrivacy::Public {
                self.notification
                    .body()
                    .map(|body| ui.foundation().intern(body.as_str()))
            } else {
                None
            };
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        role: SemanticRole::Status,
                        name: SemanticName::Text(name),
                        description,
                        state: SemanticState {
                            hidden: !self.visible,
                            inert: !self.visible,
                            ..SemanticState::default()
                        },
                        ..SemanticNode::default()
                    },
                )
                .map_err(notification_semantic_error)?;
        }
        Ok(OnScreenDisplayRef {
            control,
            notification: self.notification.clone(),
            visible: self.visible,
            redacted,
        })
    }
}

fn validate_style(style: OnScreenDisplayStyle) -> Result<(), OnScreenDisplayError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(OnScreenDisplayError::InvalidLabelSize);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct OnScreenDisplayRef {
    control: ControlHandle,
    notification: NotificationSnapshot,
    visible: bool,
    redacted: bool,
}

impl OnScreenDisplayRef {
    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn notification(&self) -> &NotificationSnapshot {
        &self.notification
    }

    pub const fn visible(&self) -> bool {
        self.visible
    }

    pub const fn redacted(&self) -> bool {
        self.redacted
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnScreenDisplayError {
    OverlayBoundary(NotificationHostError),
    MissingAccessibleName,
    InteractiveNotification,
    InvalidLabelSize,
}

impl fmt::Display for OnScreenDisplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid on-screen display: {self:?}")
    }
}

impl std::error::Error for OnScreenDisplayError {}

#[derive(Debug)]
pub enum OnScreenDisplayMountError {
    Display(OnScreenDisplayError),
    Runtime(RuntimeError),
}

impl fmt::Display for OnScreenDisplayMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Display(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for OnScreenDisplayMountError {}

impl From<OnScreenDisplayError> for OnScreenDisplayMountError {
    fn from(value: OnScreenDisplayError) -> Self {
        Self::Display(value)
    }
}

impl From<RuntimeError> for OnScreenDisplayMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
