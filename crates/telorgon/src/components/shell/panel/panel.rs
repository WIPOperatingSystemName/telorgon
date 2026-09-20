//! Named output-edge panel with an unapplied reserved-area proposal.

use std::fmt;

use crate::foundation::RectF;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    OutputEdge, OutputId, OutputRequest, OutputRevision, ReservedAreaExtent, ReservedAreaId,
    ShellLayerKind,
};
use crate::components::shell::primitives::{
    OutputViewRef, ReservedArea, ReservedAreaError, ReservedAreaRef, ShellLayerRef, ShellRootRef,
};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, Property, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRelationship, SemanticRelationshipKind, SemanticRole, SizeRule,
    UiNodeId,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelStyle {
    pub container: BoxStyle,
    pub content: BoxStyle,
    pub layout: LayoutStyle,
    pub content_layout: LayoutStyle,
}

impl Default for PanelStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            content: BoxStyle {
                width: SizeRule::Fill(1.0),
                height: SizeRule::Fill(1.0),
                ..BoxStyle::default()
            },
            layout: LayoutStyle {
                flow: Flow::Overlay,
                ..LayoutStyle::default()
            },
            content_layout: LayoutStyle::default(),
        }
    }
}

/// One panel edge and reservation proposal. Accepted usable geometry still comes from the host.
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    label: String,
    reservation: ReservedAreaId,
    edge: OutputEdge,
    extent: ReservedAreaExtent,
    style: PanelStyle,
}

impl Panel {
    pub fn new(
        label: impl Into<String>,
        reservation: ReservedAreaId,
        edge: OutputEdge,
        extent: ReservedAreaExtent,
    ) -> Result<Self, PanelError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(PanelError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            reservation,
            edge,
            extent,
            style: PanelStyle::default(),
        })
    }

    pub const fn style(mut self, style: PanelStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn reservation(&self) -> ReservedAreaId {
        self.reservation
    }

    pub const fn edge(&self) -> OutputEdge {
        self.edge
    }

    pub const fn extent(&self) -> ReservedAreaExtent {
        self.extent
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        root: ShellRootRef,
        output: OutputViewRef,
        layer: ShellLayerRef,
    ) -> Result<PanelRef, PanelMountError> {
        if layer.kind() != ShellLayerKind::Panel {
            return Err(PanelError::RequiresPanelLayer.into());
        }
        if root.output() != output.output() || layer.output() != output.output() {
            return Err(PanelError::OutputMismatch.into());
        }
        if root.grant().token() != layer.authority().grant() {
            return Err(PanelError::GrantMismatch.into());
        }
        let reservation = ReservedArea::new(self.reservation, self.edge, self.extent)
            .bind(root, output)
            .map_err(PanelError::Reservation)?;
        let logical = output.snapshot().geometry().logical_bounds();
        let local = RectF {
            x: 0.0,
            y: 0.0,
            width: logical.width,
            height: logical.height,
        };
        let extent = self.extent.get();
        let bounds = match self.edge {
            OutputEdge::Top if extent <= local.height => RectF {
                width: local.width,
                height: extent,
                ..local
            },
            OutputEdge::Bottom if extent <= local.height => RectF {
                x: 0.0,
                y: local.height - extent,
                width: local.width,
                height: extent,
            },
            OutputEdge::Left if extent <= local.width => RectF {
                width: extent,
                height: local.height,
                ..local
            },
            OutputEdge::Right if extent <= local.width => RectF {
                x: local.width - extent,
                y: 0.0,
                width: extent,
                height: local.height,
            },
            _ => return Err(PanelError::ExtentExceedsOutput.into()),
        };

        let mut style = self.style.container;
        style.width = SizeRule::Logical(bounds.width);
        style.height = SizeRule::Logical(bounds.height);
        style.transform.translation.x = bounds.x;
        style.transform.translation.y = bounds.y;
        let panel = ui
            .foundation()
            .container_node_under(layer.content_node(), style, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("panel layer is stale"))?;
        let content = ui
            .foundation()
            .container_node_under(
                panel.node,
                self.style.content,
                self.style.content_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("panel content parent is stale"))?;
        ui.foundation()
            .semantic_node(
                content.node,
                SemanticNode {
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                panel.node,
                SemanticNode {
                    role: SemanticRole::Region,
                    name: SemanticName::Text(name),
                    relationships: vec![SemanticRelationship {
                        kind: SemanticRelationshipKind::Owns,
                        target: content.node,
                    }],
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;
        Ok(PanelRef {
            panel,
            content,
            output: output.output(),
            output_revision: output.revision(),
            bounds,
            edge: self.edge,
            extent: self.extent,
            reservation,
        })
    }
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid panel semantics: {error:?}"))
}

#[derive(Clone, Copy, Debug)]
pub struct PanelRef {
    panel: ControlHandle,
    content: ControlHandle,
    output: OutputId,
    output_revision: OutputRevision,
    bounds: RectF,
    edge: OutputEdge,
    extent: ReservedAreaExtent,
    reservation: ReservedAreaRef,
}

impl PanelRef {
    pub const fn node(self) -> UiNodeId {
        self.panel.node
    }

    pub const fn content_node(self) -> UiNodeId {
        self.content.node
    }

    pub const fn output(self) -> OutputId {
        self.output
    }

    pub const fn output_revision(self) -> OutputRevision {
        self.output_revision
    }

    pub const fn bounds(self) -> RectF {
        self.bounds
    }

    pub const fn edge(self) -> OutputEdge {
        self.edge
    }

    pub const fn extent(self) -> ReservedAreaExtent {
        self.extent
    }

    pub const fn reservation(self) -> ReservedAreaRef {
        self.reservation
    }

    pub const fn propose(self) -> OutputRequest {
        self.reservation.propose()
    }

    pub const fn release(self) -> OutputRequest {
        self.reservation.release()
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.panel.style
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelError {
    MissingAccessibleName,
    RequiresPanelLayer,
    OutputMismatch,
    GrantMismatch,
    ExtentExceedsOutput,
    Reservation(ReservedAreaError),
}

impl fmt::Display for PanelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid panel: {self:?}")
    }
}

impl std::error::Error for PanelError {}

#[derive(Debug)]
pub enum PanelMountError {
    Panel(PanelError),
    Runtime(RuntimeError),
}

impl fmt::Display for PanelMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panel(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PanelMountError {}

impl From<PanelError> for PanelMountError {
    fn from(value: PanelError) -> Self {
        Self::Panel(value)
    }
}

impl From<RuntimeError> for PanelMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
