//! Output-logical regions that explicitly stop routing into lower shell layers.

use std::fmt;
use std::sync::Arc;

use crate::foundation::{PointF, RectF};
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{OutputId, ShellLayerKind};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticNode, SemanticParticipation, UiNodeId,
};

use crate::components::shell::primitives::ShellLayerRef;

#[derive(Clone, Debug, PartialEq)]
pub struct ExclusiveRegionGeometry(Arc<[RectF]>);

impl ExclusiveRegionGeometry {
    pub const MAX_RECTS: usize = 64;

    pub fn new(rects: Vec<RectF>) -> Result<Self, ExclusiveRegionError> {
        if rects.is_empty() {
            return Err(ExclusiveRegionError::Empty);
        }
        if rects.len() > Self::MAX_RECTS {
            return Err(ExclusiveRegionError::TooManyRects {
                count: rects.len(),
                max: Self::MAX_RECTS,
            });
        }
        if let Some(index) = rects.iter().position(|rect| {
            !rect.x.is_finite()
                || !rect.y.is_finite()
                || !rect.width.is_finite()
                || !rect.height.is_finite()
                || rect.width <= 0.0
                || rect.height <= 0.0
                || !rect.right().is_finite()
                || !rect.bottom().is_finite()
        }) {
            return Err(ExclusiveRegionError::InvalidRect { index });
        }
        Ok(Self(rects.into()))
    }

    pub fn as_slice(&self) -> &[RectF] {
        &self.0
    }

    pub fn decision(&self, point: PointF) -> Result<ExclusiveHitDecision, ExclusiveRegionError> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(ExclusiveRegionError::NonFinitePoint);
        }
        let blocked = self.0.iter().any(|rect| {
            point.x >= rect.x
                && point.x < rect.right()
                && point.y >= rect.y
                && point.y < rect.bottom()
        });
        Ok(if blocked {
            ExclusiveHitDecision::BlockLowerLayers
        } else {
            ExclusiveHitDecision::PassThrough
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExclusiveHitDecision {
    PassThrough,
    BlockLowerLayers,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExclusiveRegionStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExclusiveRegion {
    geometry: ExclusiveRegionGeometry,
    style: ExclusiveRegionStyle,
}

impl ExclusiveRegion {
    pub fn new(geometry: ExclusiveRegionGeometry) -> Self {
        Self {
            geometry,
            style: ExclusiveRegionStyle::default(),
        }
    }

    pub const fn style(mut self, style: ExclusiveRegionStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn geometry(&self) -> &ExclusiveRegionGeometry {
        &self.geometry
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        layer: ShellLayerRef,
    ) -> Result<ExclusiveRegionRef, ExclusiveRegionMountError> {
        let control = ui
            .foundation()
            .container_node_under(
                layer.content_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("exclusive-region layer is stale"))?;
        ui.foundation()
            .semantic_node(
                control.node,
                SemanticNode {
                    participation: SemanticParticipation::Exclude,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid exclusive-region semantics: {error:?}"))
            })?;
        Ok(ExclusiveRegionRef {
            control,
            output: layer.output(),
            layer: layer.kind(),
            geometry: self.geometry.clone(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ExclusiveRegionRef {
    control: ControlHandle,
    output: OutputId,
    layer: ShellLayerKind,
    geometry: ExclusiveRegionGeometry,
}

impl ExclusiveRegionRef {
    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn output(&self) -> OutputId {
        self.output
    }

    pub const fn layer(&self) -> ShellLayerKind {
        self.layer
    }

    pub fn geometry(&self) -> &ExclusiveRegionGeometry {
        &self.geometry
    }

    pub fn decision(&self, point: PointF) -> Result<ExclusiveHitDecision, ExclusiveRegionError> {
        self.geometry.decision(point)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExclusiveRegionError {
    Empty,
    TooManyRects { count: usize, max: usize },
    InvalidRect { index: usize },
    NonFinitePoint,
}

impl fmt::Display for ExclusiveRegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "exclusive region must contain at least one rectangle",
            Self::TooManyRects { .. } => "exclusive region exceeds its rectangle capacity",
            Self::InvalidRect { .. } => "exclusive-region rectangles must be finite and positive",
            Self::NonFinitePoint => "exclusive-region query point must be finite",
        })
    }
}

impl std::error::Error for ExclusiveRegionError {}

#[derive(Debug)]
pub struct ExclusiveRegionMountError(RuntimeError);

impl fmt::Display for ExclusiveRegionMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for ExclusiveRegionMountError {}

impl From<RuntimeError> for ExclusiveRegionMountError {
    fn from(value: RuntimeError) -> Self {
        Self(value)
    }
}
