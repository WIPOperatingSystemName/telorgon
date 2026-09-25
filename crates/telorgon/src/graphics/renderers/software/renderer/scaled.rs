use super::*;
use crate::foundation::Affine2D;

impl SoftwareRenderer {
    /// Rasterize a logical scene directly into a physical window buffer.
    pub(crate) fn render_logical<'frame>(
        &self,
        scene: &mut SoftwareScene,
        frame: &mut SoftwareFrameContext<'frame>,
        target: &SoftwareTarget<'frame>,
        request: &RenderRequest,
    ) -> RenderResult<RenderStats> {
        let sx = target.info.extent.width as f32 / scene.extent.width.max(1.0);
        let sy = target.info.extent.height as f32 / scene.extent.height.max(1.0);
        if sx == 1.0 && sy == 1.0 {
            return self.render(scene, frame, target, request);
        }
        let scale = Affine2D {
            m11: sx,
            m22: sy,
            ..Affine2D::IDENTITY
        };
        // Keep retained geometry logical so future scene deltas and repeated frames
        // cannot accumulate the output scale. Image pixels and glyph atlases stay shared.
        let spatial = scene.spatial.clone();
        let clips = scene.clips.clone();
        let damage = scene.pending_damage.clone();
        for node in &mut scene.spatial {
            node.transform = scale.then(node.transform);
        }
        if !scene.spatial.iter().any(|node| node.id == SpatialId(0)) {
            scene.spatial.push(RenderSpatialNode {
                id: SpatialId(0),
                transform: scale,
            });
        }
        for clip in &mut scene.clips {
            clip.rect = scale.transform_rect(clip.rect);
            clip.corner_radii.top_left *= sx.min(sy);
            clip.corner_radii.top_right *= sx.min(sy);
            clip.corner_radii.bottom_left *= sx.min(sy);
            clip.corner_radii.bottom_right *= sx.min(sy);
        }
        for rect in &mut scene.pending_damage.rects {
            *rect = scale.transform_rect(*rect);
        }
        let result = self.render(scene, frame, target, request);
        scene.spatial = spatial;
        scene.clips = clips;
        if result.is_err() {
            scene.pending_damage = damage;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::render::{BatchKey, PipelineKind, TargetStore};
    use crate::ui::layout::ClipId;
    use crate::ui::{CornerRadii, Outline, ShadowList, UiNodeId};

    #[test]
    fn logical_scene_scales_clips_and_damage_without_accumulating_scale() {
        let renderer = SoftwareRenderer;
        let mut scene = SoftwareScene::default();
        scene.extent = SizeF {
            width: 4.0,
            height: 4.0,
        };
        let bounds = RectF {
            x: 0.0,
            y: 0.0,
            width: 4.0,
            height: 4.0,
        };
        let red = ColorRgba8::rgba(255, 0, 0, 255);
        scene.boxes.push(BoxInstance {
            node: UiNodeId::new(0, 1),
            rect: bounds,
            view_bounds: bounds,
            background: Some(red),
            border: Border::default(),
            outline: Outline::default(),
            corner_radii: CornerRadii::default(),
            shadows: ShadowList::default(),
            opacity: 1.0,
            clip: ClipId(1),
            spatial: SpatialId(0),
        });
        scene.clips.push(RenderClip {
            id: ClipId(1),
            rect: RectF {
                width: 2.0,
                ..bounds
            },
            corner_radii: CornerRadii::default(),
        });
        scene.draw_order.push(DrawItem {
            kind: PrimitiveKind::Box,
            index: 0,
            batch: BatchKey {
                pipeline: PipelineKind::AnalyticBox,
                resource: 0,
                clip: ClipId(1),
                blend: BlendMode::Opaque,
                target: 0,
            },
        });
        let mut surface = SoftwareSurface::default();
        for size in [8, 8, 5] {
            let target = SoftwareTarget::new(RenderTargetInfo::full(SizeI {
                width: size,
                height: size,
            }));
            renderer
                .render_logical(
                    &mut scene,
                    &mut surface.begin_frame(),
                    &target,
                    &RenderRequest {
                        force: true,
                        load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                        store: TargetStore::Store,
                        region: None,
                    },
                )
                .unwrap();
            assert_eq!(&surface.pixels_rgba8()[..4], &[255, 0, 0, 255]);
            let outside = (size as usize - 1) * 4;
            assert_eq!(
                &surface.pixels_rgba8()[outside..outside + 4],
                &[0, 0, 0, 255]
            );
            assert!(scene.spatial.is_empty());
            assert_eq!(scene.clips[0].rect.width, 2.0);
        }
        // A later partial update must repaint the scaled area, including its far edge.
        scene.boxes[0].background = Some(ColorRgba8::rgba(0, 255, 0, 255));
        scene.pending_damage.rects.push(RectF {
            width: 2.0,
            ..bounds
        });
        let target = SoftwareTarget::new(RenderTargetInfo::full(SizeI {
            width: 5,
            height: 5,
        }));
        let stats = renderer
            .render_logical(
                &mut scene,
                &mut surface.begin_frame(),
                &target,
                &RenderRequest {
                    force: false,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .unwrap();
        assert!(stats.recorded);
        let offset = (4 * 5 + 1) * 4;
        assert_eq!(
            &surface.pixels_rgba8()[offset..offset + 4],
            &[0, 255, 0, 255]
        );
    }
}
