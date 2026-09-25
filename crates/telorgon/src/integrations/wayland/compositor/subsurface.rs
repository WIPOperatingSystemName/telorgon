use std::collections::BTreeMap;
use std::fmt;

use crate::foundation::PointI;

use crate::integrations::wayland::compositor::{SurfaceCommit, WaylandSurfaceId};

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
    ) -> Option<crate::integrations::wayland::compositor::WaylandBufferId> {
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
        if self.effectively_synchronized(child) {
            Ok(None)
        } else {
            Ok(self
                .nodes
                .get_mut(&child)
                .and_then(|node| node.cached_commit.take()))
        }
    }

    pub(crate) fn effectively_synchronized(&self, child: WaylandSurfaceId) -> bool {
        let mut cursor = child;
        while let Some(node) = self.nodes.get(&cursor) {
            if node.synchronized {
                return true;
            }
            cursor = node.parent;
        }
        false
    }

    pub fn stage_or_release(
        &mut self,
        child: WaylandSurfaceId,
        commit: SurfaceCommit,
    ) -> Result<Option<SurfaceCommit>, SubsurfaceError> {
        let synchronized = self.effectively_synchronized(child);
        let node = self
            .nodes
            .get_mut(&child)
            .ok_or(SubsurfaceError::UnknownSubsurface)?;
        if synchronized {
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
        let mut pending: Vec<_> = self
            .nodes
            .iter()
            .filter_map(|(&child, node)| {
                (node.parent == parent && node.synchronized).then_some(child)
            })
            .collect();
        let mut released = Vec::new();
        let mut index = 0;
        while index < pending.len() {
            let child = pending[index];
            index += 1;
            if let Some(commit) = self
                .nodes
                .get_mut(&child)
                .and_then(|node| node.cached_commit.take())
            {
                released.push((child, commit));
            }
            // Synchronization propagates through the entire subtree. Even an
            // unchanged intermediate surface must release cached descendant state.
            pending.extend(
                self.nodes
                    .iter()
                    .filter_map(|(&descendant, node)| (node.parent == child).then_some(descendant)),
            );
        }
        released
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

    #[test]
    fn desynchronized_descendant_waits_for_synchronized_ancestor() {
        let mut graph = SubsurfaceGraph::default();
        graph.add(surface(2), surface(1)).unwrap();
        graph.add(surface(3), surface(2)).unwrap();
        graph.set_synchronized(surface(3), false).unwrap();
        let commit = SurfaceCommit {
            buffer_scale: Some(2),
            ..Default::default()
        };
        assert_eq!(
            graph.stage_or_release(surface(3), commit.clone()).unwrap(),
            None
        );
        assert_eq!(graph.set_synchronized(surface(3), false).unwrap(), None);
        assert_eq!(
            graph.release_children(surface(1)),
            vec![(surface(3), commit.clone())]
        );
        assert!(graph.release_children(surface(1)).is_empty());
        graph.set_synchronized(surface(2), false).unwrap();
        assert_eq!(
            graph.stage_or_release(surface(3), commit.clone()).unwrap(),
            Some(commit)
        );
    }

    #[test]
    fn parent_commit_releases_nested_updates_without_intermediate_commit() {
        let mut graph = SubsurfaceGraph::default();
        graph.add(surface(2), surface(1)).unwrap();
        graph.add(surface(3), surface(2)).unwrap();
        graph
            .stage_or_release(surface(3), SurfaceCommit::default())
            .unwrap();
        let released = graph.release_children(surface(1));
        assert_eq!(
            released.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![surface(3)]
        );
        assert!(graph.release_children(surface(1)).is_empty());
    }

    #[test]
    fn parent_commit_does_not_cross_desynchronized_child() {
        let mut graph = SubsurfaceGraph::default();
        graph.add(surface(2), surface(1)).unwrap();
        graph.add(surface(3), surface(2)).unwrap();
        graph.set_synchronized(surface(2), false).unwrap();
        graph
            .stage_or_release(surface(3), SurfaceCommit::default())
            .unwrap();
        assert!(graph.release_children(surface(1)).is_empty());
        assert_eq!(graph.release_children(surface(2)).len(), 1);
    }
}
