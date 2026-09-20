//! Host-positioned snap preview presentation without local snapping policy.

use std::fmt;

use crate::foundation::{ColorRgba8, RectF};
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    ClientSurfaceSnapshot, OutputId, OutputRevision, OutputSnapshot, ShellLayerKind, SurfaceId,
    SurfaceRevision,
};
use crate::components::shell::primitives::{OutputViewRef, ShellLayerRef};
use crate::ui::{
    Background, BoxStyle, ControlHandle, LayoutStyle, SemanticNode, SemanticParticipation,
    SizeRule, UiNodeId,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapPreviewStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

impl Default for SnapPreviewStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle {
                decoration: crate::ui::BoxDecoration {
                    background: Background::Color(ColorRgba8::rgba(75, 132, 255, 96)),
                    ..crate::ui::BoxDecoration::default()
                },
                ..BoxStyle::default()
            },
            layout: LayoutStyle::default(),
        }
    }
}

/// One exact host proposal in output-local logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapPreview {
    surface: SurfaceId,
    surface_revision: SurfaceRevision,
    output: OutputId,
    output_revision: OutputRevision,
    bounds: RectF,
    style: SnapPreviewStyle,
}

impl SnapPreview {
    pub fn new(
        surface: SurfaceId,
        surface_revision: SurfaceRevision,
        output: OutputId,
        output_revision: OutputRevision,
        bounds: RectF,
    ) -> Result<Self, SnapPreviewError> {
        if !valid_positive_rect(bounds) {
            return Err(SnapPreviewError::InvalidBounds);
        }
        Ok(Self {
            surface,
            surface_revision,
            output,
            output_revision,
            bounds,
            style: SnapPreviewStyle::default(),
        })
    }

    pub fn from_snapshots(
        surface: &ClientSurfaceSnapshot,
        output: OutputSnapshot,
        bounds: RectF,
    ) -> Result<Self, SnapPreviewError> {
        Self::new(
            surface.id(),
            surface.revision(),
            output.id(),
            output.revision(),
            bounds,
        )
    }

    pub const fn style(mut self, style: SnapPreviewStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    pub const fn surface_revision(self) -> SurfaceRevision {
        self.surface_revision
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

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        layer: ShellLayerRef,
        output: OutputViewRef,
    ) -> Result<SnapPreviewRef, SnapPreviewMountError> {
        if layer.kind() != ShellLayerKind::Overlay {
            return Err(SnapPreviewError::RequiresOverlayLayer.into());
        }
        if layer.output() != self.output || output.output() != self.output {
            return Err(SnapPreviewError::OutputMismatch.into());
        }
        if output.revision() != self.output_revision {
            return Err(SnapPreviewError::OutputRevisionMismatch.into());
        }
        let logical = output.snapshot().geometry().logical_bounds();
        let local = RectF {
            x: 0.0,
            y: 0.0,
            width: logical.width,
            height: logical.height,
        };
        if !contains_rect(local, self.bounds) {
            return Err(SnapPreviewError::BoundsOutsideOutput.into());
        }

        let mut style = self.style.container;
        style.width = SizeRule::Logical(self.bounds.width);
        style.height = SizeRule::Logical(self.bounds.height);
        style.transform.translation.x = self.bounds.x;
        style.transform.translation.y = self.bounds.y;
        let control = ui
            .foundation()
            .container_node_under(layer.content_node(), style, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("snap-preview overlay layer is stale"))?;
        ui.foundation()
            .semantic_node(
                control.node,
                SemanticNode {
                    participation: SemanticParticipation::Exclude,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid snap-preview semantics: {error:?}"))
            })?;
        Ok(SnapPreviewRef {
            control,
            surface: self.surface,
            surface_revision: self.surface_revision,
            output: self.output,
            output_revision: self.output_revision,
            bounds: self.bounds,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SnapPreviewRef {
    control: ControlHandle,
    surface: SurfaceId,
    surface_revision: SurfaceRevision,
    output: OutputId,
    output_revision: OutputRevision,
    bounds: RectF,
}

impl SnapPreviewRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    pub const fn surface_revision(self) -> SurfaceRevision {
        self.surface_revision
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapPreviewError {
    InvalidBounds,
    RequiresOverlayLayer,
    OutputMismatch,
    OutputRevisionMismatch,
    BoundsOutsideOutput,
}

impl fmt::Display for SnapPreviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid snap preview: {self:?}")
    }
}

impl std::error::Error for SnapPreviewError {}

#[derive(Debug)]
pub enum SnapPreviewMountError {
    Preview(SnapPreviewError),
    Runtime(RuntimeError),
}

impl fmt::Display for SnapPreviewMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preview(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SnapPreviewMountError {}

impl From<SnapPreviewError> for SnapPreviewMountError {
    fn from(value: SnapPreviewError) -> Self {
        Self::Preview(value)
    }
}

impl From<RuntimeError> for SnapPreviewMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}

fn valid_positive_rect(rect: RectF) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.width > 0.0
        && rect.height.is_finite()
        && rect.height > 0.0
        && rect.right().is_finite()
        && rect.bottom().is_finite()
}

fn contains_rect(outer: RectF, inner: RectF) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}
