//! Stable output-local edge/corner geometry with pointer-independent activation alternatives.

use std::fmt;

use crate::foundation::{PointF, RectF};
use crate::shell::{OutputEdge, OutputId, OutputRevision};

use crate::components::shell::primitives::OutputViewRef;
use crate::components::shell::primitives::surface_input_region::{contains, finite_point};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputEdgeKind {
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    TopLeft,
}

impl OutputEdgeKind {
    pub const fn reservation_edge(self) -> Option<OutputEdge> {
        match self {
            Self::Top => Some(OutputEdge::Top),
            Self::Right => Some(OutputEdge::Right),
            Self::Bottom => Some(OutputEdge::Bottom),
            Self::Left => Some(OutputEdge::Left),
            Self::TopRight | Self::BottomRight | Self::BottomLeft | Self::TopLeft => None,
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputEdgeThickness(f32);

impl OutputEdgeThickness {
    pub fn new(value: f32) -> Result<Self, OutputEdgeRegionError> {
        if !value.is_finite() || value <= 0.0 {
            return Err(OutputEdgeRegionError::InvalidThickness);
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> f32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputEdgeActivation {
    Pointer,
    Touch,
    Directional,
    Accessibility,
}

impl OutputEdgeActivation {
    pub const fn requires_position(self) -> bool {
        matches!(self, Self::Pointer | Self::Touch)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputEdgeRegion {
    output: OutputId,
    revision: OutputRevision,
    kind: OutputEdgeKind,
    bounds: RectF,
}

impl OutputEdgeRegion {
    pub fn new(
        output: OutputViewRef,
        kind: OutputEdgeKind,
        thickness: OutputEdgeThickness,
    ) -> Result<Self, OutputEdgeRegionError> {
        let bounds = output.snapshot().geometry().logical_bounds();
        let local = RectF {
            x: 0.0,
            y: 0.0,
            width: bounds.width,
            height: bounds.height,
        };
        let thickness = thickness.get();
        if thickness > local.width || thickness > local.height {
            return Err(OutputEdgeRegionError::ThicknessExceedsOutput);
        }
        let max_x = local.width - thickness;
        let max_y = local.height - thickness;
        let edge_bounds = match kind {
            OutputEdgeKind::Top => RectF {
                x: 0.0,
                y: 0.0,
                width: local.width,
                height: thickness,
            },
            OutputEdgeKind::TopRight => RectF {
                x: max_x,
                y: 0.0,
                width: thickness,
                height: thickness,
            },
            OutputEdgeKind::Right => RectF {
                x: max_x,
                y: 0.0,
                width: thickness,
                height: local.height,
            },
            OutputEdgeKind::BottomRight => RectF {
                x: max_x,
                y: max_y,
                width: thickness,
                height: thickness,
            },
            OutputEdgeKind::Bottom => RectF {
                x: 0.0,
                y: max_y,
                width: local.width,
                height: thickness,
            },
            OutputEdgeKind::BottomLeft => RectF {
                x: 0.0,
                y: max_y,
                width: thickness,
                height: thickness,
            },
            OutputEdgeKind::Left => RectF {
                x: 0.0,
                y: 0.0,
                width: thickness,
                height: local.height,
            },
            OutputEdgeKind::TopLeft => RectF {
                x: 0.0,
                y: 0.0,
                width: thickness,
                height: thickness,
            },
        };
        Ok(Self {
            output: output.output(),
            revision: output.revision(),
            kind,
            bounds: edge_bounds,
        })
    }

    pub const fn output(self) -> OutputId {
        self.output
    }

    pub const fn revision(self) -> OutputRevision {
        self.revision
    }

    pub const fn kind(self) -> OutputEdgeKind {
        self.kind
    }

    pub const fn bounds(self) -> RectF {
        self.bounds
    }

    pub fn hit(
        self,
        activation: OutputEdgeActivation,
        local_point: PointF,
    ) -> Result<Option<OutputEdgeIntent>, OutputEdgeRegionError> {
        if !activation.requires_position() {
            return Err(OutputEdgeRegionError::UnexpectedPosition);
        }
        if !finite_point(local_point) {
            return Err(OutputEdgeRegionError::NonFinitePoint);
        }
        Ok(
            contains(self.bounds, local_point).then_some(OutputEdgeIntent {
                output: self.output,
                revision: self.revision,
                kind: self.kind,
                activation,
                local_position: Some(local_point),
            }),
        )
    }

    pub fn alternative(
        self,
        activation: OutputEdgeActivation,
    ) -> Result<OutputEdgeIntent, OutputEdgeRegionError> {
        if activation.requires_position() {
            return Err(OutputEdgeRegionError::MissingPosition);
        }
        Ok(OutputEdgeIntent {
            output: self.output,
            revision: self.revision,
            kind: self.kind,
            activation,
            local_position: None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputEdgeIntent {
    output: OutputId,
    revision: OutputRevision,
    kind: OutputEdgeKind,
    activation: OutputEdgeActivation,
    local_position: Option<PointF>,
}

impl OutputEdgeIntent {
    pub const fn output(self) -> OutputId {
        self.output
    }

    pub const fn revision(self) -> OutputRevision {
        self.revision
    }

    pub const fn kind(self) -> OutputEdgeKind {
        self.kind
    }

    pub const fn activation(self) -> OutputEdgeActivation {
        self.activation
    }

    pub const fn local_position(self) -> Option<PointF> {
        self.local_position
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputEdgeRegionError {
    InvalidThickness,
    ThicknessExceedsOutput,
    NonFinitePoint,
    MissingPosition,
    UnexpectedPosition,
}

impl fmt::Display for OutputEdgeRegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidThickness => "output-edge thickness must be finite and positive",
            Self::ThicknessExceedsOutput => "output-edge thickness exceeds the output",
            Self::NonFinitePoint => "output-edge point must be finite",
            Self::MissingPosition => "pointer and touch output-edge activation requires a point",
            Self::UnexpectedPosition => {
                "directional and accessibility output-edge activation has no pointer point"
            }
        })
    }
}

impl std::error::Error for OutputEdgeRegionError {}
