//! Stable shell-owned frame structure for one exact client-surface revision.

use std::fmt;
use std::sync::Arc;

use crate::foundation::PointF;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    ClientSurfaceSnapshot, OutputId, OutputRevision, ShellGrantToken, ShellLayerKind, SurfaceId,
    SurfaceRevision,
};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, Property, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRelationship, SemanticRelationshipKind, SemanticRole, SizeRule,
    UiNodeId,
};

use crate::components::shell::primitives::{OutputViewRef, ShellLayerRef};

/// Caller-owned visuals for one window frame and its stable composition hosts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowFrameStyle {
    pub container: BoxStyle,
    pub decoration: BoxStyle,
    pub client_content: BoxStyle,
    pub chrome: BoxStyle,
    pub layout: LayoutStyle,
    pub decoration_layout: LayoutStyle,
    pub client_content_layout: LayoutStyle,
    pub chrome_layout: LayoutStyle,
}

impl Default for WindowFrameStyle {
    fn default() -> Self {
        let fill = BoxStyle {
            width: SizeRule::Fill(1.0),
            height: SizeRule::Fill(1.0),
            ..BoxStyle::default()
        };
        Self {
            container: BoxStyle::default(),
            decoration: fill,
            client_content: fill,
            chrome: fill,
            layout: LayoutStyle {
                flow: Flow::Overlay,
                ..LayoutStyle::default()
            },
            decoration_layout: LayoutStyle::default(),
            client_content_layout: LayoutStyle::default(),
            chrome_layout: LayoutStyle::default(),
        }
    }
}

/// One named, immutable window-level presentation owner.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowFrame {
    label: String,
    snapshot: Arc<ClientSurfaceSnapshot>,
    style: WindowFrameStyle,
}

impl WindowFrame {
    pub fn new(
        label: impl Into<String>,
        snapshot: ClientSurfaceSnapshot,
    ) -> Result<Self, WindowFrameError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(WindowFrameError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            snapshot: Arc::new(snapshot),
            style: WindowFrameStyle::default(),
        })
    }

    pub const fn style(mut self, style: WindowFrameStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn snapshot(&self) -> &ClientSurfaceSnapshot {
        &self.snapshot
    }

    pub fn surface(&self) -> SurfaceId {
        self.snapshot.id()
    }

    pub fn revision(&self) -> SurfaceRevision {
        self.snapshot.revision()
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        layer: ShellLayerRef,
        output: OutputViewRef,
    ) -> Result<WindowFrameRef, WindowFrameMountError> {
        if layer.kind() != ShellLayerKind::Workspace {
            return Err(WindowFrameError::RequiresWorkspaceLayer.into());
        }
        if layer.output() != output.output() {
            return Err(WindowFrameError::OutputMismatch.into());
        }
        let output_bounds = output.snapshot().geometry().logical_bounds();
        self.mount_under(
            ui,
            layer.content_node(),
            output.output(),
            output.revision(),
            layer.authority().grant(),
            PointF {
                x: output_bounds.x,
                y: output_bounds.y,
            },
        )
        .map_err(Into::into)
    }

    pub(crate) fn mount_under<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        parent: UiNodeId,
        output: OutputId,
        output_revision: OutputRevision,
        grant: ShellGrantToken,
        parent_global_origin: PointF,
    ) -> Result<WindowFrameRef, RuntimeError> {
        let bounds = self.snapshot.geometry().logical_bounds();
        let mut container_style = self.style.container;
        container_style.width = SizeRule::Logical(bounds.width);
        container_style.height = SizeRule::Logical(bounds.height);
        container_style.transform.translation.x = bounds.x - parent_global_origin.x;
        container_style.transform.translation.y = bounds.y - parent_global_origin.y;

        let root = ui
            .foundation()
            .container_node_under(parent, container_style, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("window-frame layer is stale"))?;
        let decoration = ui
            .foundation()
            .container_node_under(
                root.node,
                self.style.decoration,
                self.style.decoration_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("window-frame decoration parent is stale"))?;
        let client_content = ui
            .foundation()
            .container_node_under(
                root.node,
                self.style.client_content,
                self.style.client_content_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("window-frame client-content parent is stale"))?;
        let chrome = ui
            .foundation()
            .container_node_under(
                root.node,
                self.style.chrome,
                self.style.chrome_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("window-frame chrome parent is stale"))?;

        ui.foundation()
            .semantic_node(
                decoration.node,
                SemanticNode {
                    participation: SemanticParticipation::Exclude,
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;
        for host in [client_content.node, chrome.node] {
            ui.foundation()
                .semantic_node(
                    host,
                    SemanticNode {
                        participation: SemanticParticipation::MergeDescendants,
                        ..SemanticNode::default()
                    },
                )
                .map_err(semantic_runtime_error)?;
        }
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Window,
                    name: SemanticName::Text(name),
                    relationships: vec![
                        SemanticRelationship {
                            kind: SemanticRelationshipKind::Owns,
                            target: chrome.node,
                        },
                        SemanticRelationship {
                            kind: SemanticRelationshipKind::Owns,
                            target: client_content.node,
                        },
                    ],
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;

        Ok(WindowFrameRef {
            root,
            decoration,
            client_content,
            chrome,
            output,
            output_revision,
            grant,
            snapshot: Arc::clone(&self.snapshot),
        })
    }
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid window-frame semantics: {error:?}"))
}

/// Stable mounted identities and exact source snapshot for a window frame.
#[derive(Clone, Debug)]
pub struct WindowFrameRef {
    root: ControlHandle,
    decoration: ControlHandle,
    client_content: ControlHandle,
    chrome: ControlHandle,
    output: OutputId,
    output_revision: OutputRevision,
    grant: ShellGrantToken,
    snapshot: Arc<ClientSurfaceSnapshot>,
}

impl WindowFrameRef {
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub const fn decoration_node(&self) -> UiNodeId {
        self.decoration.node
    }

    pub const fn client_content_node(&self) -> UiNodeId {
        self.client_content.node
    }

    pub const fn chrome_node(&self) -> UiNodeId {
        self.chrome.node
    }

    pub const fn output(&self) -> OutputId {
        self.output
    }

    pub const fn output_revision(&self) -> OutputRevision {
        self.output_revision
    }

    pub const fn grant(&self) -> ShellGrantToken {
        self.grant
    }

    pub fn snapshot(&self) -> &ClientSurfaceSnapshot {
        &self.snapshot
    }

    pub fn surface(&self) -> SurfaceId {
        self.snapshot.id()
    }

    pub fn revision(&self) -> SurfaceRevision {
        self.snapshot.revision()
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowFrameError {
    MissingAccessibleName,
    RequiresWorkspaceLayer,
    OutputMismatch,
}

impl fmt::Display for WindowFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingAccessibleName => "window frame accessible name is empty",
            Self::RequiresWorkspaceLayer => "window frames require an authorized workspace layer",
            Self::OutputMismatch => "window frame layer and output view do not match",
        })
    }
}

impl std::error::Error for WindowFrameError {}

#[derive(Debug)]
pub enum WindowFrameMountError {
    Frame(WindowFrameError),
    Runtime(RuntimeError),
}

impl fmt::Display for WindowFrameMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WindowFrameMountError {}

impl From<WindowFrameError> for WindowFrameMountError {
    fn from(value: WindowFrameError) -> Self {
        Self::Frame(value)
    }
}

impl From<RuntimeError> for WindowFrameMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
