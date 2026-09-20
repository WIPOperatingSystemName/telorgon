use std::num::NonZeroU64;
use std::ops::{BitOr, BitOrAssign};

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(NonZeroU64);
impl NodeId {
    pub const fn new(index: u32, generation: u32) -> Self {
        let encoded = ((generation as u64) << 32) | (index as u64 + 1);
        Self(NonZeroU64::new(encoded).expect("NodeId index must not be u32::MAX"))
    }
    pub const fn index(self) -> u32 {
        (self.0.get() as u32).wrapping_sub(1)
    }
    pub const fn generation(self) -> u32 {
        (self.0.get() >> 32) as u32
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SubtreeRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DirtyFlags(u16);
impl DirtyFlags {
    pub const NONE: Self = Self(0);
    pub const STRUCTURE: Self = Self(1 << 0);
    pub const STYLE: Self = Self(1 << 1);
    pub const MEASURE: Self = Self(1 << 2);
    pub const ARRANGE: Self = Self(1 << 3);
    pub const SPATIAL: Self = Self(1 << 4);
    pub const CLIP: Self = Self(1 << 5);
    pub const TEXT: Self = Self(1 << 6);
    pub const PAINT: Self = Self(1 << 7);
    pub const SEMANTICS: Self = Self(1 << 8);
    pub const VISIBILITY: Self = Self(1 << 9);
    pub const DRAW_ORDER: Self = Self(1 << 10);
    pub const LAYOUT: Self = Self(Self::MEASURE.0 | Self::ARRANGE.0);
    pub const ALL: Self = Self((1 << 11) - 1);
    pub const fn bits(self) -> u16 {
        self.0
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
impl BitOr for DirtyFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl BitOrAssign for DirtyFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Debug)]
pub struct NodeCore {
    pub parent: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub last_child: Option<NodeId>,
    pub previous_sibling: Option<NodeId>,
    pub next_sibling: Option<NodeId>,
    pub subtree: SubtreeRange,
    pub state_bits: u32,
    pub dirty: DirtyFlags,
    pub content_revision: u64,
    pub style_revision: u64,
    pub semantic_revision: u64,
}
impl Default for NodeCore {
    fn default() -> Self {
        Self {
            parent: None,
            first_child: None,
            last_child: None,
            previous_sibling: None,
            next_sibling: None,
            subtree: SubtreeRange::default(),
            state_bits: 0,
            dirty: DirtyFlags::ALL,
            content_revision: 1,
            style_revision: 1,
            semantic_revision: 1,
        }
    }
}

#[derive(Clone, Debug)]
struct Slot {
    generation: u32,
    core: Option<NodeCore>,
}

#[derive(Clone, Debug, Default)]
pub struct NodeArena {
    slots: Vec<Slot>,
    free: Vec<u32>,
    alive: Vec<NodeId>,
    preorder: Vec<NodeId>,
    dirty_nodes: Vec<NodeId>,
    dirty_generations: Vec<u32>,
    preorder_dirty: bool,
}

impl NodeArena {
    pub fn spawn(&mut self, parent: Option<NodeId>) -> Option<NodeId> {
        if parent.is_some_and(|id| !self.contains(id)) {
            return None;
        }
        let id = if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            let id = NodeId::new(index, slot.generation);
            slot.core = Some(NodeCore::default());
            id
        } else {
            let index = u32::try_from(self.slots.len()).ok()?;
            self.slots.push(Slot {
                generation: 1,
                core: Some(NodeCore::default()),
            });
            NodeId::new(index, 1)
        };
        self.alive.push(id);
        self.queue_dirty(id);
        if let Some(parent) = parent {
            self.attach_before(parent, id, None);
            self.mark_dirty(
                parent,
                DirtyFlags::STRUCTURE
                    | DirtyFlags::DRAW_ORDER
                    | DirtyFlags::LAYOUT
                    | DirtyFlags::PAINT,
            );
        }
        self.preorder_dirty = true;
        Some(id)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.slots
            .get(id.index() as usize)
            .is_some_and(|slot| slot.generation == id.generation() && slot.core.is_some())
    }
    pub fn core(&self, id: NodeId) -> Option<&NodeCore> {
        self.valid_slot(id)?.core.as_ref()
    }
    pub fn core_mut(&mut self, id: NodeId) -> Option<&mut NodeCore> {
        self.valid_slot_mut(id)?.core.as_mut()
    }
    pub fn alive(&self) -> &[NodeId] {
        &self.alive
    }
    pub fn dirty_nodes(&self) -> &[NodeId] {
        &self.dirty_nodes
    }
    pub fn compact_dirty(&mut self) {
        let mut write = 0;
        for read in 0..self.dirty_nodes.len() {
            let node = self.dirty_nodes[read];
            let keep = self.contains(node)
                && self
                    .core(node)
                    .is_some_and(|core| core.dirty != DirtyFlags::NONE);
            if keep {
                self.dirty_nodes[write] = node;
                write += 1;
            } else if self.dirty_generations.get(node.index() as usize).copied()
                == Some(node.generation())
            {
                self.dirty_generations[node.index() as usize] = 0;
            }
        }
        self.dirty_nodes.truncate(write);
    }
    pub fn allocated_bytes(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<Slot>()
            + self.free.capacity() * std::mem::size_of::<u32>()
            + self.alive.capacity() * std::mem::size_of::<NodeId>()
            + self.preorder.capacity() * std::mem::size_of::<NodeId>()
            + self.dirty_nodes.capacity() * std::mem::size_of::<NodeId>()
            + self.dirty_generations.capacity() * std::mem::size_of::<u32>()
    }
    pub fn children(&self, parent: NodeId) -> Children<'_> {
        Children {
            arena: self,
            next: self.core(parent).and_then(|core| core.first_child),
        }
    }
    pub fn preorder(&mut self) -> &[NodeId] {
        if self.preorder_dirty {
            self.rebuild_preorder();
        }
        &self.preorder
    }

    pub fn remove_subtree(&mut self, root: NodeId) -> Vec<NodeId> {
        if !self.contains(root) {
            return Vec::new();
        }
        let former_parent = self.core(root).and_then(|core| core.parent);
        self.detach(root);
        let mut removed = Vec::new();
        self.collect_subtree(root, &mut removed);
        for id in removed.iter().rev().copied() {
            let slot = &mut self.slots[id.index() as usize];
            slot.core = None;
            slot.generation = slot.generation.wrapping_add(1).max(1);
            if self.dirty_generations.get(id.index() as usize).copied() == Some(id.generation()) {
                self.dirty_generations[id.index() as usize] = 0;
            }
            self.free.push(id.index());
            if let Some(position) = self.alive.iter().position(|candidate| *candidate == id) {
                self.alive.swap_remove(position);
            }
        }
        if let Some(parent) = former_parent {
            self.mark_dirty(
                parent,
                DirtyFlags::STRUCTURE
                    | DirtyFlags::DRAW_ORDER
                    | DirtyFlags::LAYOUT
                    | DirtyFlags::PAINT,
            );
        }
        self.preorder_dirty = true;
        removed
    }

    pub fn reparent_before(
        &mut self,
        node: NodeId,
        parent: NodeId,
        before: Option<NodeId>,
    ) -> bool {
        if !self.contains(node) || !self.contains(parent) || node == parent || before == Some(node)
        {
            return false;
        }
        if before.is_some_and(|id| self.core(id).and_then(|core| core.parent) != Some(parent)) {
            return false;
        }
        let mut cursor = Some(parent);
        while let Some(id) = cursor {
            if id == node {
                return false;
            }
            cursor = self.core(id).and_then(|core| core.parent);
        }
        let previous_parent = self.core(node).and_then(|core| core.parent);
        let already_positioned = self.core(node).is_some_and(|core| match before {
            Some(before) => core.parent == Some(parent) && core.next_sibling == Some(before),
            None => core.parent == Some(parent) && core.next_sibling.is_none(),
        });
        if already_positioned {
            return false;
        }
        self.detach(node);
        self.attach_before(parent, node, before);
        if let Some(previous_parent) = previous_parent
            && previous_parent != parent
        {
            self.mark_dirty(
                previous_parent,
                DirtyFlags::STRUCTURE
                    | DirtyFlags::DRAW_ORDER
                    | DirtyFlags::LAYOUT
                    | DirtyFlags::PAINT,
            );
        }
        self.mark_dirty(
            parent,
            DirtyFlags::STRUCTURE | DirtyFlags::DRAW_ORDER | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
        self.preorder_dirty = true;
        true
    }

    pub fn mark_dirty(&mut self, node: NodeId, flags: DirtyFlags) {
        if let Some(core) = self.core_mut(node) {
            core.dirty |= flags;
            self.queue_dirty(node);
        }
        if flags.intersects(DirtyFlags::MEASURE) {
            let mut parent = self.core(node).and_then(|core| core.parent);
            while let Some(id) = parent {
                let Some(core) = self.core_mut(id) else { break };
                core.dirty |= DirtyFlags::LAYOUT;
                parent = core.parent;
                self.queue_dirty(id);
            }
        }
    }
    pub fn clear_dirty(&mut self, node: NodeId, flags: DirtyFlags) {
        if let Some(core) = self.core_mut(node) {
            core.dirty.remove(flags);
            if core.dirty == DirtyFlags::NONE
                && self.dirty_generations.get(node.index() as usize).copied()
                    == Some(node.generation())
            {
                self.dirty_generations[node.index() as usize] = 0;
            }
        }
    }

    fn queue_dirty(&mut self, node: NodeId) {
        let index = node.index() as usize;
        if self.dirty_generations.len() <= index {
            self.dirty_generations.resize(index + 1, 0);
        }
        if self.dirty_generations[index] != node.generation() {
            self.dirty_generations[index] = node.generation();
            self.dirty_nodes.push(node);
        }
    }

    fn valid_slot(&self, id: NodeId) -> Option<&Slot> {
        let slot = self.slots.get(id.index() as usize)?;
        (slot.generation == id.generation()).then_some(slot)
    }
    fn valid_slot_mut(&mut self, id: NodeId) -> Option<&mut Slot> {
        let slot = self.slots.get_mut(id.index() as usize)?;
        (slot.generation == id.generation()).then_some(slot)
    }

    fn attach_before(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) {
        let previous = match before {
            Some(id) => self.core(id).and_then(|core| core.previous_sibling),
            None => self.core(parent).and_then(|core| core.last_child),
        };
        {
            let child_core = self.core_mut(child).expect("child must be alive");
            child_core.parent = Some(parent);
            child_core.previous_sibling = previous;
            child_core.next_sibling = before;
        }
        if let Some(previous) = previous {
            self.core_mut(previous)
                .expect("sibling must be alive")
                .next_sibling = Some(child);
        } else {
            self.core_mut(parent)
                .expect("parent must be alive")
                .first_child = Some(child);
        }
        if let Some(before) = before {
            self.core_mut(before)
                .expect("sibling must be alive")
                .previous_sibling = Some(child);
        } else {
            self.core_mut(parent)
                .expect("parent must be alive")
                .last_child = Some(child);
        }
    }

    fn detach(&mut self, node: NodeId) {
        let Some(core) = self.core(node).cloned() else {
            return;
        };
        let Some(parent) = core.parent else { return };
        if let Some(previous) = core.previous_sibling {
            self.core_mut(previous)
                .expect("sibling must be alive")
                .next_sibling = core.next_sibling;
        } else {
            self.core_mut(parent)
                .expect("parent must be alive")
                .first_child = core.next_sibling;
        }
        if let Some(next) = core.next_sibling {
            self.core_mut(next)
                .expect("sibling must be alive")
                .previous_sibling = core.previous_sibling;
        } else {
            self.core_mut(parent)
                .expect("parent must be alive")
                .last_child = core.previous_sibling;
        }
        let core = self.core_mut(node).expect("node must be alive");
        core.parent = None;
        core.previous_sibling = None;
        core.next_sibling = None;
    }

    fn collect_subtree(&self, node: NodeId, output: &mut Vec<NodeId>) {
        output.push(node);
        let children: Vec<_> = self.children(node).collect();
        for child in children {
            self.collect_subtree(child, output);
        }
    }
    fn rebuild_preorder(&mut self) {
        self.preorder.clear();
        let roots: Vec<_> = self
            .alive
            .iter()
            .copied()
            .filter(|id| self.core(*id).is_some_and(|core| core.parent.is_none()))
            .collect();
        for root in roots {
            self.append_preorder(root);
        }
        self.preorder_dirty = false;
    }
    fn append_preorder(&mut self, node: NodeId) {
        let start = self.preorder.len() as u32;
        self.preorder.push(node);
        let children: Vec<_> = self.children(node).collect();
        for child in children {
            self.append_preorder(child);
        }
        let end = self.preorder.len() as u32;
        if let Some(core) = self.core_mut(node) {
            core.subtree = SubtreeRange { start, end };
        }
    }
}

pub struct Children<'a> {
    arena: &'a NodeArena,
    next: Option<NodeId>,
}
impl Iterator for Children<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let current = self.next?;
        self.next = self.arena.core(current).and_then(|core| core.next_sibling);
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_stale_ids_and_keeps_preorder_deterministic() {
        let mut arena = NodeArena::default();
        let root = arena.spawn(None).unwrap();
        let first = arena.spawn(Some(root)).unwrap();
        let second = arena.spawn(Some(root)).unwrap();
        assert_eq!(arena.preorder(), &[root, first, second]);
        assert_eq!(arena.remove_subtree(first), vec![first]);
        let replacement = arena.spawn(Some(root)).unwrap();
        assert_eq!(replacement.index(), first.index());
        assert_ne!(replacement.generation(), first.generation());
        assert!(!arena.contains(first));
        assert_eq!(arena.preorder(), &[root, second, replacement]);
    }
    #[test]
    fn removal_invalidates_a_complete_subtree() {
        let mut arena = NodeArena::default();
        let root = arena.spawn(None).unwrap();
        let parent = arena.spawn(Some(root)).unwrap();
        let child = arena.spawn(Some(parent)).unwrap();
        assert_eq!(arena.remove_subtree(parent), vec![parent, child]);
        assert!(!arena.contains(parent));
        assert!(!arena.contains(child));
        assert_eq!(arena.preorder(), &[root]);
    }
}
