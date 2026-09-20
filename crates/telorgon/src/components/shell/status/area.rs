//! Named panel status collection over one exact host status snapshot.

use std::fmt;
use std::rc::Rc;

use crate::foundation::ColorRgba8;
use crate::input::{Activation, ChangeSource};
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    InputSource, ShellCapabilities, StatusAction, StatusActionId, StatusEntry, StatusEntryId,
    StatusPrivacy, StatusSeverity, SystemRequest, SystemStatusRevision, SystemStatusSnapshot,
};
use crate::components::shell::primitives::ShellRootRef;
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, SemanticActions, SemanticCollection, SemanticName,
    SemanticNode, SemanticParticipation, SemanticRole, SemanticState, UiNodeId,
};

use crate::components::shell::panel::{PanelRef, TaskbarError, validate_panel};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusAreaStyle {
    pub container: BoxStyle,
    pub item: BoxStyle,
    pub layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for StatusAreaStyle {
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
pub struct StatusArea {
    label: String,
    snapshot: SystemStatusSnapshot,
    style: StatusAreaStyle,
}

impl StatusArea {
    pub fn new(
        label: impl Into<String>,
        snapshot: SystemStatusSnapshot,
    ) -> Result<Self, StatusAreaError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(StatusAreaError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            snapshot,
            style: StatusAreaStyle::default(),
        })
    }

    pub fn style(mut self, style: StatusAreaStyle) -> Result<Self, StatusAreaError> {
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
        panel: PanelRef,
        map: Map,
    ) -> Result<StatusAreaRef, StatusAreaMountError>
    where
        Action: 'static,
        Map: Fn(StatusActionIntent) -> Action + 'static,
    {
        validate_style(self.style)?;
        validate_panel(root, panel).map_err(StatusAreaError::PanelBoundary)?;
        let container = ui
            .foundation()
            .container_node_under(
                panel.content_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("status-area panel is stale"))?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                container.node,
                SemanticNode::named(SemanticRole::Status, name),
            )
            .map_err(status_semantic_error)?;

        let authorized = root
            .grant()
            .permits(ShellCapabilities::INVOKE_SYSTEM_ACTION);
        let map = Rc::new(map);
        let item_count = u32::try_from(self.snapshot.entries().len())
            .map_err(|_| RuntimeError::new("status entry count exceeds semantic bounds"))?;
        let mut entries = Vec::with_capacity(self.snapshot.entries().len());
        for (index, entry) in self.snapshot.entries().iter().enumerate() {
            let primary = primary_action(entry);
            let redacted = entry.privacy() == StatusPrivacy::Secret;
            let available = authorized
                && !redacted
                && entry.severity() != StatusSeverity::Unavailable
                && primary.is_some_and(StatusAction::enabled);
            let node = ui
                .foundation()
                .action_node_under(container.node, self.style.item, available, true, |writer| {
                    if !redacted {
                        writer.text(
                            entry.label().as_str(),
                            self.style.label_color,
                            self.style.label_size,
                        );
                        if entry.privacy() == StatusPrivacy::Public
                            && let Some(value) = entry.value()
                        {
                            writer.text(
                                value.as_str(),
                                self.style.label_color,
                                self.style.label_size,
                            );
                        }
                    }
                })
                .ok_or_else(|| RuntimeError::new("status-area container is stale"))?;
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
                let item_name = ui.foundation().intern(entry.label().as_str());
                let description = if entry.privacy() == StatusPrivacy::Public {
                    entry
                        .value()
                        .map(|value| ui.foundation().intern(value.as_str()))
                } else {
                    None
                };
                let actions = if available {
                    SemanticActions::FOCUS | SemanticActions::ACTIVATE
                } else {
                    SemanticActions::NONE
                };
                ui.foundation()
                    .semantic_node(
                        node.node,
                        SemanticNode {
                            role: SemanticRole::Button,
                            name: SemanticName::Text(item_name),
                            description,
                            state: SemanticState {
                                disabled: !available,
                                focusable: available,
                                invalid: entry.severity() == StatusSeverity::Critical,
                                selected: Some(entry.active()),
                                ..SemanticState::default()
                            },
                            actions,
                            collection: Some(SemanticCollection {
                                item_index: Some(u32::try_from(index).map_err(|_| {
                                    RuntimeError::new("status index exceeds semantic bounds")
                                })?),
                                item_count: Some(item_count),
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
                let action = primary
                    .expect("available status entry has a primary action")
                    .id();
                ui.route_activation(node.node, move |activation| {
                    map(StatusActionIntent::new(
                        revision, activation, entry_id, action,
                    ))
                })?;
            }
            entries.push(StatusAreaEntryRef {
                control: node,
                entry: entry.clone(),
                revision: self.snapshot.revision(),
                available,
                redacted,
            });
        }
        Ok(StatusAreaRef {
            container,
            panel,
            snapshot: self.snapshot.clone(),
            entries,
        })
    }
}

fn validate_style(style: StatusAreaStyle) -> Result<(), StatusAreaError> {
    if !style.label_size.is_finite() || style.label_size <= 0.0 {
        return Err(StatusAreaError::InvalidLabelSize);
    }
    Ok(())
}

pub(crate) fn primary_action(entry: &StatusEntry) -> Option<&StatusAction> {
    let primary = entry.primary_action()?;
    entry.actions().iter().find(|action| action.id() == primary)
}

pub(crate) fn status_semantic_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid status semantics: {error:?}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusActionIntent {
    revision: SystemStatusRevision,
    entry: StatusEntryId,
    action: StatusActionId,
    activation: Activation,
}

impl StatusActionIntent {
    pub(crate) const fn new(
        revision: SystemStatusRevision,
        activation: Activation,
        entry: StatusEntryId,
        action: StatusActionId,
    ) -> Self {
        Self {
            revision,
            entry,
            action,
            activation,
        }
    }

    pub const fn revision(self) -> SystemStatusRevision {
        self.revision
    }

    pub const fn entry(self) -> StatusEntryId {
        self.entry
    }

    pub const fn action(self) -> StatusActionId {
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

    pub fn request(self, source: InputSource) -> Result<SystemRequest, StatusActionSourceError> {
        let matches = match self.activation.source {
            ChangeSource::Pointer => source.is_contact(),
            ChangeSource::Keyboard | ChangeSource::Directional => source == InputSource::Keyboard,
            ChangeSource::Accessibility => source == InputSource::Accessibility,
            ChangeSource::Programmatic => source == InputSource::Programmatic,
        };
        if !matches {
            return Err(StatusActionSourceError::SourceMismatch);
        }
        Ok(self.build_request(source))
    }

    const fn build_request(self, source: InputSource) -> SystemRequest {
        SystemRequest::StatusAction {
            revision: self.revision,
            entry: self.entry,
            action: self.action,
            source,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusActionSourceError {
    SourceMismatch,
}

impl fmt::Display for StatusActionSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("status activation and shell input source do not match")
    }
}

impl std::error::Error for StatusActionSourceError {}

#[derive(Clone, Debug)]
pub struct StatusAreaRef {
    container: ControlHandle,
    panel: PanelRef,
    snapshot: SystemStatusSnapshot,
    entries: Vec<StatusAreaEntryRef>,
}

impl StatusAreaRef {
    pub const fn node(&self) -> UiNodeId {
        self.container.node
    }

    pub const fn panel(&self) -> PanelRef {
        self.panel
    }

    pub const fn snapshot(&self) -> &SystemStatusSnapshot {
        &self.snapshot
    }

    pub fn entries(&self) -> &[StatusAreaEntryRef] {
        &self.entries
    }

    pub fn entry(&self, entry: StatusEntryId) -> Option<&StatusAreaEntryRef> {
        self.entries
            .iter()
            .find(|candidate| candidate.entry.id() == entry)
    }
}

#[derive(Clone, Debug)]
pub struct StatusAreaEntryRef {
    control: ControlHandle,
    entry: StatusEntry,
    revision: SystemStatusRevision,
    available: bool,
    redacted: bool,
}

impl StatusAreaEntryRef {
    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn entry(&self) -> &StatusEntry {
        &self.entry
    }

    pub const fn revision(&self) -> SystemStatusRevision {
        self.revision
    }

    pub const fn available(&self) -> bool {
        self.available
    }

    pub const fn redacted(&self) -> bool {
        self.redacted
    }

    pub fn presented_value(&self) -> Option<&crate::shell::StatusText> {
        (self.entry.privacy() == StatusPrivacy::Public)
            .then(|| self.entry.value())
            .flatten()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusAreaError {
    PanelBoundary(TaskbarError),
    MissingAccessibleName,
    InvalidLabelSize,
}

impl fmt::Display for StatusAreaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid status area: {self:?}")
    }
}

impl std::error::Error for StatusAreaError {}

#[derive(Debug)]
pub enum StatusAreaMountError {
    Status(StatusAreaError),
    Runtime(RuntimeError),
}

impl fmt::Display for StatusAreaMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for StatusAreaMountError {}

impl From<StatusAreaError> for StatusAreaMountError {
    fn from(value: StatusAreaError) -> Self {
        Self::Status(value)
    }
}

impl From<RuntimeError> for StatusAreaMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
