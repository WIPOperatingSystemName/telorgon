use std::collections::{BTreeMap, VecDeque};

use crate::graphics::render::{RangePatch, RenderSceneDelta};

#[derive(Clone, Debug)]
pub struct SceneDeltaQueue {
    queue: VecDeque<RenderSceneDelta>,
    capacity: usize,
    high_water: usize,
}

impl SceneDeltaQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
            high_water: 0,
        }
    }

    pub fn push(&mut self, delta: RenderSceneDelta) {
        if self.queue.len() == self.capacity {
            let mut pending = self
                .queue
                .pop_back()
                .expect("a full delta queue has a tail");
            merge_scene_delta(&mut pending, delta);
            self.queue.push_back(pending);
        } else {
            self.queue.push_back(delta);
        }
        self.high_water = self.high_water.max(self.queue.len());
    }

    pub fn pop(&mut self) -> Option<RenderSceneDelta> {
        self.queue.pop_front()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn high_water(&self) -> usize {
        self.high_water
    }
}

fn merge_scene_delta(pending: &mut RenderSceneDelta, mut newer: RenderSceneDelta) {
    pending.epoch = newer.epoch;
    pending.extent = newer.extent;
    pending.background = newer.background;
    pending.boxes = merge_range_patches(
        std::mem::take(&mut pending.boxes),
        std::mem::take(&mut newer.boxes),
        newer.box_len,
    );
    pending.box_len = newer.box_len;
    pending.glyphs = merge_range_patches(
        std::mem::take(&mut pending.glyphs),
        std::mem::take(&mut newer.glyphs),
        newer.glyph_len,
    );
    pending.glyph_len = newer.glyph_len;
    pending.images = merge_range_patches(
        std::mem::take(&mut pending.images),
        std::mem::take(&mut newer.images),
        newer.image_len,
    );
    pending.image_len = newer.image_len;
    pending.materials = merge_range_patches(
        std::mem::take(&mut pending.materials),
        std::mem::take(&mut newer.materials),
        newer.material_len,
    );
    pending.material_len = newer.material_len;
    pending.clips = merge_range_patches(
        std::mem::take(&mut pending.clips),
        std::mem::take(&mut newer.clips),
        newer.clip_len,
    );
    pending.clip_len = newer.clip_len;
    pending.spatial_nodes = merge_range_patches(
        std::mem::take(&mut pending.spatial_nodes),
        std::mem::take(&mut newer.spatial_nodes),
        newer.spatial_len,
    );
    pending.spatial_len = newer.spatial_len;
    if newer.draw_order.is_some() {
        pending.draw_order = newer.draw_order;
    }
    if newer.damage.full {
        pending.damage.full = true;
        pending.damage.rects.clear();
    } else if !pending.damage.full {
        for rect in newer.damage.rects {
            pending.damage.add(rect, newer.extent);
        }
    }
    pending.atlas_extent = newer.atlas_extent;
    pending.atlas_pages.append(&mut newer.atlas_pages);
    pending.image_resources.append(&mut newer.image_resources);
    pending
        .material_resources
        .append(&mut newer.material_resources);
}

fn merge_range_patches<T: Clone>(
    older: Vec<RangePatch<T>>,
    newer: Vec<RangePatch<T>>,
    final_len: usize,
) -> Vec<RangePatch<T>> {
    let mut writes = BTreeMap::new();
    for patch in older.into_iter().chain(newer) {
        for (offset, value) in patch.values.iter().enumerate() {
            let Some(index) = patch.start.checked_add(offset) else {
                break;
            };
            if index >= final_len {
                break;
            }
            writes.insert(index, value.clone());
        }
    }

    let mut patches = Vec::new();
    let mut start = 0;
    let mut values = Vec::new();
    for (index, value) in writes {
        if !values.is_empty() && index != start + values.len() {
            patches.push(RangePatch {
                start,
                values: std::mem::take(&mut values).into(),
            });
        }
        if values.is_empty() {
            start = index;
        }
        values.push(value);
    }
    if !values.is_empty() {
        patches.push(RangePatch {
            start,
            values: values.into(),
        });
    }
    patches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::RectF;
    use crate::graphics::render::{BoxInstance, RenderScene};
    use crate::graphics::scene::NodeId;

    fn box_instance(node: NodeId) -> BoxInstance {
        BoxInstance {
            node,
            rect: RectF::default(),
            view_bounds: RectF::default(),
            background: None,
            border: Default::default(),
            outline: Default::default(),
            corner_radii: Default::default(),
            shadows: Default::default(),
            opacity: 1.0,
            clip: Default::default(),
            spatial: Default::default(),
        }
    }

    #[test]
    fn full_queue_coalesces_without_losing_the_newest_epoch() {
        let mut scene = RenderScene::default();
        let first = scene.take_delta().unwrap();
        scene.damage.full = true;
        let second = scene.take_delta().unwrap();
        let mut queue = SceneDeltaQueue::new(1);
        queue.push(first);
        queue.push(second);
        let merged = queue.pop().unwrap();
        assert_eq!(merged.epoch, 2);
        assert!(merged.damage.full);
        assert!(queue.pop().is_none());
        assert_eq!(queue.high_water(), 1);
    }

    #[test]
    fn coalesced_shrink_produces_sorted_patches_inside_the_final_length() {
        let first_node = NodeId::new(0, 0);
        let second_node = NodeId::new(1, 0);
        let mut scene = RenderScene::default();
        scene.boxes.upsert(first_node, box_instance(first_node));
        scene.boxes.upsert(second_node, box_instance(second_node));
        let first = scene.take_delta().unwrap();

        scene.boxes.remove(first_node);
        let second = scene.take_delta().unwrap();
        let mut queue = SceneDeltaQueue::new(1);
        queue.push(first);
        queue.push(second);
        let merged = queue.pop().unwrap();

        assert_eq!(merged.box_len, 1);
        assert_eq!(merged.boxes.len(), 1);
        assert_eq!(merged.boxes[0].start, 0);
        assert_eq!(merged.boxes[0].values.len(), 1);
        assert_eq!(merged.boxes[0].values[0].node, second_node);
        assert!(
            merged
                .boxes
                .iter()
                .all(|patch| { patch.start + patch.values.len() <= merged.box_len })
        );
        assert!(
            merged
                .boxes
                .windows(2)
                .all(|patches| { patches[0].start + patches[0].values.len() <= patches[1].start })
        );
    }
}
