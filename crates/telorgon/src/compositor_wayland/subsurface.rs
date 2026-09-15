use std::collections::BTreeMap;
use std::fmt;

use crate::core::PointI;

use crate::compositor_wayland::{SurfaceCommit, WaylandSurfaceId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubsurfacePosition {
    pub offset: PointI,
    pub above: Option<WaylandSurfaceId>,
}

#[derive(Clone, Debug)]
struct SubsurfaceNode {
    parent: WaylandSurfaceId,
    synchronized: bool,
    position: SubsurfacePosition,
    cached_commit: Option<SurfaceCommit>,
}

#[derive(Debug, Default)]
pub struct SubsurfaceGraph {
    nodes: BTreeMap<WaylandSurfaceId, SubsurfaceNode>,
}

impl SubsurfaceGraph {
    pub(crate) fn cached_buffer(
        &self,
        surface: WaylandSurfaceId,
    ) -> Option<crate::compositor_wayland::WaylandBufferId> {
        self.nodes
            .get(&surface)?
            .cached_commit
            .as_ref()?
            .attachment
            .flatten()
            .map(|attachment| attachment.buffer)
    }

    pub fn add(
        &mut self,
        child: WaylandSurfaceId,
        parent: WaylandSurfaceId,
    ) -> Result<(), SubsurfaceError> {
        if child == parent || self.nodes.contains_key(&child) {
            return Err(SubsurfaceError::InvalidRelationship);
        }
        let mut cursor = Some(parent);
        while let Some(surface) = cursor {
            if surface == child {
                return Err(SubsurfaceError::Cycle);
            }
            cursor = self.nodes.get(&surface).map(|node| node.parent);
        }
        self.nodes.insert(
            child,
            SubsurfaceNode {
                parent,
                synchronized: true,
                position: SubsurfacePosition::default(),
                cached_commit: None,
            },
        );
        Ok(())
    }

    pub fn remove(&mut self, child: WaylandSurfaceId) -> Result<(), SubsurfaceError> {
        if self.nodes.remove(&child).is_none() {
            return Err(SubsurfaceError::UnknownSubsurface);
        }
        Ok(())
    }

    pub fn set_synchronized(
        &mut self,
        child: WaylandSurfaceId,
        synchronized: bool,
    ) -> Result<Option<SurfaceCommit>, SubsurfaceError> {
        let node = self
            .nodes
            .get_mut(&child)
            .ok_or(SubsurfaceError::UnknownSubsurface)?;
        node.synchronized = synchronized;
        Ok((!synchronized).then(|| node.cached_commit.take()).flatten())
    }

    pub fn stage_or_release(
        &mut self,
        child: WaylandSurfaceId,
        commit: SurfaceCommit,
    ) -> Result<Option<SurfaceCommit>, SubsurfaceError> {
        let node = self
            .nodes
            .get_mut(&child)
            .ok_or(SubsurfaceError::UnknownSubsurface)?;
        if node.synchronized {
            node.cached_commit = Some(commit);
            Ok(None)
        } else {
            Ok(Some(commit))
        }
    }

    pub fn release_children(
        &mut self,
        parent: WaylandSurfaceId,
    ) -> Vec<(WaylandSurfaceId, SurfaceCommit)> {
        self.nodes
            .iter_mut()
            .filter_map(|(child, node)| {
                (node.parent == parent && node.synchronized)
                    .then(|| node.cached_commit.take().map(|commit| (*child, commit)))
                    .flatten()
            })
            .collect()
    }

    pub fn set_position(
        &mut self,
        child: WaylandSurfaceId,
        position: SubsurfacePosition,
    ) -> Result<(), SubsurfaceError> {
        let node = self
            .nodes
            .get_mut(&child)
            .ok_or(SubsurfaceError::UnknownSubsurface)?;
        if position.above == Some(child) {
            return Err(SubsurfaceError::InvalidSibling);
        }
        node.position = position;
        Ok(())
    }

    pub fn parent(&self, child: WaylandSurfaceId) -> Option<WaylandSurfaceId> {
        self.nodes.get(&child).map(|node| node.parent)
    }

    pub fn position(&self, child: WaylandSurfaceId) -> Option<SubsurfacePosition> {
        self.nodes.get(&child).map(|node| node.position)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubsurfaceError {
    UnknownSubsurface,
    InvalidRelationship,
    InvalidSibling,
    Cycle,
}

impl fmt::Display for SubsurfaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Wayland subsurface operation failed: {self:?}")
    }
}

impl std::error::Error for SubsurfaceError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(raw: u32) -> WaylandSurfaceId {
        WaylandSurfaceId::from_raw(raw).unwrap()
    }

    #[test]
    fn parent_cycles_are_rejected() {
        let mut graph = SubsurfaceGraph::default();
        graph.add(surface(2), surface(1)).unwrap();
        graph.add(surface(3), surface(2)).unwrap();
        assert_eq!(
            graph.add(surface(1), surface(3)),
            Err(SubsurfaceError::Cycle)
        );
    }
}
