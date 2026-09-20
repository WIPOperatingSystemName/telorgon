//! Explicit client-content fallback presentation that never retains stale external pixels.

use std::fmt;

use crate::runtime::{RuntimeError, Ui};
use crate::shell::{OutputId, ShellLayerKind, SurfaceId, SurfaceRevision};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticName, SemanticNode, SemanticRole,
    UiNodeId,
};

use crate::components::shell::primitives::ShellLayerRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfacePlaceholderReason {
    Unavailable,
    Protected,
    Lost,
}

impl SurfacePlaceholderReason {
    const fn accessible_name(self) -> &'static str {
        match self {
            Self::Unavailable => "Window content unavailable",
            Self::Protected => "Protected window content",
            Self::Lost => "Window content lost",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfacePlaceholderStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfacePlaceholder {
    surface: SurfaceId,
    revision: SurfaceRevision,
    reason: SurfacePlaceholderReason,
    style: SurfacePlaceholderStyle,
}

impl SurfacePlaceholder {
    pub fn new(
        surface: SurfaceId,
        revision: SurfaceRevision,
        reason: SurfacePlaceholderReason,
    ) -> Self {
        Self {
            surface,
            revision,
            reason,
            style: SurfacePlaceholderStyle::default(),
        }
    }

    pub const fn style(mut self, style: SurfacePlaceholderStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    pub const fn revision(self) -> SurfaceRevision {
        self.revision
    }

    pub const fn reason(self) -> SurfacePlaceholderReason {
        self.reason
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        layer: ShellLayerRef,
    ) -> Result<SurfacePlaceholderRef, SurfacePlaceholderMountError> {
        if layer.kind() != ShellLayerKind::Workspace {
            return Err(SurfacePlaceholderError::RequiresWorkspaceLayer.into());
        }
        let control = ui
            .foundation()
            .container_node_under(
                layer.content_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("surface-placeholder layer is stale"))?;
        let name = ui.foundation().intern(self.reason.accessible_name());
        ui.foundation()
            .semantic_node(
                control.node,
                SemanticNode {
                    role: SemanticRole::Image,
                    name: SemanticName::Text(name),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid surface-placeholder semantics: {error:?}"))
            })?;
        Ok(SurfacePlaceholderRef {
            control,
            output: layer.output(),
            surface: self.surface,
            revision: self.revision,
            reason: self.reason,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SurfacePlaceholderRef {
    control: ControlHandle,
    output: OutputId,
    surface: SurfaceId,
    revision: SurfaceRevision,
    reason: SurfacePlaceholderReason,
}

impl SurfacePlaceholderRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn output(self) -> OutputId {
        self.output
    }

    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    pub const fn revision(self) -> SurfaceRevision {
        self.revision
    }

    pub const fn reason(self) -> SurfacePlaceholderReason {
        self.reason
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfacePlaceholderError {
    RequiresWorkspaceLayer,
}

impl fmt::Display for SurfacePlaceholderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("surface placeholders require an authorized workspace layer")
    }
}

impl std::error::Error for SurfacePlaceholderError {}

#[derive(Debug)]
pub enum SurfacePlaceholderMountError {
    Placeholder(SurfacePlaceholderError),
    Runtime(RuntimeError),
}

impl fmt::Display for SurfacePlaceholderMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Placeholder(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SurfacePlaceholderMountError {}

impl From<SurfacePlaceholderError> for SurfacePlaceholderMountError {
    fn from(value: SurfacePlaceholderError) -> Self {
        Self::Placeholder(value)
    }
}

impl From<RuntimeError> for SurfacePlaceholderMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
