use std::fmt;

use crate::foundation::{RectF, RectI, SizeI};
use crate::shell::{
    ApplicationId, ClientSurfaceSnapshot, ExternalContentId, SurfaceAlphaMode,
    SurfaceBufferTransform, SurfaceCapabilities, SurfaceColorDescription, SurfaceContent,
    SurfaceContentRevision, SurfaceDamage, SurfaceGeometry, SurfaceId, SurfaceProtection,
    SurfaceRegion, SurfaceRegions, SurfaceRevision, SurfaceSampling, SurfaceStates, SurfaceTitle,
};

use crate::integrations::wayland::compositor::{
    BufferDescriptor, BufferTransform, CompositorCore, WaylandSurfaceId,
};

#[derive(Clone, Debug)]
pub struct ShellSurfaceExport {
    pub logical_bounds: RectF,
    pub parent: Option<SurfaceId>,
    pub stacking_order: i32,
    pub application: Option<ApplicationId>,
    pub title: Option<SurfaceTitle>,
    pub capabilities: SurfaceCapabilities,
    pub states: SurfaceStates,
    pub opacity: f32,
}

impl ShellSurfaceExport {
    pub fn to_snapshot(
        &self,
        core: &CompositorCore,
        surface: WaylandSurfaceId,
    ) -> Result<ClientSurfaceSnapshot, SurfaceExportError> {
        let state = core
            .world
            .surface(surface)
            .ok_or(SurfaceExportError::UnknownSurface)?
            .snapshot();
        let attachment = state
            .attachment
            .ok_or(SurfaceExportError::UnmappedSurface)?;
        let descriptor = core
            .buffer(attachment.buffer)
            .ok_or(SurfaceExportError::UnknownBuffer)?;
        let buffer_size = match descriptor {
            BufferDescriptor::Shm(buffer) => buffer.size,
            BufferDescriptor::DmaBuf(buffer) => buffer.size,
        };
        let logical_size = SizeI {
            width: (buffer_size.width / state.buffer_scale).max(1),
            height: (buffer_size.height / state.buffer_scale).max(1),
        };
        if !self.logical_bounds.width.is_finite()
            || !self.logical_bounds.height.is_finite()
            || self.logical_bounds.width <= 0.0
            || self.logical_bounds.height <= 0.0
            || (self.logical_bounds.width - logical_size.width as f32).abs() > 1.0
            || (self.logical_bounds.height - logical_size.height as f32).abs() > 1.0
        {
            return Err(SurfaceExportError::GeometryMismatch);
        }

        let full = RectF {
            x: 0.0,
            y: 0.0,
            width: self.logical_bounds.width,
            height: self.logical_bounds.height,
        };
        let region = |region: Option<&crate::integrations::wayland::compositor::Region>, default_full: bool| {
            let rectangles = match region {
                Some(region) => region
                    .rectangles()
                    .iter()
                    .take(SurfaceRegion::MAX_RECTS)
                    .map(|rect| RectF {
                        x: rect.x as f32,
                        y: rect.y as f32,
                        width: rect.width as f32,
                        height: rect.height as f32,
                    })
                    .collect(),
                None if default_full => vec![full],
                None => Vec::new(),
            };
            SurfaceRegion::new(rectangles).map_err(|_| SurfaceExportError::InvalidRegion)
        };
        let regions = SurfaceRegions::new(
            None,
            region(state.opaque_region.as_ref(), false)?,
            region(state.input_region.as_ref(), true)?,
        );
        let damage = state
            .damage
            .iter()
            .filter_map(|rect| clip_damage(*rect, buffer_size))
            .take(SurfaceDamage::MAX_RECTS)
            .collect();
        let damage = SurfaceDamage::new(damage).map_err(|_| SurfaceExportError::InvalidDamage)?;
        let revision = state.revision.max(1);
        let content = SurfaceContent::new(
            ExternalContentId::from_raw(u64::from(attachment.buffer.get()))
                .expect("Wayland buffer ids are nonzero"),
            SurfaceContentRevision::from_raw(revision).expect("surface revisions are nonzero"),
            None,
            SurfaceColorDescription::default(),
            SurfaceAlphaMode::Premultiplied,
            SurfaceSampling::Linear,
            SurfaceProtection::Unprotected,
        );
        ClientSurfaceSnapshot::new(
            SurfaceId::from_raw(u64::from(surface.get())).expect("Wayland surface ids are nonzero"),
            SurfaceRevision::from_raw(revision).expect("surface revisions are nonzero"),
            self.parent,
            self.stacking_order,
            self.application,
            self.title.clone(),
            SurfaceGeometry::new(
                self.logical_bounds,
                buffer_size,
                state.buffer_scale as f32,
                transform(state.buffer_transform),
                self.opacity,
            )
            .map_err(|_| SurfaceExportError::InvalidGeometry)?,
            regions,
            damage,
            content,
            self.capabilities,
            self.states,
        )
        .map_err(|_| SurfaceExportError::InvalidSnapshot)
    }
}

fn transform(value: BufferTransform) -> SurfaceBufferTransform {
    match value {
        BufferTransform::Normal => SurfaceBufferTransform::Normal,
        BufferTransform::Rotate90 => SurfaceBufferTransform::Rotate90,
        BufferTransform::Rotate180 => SurfaceBufferTransform::Rotate180,
        BufferTransform::Rotate270 => SurfaceBufferTransform::Rotate270,
        BufferTransform::Flipped => SurfaceBufferTransform::Flipped,
        BufferTransform::Flipped90 => SurfaceBufferTransform::Flipped90,
        BufferTransform::Flipped180 => SurfaceBufferTransform::Flipped180,
        BufferTransform::Flipped270 => SurfaceBufferTransform::Flipped270,
    }
}

fn clip_damage(rect: RectI, size: SizeI) -> Option<RectI> {
    let left = rect.x.max(0).min(size.width);
    let top = rect.y.max(0).min(size.height);
    let right = rect.x.saturating_add(rect.width).max(0).min(size.width);
    let bottom = rect.y.saturating_add(rect.height).max(0).min(size.height);
    (right > left && bottom > top).then_some(RectI {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceExportError {
    UnknownSurface,
    UnmappedSurface,
    UnknownBuffer,
    GeometryMismatch,
    InvalidRegion,
    InvalidDamage,
    InvalidGeometry,
    InvalidSnapshot,
}

impl fmt::Display for SurfaceExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cannot export Wayland surface to the Telorgon shell: {self:?}"
        )
    }
}

impl std::error::Error for SurfaceExportError {}
