use crate::foundation::SizeI;

/// One sampled image per input. Additive blending sums premultiplied weighted endpoints.
pub(in crate::host::linux_shell) fn image_scene(
    extent: SizeI,
    inputs: &[(crate::graphics::render::ImageId, f32)],
    additive: bool,
) -> crate::graphics::render::RenderScene {
    use crate::foundation::{ColorRgba8, RectF, SizeF};
    use crate::graphics::render::*;
    use crate::graphics::scene::NodeId;
    let mut scene = RenderScene::default();
    scene.extent = SizeF {
        width: extent.width as f32,
        height: extent.height as f32,
    };
    scene.background = ColorRgba8::rgba(0, 0, 0, 0);
    scene.damage.full = true;
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: extent.width as f32,
        height: extent.height as f32,
    };
    let mut order = Vec::new();
    for (index, (image, opacity)) in inputs.iter().enumerate() {
        let node = NodeId::new(index as u32 + 1, 1);
        scene.images.upsert(
            node,
            ImageInstance {
                node,
                image: *image,
                tint: None,
                rect,
                view_bounds: rect,
                content_version: 1,
                opacity: *opacity,
                clip: ClipId(0),
                spatial: SpatialId(0),
            },
        );
        order.push(DrawItem {
            kind: PrimitiveKind::Image,
            index: index as u32,
            batch: BatchKey {
                pipeline: PipelineKind::Image,
                resource: image.0,
                clip: ClipId(0),
                blend: if additive {
                    BlendMode::Add
                } else {
                    BlendMode::Alpha
                },
                target: 0,
            },
        });
    }
    scene.set_draw_order(order);
    scene
}

/// Snapshot bounds in units of the window geometry, independent of shadow padding.
pub(super) fn relative_bounds(
    bounds: crate::foundation::RectI,
    window: crate::foundation::RectI,
) -> crate::foundation::RectF {
    crate::foundation::RectF {
        x: (bounds.x - window.x) as f32 / window.width.max(1) as f32,
        y: (bounds.y - window.y) as f32 / window.height.max(1) as f32,
        width: bounds.width as f32 / window.width.max(1) as f32,
        height: bounds.height as f32 / window.height.max(1) as f32,
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::host::linux_shell) struct SnapshotInput {
    pub id: u64,
    pub weight: f32,
    /// Destination in normalized output coordinates. May extend beyond the canvas.
    pub target: crate::foundation::RectF,
}
impl From<(u64, f32)> for SnapshotInput {
    fn from((id, weight): (u64, f32)) -> Self {
        Self {
            id,
            weight,
            target: crate::foundation::RectF {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
        }
    }
}
impl SnapshotInput {
    pub fn aligned(
        id: u64,
        weight: f32,
        from: crate::foundation::RectF,
        to: crate::foundation::RectF,
    ) -> Self {
        Self {
            id,
            weight,
            target: crate::foundation::RectF {
                x: (from.x - to.x) / to.width,
                y: (from.y - to.y) / to.height,
                width: from.width / to.width,
                height: from.height / to.height,
            },
        }
    }
}

pub(in crate::host::linux_shell) fn position_images(
    scene: &mut crate::graphics::render::RenderScene,
    inputs: &[SnapshotInput],
) {
    for (i, input) in inputs.iter().enumerate() {
        let node = crate::graphics::scene::NodeId::new(i as u32 + 1, 1);
        let mut image = *scene.images.get(node).expect("snapshot image");
        image.rect = crate::foundation::RectF {
            x: input.target.x * scene.extent.width,
            y: input.target.y * scene.extent.height,
            width: input.target.width * scene.extent.width,
            height: input.target.height * scene.extent.height,
        };
        image.view_bounds = image.rect;
        scene.images.upsert(node, image);
    }
}
