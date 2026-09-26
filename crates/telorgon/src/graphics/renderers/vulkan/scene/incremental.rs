//! Incremental CPU-side conversion; submission and buffer lifetime stay with the scene.
use super::*;

impl VulkanScene {
    pub(super) fn patch_images(&mut self, delta: &RenderSceneDelta) {
        let rebuild_images = !delta.image_resources.is_empty();
        apply_patches(&mut self.images, &delta.images, delta.image_len);
        if rebuild_images {
            let image_resources = &self.image_resources;
            let external_images = &self.external_images;
            self.gpu_images = self
                .images
                .iter()
                .map(|instance| {
                    let alpha = image_resources
                        .get(&instance.image)
                        .map(|resource| resource.alpha_mode)
                        .or_else(|| {
                            external_images
                                .get(&instance.image)
                                .map(|resource| resource.image.alpha_mode)
                        });
                    convert_image(instance, alpha)
                })
                .collect();
            self.image_dirty.add(0..self.gpu_images.len());
        } else {
            let image_resources = &self.image_resources;
            let external_images = &self.external_images;
            patch_gpu_values(
                &mut self.gpu_images,
                &delta.images,
                delta.image_len,
                |instance| {
                    let alpha = image_resources
                        .get(&instance.image)
                        .map(|resource| resource.alpha_mode)
                        .or_else(|| {
                            external_images
                                .get(&instance.image)
                                .map(|resource| resource.image.alpha_mode)
                        });
                    convert_image(instance, alpha)
                },
                &mut self.image_dirty,
            );
        }
    }

    pub(super) fn patch_spatial_tables(&mut self, delta: &RenderSceneDelta) {
        let rebuild_clips = delta.clip_len != self.clips.len()
            || delta.clips.iter().any(|patch| {
                patch.values.iter().enumerate().any(|(offset, clip)| {
                    self.clips
                        .get(patch.start + offset)
                        .is_none_or(|old| old.id != clip.id)
                })
            });
        apply_patches(&mut self.clips, &delta.clips, delta.clip_len);
        if rebuild_clips {
            self.rebuild_clips();
        } else {
            for patch in &delta.clips {
                for clip in patch.values.iter().filter(|clip| clip.id.0 != 0) {
                    let index = clip.id.0 as usize;
                    self.gpu_clips[index] = convert_clip(clip);
                    self.clip_dirty.add(index..index + 1);
                }
            }
        }
        let rebuild_spatial = delta.spatial_len != self.spatial.len()
            || delta.spatial_nodes.iter().any(|patch| {
                patch.values.iter().enumerate().any(|(offset, spatial)| {
                    self.spatial
                        .get(patch.start + offset)
                        .is_none_or(|old| old.id != spatial.id)
                })
            });
        apply_patches(&mut self.spatial, &delta.spatial_nodes, delta.spatial_len);
        if rebuild_spatial {
            self.rebuild_spatial();
        } else {
            for patch in &delta.spatial_nodes {
                for spatial in patch.values.iter() {
                    let index = spatial.id.0 as usize;
                    self.gpu_spatial[index] = convert_spatial(spatial);
                    self.spatial_dirty.add(index..index + 1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::{Affine2D, RectF};
    use crate::ui::layout::{ClipId, SpatialId};

    #[test]
    fn stable_clip_and_spatial_ids_dirty_only_changed_gpu_entries() {
        let mut source = crate::graphics::render::RenderScene::default();
        for index in 1..20 {
            let node = crate::ui::UiNodeId::new(index, 1);
            source.clips.upsert(
                node,
                RenderClip {
                    id: ClipId(index),
                    rect: RectF {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 100.0,
                    },
                    corner_radii: Default::default(),
                },
            );
            source.spatial_nodes.upsert(
                node,
                RenderSpatialNode {
                    id: SpatialId(index),
                    transform: Affine2D::IDENTITY,
                },
            );
        }
        let mut scene = VulkanScene::default();
        scene.apply(&source.take_delta().unwrap());
        scene.clip_dirty.take();
        scene.spatial_dirty.take();
        let node = crate::ui::UiNodeId::new(3, 1);
        let mut clip = *source.clips.get(node).unwrap();
        clip.rect.x = 4.0;
        source.clips.upsert(node, clip);
        source.spatial_nodes.upsert(
            node,
            RenderSpatialNode {
                id: SpatialId(3),
                transform: Affine2D::translation(2.0, 4.0),
            },
        );
        scene.apply(&source.take_delta().unwrap());
        assert_eq!(scene.clip_dirty.ranges, vec![3..4]);
        assert_eq!(scene.spatial_dirty.ranges, vec![3..4]);
        assert_eq!(scene.gpu_clips.len(), 20);
        assert_eq!(scene.gpu_spatial.len(), 20);
        source.clips.remove(node);
        source.spatial_nodes.remove(node);
        scene.apply(&source.take_delta().unwrap());
        assert_eq!(
            bytemuck::bytes_of(&scene.gpu_clips[3]),
            bytemuck::bytes_of(&none_clip())
        );
        assert_eq!(
            bytemuck::bytes_of(&scene.gpu_spatial[3]),
            bytemuck::bytes_of(&identity_spatial())
        );
    }
}
