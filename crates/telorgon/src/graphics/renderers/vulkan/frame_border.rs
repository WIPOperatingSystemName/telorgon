use super::{
    VulkanCompositePlacement, VulkanCompositeScene, VulkanDevice, VulkanFrameContext,
    VulkanMaterializationTarget, VulkanScene,
};
use crate::foundation::{ColorRgba8, RectI, SizeI};
use crate::graphics::render::{
    BoxInstance, ImageAlphaMode, RenderBackend, RenderRequest, RenderResult, RenderStats,
    TargetLoad, TargetStore, frame_border,
};

pub(super) struct IsolatedFrame {
    epoch: u64,
    extent: SizeI,
    border: BoxInstance,
    pub output: VulkanScene,
    interior: VulkanMaterializationTarget,
    resolved: VulkanMaterializationTarget,
}

impl VulkanDevice {
    pub(super) fn prepare_frame_border(
        &self,
        source: &mut VulkanScene,
        extent: SizeI,
        frame: &mut VulkanFrameContext<'_>,
    ) -> RenderResult<RenderStats> {
        let Some(border) = source.frame_border.clone() else {
            return Ok(RenderStats::default());
        };
        if extent.width <= 0 || extent.height <= 0 {
            return Ok(RenderStats::default());
        }
        if source.isolated_frame.as_ref().is_some_and(|cached| {
            cached.epoch == source.epoch && cached.extent == extent && cached.border == border
        }) {
            return Ok(RenderStats::default());
        }
        let (previous_interior, previous_resolved) =
            source.isolated_frame.take().map_or((None, None), |cached| {
                let IsolatedFrame {
                    output,
                    interior,
                    resolved,
                    ..
                } = *cached;
                drop(output);
                (Some(interior), Some(resolved))
            });
        let reuse = |previous: Option<VulkanMaterializationTarget>| match previous
            .filter(|target| target.extent() == extent && target.can_recycle())
        {
            Some(target) => Ok(target),
            None => VulkanMaterializationTarget::new_traced(self, extent, &mut |_| {}),
        };
        let mut interior = reuse(previous_interior)?;
        let mut resolved = reuse(previous_resolved)?;
        frame
            .core
            .images
            .extend([interior.image(), resolved.image()]);
        let placement = [VulkanCompositePlacement {
            scene_index: 0,
            target: RectI {
                x: 0,
                y: 0,
                width: extent.width,
                height: extent.height,
            },
            clip: None,
            rounded_clips: [None; 2],
        }];
        let request = RenderRequest {
            force: true,
            load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 0)),
            store: TargetStore::Store,
            region: None,
        };
        // Normalize shared edge coverage while composing children; apply it only once below.
        source.coverage_normalization = Some(frame_border::inner(&border));
        source.mark_frame_background(border.node);
        let painted = self.render_composite_direct(
            &mut [VulkanCompositeScene { scene: source }],
            &placement,
            &mut VulkanFrameContext {
                core: &mut *frame.core,
            },
            &interior.target(),
            &request,
        );
        source.coverage_normalization = None;
        let mut stats = painted?;
        interior.mark_initialized();

        let mut resolve = self.create_scene()?;
        resolve.bind_materialized_image(
            frame_border::IMAGE,
            &interior,
            ImageAlphaMode::Premultiplied,
        )?;
        self.apply_scene_delta(
            &mut resolve,
            &frame_border::image_scene(source.extent)
                .take_delta()
                .unwrap(),
        )?;
        let mut ring = self.create_scene()?;
        self.apply_scene_delta(
            &mut ring,
            &frame_border::border_scene(source.extent, &border)
                .take_delta()
                .unwrap(),
        )?;
        let sx = extent.width as f32 / source.extent.width;
        let sy = extent.height as f32 / source.extent.height;
        let clips = frame_border::resolve_clips(&border).map(|clip| {
            clip.map(|mut c| {
                c.rect.x *= sx;
                c.rect.y *= sy;
                c.rect.width *= sx;
                c.rect.height *= sy;
                c.radii.top_left *= sx.min(sy);
                c.radii.top_right *= sx.min(sy);
                c.radii.bottom_left *= sx.min(sy);
                c.radii.bottom_right *= sx.min(sy);
                c
            })
        });
        let resolve_placements = [
            VulkanCompositePlacement {
                rounded_clips: clips,
                ..placement[0]
            },
            VulkanCompositePlacement {
                scene_index: 1,
                ..placement[0]
            },
        ];
        let resolve_stats = self.render_composite_direct(
            &mut [
                VulkanCompositeScene {
                    scene: &mut resolve,
                },
                VulkanCompositeScene { scene: &mut ring },
            ],
            &resolve_placements,
            &mut VulkanFrameContext {
                core: &mut *frame.core,
            },
            &resolved.target(),
            &request,
        )?;
        accumulate(&mut stats, resolve_stats);
        resolved.mark_initialized();
        let mut output = self.create_scene()?;
        output.bind_materialized_image(
            frame_border::IMAGE,
            &resolved,
            ImageAlphaMode::Premultiplied,
        )?;
        self.apply_scene_delta(
            &mut output,
            &frame_border::image_scene(source.extent)
                .take_delta()
                .unwrap(),
        )?;
        source.isolated_frame = Some(Box::new(IsolatedFrame {
            epoch: source.epoch,
            extent,
            border,
            output,
            interior,
            resolved,
        }));
        Ok(stats)
    }
}

pub(super) fn accumulate(total: &mut RenderStats, next: RenderStats) {
    total.recorded |= next.recorded;
    total.epoch = total.epoch.max(next.epoch);
    total.upload_bytes_recorded += next.upload_bytes_recorded;
    total.buffer_copies += next.buffer_copies;
    total.buffer_allocations += next.buffer_allocations;
    total.descriptor_writes += next.descriptor_writes;
    total.passes += next.passes;
    total.barriers += next.barriers;
    total.batches += next.batches;
    total.draws += next.draws;
    total.dispatches += next.dispatches;
    total.damage_area += next.damage_area;
}
