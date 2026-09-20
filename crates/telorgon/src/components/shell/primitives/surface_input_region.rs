//! Exact output-to-surface-local eligibility mapping for one immutable surface revision.

use std::fmt;

use crate::foundation::{PointF, RectF};
use crate::shell::{OutputId, SurfaceGeometry, SurfaceId, SurfaceRegion, SurfaceRevision};

use crate::components::shell::primitives::ClientSurfaceRef;

#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceInputRegion {
    output: OutputId,
    surface: SurfaceId,
    revision: SurfaceRevision,
    geometry: SurfaceGeometry,
    region: SurfaceRegion,
}

impl SurfaceInputRegion {
    pub fn from_surface(surface: &ClientSurfaceRef) -> Self {
        Self {
            output: surface.output(),
            surface: surface.surface(),
            revision: surface.revision(),
            geometry: surface.snapshot().geometry(),
            region: surface.snapshot().regions().input().clone(),
        }
    }

    pub const fn output(&self) -> OutputId {
        self.output
    }

    pub const fn surface(&self) -> SurfaceId {
        self.surface
    }

    pub const fn revision(&self) -> SurfaceRevision {
        self.revision
    }

    pub const fn geometry(&self) -> SurfaceGeometry {
        self.geometry
    }

    pub const fn region(&self) -> &SurfaceRegion {
        &self.region
    }

    pub fn map(
        &self,
        output_point: PointF,
    ) -> Result<SurfaceInputMapping, SurfaceInputRegionError> {
        if !finite_point(output_point) {
            return Err(SurfaceInputRegionError::NonFiniteOutputPoint);
        }
        let bounds = self.geometry.logical_bounds();
        let local = PointF {
            x: output_point.x - bounds.x,
            y: output_point.y - bounds.y,
        };
        if !finite_point(local) {
            return Err(SurfaceInputRegionError::NonFiniteLocalPoint);
        }
        let local_bounds = RectF {
            x: 0.0,
            y: 0.0,
            width: bounds.width,
            height: bounds.height,
        };
        if !contains(local_bounds, local) {
            return Ok(SurfaceInputMapping::OutsideSurface { local });
        }
        if self.region.iter().any(|rect| contains(rect, local)) {
            Ok(SurfaceInputMapping::Eligible { local })
        } else {
            Ok(SurfaceInputMapping::OutsideInputRegion { local })
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfaceInputMapping {
    OutsideSurface { local: PointF },
    OutsideInputRegion { local: PointF },
    Eligible { local: PointF },
}

impl SurfaceInputMapping {
    pub const fn local(self) -> PointF {
        match self {
            Self::OutsideSurface { local }
            | Self::OutsideInputRegion { local }
            | Self::Eligible { local } => local,
        }
    }

    pub const fn is_eligible(self) -> bool {
        matches!(self, Self::Eligible { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceInputRegionError {
    NonFiniteOutputPoint,
    NonFiniteLocalPoint,
}

impl fmt::Display for SurfaceInputRegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteOutputPoint => "surface input output point must be finite",
            Self::NonFiniteLocalPoint => "surface input local mapping must remain finite",
        })
    }
}

impl std::error::Error for SurfaceInputRegionError {}

pub(crate) const fn finite_point(point: PointF) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

pub(crate) const fn contains(rect: RectF, point: PointF) -> bool {
    point.x >= rect.x
        && point.x < rect.x + rect.width
        && point.y >= rect.y
        && point.y < rect.y + rect.height
}

pub(crate) fn region_within_surface(region: &SurfaceRegion, geometry: SurfaceGeometry) -> bool {
    let bounds = geometry.logical_bounds();
    region.iter().all(|rect| {
        rect.x >= 0.0
            && rect.y >= 0.0
            && rect.right() <= bounds.width
            && rect.bottom() <= bounds.height
    })
}
