use crate::graphics::scene::NodeId;

#[derive(Clone, Debug)]
pub struct SparseSet<T> {
    dense_nodes: Vec<NodeId>,
    dense_values: Vec<T>,
    sparse: Vec<u32>,
}
impl<T> Default for SparseSet<T> {
    fn default() -> Self {
        Self {
            dense_nodes: Vec::new(),
            dense_values: Vec::new(),
            sparse: Vec::new(),
        }
    }
}
impl<T> SparseSet<T> {
    pub fn len(&self) -> usize {
        self.dense_values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.dense_values.is_empty()
    }
    pub fn nodes(&self) -> &[NodeId] {
        &self.dense_nodes
    }
    pub fn values(&self) -> &[T] {
        &self.dense_values
    }
    pub fn values_mut(&mut self) -> &mut [T] {
        &mut self.dense_values
    }
    pub fn contains(&self, node: NodeId) -> bool {
        self.dense_index(node).is_some()
    }
    pub fn get(&self, node: NodeId) -> Option<&T> {
        self.dense_index(node)
            .map(|index| &self.dense_values[index])
    }
    pub fn get_mut(&mut self, node: NodeId) -> Option<&mut T> {
        let index = self.dense_index(node)?;
        self.dense_values.get_mut(index)
    }
    pub fn insert(&mut self, node: NodeId, value: T) -> Option<T> {
        if let Some(index) = self.dense_index(node) {
            return Some(std::mem::replace(&mut self.dense_values[index], value));
        }
        let sparse_index = node.index() as usize;
        if self.sparse.len() <= sparse_index {
            self.sparse.resize(sparse_index + 1, 0);
        }
        let dense_index = self.dense_values.len();
        self.dense_nodes.push(node);
        self.dense_values.push(value);
        self.sparse[sparse_index] = dense_index as u32 + 1;
        None
    }
    pub fn remove(&mut self, node: NodeId) -> Option<T> {
        let index = self.dense_index(node)?;
        self.sparse[node.index() as usize] = 0;
        self.dense_nodes.swap_remove(index);
        let value = self.dense_values.swap_remove(index);
        if index < self.dense_nodes.len() {
            self.sparse[self.dense_nodes[index].index() as usize] = index as u32 + 1;
        }
        Some(value)
    }
    pub fn clear(&mut self) {
        self.dense_nodes.clear();
        self.dense_values.clear();
        self.sparse.fill(0);
    }
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &T)> {
        self.dense_nodes
            .iter()
            .copied()
            .zip(self.dense_values.iter())
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (NodeId, &mut T)> {
        self.dense_nodes
            .iter()
            .copied()
            .zip(self.dense_values.iter_mut())
    }
    pub fn allocated_bytes(&self) -> usize {
        self.dense_nodes.capacity() * std::mem::size_of::<NodeId>()
            + self.dense_values.capacity() * std::mem::size_of::<T>()
            + self.sparse.capacity() * std::mem::size_of::<u32>()
    }
    fn dense_index(&self, node: NodeId) -> Option<usize> {
        let encoded = *self.sparse.get(node.index() as usize)?;
        if encoded == 0 {
            return None;
        }
        let index = encoded as usize - 1;
        (self.dense_nodes.get(index).copied() == Some(node)).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn swap_remove_updates_sparse_mapping() {
        let mut set = SparseSet::default();
        let a = NodeId::new(1, 1);
        let b = NodeId::new(7, 2);
        let c = NodeId::new(3, 4);
        set.insert(a, 10);
        set.insert(b, 20);
        set.insert(c, 30);
        assert_eq!(set.remove(b), Some(20));
        assert_eq!(set.get(c), Some(&30));
        assert_eq!(set.get(NodeId::new(3, 3)), None);
    }
}
