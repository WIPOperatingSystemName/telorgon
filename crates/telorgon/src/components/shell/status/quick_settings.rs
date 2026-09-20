//! Overlay quick-settings menu over every exact host status action.

use std::fmt;
use std::rc::Rc;

use crate::foundation::ColorRgba8;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    ShellCapabilities, ShellLayerKind, StatusAction, StatusActionId, StatusEntryId, StatusPrivacy,
    StatusSeverity, SystemStatusRevision, SystemStatusSnapshot,
};
use crate::components::shell::primitives::{ShellLayerRef, ShellRootRef};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, SemanticActions, SemanticCollection, SemanticName,
    SemanticNode, SemanticParticipation, SemanticRole, SemanticState, UiNodeId,
};

use super::{StatusActionIntent, status_semantic_error};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuickSettingsStyle {
    pub container: BoxStyle,
    pub item: BoxStyle,
    pub layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for QuickSettingsStyle {
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
pub struct QuickSettings {
    label: String,
    snapshot: SystemStatusSnapshot,
    style: QuickSettingsStyle,
}

impl QuickSettings {
    pub fn new(
        label: impl Into<String>,
        snapshot: SystemStatusSnapshot,
    ) -> Result<Self, QuickSettingsError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(QuickSettingsError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            snapshot,
            style: QuickSettingsStyle::default(),
        })
    }

    pub fn style(mut self, style: QuickSettingsStyle) -> Result<Self, QuickSettingsError> {
        validate_style(style)?;
        self.style = style;
        Ok(self)
    }

    pub const fn snapshot(&self) -> &SystemStatusSnapshot {
        &self.snapshot
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        root: ShellRootRef,
        layer: ShellLayerRef,
        map: Map,
    ) -> Result<QuickSettingsRef, QuickSettingsMountError>
    where
        Action: 'static,
        Map: Fn(StatusActionIntent) -> Action + 'static,
    {
        validate_style(self.style)?;
        validate_overlay(root, layer)?;
        let container = ui
            .foundation()
            .container_node_under(
                layer.content_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("quick-settings overlay is stale"))?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                container.node,
                SemanticNode::named(SemanticRole::Menu, name),
            )
            .map_err(status_semantic_error)?;

        let authorized = root
            .grant()
            .permits(ShellCapabilities::INVOKE_SYSTEM_ACTION);
        let action_count = self
            .snapshot
            .entries()
            .iter()
            .map(|entry| entry.actions().len())
            .sum::<usize>();
        let semantic_count = u32::try_from(action_count)
            .map_err(|_| RuntimeError::new("status action count exceeds semantic bounds"))?;
        let map = Rc::new(map);
        let mut actions = Vec::with_capacity(action_count);
        for entry in self.snapshot.entries() {
            for action in entry.actions() {
                let index = actions.len();
                let redacted = entry.privacy() == StatusPrivacy::Secret;
                let available = authorized
                    && !redacted
                    && entry.severity() != StatusSeverity::Unavailable
                    && action.enabled();
                let node = ui
                    .foundation()
                    .action_node_under(container.node, self.style.item, available, true, |writer| {
                        if !redacted {
                            writer.text(
                                action.label().as_str(),
                                self.style.label_color,
                                self.style.label_size,
                            );
                        }
                    })
                    .ok_or_else(|| RuntimeError::new("quick-settings container is stale"))?;
                if redacted {
                    ui.foundation()
                        .semantic_node(
                            node.node,
                            SemanticNode {
                                participation: SemanticParticipation::Exclude,
                                ..SemanticNode::default()
                            },
                        )
                        .map_err(status_semantic_error)?;
                } else {
                    let item_name = ui.foundation().intern(action.label().as_str());
                    let semantic_actions = if available {
                        SemanticActions::FOCUS | SemanticActions::ACTIVATE
                    } else {
                        SemanticActions::NONE
                    };
                    ui.foundation()
                        .semantic_node(
                            node.node,
                            SemanticNode {
                                role: SemanticRole::MenuItem,
                                name: SemanticName::Text(item_name),
                                state: SemanticState {
                                    disabled: !available,
                                    focusable: available,
                                    selected: Some(entry.active()),
                                    ..SemanticState::default()
                                },
                                actions: semantic_actions,
                                collection: Some(SemanticCollection {
                                    item_index: Some(u32::try_from(index).map_err(|_| {
                                        RuntimeError::new("status action index exceeds bounds")
                                    })?),
                                    item_count: Some(semantic_count),
                                    ..SemanticCollection::default()
                                }),
                                ..SemanticNode::default()
                            },
                        )
                        .map_err(status_semantic_error)?;
                }
                if available {
                    let map = Rc::clone(&map);
                    let revision = self.snapshot.revision();
                    let entry_id = entry.id();
                    let action_id = action.id();
                    ui.route_activation(node.node, move |activation| {
                        map(StatusActionIntent::new(
                            revision, activation, entry_id, action_id,
                        ))
                    })?;
                }
                actions.push(QuickSettingsActionRef {
                    control: node,
                    revision: self.snapshot.revision(),
                    entry: entry.id(),
                    action: action.clone(),
                    available,
                    redacted,
                });
            }
        }
        Ok(QuickSettingsRef {
            container,
            snapshot: self.snapshot.clone(),
            actions,
        })
    }
}

fn validate_style(style: QuickSettingsStyle) -> Result<(), QuickSettingsError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(QuickSettingsError::InvalidLabelSize);
    }
    Ok(())
}

fn validate_overlay(root: ShellRootRef, layer: ShellLayerRef) -> Result<(), QuickSettingsError> {
    if layer.kind() != ShellLayerKind::Overlay {
        return Err(QuickSettingsError::RequiresOverlayLayer);
    }
    if root.output() != layer.output() {
        return Err(QuickSettingsError::OutputMismatch);
    }
    if root.grant().token() != layer.authority().grant() {
        return Err(QuickSettingsError::GrantMismatch);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct QuickSettingsRef {
    container: ControlHandle,
    snapshot: SystemStatusSnapshot,
    actions: Vec<QuickSettingsActionRef>,
}

impl QuickSettingsRef {
    pub const fn node(&self) -> UiNodeId {
        self.container.node
    }

    pub const fn snapshot(&self) -> &SystemStatusSnapshot {
        &self.snapshot
    }

    pub fn actions(&self) -> &[QuickSettingsActionRef] {
        &self.actions
    }

    pub fn action(&self, action: StatusActionId) -> Option<&QuickSettingsActionRef> {
        self.actions
            .iter()
            .find(|candidate| candidate.action.id() == action)
    }
}

#[derive(Clone, Debug)]
pub struct QuickSettingsActionRef {
    control: ControlHandle,
    revision: SystemStatusRevision,
    entry: StatusEntryId,
    action: StatusAction,
    available: bool,
    redacted: bool,
}

impl QuickSettingsActionRef {
    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn revision(&self) -> SystemStatusRevision {
        self.revision
    }

    pub const fn entry(&self) -> StatusEntryId {
        self.entry
    }

    pub const fn action(&self) -> &StatusAction {
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
pub enum QuickSettingsError {
    MissingAccessibleName,
    InvalidLabelSize,
    RequiresOverlayLayer,
    OutputMismatch,
    GrantMismatch,
}

impl fmt::Display for QuickSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid quick settings: {self:?}")
    }
}

impl std::error::Error for QuickSettingsError {}

#[derive(Debug)]
pub enum QuickSettingsMountError {
    QuickSettings(QuickSettingsError),
    Runtime(RuntimeError),
}

impl fmt::Display for QuickSettingsMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QuickSettings(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for QuickSettingsMountError {}

impl From<QuickSettingsError> for QuickSettingsMountError {
    fn from(value: QuickSettingsError) -> Self {
        Self::QuickSettings(value)
    }
}

impl From<RuntimeError> for QuickSettingsMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
