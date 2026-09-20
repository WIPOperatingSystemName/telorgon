//! Capability-checked begin-resize intentions derived from explicit edge/corner geometry.

use std::fmt;

use crate::foundation::PointF;
use crate::shell::{
    InputSource, OutputId, ResizeEdge, ShellCapabilities, ShellGrantToken, SurfaceCapabilities,
    SurfaceId, SurfaceInputContact, SurfaceRegion, SurfaceRequest, SurfaceRevision,
};

use crate::components::shell::primitives::surface_input_region::{
    contains, finite_point, region_within_surface,
};
use crate::components::shell::primitives::{ClientSurfaceRef, ShellRootRef};

#[derive(Clone, Debug, PartialEq)]
pub struct ResizeRegion {
    output: OutputId,
    grant: ShellGrantToken,
    surface: SurfaceId,
    revision: SurfaceRevision,
    edge: ResizeEdge,
    logical_origin: PointF,
    region: SurfaceRegion,
    capable: bool,
}

impl ResizeRegion {
    pub fn new(
        surface: &ClientSurfaceRef,
        edge: ResizeEdge,
        region: SurfaceRegion,
    ) -> Result<Self, ResizeRegionError> {
        if region.is_empty() {
            return Err(ResizeRegionError::EmptyRegion);
        }
        if !region_within_surface(&region, surface.snapshot().geometry()) {
            return Err(ResizeRegionError::RegionOutsideSurface);
        }
        let bounds = surface.snapshot().geometry().logical_bounds();
        Ok(Self {
            output: surface.output(),
            grant: surface.grant(),
            surface: surface.surface(),
            revision: surface.revision(),
            edge,
            logical_origin: PointF {
                x: bounds.x,
                y: bounds.y,
            },
            region,
            capable: surface
                .snapshot()
                .capabilities()
                .contains(SurfaceCapabilities::RESIZE),
        })
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

    pub const fn edge(&self) -> ResizeEdge {
        self.edge
    }

    pub const fn region(&self) -> &SurfaceRegion {
        &self.region
    }

    pub fn intent(
        &self,
        root: ShellRootRef,
        output_point: PointF,
        contact: SurfaceInputContact,
    ) -> Result<Option<ResizeRegionIntent>, ResizeRegionError> {
        if root.output() != self.output {
            return Err(ResizeRegionError::OutputMismatch);
        }
        if root.grant().token() != self.grant {
            return Err(ResizeRegionError::GrantMismatch);
        }
        if !root.grant().permits(ShellCapabilities::RESIZE_SURFACE) {
            return Err(ResizeRegionError::NotAuthorized);
        }
        if !self.capable {
            return Err(ResizeRegionError::SurfaceNotCapable);
        }
        if !finite_point(output_point) {
            return Err(ResizeRegionError::NonFinitePoint);
        }
        let local = PointF {
            x: output_point.x - self.logical_origin.x,
            y: output_point.y - self.logical_origin.y,
        };
        if !finite_point(local) {
            return Err(ResizeRegionError::NonFinitePoint);
        }
        if !self.region.iter().any(|rect| contains(rect, local)) {
            return Ok(None);
        }
        Ok(Some(ResizeRegionIntent {
            request: SurfaceRequest::BeginResize {
                surface: self.surface,
                edge: self.edge,
                contact: contact.contact(),
            },
            revision: self.revision,
            contact,
            output_position: output_point,
            local_position: local,
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeRegionIntent {
    request: SurfaceRequest,
    revision: SurfaceRevision,
    contact: SurfaceInputContact,
    output_position: PointF,
    local_position: PointF,
}

impl ResizeRegionIntent {
    pub const fn request(self) -> SurfaceRequest {
        self.request
    }

    pub const fn revision(self) -> SurfaceRevision {
        self.revision
    }

    pub const fn contact(self) -> SurfaceInputContact {
        self.contact
    }

    pub const fn source(self) -> InputSource {
        self.contact.source()
    }

    pub const fn output_position(self) -> PointF {
        self.output_position
    }

    pub const fn local_position(self) -> PointF {
        self.local_position
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeRegionError {
    EmptyRegion,
    RegionOutsideSurface,
    OutputMismatch,
    GrantMismatch,
    NotAuthorized,
    SurfaceNotCapable,
    NonFinitePoint,
}

impl fmt::Display for ResizeRegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRegion => "resize region cannot be empty",
            Self::RegionOutsideSurface => "resize region exceeds surface-local bounds",
            Self::OutputMismatch => "resize region and shell root outputs do not match",
            Self::GrantMismatch => "resize region and shell root grants do not match",
            Self::NotAuthorized => "shell root cannot begin a surface resize",
            Self::SurfaceNotCapable => "surface snapshot does not permit resizing",
            Self::NonFinitePoint => "resize-region point must be finite",
        })
    }
}

impl std::error::Error for ResizeRegionError {}
