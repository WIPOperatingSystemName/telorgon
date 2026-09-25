//! Isolate chrome contents before partitioning coverage with their enclosing border.

use std::sync::Arc;

use super::*;
use crate::foundation::{ColorRgba8, RectF, SizeF};
use crate::graphics::scene::NodeId;

pub(crate) const IMAGE: ImageId = ImageId(u32::MAX - 3);

pub(crate) fn inner(border: &BoxInstance) -> RoundedClip {
    RoundedClip::new(border.rect, border.corner_radii).inset(border.border)
}

pub(crate) fn has_border(border: &BoxInstance) -> bool {
    [
        border.border.top,
        border.border.right,
        border.border.bottom,
        border.border.left,
    ]
    .iter()
    .any(|side| side.width > 0.0 && side.color.a > 0 && border.opacity > 0.0)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct InteriorPaint {
    pub contour: RoundedClip,
    pub root: NodeId,
    pub protect_border: bool,
}

impl InteriorPaint {
    pub fn new(border: &BoxInstance) -> Self {
        Self {
            contour: inner(border),
            root: border.node,
            protect_border: has_border(border),
        }
    }

    pub fn coverage(self, point: crate::foundation::PointF) -> f32 {
        if !self.protect_border {
            return 1.0;
        }
        // Keep descendant paint out of every pixel touched by the border's AA band.
        // This contour is in raster pixels; layout and input bounds stay unchanged.
        self.contour
            .inset(Border::all(1.0, ColorRgba8::rgba(0, 0, 0, 0)))
            .coverage(point)
    }
}

pub(crate) fn prepare_interior(delta: &mut RenderSceneDelta, border: &BoxInstance) {
    for patch in &mut delta.boxes {
        for instance in Arc::make_mut(&mut patch.values) {
            if instance.node == border.node {
                // Layout still reserves the border. Only its paint moves to the resolve pass.
                instance.border = Border::default();
                instance.corner_radii = Default::default();
                instance.shadows = Default::default();
                instance.outline = Default::default();
            }
        }
    }
}

pub(crate) fn image_scene(extent: SizeF) -> RenderScene {
    let mut scene = RenderScene::default();
    scene.extent = extent;
    scene.background = ColorRgba8::rgba(0, 0, 0, 0);
    let node = NodeId::new(0, 1);
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: extent.width,
        height: extent.height,
    };
    scene.images.upsert(
        node,
        ImageInstance {
            node,
            image: IMAGE,
            tint: None,
            rect,
            view_bounds: rect,
            content_version: 1,
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(0),
        },
    );
    scene.set_draw_order(vec![DrawItem {
        kind: PrimitiveKind::Image,
        index: 0,
        batch: BatchKey {
            pipeline: PipelineKind::Image,
            resource: IMAGE.0,
            clip: ClipId(0),
            blend: BlendMode::Alpha,
            target: 0,
        },
    }]);
    scene
}

pub(crate) fn border_scene(extent: SizeF, border: &BoxInstance) -> RenderScene {
    let mut scene = RenderScene::default();
    scene.extent = extent;
    scene.background = ColorRgba8::rgba(0, 0, 0, 0);
    let mut ring = border.clone();
    ring.node = NodeId::new(2, 1);
    ring.background = None;
    ring.shadows = Default::default();
    ring.outline = Default::default();
    ring.clip = ClipId(0);
    ring.spatial = SpatialId(0);
    scene.boxes.upsert(ring.node, ring);
    scene.set_draw_order(vec![DrawItem {
        kind: PrimitiveKind::Box,
        index: 0,
        // These are disjoint subpixel areas, not two source-over layers.
        batch: BatchKey {
            pipeline: PipelineKind::AnalyticBox,
            resource: 0,
            clip: ClipId(0),
            blend: BlendMode::Add,
            target: 0,
        },
    }]);
    scene
}

pub(crate) fn resolve_clips(border: &BoxInstance) -> [Option<RoundedClip>; 2] {
    [
        Some(RoundedClip::new(border.rect, border.corner_radii)),
        Some(inner(border)),
    ]
}
