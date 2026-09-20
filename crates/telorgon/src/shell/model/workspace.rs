//! Ordered, immutable workspace snapshots supplied by a shell policy host.

use std::collections::HashSet;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

use crate::foundation::RectF;

use crate::shell::{OutputId, SurfaceId, WorkspaceId};

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceRevision(NonZeroU64);

impl WorkspaceRevision {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub const fn from_raw(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceName(Box<str>);

impl WorkspaceName {
    pub const MAX_BYTES: usize = 128;

    pub fn new(value: impl AsRef<str>) -> Result<Self, WorkspaceNameError> {
        let value = value.as_ref();
        if value.trim().is_empty() || value.len() > Self::MAX_BYTES {
            return Err(WorkspaceNameError::InvalidName);
        }
        Ok(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for WorkspaceName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("WorkspaceName")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for WorkspaceName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceNameError {
    InvalidName,
}

impl fmt::Display for WorkspaceNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("workspace name must be nonempty and at most 128 bytes")
    }
}

impl std::error::Error for WorkspaceNameError {}

/// Host-owned placement of one surface within a workspace's painter order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkspaceSurface {
    surface: SurfaceId,
    output: OutputId,
    bounds: RectF,
}

impl WorkspaceSurface {
    pub fn new(
        surface: SurfaceId,
        output: OutputId,
        bounds: RectF,
    ) -> Result<Self, WorkspaceSurfaceError> {
        if !valid_positive_rect(bounds) {
            return Err(WorkspaceSurfaceError::InvalidBounds);
        }
        Ok(Self {
            surface,
            output,
            bounds,
        })
    }

    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    pub const fn output(self) -> OutputId {
        self.output
    }

    pub const fn bounds(self) -> RectF {
        self.bounds
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceSurfaceError {
    InvalidBounds,
}

impl fmt::Display for WorkspaceSurfaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("workspace surface bounds must be finite and positive")
    }
}

impl std::error::Error for WorkspaceSurfaceError {}

/// One workspace revision with surfaces ordered back-to-front.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceSnapshot {
    id: WorkspaceId,
    revision: WorkspaceRevision,
    order: u32,
    name: WorkspaceName,
    active: bool,
    surfaces: Arc<[WorkspaceSurface]>,
}

impl WorkspaceSnapshot {
    pub const MAX_SURFACES: usize = 4096;

    pub fn new(
        id: WorkspaceId,
        revision: WorkspaceRevision,
        order: u32,
        name: WorkspaceName,
        active: bool,
        surfaces: Vec<WorkspaceSurface>,
    ) -> Result<Self, WorkspaceSnapshotError> {
        if surfaces.len() > Self::MAX_SURFACES {
            return Err(WorkspaceSnapshotError::TooManySurfaces {
                count: surfaces.len(),
                max: Self::MAX_SURFACES,
            });
        }
        let mut seen = HashSet::with_capacity(surfaces.len());
        if let Some(surface) = surfaces
            .iter()
            .map(|placement| placement.surface)
            .find(|surface| !seen.insert(*surface))
        {
            return Err(WorkspaceSnapshotError::DuplicateSurface { surface });
        }
        Ok(Self {
            id,
            revision,
            order,
            name,
            active,
            surfaces: surfaces.into(),
        })
    }

    pub const fn id(&self) -> WorkspaceId {
        self.id
    }

    pub const fn revision(&self) -> WorkspaceRevision {
        self.revision
    }

    pub const fn order(&self) -> u32 {
        self.order
    }

    pub const fn name(&self) -> &WorkspaceName {
        &self.name
    }

    pub const fn active(&self) -> bool {
        self.active
    }

    pub fn surfaces(&self) -> &[WorkspaceSurface] {
        &self.surfaces
    }

    pub fn surface(&self, id: SurfaceId) -> Option<WorkspaceSurface> {
        self.surfaces
            .iter()
            .copied()
            .find(|placement| placement.surface == id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceSnapshotError {
    TooManySurfaces { count: usize, max: usize },
    DuplicateSurface { surface: SurfaceId },
}

impl fmt::Display for WorkspaceSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManySurfaces { count, max } => {
                write!(
                    formatter,
                    "workspace has {count} surfaces; maximum is {max}"
                )
            }
            Self::DuplicateSurface { surface } => {
                write!(
                    formatter,
                    "surface {surface} appears more than once in the workspace"
                )
            }
        }
    }
}

impl std::error::Error for WorkspaceSnapshotError {}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(surface: u64, x: f32) -> WorkspaceSurface {
        WorkspaceSurface::new(
            SurfaceId::from_raw(surface).unwrap(),
            OutputId::from_raw(1).unwrap(),
            RectF {
                x,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
        )
        .unwrap()
    }

    #[test]
    fn snapshot_preserves_host_workspace_and_back_to_front_surface_order() {
        let snapshot = WorkspaceSnapshot::new(
            WorkspaceId::from_raw(4).unwrap(),
            WorkspaceRevision::from_raw(9).unwrap(),
            2,
            WorkspaceName::new("Development").unwrap(),
            true,
            vec![placement(1, 0.0), placement(2, 100.0)],
        )
        .unwrap();

        assert_eq!(snapshot.name().as_str(), "Development");
        assert!(snapshot.active());
        assert_eq!(snapshot.surfaces()[0].surface().get(), 1);
        assert_eq!(snapshot.surfaces()[1].surface().get(), 2);
        assert_eq!(
            snapshot
                .surface(SurfaceId::from_raw(2).unwrap())
                .unwrap()
                .bounds()
                .x,
            100.0
        );
    }

    #[test]
    fn duplicate_membership_is_rejected_without_reordering() {
        assert!(matches!(
            WorkspaceSnapshot::new(
                WorkspaceId::from_raw(4).unwrap(),
                WorkspaceRevision::INITIAL,
                0,
                WorkspaceName::new("One").unwrap(),
                false,
                vec![placement(1, 0.0), placement(1, 10.0)],
            ),
            Err(WorkspaceSnapshotError::DuplicateSurface { .. })
        ));
    }
}
