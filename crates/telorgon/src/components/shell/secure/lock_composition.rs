//! Controlled content host on an explicitly authorized lock layer.

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
pub struct LockCompositionStyle {
    pub container: BoxStyle,
    pub content: BoxStyle,
    pub layout: LayoutStyle,
    pub content_layout: LayoutStyle,
}

impl Default for LockCompositionStyle {
    fn default() -> Self {
        let fill = BoxStyle {
            width: SizeRule::Fill(1.0),
            height: SizeRule::Fill(1.0),
            ..BoxStyle::default()
        };
        Self {
            container: fill,
            content: fill,
            layout: LayoutStyle::default(),
            content_layout: LayoutStyle::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LockComposition {
    label: String,
    active: bool,
    style: LockCompositionStyle,
}

impl LockComposition {
    pub fn new(label: impl Into<String>, active: bool) -> Result<Self, LockCompositionError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(LockCompositionError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            active,
            style: LockCompositionStyle::default(),
        })
    }

    pub const fn style(mut self, style: LockCompositionStyle) -> Self {
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
    ) -> Result<LockCompositionRef, LockCompositionMountError> {
        validate_lock(root, layer)?;
        let container = ui
            .foundation()
            .layer_node_under(
                layer.content_node(),
                self.active,
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("lock layer is stale"))?;
        let content = ui
            .foundation()
            .container_node_under(
                container.node,
                self.style.content,
                self.style.content_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("lock composition is stale"))?;
        ui.foundation()
            .semantic_node(
                content.node,
                SemanticNode {
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(lock_semantic_error)?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                container.node,
                SemanticNode {
                    role: SemanticRole::Application,
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
            .map_err(lock_semantic_error)?;
        Ok(LockCompositionRef {
            container,
            content,
            output: layer.output(),
            grant: layer.authority().grant(),
            active: self.active,
        })
    }
}

fn validate_lock(root: ShellRootRef, layer: ShellLayerRef) -> Result<(), LockCompositionError> {
    if layer.kind() != ShellLayerKind::Lock {
        return Err(LockCompositionError::RequiresLockLayer);
    }
    if root.output() != layer.output() {
        return Err(LockCompositionError::OutputMismatch);
    }
    if root.grant().token() != layer.authority().grant() {
        return Err(LockCompositionError::GrantMismatch);
    }
    Ok(())
}

fn lock_semantic_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid lock-composition semantics: {error:?}"))
}

#[derive(Clone, Copy, Debug)]
pub struct LockCompositionRef {
    container: ControlHandle,
    content: ControlHandle,
    output: OutputId,
    grant: ShellGrantToken,
    active: bool,
}

impl LockCompositionRef {
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
pub enum LockCompositionError {
    MissingAccessibleName,
    RequiresLockLayer,
    OutputMismatch,
    GrantMismatch,
}

impl fmt::Display for LockCompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid lock composition: {self:?}")
    }
}

impl std::error::Error for LockCompositionError {}

#[derive(Debug)]
pub enum LockCompositionMountError {
    Lock(LockCompositionError),
    Runtime(RuntimeError),
}

impl fmt::Display for LockCompositionMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lock(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for LockCompositionMountError {}

impl From<LockCompositionError> for LockCompositionMountError {
    fn from(value: LockCompositionError) -> Self {
        Self::Lock(value)
    }
}

impl From<RuntimeError> for LockCompositionMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
