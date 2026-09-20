//! Validated parent-before-child client-surface tree assembly.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use crate::runtime::{RuntimeError, Ui};
use crate::shell::{ClientSurfaceSnapshot, OutputId, ShellLayerKind, SurfaceId};

use crate::components::shell::primitives::{ClientSurface, ClientSurfaceRef, ShellLayerRef};

/// One bounded host-ordered surface tree. Input order is retained as exact painter order.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceTree {
    surfaces: Arc<[Arc<ClientSurfaceSnapshot>]>,
}

impl SurfaceTree {
    pub const MAX_SURFACES: usize = 256;

    pub fn new(surfaces: Vec<ClientSurfaceSnapshot>) -> Result<Self, SurfaceTreeError> {
        if surfaces.is_empty() {
            return Err(SurfaceTreeError::Empty);
        }
        if surfaces.len() > Self::MAX_SURFACES {
            return Err(SurfaceTreeError::TooManySurfaces {
                count: surfaces.len(),
                max: Self::MAX_SURFACES,
            });
        }
        if surfaces[0].parent().is_some() {
            return Err(SurfaceTreeError::RootHasParent {
                root: surfaces[0].id(),
            });
        }

        let root = surfaces[0].id();
        let mut seen = BTreeSet::new();
        for (index, surface) in surfaces.iter().enumerate() {
            if !seen.insert(surface.id()) {
                return Err(SurfaceTreeError::DuplicateSurface {
                    surface: surface.id(),
                });
            }
            if index > 0 {
                let parent = surface.parent().ok_or(SurfaceTreeError::AdditionalRoot {
                    surface: surface.id(),
                })?;
                if !seen.contains(&parent) {
                    return Err(SurfaceTreeError::ParentMustPrecedeChild {
                        surface: surface.id(),
                        parent,
                    });
                }
            }
        }
        debug_assert!(seen.contains(&root));

        Ok(Self {
            surfaces: surfaces.into_iter().map(Arc::new).collect(),
        })
    }

    pub fn root(&self) -> &ClientSurfaceSnapshot {
        &self.surfaces[0]
    }

    pub fn surfaces(&self) -> impl ExactSizeIterator<Item = &ClientSurfaceSnapshot> {
        self.surfaces.iter().map(AsRef::as_ref)
    }

    pub fn len(&self) -> usize {
        self.surfaces.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        layer: ShellLayerRef,
    ) -> Result<SurfaceTreeRef, SurfaceTreeMountError> {
        if layer.kind() != ShellLayerKind::Workspace {
            return Err(SurfaceTreeError::RequiresWorkspaceLayer.into());
        }

        let mut mounted = Vec::with_capacity(self.surfaces.len());
        let mut nodes = BTreeMap::new();
        for snapshot in self.surfaces.iter() {
            let parent = match snapshot.parent() {
                Some(parent) => *nodes
                    .get(&parent)
                    .ok_or_else(|| RuntimeError::new("validated surface parent was not mounted"))?,
                None => layer.content_node(),
            };
            let reference = ClientSurface::from_shared(snapshot.clone()).mount_under(
                ui,
                parent,
                layer.output(),
                layer.authority().grant(),
                None,
            )?;
            nodes.insert(snapshot.id(), reference.node());
            mounted.push(reference);
        }

        Ok(SurfaceTreeRef {
            output: layer.output(),
            surfaces: mounted.into(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct SurfaceTreeRef {
    output: OutputId,
    surfaces: Arc<[ClientSurfaceRef]>,
}

impl SurfaceTreeRef {
    pub const fn output(&self) -> OutputId {
        self.output
    }

    pub fn root(&self) -> &ClientSurfaceRef {
        &self.surfaces[0]
    }

    pub fn surfaces(&self) -> &[ClientSurfaceRef] {
        &self.surfaces
    }

    pub fn surface(&self, id: SurfaceId) -> Option<&ClientSurfaceRef> {
        self.surfaces.iter().find(|surface| surface.surface() == id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceTreeError {
    Empty,
    TooManySurfaces {
        count: usize,
        max: usize,
    },
    RootHasParent {
        root: SurfaceId,
    },
    AdditionalRoot {
        surface: SurfaceId,
    },
    DuplicateSurface {
        surface: SurfaceId,
    },
    ParentMustPrecedeChild {
        surface: SurfaceId,
        parent: SurfaceId,
    },
    RequiresWorkspaceLayer,
}

impl fmt::Display for SurfaceTreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "surface tree must contain one root",
            Self::TooManySurfaces { .. } => "surface tree exceeds its bounded surface capacity",
            Self::RootHasParent { .. } => "surface tree root cannot have a parent",
            Self::AdditionalRoot { .. } => "surface tree contains more than one root",
            Self::DuplicateSurface { .. } => "surface tree contains a duplicate identity",
            Self::ParentMustPrecedeChild { .. } => {
                "surface-tree painter order must place each parent before its child"
            }
            Self::RequiresWorkspaceLayer => "surface trees require an authorized workspace layer",
        })
    }
}

impl std::error::Error for SurfaceTreeError {}

#[derive(Debug)]
pub enum SurfaceTreeMountError {
    Tree(SurfaceTreeError),
    Runtime(RuntimeError),
}

impl fmt::Display for SurfaceTreeMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tree(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SurfaceTreeMountError {}

impl From<SurfaceTreeError> for SurfaceTreeMountError {
    fn from(value: SurfaceTreeError) -> Self {
        Self::Tree(value)
    }
}

impl From<RuntimeError> for SurfaceTreeMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
