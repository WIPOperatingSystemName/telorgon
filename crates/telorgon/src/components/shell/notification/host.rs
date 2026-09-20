//! Privacy-aware transient notification presentation over exact host snapshots.

use std::collections::HashSet;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use crate::foundation::ColorRgba8;
use crate::input::{Activation, ChangeSource};
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    InputSource, NotificationAction, NotificationActionId, NotificationDeliveryState,
    NotificationId, NotificationPriority, NotificationPrivacy, NotificationRevision,
    NotificationSnapshot, ShellCapabilities, ShellLayerKind, SystemRequest,
};
use crate::components::shell::primitives::{ShellLayerRef, ShellRootRef};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, SemanticActions, SemanticCollection, SemanticName,
    SemanticNode, SemanticParticipation, SemanticRole, SemanticState, UiNodeId,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationCatalog(Arc<[NotificationSnapshot]>);

impl NotificationCatalog {
    pub const MAX_NOTIFICATIONS: usize = 4096;

    pub fn new(notifications: Vec<NotificationSnapshot>) -> Result<Self, NotificationCatalogError> {
        if notifications.len() > Self::MAX_NOTIFICATIONS {
            return Err(NotificationCatalogError::TooMany {
                count: notifications.len(),
                max: Self::MAX_NOTIFICATIONS,
            });
        }
        let mut seen = HashSet::with_capacity(notifications.len());
        if let Some(notification) = notifications
            .iter()
            .map(NotificationSnapshot::id)
            .find(|notification| !seen.insert(*notification))
        {
            return Err(NotificationCatalogError::DuplicateNotification { notification });
        }
        Ok(Self(notifications.into()))
    }

    pub fn notifications(&self) -> &[NotificationSnapshot] {
        &self.0
    }

    pub fn notification(&self, notification: NotificationId) -> Option<&NotificationSnapshot> {
        self.0
            .iter()
            .find(|candidate| candidate.id() == notification)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationCatalogError {
    TooMany { count: usize, max: usize },
    DuplicateNotification { notification: NotificationId },
}

impl fmt::Display for NotificationCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid notification catalog: {self:?}")
    }
}

impl std::error::Error for NotificationCatalogError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotificationActionIntent {
    notification: NotificationId,
    revision: NotificationRevision,
    action: NotificationActionId,
    activation: Activation,
}

impl NotificationActionIntent {
    pub(crate) const fn new(
        notification: NotificationId,
        revision: NotificationRevision,
        action: NotificationActionId,
        activation: Activation,
    ) -> Self {
        Self {
            notification,
            revision,
            action,
            activation,
        }
    }

    pub const fn notification(self) -> NotificationId {
        self.notification
    }

    pub const fn revision(self) -> NotificationRevision {
        self.revision
    }

    pub const fn action(self) -> NotificationActionId {
        self.action
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

    pub fn inferred_request(self) -> Option<SystemRequest> {
        self.inferred_source()
            .map(|source| self.build_request(source))
    }

    pub fn request(
        self,
        source: InputSource,
    ) -> Result<SystemRequest, NotificationActionSourceError> {
        let matches = match self.activation.source {
            ChangeSource::Pointer => source.is_contact(),
            ChangeSource::Keyboard | ChangeSource::Directional => source == InputSource::Keyboard,
            ChangeSource::Accessibility => source == InputSource::Accessibility,
            ChangeSource::Programmatic => source == InputSource::Programmatic,
        };
        if !matches {
            return Err(NotificationActionSourceError::SourceMismatch);
        }
        Ok(self.build_request(source))
    }

    const fn build_request(self, source: InputSource) -> SystemRequest {
        SystemRequest::NotificationAction {
            notification: self.notification,
            revision: self.revision,
            action: self.action,
            source,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationActionSourceError {
    SourceMismatch,
}

impl fmt::Display for NotificationActionSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("notification activation and shell input source do not match")
    }
}

impl std::error::Error for NotificationActionSourceError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NotificationHostStyle {
    pub container: BoxStyle,
    pub notification: BoxStyle,
    pub action: BoxStyle,
    pub layout: LayoutStyle,
    pub notification_layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for NotificationHostStyle {
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

#[derive(Clone, Debug, PartialEq)]
pub struct NotificationHost {
    label: String,
    catalog: NotificationCatalog,
    style: NotificationHostStyle,
}

impl NotificationHost {
    pub fn new(
        label: impl Into<String>,
        notifications: Vec<NotificationSnapshot>,
    ) -> Result<Self, NotificationHostError> {
        Self::from_catalog(label, NotificationCatalog::new(notifications)?)
    }

    pub fn from_catalog(
        label: impl Into<String>,
        catalog: NotificationCatalog,
    ) -> Result<Self, NotificationHostError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(NotificationHostError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            catalog,
            style: NotificationHostStyle::default(),
        })
    }

    pub fn style(mut self, style: NotificationHostStyle) -> Result<Self, NotificationHostError> {
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
    ) -> Result<NotificationHostRef, NotificationHostMountError>
    where
        Action: 'static,
        Map: Fn(NotificationActionIntent) -> Action + 'static,
    {
        validate_style(self.style)?;
        validate_overlay(root, layer)?;
        let presentation = mount_notification_catalog(
            ui,
            layer.content_node(),
            &self.label,
            &self.catalog,
            NotificationPresentationMode::Transient,
            self.style,
            root.grant()
                .permits(ShellCapabilities::INVOKE_SYSTEM_ACTION),
            map,
        )?;
        Ok(NotificationHostRef {
            container: presentation.container,
            catalog: self.catalog.clone(),
            notifications: presentation.notifications,
        })
    }
}

fn validate_style(style: NotificationHostStyle) -> Result<(), NotificationHostError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(NotificationHostError::InvalidLabelSize);
    }
    Ok(())
}

pub(crate) fn validate_overlay(
    root: ShellRootRef,
    layer: ShellLayerRef,
) -> Result<(), NotificationHostError> {
    if layer.kind() != ShellLayerKind::Overlay {
        return Err(NotificationHostError::RequiresOverlayLayer);
    }
    if root.output() != layer.output() {
        return Err(NotificationHostError::OutputMismatch);
    }
    if root.grant().token() != layer.authority().grant() {
        return Err(NotificationHostError::GrantMismatch);
    }
    Ok(())
}

pub(crate) struct NotificationPresentationRef {
    pub(crate) container: ControlHandle,
    pub(crate) notifications: Vec<NotificationRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotificationPresentationMode {
    Transient,
    Center,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn mount_notification_catalog<Action, Map>(
    ui: &mut Ui<'_, '_, Action>,
    parent: UiNodeId,
    label: &str,
    catalog: &NotificationCatalog,
    mode: NotificationPresentationMode,
    style: NotificationHostStyle,
    authorized: bool,
    map: Map,
) -> Result<NotificationPresentationRef, RuntimeError>
where
    Action: 'static,
    Map: Fn(NotificationActionIntent) -> Action + 'static,
{
    let container = ui
        .foundation()
        .container_node_under(parent, style.container, style.layout, |_| {})
        .ok_or_else(|| RuntimeError::new("notification presentation parent is stale"))?;
    let name = ui.foundation().intern(label);
    let container_role = match mode {
        NotificationPresentationMode::Transient => SemanticRole::Region,
        NotificationPresentationMode::Center => SemanticRole::List,
    };
    ui.foundation()
        .semantic_node(container.node, SemanticNode::named(container_role, name))
        .map_err(notification_semantic_error)?;

    let item_count = u32::try_from(catalog.notifications().len())
        .map_err(|_| RuntimeError::new("notification count exceeds semantic bounds"))?;
    let map = Rc::new(map);
    let mut notifications = Vec::with_capacity(catalog.notifications().len());
    for (index, notification) in catalog.notifications().iter().enumerate() {
        let redacted = notification.privacy() == NotificationPrivacy::Secret;
        let acknowledged =
            notification.lifecycle().delivery == NotificationDeliveryState::Acknowledged;
        let presented = mode == NotificationPresentationMode::Center || !acknowledged;
        let card = ui
            .foundation()
            .container_node_under(
                container.node,
                style.notification,
                style.notification_layout,
                |writer| {
                    if !redacted {
                        writer.text(
                            notification.title().as_str(),
                            style.label_color,
                            style.label_size,
                        );
                        if notification.privacy() == NotificationPrivacy::Public
                            && let Some(body) = notification.body()
                        {
                            writer.text(body.as_str(), style.label_color, style.label_size);
                        }
                    }
                },
            )
            .ok_or_else(|| RuntimeError::new("notification presentation is stale"))?;
        if redacted {
            ui.foundation()
                .semantic_node(
                    card.node,
                    SemanticNode {
                        participation: SemanticParticipation::Exclude,
                        ..SemanticNode::default()
                    },
                )
                .map_err(notification_semantic_error)?;
        } else {
            let title = ui.foundation().intern(notification.title().as_str());
            let description = if notification.privacy() == NotificationPrivacy::Public {
                notification
                    .body()
                    .map(|body| ui.foundation().intern(body.as_str()))
            } else {
                None
            };
            let role = match mode {
                NotificationPresentationMode::Center => SemanticRole::ListItem,
                NotificationPresentationMode::Transient
                    if matches!(
                        notification.priority(),
                        NotificationPriority::High | NotificationPriority::Critical
                    ) =>
                {
                    SemanticRole::Alert
                }
                NotificationPresentationMode::Transient => SemanticRole::Status,
            };
            ui.foundation()
                .semantic_node(
                    card.node,
                    SemanticNode {
                        role,
                        name: SemanticName::Text(title),
                        description,
                        state: SemanticState {
                            hidden: !presented,
                            inert: !presented,
                            ..SemanticState::default()
                        },
                        collection: (mode == NotificationPresentationMode::Center).then_some(
                            SemanticCollection {
                                item_index: Some(u32::try_from(index).map_err(|_| {
                                    RuntimeError::new("notification index exceeds semantic bounds")
                                })?),
                                item_count: Some(item_count),
                                ..SemanticCollection::default()
                            },
                        ),
                        ..SemanticNode::default()
                    },
                )
                .map_err(notification_semantic_error)?;
        }

        let mut action_refs = Vec::with_capacity(notification.actions().len());
        for action in notification.actions() {
            let available = authorized && !redacted && presented && action.enabled();
            let action_node = ui
                .foundation()
                .action_node_under(card.node, style.action, available, true, |writer| {
                    if !redacted {
                        writer.text(action.label().as_str(), style.label_color, style.label_size);
                    }
                })
                .ok_or_else(|| RuntimeError::new("notification card is stale"))?;
            if redacted {
                ui.foundation()
                    .semantic_node(
                        action_node.node,
                        SemanticNode {
                            participation: SemanticParticipation::Exclude,
                            ..SemanticNode::default()
                        },
                    )
                    .map_err(notification_semantic_error)?;
            } else {
                let action_name = ui.foundation().intern(action.label().as_str());
                let actions = if available {
                    SemanticActions::FOCUS | SemanticActions::ACTIVATE
                } else {
                    SemanticActions::NONE
                };
                ui.foundation()
                    .semantic_node(
                        action_node.node,
                        SemanticNode {
                            role: SemanticRole::Button,
                            name: SemanticName::Text(action_name),
                            state: SemanticState {
                                disabled: !available,
                                focusable: available,
                                hidden: !presented,
                                inert: !presented,
                                ..SemanticState::default()
                            },
                            actions,
                            ..SemanticNode::default()
                        },
                    )
                    .map_err(notification_semantic_error)?;
            }
            if available {
                let map = Rc::clone(&map);
                let notification_id = notification.id();
                let revision = notification.revision();
                let action_id = action.id();
                ui.route_activation(action_node.node, move |activation| {
                    map(NotificationActionIntent::new(
                        notification_id,
                        revision,
                        action_id,
                        activation,
                    ))
                })?;
            }
            action_refs.push(NotificationActionRef {
                control: action_node,
                action: action.clone(),
                available,
                redacted,
            });
        }
        notifications.push(NotificationRef {
            card,
            snapshot: notification.clone(),
            actions: action_refs,
            redacted,
            presented,
        });
    }
    Ok(NotificationPresentationRef {
        container,
        notifications,
    })
}

pub(crate) fn notification_semantic_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid notification semantics: {error:?}"))
}

#[derive(Clone, Debug)]
pub struct NotificationHostRef {
    container: ControlHandle,
    catalog: NotificationCatalog,
    notifications: Vec<NotificationRef>,
}

impl NotificationHostRef {
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
            .find(|candidate| candidate.snapshot.id() == notification)
    }
}

#[derive(Clone, Debug)]
pub struct NotificationRef {
    card: ControlHandle,
    snapshot: NotificationSnapshot,
    actions: Vec<NotificationActionRef>,
    redacted: bool,
    presented: bool,
}

impl NotificationRef {
    pub const fn node(&self) -> UiNodeId {
        self.card.node
    }

    pub const fn snapshot(&self) -> &NotificationSnapshot {
        &self.snapshot
    }

    pub fn actions(&self) -> &[NotificationActionRef] {
        &self.actions
    }

    pub fn action(&self, action: NotificationActionId) -> Option<&NotificationActionRef> {
        self.actions
            .iter()
            .find(|candidate| candidate.action.id() == action)
    }

    pub const fn redacted(&self) -> bool {
        self.redacted
    }

    pub const fn presented(&self) -> bool {
        self.presented
    }

    pub fn presented_body(&self) -> Option<&crate::shell::NotificationText> {
        (self.snapshot.privacy() == NotificationPrivacy::Public)
            .then(|| self.snapshot.body())
            .flatten()
    }
}

#[derive(Clone, Debug)]
pub struct NotificationActionRef {
    control: ControlHandle,
    action: NotificationAction,
    available: bool,
    redacted: bool,
}

impl NotificationActionRef {
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
pub enum NotificationHostError {
    Catalog(NotificationCatalogError),
    MissingAccessibleName,
    InvalidLabelSize,
    RequiresOverlayLayer,
    OutputMismatch,
    GrantMismatch,
}

impl fmt::Display for NotificationHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid notification host: {self:?}")
    }
}

impl std::error::Error for NotificationHostError {}

impl From<NotificationCatalogError> for NotificationHostError {
    fn from(value: NotificationCatalogError) -> Self {
        Self::Catalog(value)
    }
}

#[derive(Debug)]
pub enum NotificationHostMountError {
    Notification(NotificationHostError),
    Runtime(RuntimeError),
}

impl fmt::Display for NotificationHostMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Notification(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for NotificationHostMountError {}

impl From<NotificationHostError> for NotificationHostMountError {
    fn from(value: NotificationHostError) -> Self {
        Self::Notification(value)
    }
}

impl From<RuntimeError> for NotificationHostMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
