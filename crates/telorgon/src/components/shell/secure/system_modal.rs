//! Controlled generic content host on an explicitly authorized system-modal layer.

use std::fmt;

use crate::runtime::{RuntimeError, Ui};
use crate::shell::{OutputId, ShellGrantToken, ShellLayerKind};
use crate::components::shell::primitives::{ShellLayerRef, ShellRootRef};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, SemanticName, SemanticNode, SemanticParticipation,
    SemanticRelationship, SemanticRelationshipKind, SemanticRole, SemanticState, SizeRule,
    UiNodeId,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemModalHostStyle {
    pub container: BoxStyle,
    pub content: BoxStyle,
    pub layout: LayoutStyle,
    pub content_layout: LayoutStyle,
}

impl Default for SystemModalHostStyle {
    fn default() -> Self {
        let fill = BoxStyle {
            width: SizeRule::Fill(1.0),
            height: SizeRule::Fill(1.0),
            ..BoxStyle::default()
        };
        Self {
            container: fill,
            content: BoxStyle::default(),
            layout: LayoutStyle::default(),
            content_layout: LayoutStyle::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemModalHost {
    label: String,
    active: bool,
    style: SystemModalHostStyle,
}

impl SystemModalHost {
    pub fn new(label: impl Into<String>, active: bool) -> Result<Self, SystemModalHostError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(SystemModalHostError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            active,
            style: SystemModalHostStyle::default(),
        })
    }

    pub const fn style(mut self, style: SystemModalHostStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn active(&self) -> bool {
        self.active
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        root: ShellRootRef,
        layer: ShellLayerRef,
    ) -> Result<SystemModalHostRef, SystemModalHostMountError> {
        validate_system_modal(root, layer)?;
        let container = ui
            .foundation()
            .layer_node_under(
                layer.content_node(),
                self.active,
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("system-modal layer is stale"))?;
        let content = ui
            .foundation()
            .container_node_under(
                container.node,
                self.style.content,
                self.style.content_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("system-modal host is stale"))?;
        ui.foundation()
            .semantic_node(
                content.node,
                SemanticNode {
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(modal_semantic_error)?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                container.node,
                SemanticNode {
                    role: SemanticRole::Dialog,
                    name: SemanticName::Text(name),
                    state: SemanticState {
                        hidden: !self.active,
                        inert: !self.active,
                        ..SemanticState::default()
                    },
                    relationships: vec![SemanticRelationship {
                        kind: SemanticRelationshipKind::Owns,
                        target: content.node,
                    }],
                    ..SemanticNode::default()
                },
            )
            .map_err(modal_semantic_error)?;
        Ok(SystemModalHostRef {
            container,
            content,
            output: layer.output(),
            grant: layer.authority().grant(),
            active: self.active,
        })
    }
}

fn validate_system_modal(
    root: ShellRootRef,
    layer: ShellLayerRef,
) -> Result<(), SystemModalHostError> {
    if layer.kind() != ShellLayerKind::SystemModal {
        return Err(SystemModalHostError::RequiresSystemModalLayer);
    }
    if root.output() != layer.output() {
        return Err(SystemModalHostError::OutputMismatch);
    }
    if root.grant().token() != layer.authority().grant() {
        return Err(SystemModalHostError::GrantMismatch);
    }
    Ok(())
}

fn modal_semantic_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid system-modal semantics: {error:?}"))
}

#[derive(Clone, Copy, Debug)]
pub struct SystemModalHostRef {
    container: ControlHandle,
    content: ControlHandle,
    output: OutputId,
    grant: ShellGrantToken,
    active: bool,
}

impl SystemModalHostRef {
    pub const fn node(self) -> UiNodeId {
        self.container.node
    }

    pub const fn content_node(self) -> UiNodeId {
        self.content.node
    }

    pub const fn output(self) -> OutputId {
        self.output
    }

    pub const fn grant(self) -> ShellGrantToken {
        self.grant
    }

    pub const fn active(self) -> bool {
        self.active
    }

    /// Host policy must apply this intent to lower input, focus, and semantic routes.
    pub const fn requires_lower_layers_inert(self) -> bool {
        self.active
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemModalHostError {
    MissingAccessibleName,
    RequiresSystemModalLayer,
    OutputMismatch,
    GrantMismatch,
}

impl fmt::Display for SystemModalHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid system-modal host: {self:?}")
    }
}

impl std::error::Error for SystemModalHostError {}

#[derive(Debug)]
pub enum SystemModalHostMountError {
    Modal(SystemModalHostError),
    Runtime(RuntimeError),
}

impl fmt::Display for SystemModalHostMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Modal(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SystemModalHostMountError {}

impl From<SystemModalHostError> for SystemModalHostMountError {
    fn from(value: SystemModalHostError) -> Self {
        Self::Modal(value)
    }
}

impl From<RuntimeError> for SystemModalHostMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
