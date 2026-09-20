//! Capability-checked begin-move intentions derived from explicit surface-local geometry.

use std::fmt;

use crate::foundation::PointF;
use crate::shell::{
    InputSource, OutputId, ShellCapabilities, ShellGrantToken, SurfaceCapabilities, SurfaceId,
    SurfaceInputContact, SurfaceRegion, SurfaceRequest, SurfaceRevision,
};

use crate::components::shell::primitives::surface_input_region::{
    contains, finite_point, region_within_surface,
};
use crate::components::shell::primitives::{ClientSurfaceRef, ShellRootRef};

#[derive(Clone, Debug, PartialEq)]
pub struct DragRegion {
    output: OutputId,
    grant: ShellGrantToken,
    surface: SurfaceId,
    revision: SurfaceRevision,
    logical_origin: PointF,
    region: SurfaceRegion,
    capable: bool,
}

impl DragRegion {
    pub fn new(surface: &ClientSurfaceRef, region: SurfaceRegion) -> Result<Self, DragRegionError> {
        if region.is_empty() {
            return Err(DragRegionError::EmptyRegion);
        }
        if !region_within_surface(&region, surface.snapshot().geometry()) {
            return Err(DragRegionError::RegionOutsideSurface);
        }
        let bounds = surface.snapshot().geometry().logical_bounds();
        Ok(Self {
            output: surface.output(),
            grant: surface.grant(),
            surface: surface.surface(),
            revision: surface.revision(),
            logical_origin: PointF {
                x: bounds.x,
                y: bounds.y,
            },
            region,
            capable: surface
                .snapshot()
                .capabilities()
                .contains(SurfaceCapabilities::MOVE),
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

    pub const fn region(&self) -> &SurfaceRegion {
        &self.region
    }

    pub fn intent(
        &self,
        root: ShellRootRef,
        output_point: PointF,
        contact: SurfaceInputContact,
    ) -> Result<Option<DragRegionIntent>, DragRegionError> {
        if root.output() != self.output {
            return Err(DragRegionError::OutputMismatch);
        }
        if root.grant().token() != self.grant {
            return Err(DragRegionError::GrantMismatch);
        }
        if !root.grant().permits(ShellCapabilities::MOVE_SURFACE) {
            return Err(DragRegionError::NotAuthorized);
        }
        if !self.capable {
            return Err(DragRegionError::SurfaceNotCapable);
        }
        if !finite_point(output_point) {
            return Err(DragRegionError::NonFinitePoint);
        }
        let local = PointF {
            x: output_point.x - self.logical_origin.x,
            y: output_point.y - self.logical_origin.y,
        };
        if !finite_point(local) {
            return Err(DragRegionError::NonFinitePoint);
        }
        if !self.region.iter().any(|rect| contains(rect, local)) {
            return Ok(None);
        }
        Ok(Some(DragRegionIntent {
            request: SurfaceRequest::BeginMove {
                surface: self.surface,
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
pub struct DragRegionIntent {
    request: SurfaceRequest,
    revision: SurfaceRevision,
    contact: SurfaceInputContact,
    output_position: PointF,
    local_position: PointF,
}

impl DragRegionIntent {
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
pub enum DragRegionError {
    EmptyRegion,
    RegionOutsideSurface,
    OutputMismatch,
    GrantMismatch,
    NotAuthorized,
    SurfaceNotCapable,
    NonFinitePoint,
}

impl fmt::Display for DragRegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRegion => "drag region cannot be empty",
            Self::RegionOutsideSurface => "drag region exceeds surface-local bounds",
            Self::OutputMismatch => "drag region and shell root outputs do not match",
            Self::GrantMismatch => "drag region and shell root grants do not match",
            Self::NotAuthorized => "shell root cannot begin a surface move",
            Self::SurfaceNotCapable => "surface snapshot does not permit moving",
            Self::NonFinitePoint => "drag-region point must be finite",
        })
    }
}

impl std::error::Error for DragRegionError {}
