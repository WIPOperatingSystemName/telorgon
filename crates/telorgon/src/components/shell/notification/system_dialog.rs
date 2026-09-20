//! Critical notification dialog on an explicitly authorized system-modal layer.

use std::fmt;
use std::rc::Rc;

use crate::foundation::ColorRgba8;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    NotificationAction, NotificationActionId, NotificationDeliveryState, NotificationPriority,
    NotificationPrivacy, NotificationSnapshot, ShellCapabilities, ShellLayerKind,
};
use crate::components::shell::primitives::{ShellLayerRef, ShellRootRef};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, SemanticActions, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRole, SemanticState, UiNodeId,
};

use super::{NotificationActionIntent, notification_semantic_error};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemDialogStyle {
    pub container: BoxStyle,
    pub action: BoxStyle,
    pub layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for SystemDialogStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            action: BoxStyle::default(),
            layout: LayoutStyle {
                flow: Flow::Vertical,
                ..LayoutStyle::default()
            },
            label_color: ColorRgba8::rgba(245, 247, 250, 255),
            label_size: 15.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemDialog {
    label: String,
    notification: NotificationSnapshot,
    style: SystemDialogStyle,
}

impl SystemDialog {
    pub fn new(
        label: impl Into<String>,
        notification: NotificationSnapshot,
    ) -> Result<Self, SystemDialogError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(SystemDialogError::MissingAccessibleName);
        }
        if notification.priority() != NotificationPriority::Critical {
            return Err(SystemDialogError::RequiresCriticalPriority);
        }
        if notification.lifecycle().delivery == NotificationDeliveryState::Acknowledged {
            return Err(SystemDialogError::AcknowledgedNotification);
        }
        Ok(Self {
            label,
            notification,
            style: SystemDialogStyle::default(),
        })
    }

    pub fn style(mut self, style: SystemDialogStyle) -> Result<Self, SystemDialogError> {
        validate_style(style)?;
        self.style = style;
        Ok(self)
    }

    pub const fn notification(&self) -> &NotificationSnapshot {
        &self.notification
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        root: ShellRootRef,
        layer: ShellLayerRef,
        map: Map,
    ) -> Result<SystemDialogRef, SystemDialogMountError>
    where
        Action: 'static,
        Map: Fn(NotificationActionIntent) -> Action + 'static,
    {
        validate_style(self.style)?;
        validate_system_modal(root, layer)?;
        let redacted = self.notification.privacy() == NotificationPrivacy::Secret;
        let container = ui
            .foundation()
            .container_node_under(
                layer.content_node(),
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
            .ok_or_else(|| RuntimeError::new("system-modal layer is stale"))?;
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
                container.node,
                SemanticNode {
                    role: SemanticRole::Dialog,
                    name: SemanticName::Text(name),
                    description,
                    ..SemanticNode::default()
                },
            )
            .map_err(notification_semantic_error)?;

        let authorized = root
            .grant()
            .permits(ShellCapabilities::INVOKE_SYSTEM_ACTION);
        let map = Rc::new(map);
        let mut actions = Vec::with_capacity(self.notification.actions().len());
        for action in self.notification.actions() {
            let available = authorized && !redacted && action.enabled();
            let node = ui
                .foundation()
                .action_node_under(
                    container.node,
                    self.style.action,
                    available,
                    true,
                    |writer| {
                        if !redacted {
                            writer.text(
                                action.label().as_str(),
                                self.style.label_color,
                                self.style.label_size,
                            );
                        }
                    },
                )
                .ok_or_else(|| RuntimeError::new("system dialog is stale"))?;
            if redacted {
                ui.foundation()
                    .semantic_node(
                        node.node,
                        SemanticNode {
                            participation: SemanticParticipation::Exclude,
                            ..SemanticNode::default()
                        },
                    )
                    .map_err(notification_semantic_error)?;
            } else {
                let action_name = ui.foundation().intern(action.label().as_str());
                let semantic_actions = if available {
                    SemanticActions::FOCUS | SemanticActions::ACTIVATE
                } else {
                    SemanticActions::NONE
                };
                ui.foundation()
                    .semantic_node(
                        node.node,
                        SemanticNode {
                            role: SemanticRole::Button,
                            name: SemanticName::Text(action_name),
                            state: SemanticState {
                                disabled: !available,
                                focusable: available,
                                ..SemanticState::default()
                            },
                            actions: semantic_actions,
                            ..SemanticNode::default()
                        },
                    )
                    .map_err(notification_semantic_error)?;
            }
            if available {
                let map = Rc::clone(&map);
                let notification = self.notification.id();
                let revision = self.notification.revision();
                let action_id = action.id();
                ui.route_activation(node.node, move |activation| {
                    map(NotificationActionIntent::new(
                        notification,
                        revision,
                        action_id,
                        activation,
                    ))
                })?;
            }
            actions.push(SystemDialogActionRef {
                control: node,
                action: action.clone(),
                available,
                redacted,
            });
        }
        Ok(SystemDialogRef {
            container,
            notification: self.notification.clone(),
            actions,
            redacted,
        })
    }
}

fn validate_style(style: SystemDialogStyle) -> Result<(), SystemDialogError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(SystemDialogError::InvalidLabelSize);
    }
    Ok(())
}

fn validate_system_modal(
    root: ShellRootRef,
    layer: ShellLayerRef,
) -> Result<(), SystemDialogError> {
    if layer.kind() != ShellLayerKind::SystemModal {
        return Err(SystemDialogError::RequiresSystemModalLayer);
    }
    if root.output() != layer.output() {
        return Err(SystemDialogError::OutputMismatch);
    }
    if root.grant().token() != layer.authority().grant() {
        return Err(SystemDialogError::GrantMismatch);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct SystemDialogRef {
    container: ControlHandle,
    notification: NotificationSnapshot,
    actions: Vec<SystemDialogActionRef>,
    redacted: bool,
}

impl SystemDialogRef {
    pub const fn node(&self) -> UiNodeId {
        self.container.node
    }

    pub const fn notification(&self) -> &NotificationSnapshot {
        &self.notification
    }

    pub fn actions(&self) -> &[SystemDialogActionRef] {
        &self.actions
    }

    pub fn action(&self, action: NotificationActionId) -> Option<&SystemDialogActionRef> {
        self.actions
            .iter()
            .find(|candidate| candidate.action.id() == action)
    }

    pub const fn redacted(&self) -> bool {
        self.redacted
    }

    pub const fn requires_lower_layers_inert(&self) -> bool {
        true
    }
}

#[derive(Clone, Debug)]
pub struct SystemDialogActionRef {
    control: ControlHandle,
    action: NotificationAction,
    available: bool,
    redacted: bool,
}

impl SystemDialogActionRef {
    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn action(&self) -> &NotificationAction {
        &self.action
    }

    pub const fn available(&self) -> bool {
        self.available
    }

    pub const fn redacted(&self) -> bool {
        self.redacted
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SystemDialogError {
    MissingAccessibleName,
    RequiresCriticalPriority,
    AcknowledgedNotification,
    InvalidLabelSize,
    RequiresSystemModalLayer,
    OutputMismatch,
    GrantMismatch,
}

impl fmt::Display for SystemDialogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid system dialog: {self:?}")
    }
}

impl std::error::Error for SystemDialogError {}

#[derive(Debug)]
pub enum SystemDialogMountError {
    Dialog(SystemDialogError),
    Runtime(RuntimeError),
}

impl fmt::Display for SystemDialogMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dialog(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SystemDialogMountError {}

impl From<SystemDialogError> for SystemDialogMountError {
    fn from(value: SystemDialogError) -> Self {
        Self::Dialog(value)
    }
}

impl From<RuntimeError> for SystemDialogMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
