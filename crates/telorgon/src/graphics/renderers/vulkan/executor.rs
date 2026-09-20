use std::collections::BTreeSet;
use std::sync::Arc;

use crate::foundation::{ColorRgba8, RectF, RectI, SizeF};
use crate::graphics::gpu_abi::GpuView;
use crate::graphics::render::{
    AlphaMode, ColorSpace, ImageResourceDelta, MaterialResourceDelta, PrimitiveKind, RangePatch,
    RenderBackend, RenderError, RenderErrorKind, RenderRequest, RenderResult, RenderSceneDelta,
    RenderStats, SceneUpdateStats, TargetLoad, TargetStore,
};
use ash::vk::{self, Handle};

use crate::graphics::renderers::vulkan::buffer::AllocatedBuffer;
use crate::graphics::renderers::vulkan::driver_workarounds::DriverWorkarounds;
use crate::graphics::renderers::vulkan::error::{invalid_scene, unsupported};
use crate::graphics::renderers::vulkan::external_image::{
    ExternalImageInner, VulkanExternalAcquire, VulkanExternalRelease,
};
#[cfg(feature = "instrumentation")]
use crate::graphics::renderers::vulkan::frame::{
    PROFILER_TIMESTAMP_RENDER_BEGIN, PROFILER_TIMESTAMP_RENDER_END, PROFILER_TIMESTAMP_UPLOAD_END,
};
use crate::graphics::renderers::vulkan::scene::{DrawBatch, validate_draw_order, validate_texture_count};
use crate::graphics::renderers::vulkan::sync::{
    after_storage_upload, before_storage_upload, color_to_final, fragment_sampled_state,
    image_transition, target_to_color,
};
use crate::graphics::renderers::vulkan::upload::StagedUploads;
use crate::graphics::renderers::vulkan::{VulkanDevice, VulkanFrameContext, VulkanScene, VulkanTarget};

impl RenderBackend for VulkanDevice {
    type Scene = VulkanScene;
    type FrameContext<'frame> = VulkanFrameContext<'frame>;
    type Target<'frame> = VulkanTarget<'frame>;

    fn create_scene(&self) -> RenderResult<Self::Scene> {
        Ok(VulkanScene::new(self.inner.id))
    }

    fn apply_scene_delta(
        &self,
        scene: &mut Self::Scene,
        delta: &RenderSceneDelta,
    ) -> RenderResult<SceneUpdateStats> {
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("delta.apply");
        if scene.device_id != self.inner.id {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "Vulkan scene belongs to another logical device",
            ));
        }
        scene.apply_delta_checked(delta)
    }

    fn render<'frame>(
        &self,
        scene: &mut Self::Scene,
        frame: &mut Self::FrameContext<'frame>,
        target: &Self::Target<'frame>,
        request: &RenderRequest,
    ) -> RenderResult<RenderStats> {
        #[cfg(feature = "instrumentation")]
        let _frame_span = crate::runtime::instrumentation::span!("vulkan.frame");
        if frame.core.device.inner.id != self.inner.id
            || target.device_id != self.inner.id
            || scene.device_id != self.inner.id
        {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "Vulkan scene, frame, target, and backend must belong to one device",
            ));
        }
        if frame.core.rendered {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "Vulkan records one target per owned frame",
            ));
        }
        // The target region is the viewport that maps the logical scene into the target. A
        // request region is only a damage/render clip inside that viewport. Treating damage as
        // the viewport scales the whole scene into the damaged rectangle (and, for a compositor,
        // produces an apparent desktop-within-a-window feedback image).
        let target_region = target.info.region;
        let render_region = request.region.unwrap_or(target_region);
        validate_render_region(render_region, target)?;
        if target.info.sample_count != 1 {
            return Err(unsupported("Vulkan supports one sample per pixel"));
        }
        if !matches!(
            target.info.color_space,
            ColorSpace::Linear | ColorSpace::Srgb
        ) {
            return Err(unsupported(
                "Vulkan supports linear and hardware sRGB targets only",
            ));
        }
        if matches!(request.load, TargetLoad::Preserve)
            && target.initial_state.layout == vk::ImageLayout::UNDEFINED
        {
            return Err(RenderError::new(
                RenderErrorKind::InvalidTarget,
                "cannot preserve an uninitialized Vulkan target",
            ));
        }
        validate_retained_scene(scene)?;

        let external_start = frame.core.external_images.len();
        let external_barriers = {
            #[cfg(feature = "instrumentation")]
            let _span = crate::runtime::instrumentation::span!("barriers.record");
            begin_external_image_uses(frame.core, scene)?
        };

        let plan = {
            #[cfg(feature = "instrumentation")]
            let _span = crate::runtime::instrumentation::span!("uploads.plan");
            scene.prepare_uploads(&self.inner)?
        };
        let buffer_allocations = plan.buffer_allocations;
        let buffer_growths = plan.buffer_growths;
        let view_mapping = ViewMapping::new(scene.extent, target_region);
        let view = gpu_view(scene, target, view_mapping);
        let staged = {
            #[cfg(feature = "instrumentation")]
            let _span = crate::runtime::instrumentation::span!("uploads.stage");
            let staged = StagedUploads::build(&view, plan, frame.core.staging.size())?;
            frame.core.staging.write(&staged.bytes)?;
            staged
        };
        frame.core.staging_bytes_used = staged.bytes.len();
        let descriptor_writes = {
            #[cfg(feature = "instrumentation")]
            let _span = crate::runtime::instrumentation::span!("descriptors.bind");
            bind_scene_descriptors(&self.inner.raw, self.inner.sampler, frame.core, scene)
        };
        let (buffer_copies, upload_barriers) = {
            #[cfg(feature = "instrumentation")]
            let _span = crate::runtime::instrumentation::span!("uploads.record");
            record_uploads(
                &self.inner.raw,
                frame.core.command_buffer,
                frame.core.staging.raw(),
                &staged,
                self.inner.driver_workarounds,
            )
        };
        frame.core.images.extend(staged.retained_images());
        #[cfg(feature = "instrumentation")]
        frame.core.write_profiler_timestamp(
            PROFILER_TIMESTAMP_UPLOAD_END,
            vk::PipelineStageFlags2::TRANSFER,
        );

        if frame.core.external_images.len() > external_start {
            let sampled = fragment_sampled_state();
            let barriers = frame
                .core
                .external_images
                .iter()
                .skip(external_start)
                .map(|image| {
                    let mut initial = image.initial_use.state();
                    if matches!(image.acquire, VulkanExternalAcquire::BinarySemaphore(_))
                        || image.initial_queue_family != vk::QUEUE_FAMILY_IGNORED
                    {
                        // The semaphore wait supplies the cross-submission memory dependency. The
                        // foreign-family case likewise has no Vulkan source scope in this queue.
                        // The barrier owns the layout/ownership transition and destination access.
                        initial.stage = vk::PipelineStageFlags2::NONE;
                        initial.access = vk::AccessFlags2::NONE;
                    }
                    let mut barrier = image_transition(image.image, initial, sampled);
                    if image.initial_queue_family != vk::QUEUE_FAMILY_IGNORED {
                        barrier.src_queue_family_index = image.initial_queue_family;
                        barrier.dst_queue_family_index = frame.core.device.inner.queue_family;
                    }
                    barrier
                })
                .collect::<Vec<_>>();
            unsafe {
                self.inner.raw.cmd_pipeline_barrier2(
                    frame.core.command_buffer,
                    &vk::DependencyInfo::default().image_memory_barriers(&barriers),
                );
            }
        }

        let mut initial_barrier = target_to_color(target.image, target.initial_state);
        if target.initial_queue_family != vk::QUEUE_FAMILY_IGNORED {
            initial_barrier.src_queue_family_index = target.initial_queue_family;
            initial_barrier.dst_queue_family_index = self.inner.queue_family;
        }
        unsafe {
            self.inner.raw.cmd_pipeline_barrier2(
                frame.core.command_buffer,
                &vk::DependencyInfo::default().image_memory_barriers(&[initial_barrier]),
            );
        }
        let (load_op, clear) = match request.load {
            TargetLoad::Preserve => (vk::AttachmentLoadOp::LOAD, vk::ClearValue::default()),
            TargetLoad::Clear(color) => (
                vk::AttachmentLoadOp::CLEAR,
                vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: linear_clear(color),
                    },
                },
            ),
        };
        let store_op = match request.store {
            TargetStore::Store => vk::AttachmentStoreOp::STORE,
            TargetStore::Discard => vk::AttachmentStoreOp::DONT_CARE,
        };
        let attachment = vk::RenderingAttachmentInfo::default()
            .image_view(target.view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(load_op)
            .store_op(store_op)
            .clear_value(clear);
        let render_area = rect2d(render_region);
        #[cfg(feature = "instrumentation")]
        frame.core.write_profiler_timestamp(
            PROFILER_TIMESTAMP_RENDER_BEGIN,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
        );
        unsafe {
            self.inner.raw.cmd_begin_rendering(
                frame.core.command_buffer,
                &vk::RenderingInfo::default()
                    .render_area(render_area)
                    .layer_count(1)
                    .color_attachments(&[attachment]),
            );
            self.inner.raw.cmd_bind_descriptor_sets(
                frame.core.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.inner.layouts.pipeline,
                0,
                &[
                    frame.core.descriptor_sets.view,
                    frame.core.descriptor_sets.scene,
                ],
                &[],
            );
            self.inner.raw.cmd_set_viewport(
                frame.core.command_buffer,
                0,
                &[vk::Viewport {
                    x: target_region.x as f32,
                    y: target_region.y as f32,
                    width: target_region.width as f32,
                    height: target_region.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
        }
        let mut bound_pipeline = None;
        let mut bound_primitive = None;
        let mut bound_texture = None;
        #[cfg(feature = "instrumentation")]
        let draws_span = crate::runtime::instrumentation::span!("draws.record");
        for batch in &scene.batches {
            let scissor = batch_scissor(scene, batch, view_mapping, render_region);
            if scissor.extent.width == 0 || scissor.extent.height == 0 {
                continue;
            }
            let pipeline = self.pipeline(target.format, batch.key.pipeline, batch.key.blend)?;
            let primitive_index = primitive_index(batch.kind);
            unsafe {
                if bound_pipeline != Some(pipeline) {
                    self.inner.raw.cmd_bind_pipeline(
                        frame.core.command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        pipeline,
                    );
                    bound_pipeline = Some(pipeline);
                }
                if bound_primitive != Some(primitive_index) {
                    self.inner.raw.cmd_bind_descriptor_sets(
                        frame.core.command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.inner.layouts.pipeline,
                        2,
                        &[frame.core.descriptor_sets.primitives[primitive_index]],
                        &[],
                    );
                    bound_primitive = Some(primitive_index);
                }
                if let Some(texture_slot) = scene.texture_slot(batch)
                    && bound_texture != Some(texture_slot)
                {
                    self.inner.raw.cmd_bind_descriptor_sets(
                        frame.core.command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.inner.layouts.pipeline,
                        3,
                        &[frame.core.descriptor_sets.textures[texture_slot]],
                        &[],
                    );
                    bound_texture = Some(texture_slot);
                }
                self.inner
                    .raw
                    .cmd_set_scissor(frame.core.command_buffer, 0, &[scissor]);
                self.inner.raw.cmd_draw(
                    frame.core.command_buffer,
                    4,
                    batch.instance_count,
                    0,
                    batch.first_instance,
                );
            }
        }
        #[cfg(feature = "instrumentation")]
        drop(draws_span);
        unsafe {
            self.inner.raw.cmd_end_rendering(frame.core.command_buffer);
            #[cfg(feature = "instrumentation")]
            frame.core.write_profiler_timestamp(
                PROFILER_TIMESTAMP_RENDER_END,
                vk::PipelineStageFlags2::ALL_GRAPHICS,
            );
            if frame.core.external_images.len() > external_start {
                let sampled = fragment_sampled_state();
                let barriers = frame
                    .core
                    .external_images
                    .iter()
                    .skip(external_start)
                    .map(|image| {
                        let mut final_state = image.final_use.state();
                        if matches!(image.release, VulkanExternalRelease::BinarySemaphore(_))
                            || image.final_queue_family != vk::QUEUE_FAMILY_IGNORED
                        {
                            // The returned semaphore signal publishes all prior reads; the host's
                            // next wait supplies its destination scope. Foreign-family release has
                            // no destination execution scope in this queue either.
                            final_state.stage = vk::PipelineStageFlags2::NONE;
                            final_state.access = vk::AccessFlags2::NONE;
                        }
                        let mut barrier = image_transition(image.image, sampled, final_state);
                        if image.final_queue_family != vk::QUEUE_FAMILY_IGNORED {
                            barrier.src_queue_family_index = frame.core.device.inner.queue_family;
                            barrier.dst_queue_family_index = image.final_queue_family;
                        }
                        barrier
                    })
                    .collect::<Vec<_>>();
                self.inner.raw.cmd_pipeline_barrier2(
                    frame.core.command_buffer,
                    &vk::DependencyInfo::default().image_memory_barriers(&barriers),
                );
            }
            if target.final_state
                != crate::graphics::renderers::vulkan::target::VulkanImageState::COLOR_ATTACHMENT
            {
                let mut final_barrier = color_to_final(target.image, target.final_state);
                if target.final_queue_family != vk::QUEUE_FAMILY_IGNORED {
                    final_barrier.src_queue_family_index = self.inner.queue_family;
                    final_barrier.dst_queue_family_index = target.final_queue_family;
                }
                self.inner.raw.cmd_pipeline_barrier2(
                    frame.core.command_buffer,
                    &vk::DependencyInfo::default().image_memory_barriers(&[final_barrier]),
                );
            }
        }
        frame.core.buffers.extend(scene.retained_buffers());
        frame.core.images.extend(scene.retained_images());
        frame.core.rendered = true;
        scene.commit_uploads(
            staged.byte_count,
            buffer_copies,
            buffer_allocations,
            buffer_growths,
            descriptor_writes,
        );
        let draw_count = scene.batches.len() as u32;
        let stats = RenderStats {
            recorded: true,
            epoch: scene.epoch,
            upload_bytes_recorded: staged.byte_count,
            buffer_copies,
            buffer_allocations,
            descriptor_writes,
            passes: 1,
            barriers: upload_barriers
                + 1
                + external_barriers
                + u32::from(
                    target.final_state
                        != crate::graphics::renderers::vulkan::target::VulkanImageState::COLOR_ATTACHMENT,
                ),
            batches: draw_count,
            draws: draw_count,
            dispatches: 0,
            damage_area: render_region.width as f32 * render_region.height as f32,
        };
        #[cfg(feature = "instrumentation")]
        {
            crate::runtime::instrumentation::counter!("render.upload_bytes", stats.upload_bytes_recorded);
            crate::runtime::instrumentation::counter!("render.buffer_copies", stats.buffer_copies);
            crate::runtime::instrumentation::counter!("render.buffer_allocations", stats.buffer_allocations);
            crate::runtime::instrumentation::counter!("render.descriptor_writes", stats.descriptor_writes);
            crate::runtime::instrumentation::counter!("render.passes", stats.passes);
            crate::runtime::instrumentation::counter!("render.barriers", stats.barriers);
            crate::runtime::instrumentation::counter!("render.batches", stats.batches);
            crate::runtime::instrumentation::counter!("render.draws", stats.draws);
            crate::runtime::instrumentation::counter!("render.damage_area", stats.damage_area);
        }
        Ok(stats)
    }
}

impl VulkanScene {
    /// CPU-only retained-state admission, shared by the device entry point and regression tests.
    pub(crate) fn apply_delta_checked(
        &mut self,
        delta: &RenderSceneDelta,
    ) -> RenderResult<SceneUpdateStats> {
        if delta.epoch <= self.epoch {
            return Ok(SceneUpdateStats {
                epoch: self.epoch,
                ..SceneUpdateStats::default()
            });
        }
        validate_delta(self, delta)?;
        self.apply(delta);
        Ok(SceneUpdateStats {
            epoch: self.epoch,
            upload_bytes_queued: self.pending_upload_bytes(),
            descriptor_writes_queued: self.queued_descriptor_writes(),
        })
    }
}

fn validate_delta(scene: &VulkanScene, delta: &RenderSceneDelta) -> RenderResult<()> {
    validate_patch_ranges("boxes", &delta.boxes, delta.box_len)?;
    validate_patch_ranges("glyphs", &delta.glyphs, delta.glyph_len)?;
    validate_patch_ranges("images", &delta.images, delta.image_len)?;
    validate_patch_ranges("materials", &delta.materials, delta.material_len)?;
    validate_patch_ranges("clips", &delta.clips, delta.clip_len)?;
    validate_patch_ranges("spatial nodes", &delta.spatial_nodes, delta.spatial_len)?;
    validate_growth("boxes", scene.boxes.len(), delta.box_len, &delta.boxes)?;
    validate_growth("glyphs", scene.glyphs.len(), delta.glyph_len, &delta.glyphs)?;
    validate_growth("images", scene.images.len(), delta.image_len, &delta.images)?;
    validate_growth(
        "materials",
        scene.materials.len(),
        delta.material_len,
        &delta.materials,
    )?;
    validate_growth("clips", scene.clips.len(), delta.clip_len, &delta.clips)?;
    validate_growth(
        "spatial nodes",
        scene.spatial.len(),
        delta.spatial_len,
        &delta.spatial_nodes,
    )?;
    validate_resource_updates(delta)?;
    if delta.image_resources.iter().any(|update| match update {
        ImageResourceDelta::Write(value) => scene.has_external_image(value.image),
        ImageResourceDelta::Remove(id) => scene.has_external_image(*id),
    }) {
        return Err(invalid_scene(
            "one image ID cannot receive uploaded updates while bound to external content",
        ));
    }

    let boxes = patched_view(&scene.boxes, &delta.boxes, delta.box_len);
    let glyphs = patched_view(&scene.glyphs, &delta.glyphs, delta.glyph_len);
    let images = patched_view(&scene.images, &delta.images, delta.image_len);
    let materials = patched_view(&scene.materials, &delta.materials, delta.material_len);
    let clips = patched_view(&scene.clips, &delta.clips, delta.clip_len);
    let spatial = patched_view(&scene.spatial, &delta.spatial_nodes, delta.spatial_len);
    let draw_order = delta.draw_order.as_deref().unwrap_or(&scene.draw_order);
    validate_draw_order(draw_order).map_err(invalid_scene)?;
    validate_texture_count(draw_order).map_err(unsupported)?;
    let spatial_ids = spatial
        .iter()
        .map(|item| item.id.0)
        .collect::<BTreeSet<_>>();
    let clip_ids = clips.iter().map(|item| item.id.0).collect::<BTreeSet<_>>();
    if spatial_ids.iter().copied().max().unwrap_or(0) > 1_000_000
        || clip_ids.iter().copied().max().unwrap_or(0) > 1_000_000
    {
        return Err(unsupported(
            "Vulkan spatial or clip ID exceeds the retained-table limit",
        ));
    }
    let valid_refs = |spatial_id: u32, clip_id: u32| {
        (spatial_id == 0 || spatial_ids.contains(&spatial_id))
            && (clip_id == 0 || clip_ids.contains(&clip_id))
    };
    if boxes
        .iter()
        .any(|item| !valid_refs(item.spatial.0, item.clip.0))
        || glyphs
            .iter()
            .any(|item| !valid_refs(item.spatial.0, item.clip.0))
        || images
            .iter()
            .any(|item| !valid_refs(item.spatial.0, item.clip.0))
        || materials
            .iter()
            .any(|item| !valid_refs(item.spatial.0, item.clip.0))
    {
        return Err(invalid_scene(
            "Vulkan primitive references a missing spatial or clip node",
        ));
    }
    let mut image_resources = scene.image_resource_ids();
    for update in &delta.image_resources {
        match update {
            ImageResourceDelta::Write(value) => {
                image_resources.insert(value.image.0);
            }
            ImageResourceDelta::Remove(id) => {
                image_resources.remove(&id.0);
            }
        }
    }
    let mut material_resources = scene.material_resources.clone();
    for update in &delta.material_resources {
        match update {
            MaterialResourceDelta::Upsert(value) => {
                material_resources.insert(value.material.0, *value);
            }
            MaterialResourceDelta::Remove(id) => {
                material_resources.remove(&id.0);
            }
        }
    }
    for draw in draw_order {
        match draw.kind {
            PrimitiveKind::Box if draw.index as usize >= boxes.len() => {
                return Err(invalid_scene("Vulkan draw index references a missing box"));
            }
            PrimitiveKind::Glyph if draw.index as usize >= glyphs.len() => {
                return Err(invalid_scene(
                    "Vulkan draw index references a missing glyph",
                ));
            }
            PrimitiveKind::Image if draw.index as usize >= images.len() => {
                return Err(invalid_scene(
                    "Vulkan draw index references a missing image",
                ));
            }
            PrimitiveKind::Material if draw.index as usize >= materials.len() => {
                return Err(invalid_scene(
                    "Vulkan draw index references a missing material",
                ));
            }
            _ => {}
        }
        let instance_clip = match draw.kind {
            PrimitiveKind::Box => boxes[draw.index as usize].clip,
            PrimitiveKind::Glyph => glyphs[draw.index as usize].clip,
            PrimitiveKind::Image => images[draw.index as usize].clip,
            PrimitiveKind::Material => materials[draw.index as usize].clip,
        };
        if draw.batch.clip != instance_clip {
            return Err(invalid_scene(
                "Vulkan draw batch clip does not match its primitive clip",
            ));
        }
        if draw.kind == PrimitiveKind::Image {
            let instance = &images[draw.index as usize];
            if draw.batch.resource != instance.image.0
                || !image_resources.contains(&instance.image.0)
            {
                return Err(invalid_scene(
                    "Vulkan image draw references a missing or mismatched image resource",
                ));
            }
            if let Some(content_version) = scene.external_image_content_version(instance.image)
                && content_version != instance.content_version
            {
                return Err(invalid_scene(
                    "Vulkan external image content version does not match its image instance",
                ));
            }
        }
        if draw.kind == PrimitiveKind::Material {
            let instance = &materials[draw.index as usize];
            if draw.batch.resource != instance.material.0
                || !material_resources.contains_key(&instance.material.0)
            {
                return Err(invalid_scene(
                    "Vulkan material draw references a missing or mismatched material resource",
                ));
            }
            let resource = material_resources[&instance.material.0];
            match resource.kind {
                crate::graphics::render::MaterialKind::LiquidGlass(parameters) => {
                    if draw.batch.pipeline != crate::graphics::render::PipelineKind::LiquidGlass
                        || !parameters.valid()
                        || !image_resources.contains(&parameters.backdrop.0)
                        || scene.has_external_image(parameters.backdrop)
                        || !image_resources.contains(&parameters.sharp_backdrop.0)
                        || scene.has_external_image(parameters.sharp_backdrop)
                    {
                        return Err(invalid_scene(
                            "liquid glass needs finite parameters, its dedicated pipeline, and an owned backdrop",
                        ));
                    }
                }
                crate::graphics::render::MaterialKind::GaussianBlur(parameters) => {
                    if draw.batch.pipeline != crate::graphics::render::PipelineKind::GaussianBlur
                        || !parameters.valid()
                        || !image_resources.contains(&parameters.source.0)
                        || scene.has_external_image(parameters.source)
                    {
                        return Err(invalid_scene(
                            "Gaussian blur requires valid parameters, its own pipeline and an owned source",
                        ));
                    }
                }
                _ if draw.batch.pipeline == crate::graphics::render::PipelineKind::GaussianBlur => {
                    return Err(invalid_scene(
                        "Gaussian blur pipeline requires Gaussian blur parameters",
                    ));
                }
                _ if draw.batch.pipeline == crate::graphics::render::PipelineKind::LiquidGlass => {
                    return Err(invalid_scene(
                        "liquid glass pipeline requires liquid glass parameters",
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn validate_resource_updates(delta: &RenderSceneDelta) -> RenderResult<()> {
    for update in &delta.image_resources {
        if let ImageResourceDelta::Write(update) = update {
            let rect = update.rect;
            if update.extent.width <= 0
                || update.extent.height <= 0
                || rect.x < 0
                || rect.y < 0
                || rect.width <= 0
                || rect.height <= 0
                || rect.right() > update.extent.width
                || rect.bottom() > update.extent.height
                || update.row_bytes < rect.width as usize * 4
                || !update.row_bytes.is_multiple_of(4)
                || update.pixels.len() < update.row_bytes.saturating_mul(rect.height as usize)
            {
                return Err(invalid_scene(
                    "Vulkan image resource update has invalid geometry, stride, or payload",
                ));
            }
        }
    }
    if delta.atlas_extent.width <= 0 || delta.atlas_extent.height <= 0 {
        return Err(invalid_scene("Vulkan glyph atlas extent must be positive"));
    }
    for page in &delta.atlas_pages {
        if page.x < 0
            || page.y < 0
            || page.width <= 0
            || page.height <= 0
            || page.x + page.width > delta.atlas_extent.width
            || page.y + page.height > delta.atlas_extent.height
            || page.pixels_a8.len() < page.width as usize * page.height as usize
        {
            return Err(invalid_scene(
                "Vulkan glyph atlas update is outside its atlas or has a short payload",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_retained_scene(scene: &VulkanScene) -> RenderResult<()> {
    for draw in &scene.draw_order {
        let spatial_clip = match draw.kind {
            PrimitiveKind::Box => scene
                .boxes
                .get(draw.index as usize)
                .map(|item| (item.spatial.0, item.clip.0)),
            PrimitiveKind::Glyph => scene
                .glyphs
                .get(draw.index as usize)
                .map(|item| (item.spatial.0, item.clip.0)),
            PrimitiveKind::Image => scene
                .images
                .get(draw.index as usize)
                .map(|item| (item.spatial.0, item.clip.0)),
            PrimitiveKind::Material => scene
                .materials
                .get(draw.index as usize)
                .map(|item| (item.spatial.0, item.clip.0)),
        }
        .ok_or_else(|| invalid_scene("Vulkan retained draw index is out of range"))?;
        if spatial_clip.0 as usize >= scene.gpu_spatial.len()
            || (spatial_clip.1 != 0 && spatial_clip.1 as usize >= scene.gpu_clips.len())
        {
            return Err(invalid_scene(
                "Vulkan retained primitive references an unavailable spatial or clip slot",
            ));
        }
        if draw.kind == PrimitiveKind::Image {
            let instance = &scene.images[draw.index as usize];
            if !scene.image_resource_ids().contains(&instance.image.0) {
                return Err(invalid_scene(
                    "Vulkan retained image draw references an unavailable resource",
                ));
            }
            if let Some(content_version) = scene.external_image_content_version(instance.image)
                && content_version != instance.content_version
            {
                return Err(invalid_scene(format!(
                    "Vulkan retained external image content version is stale: image={} scene_epoch={} draw_version={} bound_version={content_version}",
                    instance.image.0, scene.epoch, instance.content_version,
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn begin_external_image_uses(
    frame: &mut crate::graphics::renderers::vulkan::frame::FrameCore,
    scene: &VulkanScene,
) -> RenderResult<u32> {
    let external = scene.retained_external_images();
    if external.is_empty() {
        return Ok(0);
    }
    if frame.device.hosted.is_none() && !frame.device.external_image_capabilities().linux_dma_buf {
        return Err(unsupported(
            "external image leases require hosted Vulkan or owned Linux DMA-BUF interop",
        ));
    }
    let mut waits = frame
        .external_images
        .iter()
        .filter_map(|image| match image.acquire {
            VulkanExternalAcquire::BinarySemaphore(semaphore) => Some(semaphore.as_raw()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut signals = frame
        .external_images
        .iter()
        .filter_map(|image| match image.release {
            VulkanExternalRelease::BinarySemaphore(semaphore) => Some(semaphore.as_raw()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for image in &external {
        if let VulkanExternalAcquire::BinarySemaphore(semaphore) = image.acquire
            && !waits.insert(semaphore.as_raw())
        {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "one binary acquire semaphore cannot be waited more than once in a host submit",
            ));
        }
        if let VulkanExternalRelease::BinarySemaphore(semaphore) = image.release
            && !signals.insert(semaphore.as_raw())
        {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "one binary release semaphore cannot be signaled more than once in a host submit",
            ));
        }
    }
    let mut begun: Vec<Arc<ExternalImageInner>> = Vec::with_capacity(external.len());
    for image in external {
        if let Err(error) = image.begin_use(frame.frame_id) {
            for prior in &begun {
                prior.cancel_use(frame.frame_id);
            }
            return Err(error);
        }
        begun.push(image);
    }
    let count = begun.len() as u32;
    frame.external_images.extend(begun);
    Ok(count.saturating_mul(2))
}

// Validation remains transactional: changed arrays are private copies, unchanged arrays
// are read directly from the retained scene. Shrink-only updates can borrow a prefix.
fn patched_view<'a, T: Clone>(
    retained: &'a [T],
    patches: &[RangePatch<T>],
    final_len: usize,
) -> std::borrow::Cow<'a, [T]> {
    if patches.is_empty() && final_len <= retained.len() {
        return std::borrow::Cow::Borrowed(&retained[..final_len]);
    }
    let mut values = retained.to_vec();
    apply_local_patches(&mut values, patches, final_len);
    std::borrow::Cow::Owned(values)
}

#[cfg(test)]
mod patched_view_tests;

fn apply_local_patches<T: Clone>(target: &mut Vec<T>, patches: &[RangePatch<T>], final_len: usize) {
    for patch in patches {
        let end = patch.start + patch.values.len();
        if end > target.len() {
            target.resize(end, patch.values[0].clone());
        }
        if !patch.values.is_empty() {
            target[patch.start..end].clone_from_slice(&patch.values);
        }
    }
    target.truncate(final_len);
}

fn validate_patch_ranges<T>(name: &str, patches: &[RangePatch<T>], len: usize) -> RenderResult<()> {
    let mut end = 0;
    for patch in patches {
        if patch.start < end || patch.start.saturating_add(patch.values.len()) > len {
            return Err(invalid_scene(format!(
                "Vulkan scene delta has invalid {name} patch ranges"
            )));
        }
        end = patch.start + patch.values.len();
    }
    Ok(())
}

fn validate_growth<T>(
    name: &str,
    old_len: usize,
    new_len: usize,
    patches: &[RangePatch<T>],
) -> RenderResult<()> {
    if new_len <= old_len {
        return Ok(());
    }
    let mut covered = old_len;
    for patch in patches {
        let end = patch.start + patch.values.len();
        if patch.start <= covered && end > covered {
            covered = end;
        }
    }
    if covered < new_len {
        return Err(invalid_scene(format!(
            "Vulkan {name} growth leaves uninitialized slots"
        )));
    }
    Ok(())
}

fn bind_scene_descriptors(
    device: &ash::Device,
    sampler: vk::Sampler,
    frame: &mut crate::graphics::renderers::vulkan::frame::FrameCore,
    scene: &VulkanScene,
) -> u32 {
    let (spatial, draw_indices) = scene.common_buffers();
    let (spatial_generation, draw_generation, box_generation, clip_generation) =
        scene.buffer_generations();
    let scene_changed = frame.descriptor_bindings.scene_id != scene.id;
    let mut desired = frame.descriptor_bindings;
    desired.scene_id = scene.id;
    let mut buffer_ops = Vec::<(vk::DescriptorSet, u32, vk::DescriptorBufferInfo)>::new();
    if scene_changed
        || desired.spatial != spatial.raw()
        || desired.spatial_generation != spatial_generation
    {
        buffer_ops.push((frame.descriptor_sets.scene, 0, buffer_info(spatial)));
        desired.spatial = spatial.raw();
        desired.spatial_generation = spatial_generation;
    }
    if scene_changed
        || desired.draw_indices != draw_indices.raw()
        || desired.draw_indices_generation != draw_generation
    {
        buffer_ops.push((frame.descriptor_sets.scene, 2, buffer_info(draw_indices)));
        desired.draw_indices = draw_indices.raw();
        desired.draw_indices_generation = draw_generation;
    }
    if let Some(clips) = scene.clip_buffer() {
        if scene_changed
            || desired.clips != clips.raw()
            || desired.clips_generation != clip_generation
        {
            buffer_ops.push((frame.descriptor_sets.scene, 1, buffer_info(clips)));
            desired.clips = clips.raw();
            desired.clips_generation = clip_generation;
        }
    } else if desired.clips != vk::Buffer::null() {
        buffer_ops.push((frame.descriptor_sets.scene, 1, buffer_info(&frame.staging)));
        desired.clips = vk::Buffer::null();
        desired.clips_generation = 0;
    }
    for kind in [
        PrimitiveKind::Box,
        PrimitiveKind::Glyph,
        PrimitiveKind::Image,
        PrimitiveKind::Material,
    ] {
        let index = primitive_index(kind);
        let Some(buffer) = scene.primitive_buffer(kind) else {
            continue;
        };
        let generation = scene.primitive_generation(kind);
        if scene_changed
            || desired.primitives[index].instances != buffer.raw()
            || desired.primitives[index].instances_generation != generation
        {
            buffer_ops.push((
                frame.descriptor_sets.primitives[index],
                0,
                buffer_info(buffer),
            ));
            desired.primitives[index].instances = buffer.raw();
            desired.primitives[index].instances_generation = generation;
            if kind == PrimitiveKind::Box {
                desired.boxes = buffer.raw();
                desired.boxes_generation = box_generation;
            }
        }
        if kind == PrimitiveKind::Material
            && let Some(parameters) = scene.material_parameter_buffer()
        {
            let generation = scene.material_parameter_generation();
            if scene_changed
                || desired.primitives[index].parameters != parameters.raw()
                || desired.primitives[index].parameters_generation != generation
            {
                buffer_ops.push((
                    frame.descriptor_sets.primitives[index],
                    1,
                    buffer_info(parameters),
                ));
                desired.primitives[index].parameters = parameters.raw();
                desired.primitives[index].parameters_generation = generation;
            }
        }
    }
    let mut image_ops = Vec::<(vk::DescriptorSet, u32, vk::DescriptorImageInfo)>::new();
    for slot in 0..scene.texture_count() {
        for binding in 0..2 {
            if let Some((view, generation)) = scene.texture(slot, binding)
                && (scene_changed
                    || desired.textures[slot].views[binding as usize] != view
                    || desired.textures[slot].generations[binding as usize] != generation)
            {
                image_ops.push((
                    frame.descriptor_sets.textures[slot],
                    binding,
                    vk::DescriptorImageInfo {
                        sampler,
                        image_view: view,
                        image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    },
                ));
                desired.textures[slot].views[binding as usize] = view;
                desired.textures[slot].generations[binding as usize] = generation;
            }
        }
    }
    let buffer_infos = buffer_ops
        .iter()
        .map(|(_, _, info)| [*info])
        .collect::<Vec<_>>();
    let image_infos = image_ops
        .iter()
        .map(|(_, _, info)| [*info])
        .collect::<Vec<_>>();
    let mut writes = Vec::with_capacity(buffer_ops.len() + image_ops.len());
    for ((set, binding, _), info) in buffer_ops.iter().zip(&buffer_infos) {
        writes.push(
            vk::WriteDescriptorSet::default()
                .dst_set(*set)
                .dst_binding(*binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(info),
        );
    }
    for ((set, binding, _), info) in image_ops.iter().zip(&image_infos) {
        writes.push(
            vk::WriteDescriptorSet::default()
                .dst_set(*set)
                .dst_binding(*binding)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(info),
        );
    }
    unsafe { device.update_descriptor_sets(&writes, &[]) };
    frame.descriptor_bindings = desired;
    writes.len() as u32
}

pub(crate) fn record_uploads(
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    staging: vk::Buffer,
    uploads: &StagedUploads,
    workarounds: DriverWorkarounds,
) -> (u32, u32) {
    if uploads.destinations.is_empty() && uploads.image_destinations.is_empty() {
        return (0, 0);
    }
    let before_buffers = uploads
        .destinations
        .iter()
        .map(|destination| {
            before_storage_upload(destination.buffer.raw(), destination.previously_initialized)
        })
        .collect::<Vec<_>>();
    let mut before_images = uploads
        .image_destinations
        .iter()
        .map(|destination| {
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(if destination.previously_initialized {
                    vk::PipelineStageFlags2::FRAGMENT_SHADER
                } else {
                    vk::PipelineStageFlags2::NONE
                })
                .src_access_mask(if destination.previously_initialized {
                    vk::AccessFlags2::SHADER_SAMPLED_READ
                } else {
                    vk::AccessFlags2::NONE
                })
                .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .old_layout(if destination.previously_initialized {
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
                } else {
                    vk::ImageLayout::UNDEFINED
                })
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .image(destination.image.raw())
                .subresource_range(color_range())
        })
        .collect::<Vec<_>>();
    before_images.extend(uploads.image_destinations.iter().filter_map(|destination| {
        destination.preserve_from.as_ref().map(|source| {
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .src_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
                .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .image(source.raw())
                .subresource_range(color_range())
        })
    }));
    unsafe {
        device.cmd_pipeline_barrier2(
            command_buffer,
            &vk::DependencyInfo::default()
                .buffer_memory_barriers(&before_buffers)
                .image_memory_barriers(&before_images),
        );
        for destination in &uploads.destinations {
            device.cmd_copy_buffer2(
                command_buffer,
                &vk::CopyBufferInfo2::default()
                    .src_buffer(staging)
                    .dst_buffer(destination.buffer.raw())
                    .regions(&destination.regions),
            );
        }
        for destination in &uploads.image_destinations {
            if let Some(source) = &destination.preserve_from {
                device.cmd_copy_image2(
                    command_buffer,
                    &vk::CopyImageInfo2::default()
                        .src_image(source.raw())
                        .src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .dst_image(destination.image.raw())
                        .dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .regions(&[vk::ImageCopy2::default()
                            .src_subresource(vk::ImageSubresourceLayers {
                                aspect_mask: vk::ImageAspectFlags::COLOR,
                                mip_level: 0,
                                base_array_layer: 0,
                                layer_count: 1,
                            })
                            .dst_subresource(vk::ImageSubresourceLayers {
                                aspect_mask: vk::ImageAspectFlags::COLOR,
                                mip_level: 0,
                                base_array_layer: 0,
                                layer_count: 1,
                            })
                            .extent(vk::Extent3D {
                                width: destination.image.extent.width,
                                height: destination.image.extent.height,
                                depth: 1,
                            })]),
                );
            }
            device.cmd_copy_buffer_to_image2(
                command_buffer,
                &vk::CopyBufferToImageInfo2::default()
                    .src_buffer(staging)
                    .dst_image(destination.image.raw())
                    .dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .regions(&destination.regions),
            );
        }
    }
    let after_buffers = uploads
        .destinations
        .iter()
        .map(|destination| after_storage_upload(destination.buffer.raw(), workarounds))
        .collect::<Vec<_>>();
    let mut after_images = uploads
        .image_destinations
        .iter()
        .map(|destination| {
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image(destination.image.raw())
                .subresource_range(color_range())
        })
        .collect::<Vec<_>>();
    after_images.extend(uploads.image_destinations.iter().filter_map(|destination| {
        destination.preserve_from.as_ref().map(|source| {
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_READ)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image(source.raw())
                .subresource_range(color_range())
        })
    }));
    unsafe {
        device.cmd_pipeline_barrier2(
            command_buffer,
            &vk::DependencyInfo::default()
                .buffer_memory_barriers(&after_buffers)
                .image_memory_barriers(&after_images),
        );
    }
    (uploads.copy_count(), 2)
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct ViewMapping {
    logical_extent: SizeF,
    target_region: RectI,
}

impl ViewMapping {
    pub(crate) fn new(logical_extent: SizeF, target_region: RectI) -> Self {
        Self {
            logical_extent: SizeF {
                width: logical_extent.width.max(1.0),
                height: logical_extent.height.max(1.0),
            },
            target_region,
        }
    }

    pub(crate) fn logical_bounds(self) -> RectF {
        RectF {
            x: 0.0,
            y: 0.0,
            width: self.logical_extent.width,
            height: self.logical_extent.height,
        }
    }

    pub(crate) fn logical_rect_to_scissor(self, rect: RectF) -> vk::Rect2D {
        let Some(rect) = rect.intersection(self.logical_bounds()) else {
            return empty_scissor(self.target_region);
        };
        let scale_x = self.target_region.width as f32 / self.logical_extent.width;
        let scale_y = self.target_region.height as f32 / self.logical_extent.height;
        let left = (self.target_region.x as f32 + rect.x * scale_x)
            .floor()
            .max(self.target_region.x as f32) as i32;
        let top = (self.target_region.y as f32 + rect.y * scale_y)
            .floor()
            .max(self.target_region.y as f32) as i32;
        let right = (self.target_region.x as f32 + rect.right() * scale_x)
            .ceil()
            .min(self.target_region.right() as f32) as i32;
        let bottom = (self.target_region.y as f32 + rect.bottom() * scale_y)
            .ceil()
            .min(self.target_region.bottom() as f32) as i32;
        vk::Rect2D {
            offset: vk::Offset2D { x: left, y: top },
            extent: vk::Extent2D {
                width: right.saturating_sub(left) as u32,
                height: bottom.saturating_sub(top) as u32,
            },
        }
    }
}

pub(crate) fn batch_scissor(
    scene: &VulkanScene,
    batch: &DrawBatch,
    mapping: ViewMapping,
    render_region: RectI,
) -> vk::Rect2D {
    let scene_scissor = if batch.key.clip.0 == 0 {
        rect2d(mapping.target_region)
    } else {
        let clipped = scene
            .clips
            .iter()
            .find(|clip| clip.id == batch.key.clip)
            .map(|clip| clip.rect);
        let Some(rect) = clipped else {
            return empty_scissor(render_region);
        };
        mapping.logical_rect_to_scissor(rect)
    };
    intersect_scissor(scene_scissor, render_region)
}

pub(crate) fn intersect_scissor(scissor: vk::Rect2D, region: RectI) -> vk::Rect2D {
    let left = scissor.offset.x.max(region.x);
    let top = scissor.offset.y.max(region.y);
    let right = scissor
        .offset
        .x
        .saturating_add(scissor.extent.width as i32)
        .min(region.right());
    let bottom = scissor
        .offset
        .y
        .saturating_add(scissor.extent.height as i32)
        .min(region.bottom());
    if right <= left || bottom <= top {
        return empty_scissor(region);
    }
    vk::Rect2D {
        offset: vk::Offset2D { x: left, y: top },
        extent: vk::Extent2D {
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        },
    }
}

fn empty_scissor(region: RectI) -> vk::Rect2D {
    vk::Rect2D {
        offset: vk::Offset2D {
            x: region.x,
            y: region.y,
        },
        extent: vk::Extent2D::default(),
    }
}

pub(crate) fn primitive_index(kind: PrimitiveKind) -> usize {
    match kind {
        PrimitiveKind::Box => 0,
        PrimitiveKind::Glyph => 1,
        PrimitiveKind::Image => 2,
        PrimitiveKind::Material => 3,
    }
}
pub(crate) fn buffer_info(buffer: &Arc<AllocatedBuffer>) -> vk::DescriptorBufferInfo {
    vk::DescriptorBufferInfo {
        buffer: buffer.raw(),
        offset: 0,
        range: buffer.size(),
    }
}
fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}
pub(crate) fn rect2d(region: RectI) -> vk::Rect2D {
    vk::Rect2D {
        offset: vk::Offset2D {
            x: region.x,
            y: region.y,
        },
        extent: vk::Extent2D {
            width: region.width as u32,
            height: region.height as u32,
        },
    }
}

fn validate_render_region(region: RectI, target: &VulkanTarget<'_>) -> RenderResult<()> {
    let target_region = target.info.region;
    if region.x < target_region.x
        || region.y < target_region.y
        || region.width <= 0
        || region.height <= 0
        || region.right() > target_region.right()
        || region.bottom() > target_region.bottom()
    {
        return Err(RenderError::new(
            RenderErrorKind::InvalidTarget,
            "Vulkan render region is outside the target region",
        ));
    }
    Ok(())
}
#[cfg(any(target_os = "linux", test))]
pub(crate) fn set_placement_clips(
    view: &mut GpuView,
    clips: [Option<crate::graphics::render::RoundedClip>; 2],
) {
    view.epoch_flags[3] &= !0b110;
    view.placement_clip_rects = [[0.0, 0.0, -1.0, -1.0]; 2];
    view.placement_clip_radii = [[0.0; 4]; 2];
    for (index, clip) in clips.into_iter().enumerate() {
        if let Some(clip) = clip {
            if clip.inverted {
                view.epoch_flags[3] |= 1 << (index + 1);
            }
            view.placement_clip_rects[index] =
                [clip.rect.x, clip.rect.y, clip.rect.width, clip.rect.height];
            view.placement_clip_radii[index] = [
                clip.radii.top_left,
                clip.radii.top_right,
                clip.radii.bottom_right,
                clip.radii.bottom_left,
            ];
        }
    }
}

#[cfg(test)]
mod clip_tests;

pub(crate) fn gpu_view(
    scene: &VulkanScene,
    target: &VulkanTarget<'_>,
    mapping: ViewMapping,
) -> GpuView {
    let width = mapping.logical_extent.width;
    let height = mapping.logical_extent.height;
    let region = mapping.target_region;
    GpuView {
        clip_from_view_0: [2.0 / width, 0.0, 0.0, -1.0],
        clip_from_view_1: [0.0, 2.0 / height, 0.0, -1.0],
        clip_from_view_2: [0.0, 0.0, 1.0, 0.0],
        clip_from_view_3: [0.0, 0.0, 0.0, 1.0],
        view_size_scale: [width, height, 1.0, 1.0],
        target_size_origin: [
            target.extent.width as f32,
            target.extent.height as f32,
            region.x as f32,
            region.y as f32,
        ],
        render_size_inverse: [
            region.width as f32,
            region.height as f32,
            1.0 / target.extent.width as f32,
            1.0 / target.extent.height as f32,
        ],
        epoch_flags: [
            scene.epoch as u32,
            (scene.epoch >> 32) as u32,
            match target.info.color_space {
                ColorSpace::Linear => 0,
                ColorSpace::Srgb => 1,
                ColorSpace::Extended | ColorSpace::BackendDefined => unreachable!(),
            },
            u32::from(target.info.alpha_mode == AlphaMode::Opaque),
        ],
        placement_clip_rects: [[0.0, 0.0, -1.0, -1.0]; 2],
        placement_clip_radii: [[0.0; 4]; 2],
    }
}
pub(crate) fn linear_clear(color: ColorRgba8) -> [f32; 4] {
    let alpha = color.a as f32 / 255.0;
    let decode = |byte: u8| {
        let value = byte as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    [
        decode(color.r) * alpha,
        decode(color.g) * alpha,
        decode(color.b) * alpha,
        alpha,
    ]
}

#[cfg(test)]
mod tests;
