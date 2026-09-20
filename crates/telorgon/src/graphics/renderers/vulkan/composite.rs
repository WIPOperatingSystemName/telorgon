use std::sync::Arc;

use ash::vk;

use crate::foundation::RectI;
use crate::graphics::gpu_abi::GpuView;
use crate::graphics::render::{
    BlendMode, ColorSpace, PrimitiveKind, RenderError, RenderErrorKind, RenderRequest,
    RenderResult, RenderStats, RoundedClip, TargetLoad, TargetStore,
};

use super::buffer::AllocatedBuffer;
use super::descriptor::{
    CompositeDescriptorSets, MAX_COMPOSITE_SCENES, MAX_COMPOSITE_TEXTURE_SETS,
};
use super::error::unsupported;
use super::executor::{
    ViewMapping, batch_scissor, begin_external_image_uses, buffer_info, gpu_view,
    intersect_scissor, linear_clear, primitive_index, record_uploads, rect2d, set_placement_clips,
    validate_retained_scene,
};
use super::external_image::{VulkanExternalAcquire, VulkanExternalRelease};
use super::scene::VulkanScene;
use super::sync::{color_to_final, fragment_sampled_state, image_transition, target_to_color};
use super::target::VulkanImageState;
use super::upload::{SceneUploadPlan, StagedUploads};
use super::{VulkanDevice, VulkanFrameContext, VulkanTarget};

#[cfg(feature = "instrumentation")]
use super::frame::{
    PROFILER_TIMESTAMP_RENDER_BEGIN, PROFILER_TIMESTAMP_RENDER_END, PROFILER_TIMESTAMP_UPLOAD_END,
};

pub struct VulkanCompositeScene<'scene> {
    pub scene: &'scene mut VulkanScene,
}

#[derive(Clone, Copy, Debug)]
pub struct VulkanCompositePlacement {
    pub scene_index: usize,
    pub target: RectI,
    pub clip: Option<RectI>,
    pub rounded_clips: [Option<RoundedClip>; 2],
}

fn placement_blend(blend: BlendMode, rounded: bool) -> BlendMode {
    if rounded && blend == BlendMode::Opaque {
        BlendMode::Alpha
    } else {
        blend
    }
}

struct PreparedPlacement {
    descriptors: CompositeDescriptorSets,
    staged: StagedUploads,
    mapping: ViewMapping,
    target: RectI,
    clip: Option<RectI>,
    rounded: bool,
    scene_index: usize,
}

#[derive(Default)]
struct SceneCommit {
    byte_count: u64,
    buffer_copies: u32,
    buffer_allocations: u32,
    buffer_growths: u32,
    descriptor_writes: u32,
}

impl VulkanDevice {
    /// Records ordered retained scenes into one Vulkan render pass within an owned frame.
    ///
    /// Scene resources remain independent (including glyph atlases), while placements share the
    /// output attachment and command stream. An owned frame may record multiple composite passes
    /// for distinct targets before submission. This is the direct compositor path used by the
    /// Linux desktop host; it never invokes or consumes the software renderer.
    pub fn render_composite<'frame>(
        &self,
        scenes: &mut [VulkanCompositeScene<'_>],
        placements: &[VulkanCompositePlacement],
        frame: &mut VulkanFrameContext<'frame>,
        target: &VulkanTarget<'frame>,
        request: &RenderRequest,
    ) -> RenderResult<RenderStats> {
        #[cfg(feature = "instrumentation")]
        let _frame_span = crate::runtime::instrumentation::span!("vulkan.composite.frame");
        if frame.core.device.inner.id != self.inner.id || target.device_id != self.inner.id {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "Vulkan composite frame, target, and backend must belong to one device",
            ));
        }
        if scenes
            .iter()
            .any(|scene| scene.scene.device_id != self.inner.id)
        {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "Vulkan composite scenes must belong to the recording device",
            ));
        }
        if placements.len() > MAX_COMPOSITE_SCENES as usize {
            return Err(unsupported("Vulkan desktop placement limit exceeded"));
        }
        if placements
            .iter()
            .any(|placement| placement.scene_index >= scenes.len())
        {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "Vulkan desktop placement references a missing scene",
            ));
        }
        if placements
            .iter()
            .flat_map(|p| p.rounded_clips)
            .flatten()
            .any(|c| !c.is_valid())
        {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "invalid rounded composite clip",
            ));
        }
        let total_texture_sets = placements
            .iter()
            .map(|placement| scenes[placement.scene_index].scene.texture_count())
            .sum::<usize>();
        if total_texture_sets > MAX_COMPOSITE_TEXTURE_SETS as usize {
            return Err(unsupported("Vulkan desktop texture-set limit exceeded"));
        }
        let target_region = target.info.region;
        let render_region = request.region.unwrap_or(target_region);
        validate_render_region(render_region, target_region)?;
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
        // Cull before uploads, descriptors, validation and staging; retain unseen scene deltas.
        let placements = placements
            .iter()
            .copied()
            .filter(|p| placement_intersects(*p, render_region))
            .collect::<Vec<_>>();
        let descriptor_pool = frame
            .core
            .composite_descriptor_pool
            .clone()
            .ok_or_else(|| {
                RenderError::new(
                    RenderErrorKind::HostContract,
                    "Vulkan composite rendering requires an owned frame",
                )
            })?;

        let mut used = vec![false; scenes.len()];
        for placement in &placements {
            used[placement.scene_index] = true;
        }
        let mut plans = Vec::with_capacity(scenes.len());
        let mut commits = Vec::with_capacity(scenes.len());
        let mut external_barriers = 0_u32;
        let external_start = frame.core.external_images.len();
        for (index, scene) in scenes.iter_mut().enumerate() {
            if used[index] {
                validate_retained_scene(scene.scene)?;
                external_barriers = external_barriers
                    .saturating_add(begin_external_image_uses(frame.core, scene.scene)?);
                let plan = scene.scene.prepare_uploads(&self.inner)?;
                commits.push(SceneCommit {
                    buffer_allocations: plan.buffer_allocations,
                    buffer_growths: plan.buffer_growths,
                    ..SceneCommit::default()
                });
                plans.push(Some(plan));
            } else {
                commits.push(SceneCommit::default());
                plans.push(None);
            }
        }

        // Composite passes use freshly allocated descriptor sets. Keeping one absolute staging
        // cursor likewise lets an owned command buffer populate several independent targets
        // before its final desktop pass without later CPU writes changing earlier commands.
        let staging_start = frame.core.staging_bytes_used;
        let mut staging_bytes = Vec::new();
        let mut prepared = Vec::with_capacity(placements.len());
        let texture_counts = placements
            .iter()
            .map(|placement| scenes[placement.scene_index].scene.texture_count())
            .collect::<Vec<_>>();
        let descriptor_groups = descriptor_pool
            .lock()
            .map_err(|_| super::error::internal("composite descriptor arena poisoned"))?
            .allocate(&self.inner.layouts, &texture_counts)?;
        for (placement, descriptors) in placements.iter().zip(descriptor_groups) {
            let scene = &scenes[placement.scene_index].scene;
            let mapping = ViewMapping::new(scene.extent, placement.target);
            let mut view = gpu_view(scene, target, mapping);
            set_placement_clips(&mut view, placement.rounded_clips);
            let plan = plans[placement.scene_index]
                .take()
                .unwrap_or_else(SceneUploadPlan::default);
            let staged = StagedUploads::append_at(
                &view,
                plan,
                frame.core.staging.size(),
                &mut staging_bytes,
                self.inner.uniform_buffer_offset_alignment as usize,
                staging_start,
            )?;
            let descriptor_writes = write_descriptors(
                &self.inner.raw,
                self.inner.sampler,
                &frame.core.staging,
                staged.view_offset,
                &descriptors,
                scene,
            );
            commits[placement.scene_index].descriptor_writes = commits[placement.scene_index]
                .descriptor_writes
                .saturating_add(descriptor_writes);
            prepared.push(PreparedPlacement {
                descriptors,
                staged,
                mapping,
                target: placement.target,
                clip: placement.clip,
                rounded: placement.rounded_clips.iter().any(Option::is_some),
                scene_index: placement.scene_index,
            });
        }
        if plans.iter().any(Option::is_some) {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "updated Vulkan desktop scene has no visible placement",
            ));
        }
        frame
            .core
            .staging
            .write_at(staging_start as u64, &staging_bytes)?;
        frame.core.staging_bytes_used = staging_start + staging_bytes.len();

        let mut buffer_copies = 0_u32;
        let mut upload_barriers = 0_u32;
        for placement in &prepared {
            let (copies, barriers) = record_uploads(
                &self.inner.raw,
                frame.core.command_buffer,
                frame.core.staging.raw(),
                &placement.staged,
                self.inner.driver_workarounds,
            );
            let commit = &mut commits[placement.scene_index];
            commit.byte_count = commit
                .byte_count
                .saturating_add(placement.staged.byte_count);
            commit.buffer_copies = commit.buffer_copies.saturating_add(copies);
            buffer_copies = buffer_copies.saturating_add(copies);
            upload_barriers = upload_barriers.saturating_add(barriers);
            frame.core.images.extend(placement.staged.retained_images());
        }
        #[cfg(feature = "instrumentation")]
        frame.core.write_profiler_timestamp(
            PROFILER_TIMESTAMP_UPLOAD_END,
            vk::PipelineStageFlags2::TRANSFER,
        );

        transition_external_images_to_sampled(frame.core, external_start);
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
        #[cfg(feature = "instrumentation")]
        frame.core.write_profiler_timestamp(
            PROFILER_TIMESTAMP_RENDER_BEGIN,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
        );
        unsafe {
            self.inner.raw.cmd_begin_rendering(
                frame.core.command_buffer,
                &vk::RenderingInfo::default()
                    .render_area(rect2d(render_region))
                    .layer_count(1)
                    .color_attachments(&[attachment]),
            );
        }

        let mut draws = 0_u32;
        for placement in &prepared {
            let scene = &*scenes[placement.scene_index].scene;
            unsafe {
                self.inner.raw.cmd_bind_descriptor_sets(
                    frame.core.command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.inner.layouts.pipeline,
                    0,
                    &[placement.descriptors.view, placement.descriptors.scene],
                    &[],
                );
                self.inner.raw.cmd_set_viewport(
                    frame.core.command_buffer,
                    0,
                    &[vk::Viewport {
                        x: placement.target.x as f32,
                        y: placement.target.y as f32,
                        width: placement.target.width as f32,
                        height: placement.target.height as f32,
                        min_depth: 0.0,
                        max_depth: 1.0,
                    }],
                );
            }
            let mut bound_pipeline = None;
            let mut bound_primitive = None;
            let mut bound_texture = None;
            for batch in &scene.batches {
                let mut scissor = batch_scissor(scene, batch, placement.mapping, render_region);
                if let Some(clip) = placement.clip {
                    scissor = intersect_scissor(scissor, clip);
                }
                if scissor.extent.width == 0 || scissor.extent.height == 0 {
                    continue;
                }
                // Opaque images need source-over at antialiased edges. Weighted additive
                // endpoints must keep addition after coverage is applied.
                let blend = placement_blend(batch.key.blend, placement.rounded);
                let pipeline = self.pipeline(target.format, batch.key.pipeline, blend)?;
                let primitive = primitive_index(batch.kind);
                unsafe {
                    if bound_pipeline != Some(pipeline) {
                        self.inner.raw.cmd_bind_pipeline(
                            frame.core.command_buffer,
                            vk::PipelineBindPoint::GRAPHICS,
                            pipeline,
                        );
                        bound_pipeline = Some(pipeline);
                    }
                    if bound_primitive != Some(primitive) {
                        self.inner.raw.cmd_bind_descriptor_sets(
                            frame.core.command_buffer,
                            vk::PipelineBindPoint::GRAPHICS,
                            self.inner.layouts.pipeline,
                            2,
                            &[placement.descriptors.primitives[primitive]],
                            &[],
                        );
                        bound_primitive = Some(primitive);
                    }
                    if let Some(texture_slot) = scene.texture_slot(batch)
                        && bound_texture != Some(texture_slot)
                    {
                        self.inner.raw.cmd_bind_descriptor_sets(
                            frame.core.command_buffer,
                            vk::PipelineBindPoint::GRAPHICS,
                            self.inner.layouts.pipeline,
                            3,
                            &[placement.descriptors.textures[texture_slot]],
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
                draws = draws.saturating_add(1);
            }
        }

        unsafe {
            self.inner.raw.cmd_end_rendering(frame.core.command_buffer);
            #[cfg(feature = "instrumentation")]
            frame.core.write_profiler_timestamp(
                PROFILER_TIMESTAMP_RENDER_END,
                vk::PipelineStageFlags2::ALL_GRAPHICS,
            );
            release_external_images(frame.core, external_start);
            if target.final_state != VulkanImageState::COLOR_ATTACHMENT {
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

        let mut upload_bytes = 0_u64;
        let mut allocations = 0_u32;
        let mut descriptor_writes = 0_u32;
        for (index, (scene, commit)) in scenes.iter_mut().zip(commits).enumerate() {
            if !used[index] {
                continue;
            }
            upload_bytes = upload_bytes.saturating_add(commit.byte_count);
            allocations = allocations.saturating_add(commit.buffer_allocations);
            descriptor_writes = descriptor_writes.saturating_add(commit.descriptor_writes);
            scene.scene.commit_uploads(
                commit.byte_count,
                commit.buffer_copies,
                commit.buffer_allocations,
                commit.buffer_growths,
                commit.descriptor_writes,
            );
            frame.core.buffers.extend(scene.scene.retained_buffers());
            frame.core.images.extend(scene.scene.retained_images());
        }
        frame.core.rendered = true;
        let barriers = upload_barriers
            .saturating_add(1)
            .saturating_add(external_barriers)
            .saturating_add(u32::from(
                target.final_state != VulkanImageState::COLOR_ATTACHMENT,
            ));
        let stats = RenderStats {
            recorded: true,
            epoch: scenes
                .iter()
                .map(|scene| scene.scene.epoch)
                .max()
                .unwrap_or(0),
            upload_bytes_recorded: upload_bytes,
            buffer_copies,
            buffer_allocations: allocations,
            descriptor_writes,
            passes: 1,
            barriers,
            batches: draws,
            draws,
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

fn write_descriptors(
    device: &ash::Device,
    sampler: vk::Sampler,
    staging: &Arc<AllocatedBuffer>,
    view_offset: u64,
    sets: &CompositeDescriptorSets,
    scene: &VulkanScene,
) -> u32 {
    let (spatial, draw_indices) = scene.common_buffers();
    let mut buffer_ops = vec![
        (
            sets.view,
            0,
            vk::DescriptorType::UNIFORM_BUFFER,
            vk::DescriptorBufferInfo {
                buffer: staging.raw(),
                offset: view_offset,
                range: size_of::<GpuView>() as u64,
            },
        ),
        (
            sets.scene,
            0,
            vk::DescriptorType::STORAGE_BUFFER,
            buffer_info(spatial),
        ),
        (
            sets.scene,
            2,
            vk::DescriptorType::STORAGE_BUFFER,
            buffer_info(draw_indices),
        ),
        (
            sets.scene,
            1,
            vk::DescriptorType::STORAGE_BUFFER,
            scene
                .clip_buffer()
                .map_or_else(|| buffer_info(staging), buffer_info),
        ),
    ];
    for kind in [
        PrimitiveKind::Box,
        PrimitiveKind::Glyph,
        PrimitiveKind::Image,
        PrimitiveKind::Material,
    ] {
        let index = primitive_index(kind);
        if let Some(buffer) = scene.primitive_buffer(kind) {
            buffer_ops.push((
                sets.primitives[index],
                0,
                vk::DescriptorType::STORAGE_BUFFER,
                buffer_info(buffer),
            ));
        }
        if kind == PrimitiveKind::Material
            && let Some(parameters) = scene.material_parameter_buffer()
        {
            buffer_ops.push((
                sets.primitives[index],
                1,
                vk::DescriptorType::STORAGE_BUFFER,
                buffer_info(parameters),
            ));
        }
    }
    let image_ops = (0..scene.texture_count())
        .flat_map(|slot| {
            (0..2).filter_map(move |binding| {
                scene.texture(slot, binding).map(|(view, _)| {
                    (
                        sets.textures[slot],
                        binding,
                        vk::DescriptorImageInfo {
                            sampler,
                            image_view: view,
                            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                        },
                    )
                })
            })
        })
        .collect::<Vec<_>>();
    let buffer_infos = buffer_ops
        .iter()
        .map(|(_, _, _, info)| [*info])
        .collect::<Vec<_>>();
    let image_infos = image_ops
        .iter()
        .map(|(_, _, info)| [*info])
        .collect::<Vec<_>>();
    let mut writes = Vec::with_capacity(buffer_ops.len() + image_ops.len());
    for ((set, binding, descriptor_type, _), info) in buffer_ops.iter().zip(&buffer_infos) {
        writes.push(
            vk::WriteDescriptorSet::default()
                .dst_set(*set)
                .dst_binding(*binding)
                .descriptor_type(*descriptor_type)
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
    writes.len() as u32
}

fn transition_external_images_to_sampled(
    core: &mut super::frame::FrameCore,
    external_start: usize,
) {
    if core.external_images.len() <= external_start {
        return;
    }
    let sampled = fragment_sampled_state();
    let barriers = core
        .external_images
        .iter()
        .skip(external_start)
        .map(|image| {
            let mut initial = image.initial_use.state();
            if matches!(image.acquire, VulkanExternalAcquire::BinarySemaphore(_))
                || image.initial_queue_family != vk::QUEUE_FAMILY_IGNORED
            {
                initial.stage = vk::PipelineStageFlags2::NONE;
                initial.access = vk::AccessFlags2::NONE;
            }
            let mut barrier = image_transition(image.image, initial, sampled);
            if image.initial_queue_family != vk::QUEUE_FAMILY_IGNORED {
                barrier.src_queue_family_index = image.initial_queue_family;
                barrier.dst_queue_family_index = core.device.inner.queue_family;
            }
            barrier
        })
        .collect::<Vec<_>>();
    unsafe {
        core.device.inner.raw.cmd_pipeline_barrier2(
            core.command_buffer,
            &vk::DependencyInfo::default().image_memory_barriers(&barriers),
        );
    }
}

unsafe fn release_external_images(core: &mut super::frame::FrameCore, external_start: usize) {
    if core.external_images.len() <= external_start {
        return;
    }
    let sampled = fragment_sampled_state();
    let barriers = core
        .external_images
        .iter()
        .skip(external_start)
        .map(|image| {
            let mut final_state = image.final_use.state();
            if matches!(image.release, VulkanExternalRelease::BinarySemaphore(_))
                || image.final_queue_family != vk::QUEUE_FAMILY_IGNORED
            {
                final_state.stage = vk::PipelineStageFlags2::NONE;
                final_state.access = vk::AccessFlags2::NONE;
            }
            let mut barrier = image_transition(image.image, sampled, final_state);
            if image.final_queue_family != vk::QUEUE_FAMILY_IGNORED {
                barrier.src_queue_family_index = core.device.inner.queue_family;
                barrier.dst_queue_family_index = image.final_queue_family;
            }
            barrier
        })
        .collect::<Vec<_>>();
    unsafe {
        core.device.inner.raw.cmd_pipeline_barrier2(
            core.command_buffer,
            &vk::DependencyInfo::default().image_memory_barriers(&barriers),
        );
    }
}

fn validate_render_region(region: RectI, target_region: RectI) -> RenderResult<()> {
    if region.x < target_region.x
        || region.y < target_region.y
        || region.width <= 0
        || region.height <= 0
        || region.right() > target_region.right()
        || region.bottom() > target_region.bottom()
    {
        return Err(RenderError::new(
            RenderErrorKind::InvalidTarget,
            "Vulkan composite render region is outside the target region",
        ));
    }
    Ok(())
}

fn placement_intersects(placement: VulkanCompositePlacement, damage: RectI) -> bool {
    let mut rect = intersect_scissor(rect2d(placement.target), damage);
    if let Some(clip) = placement.clip {
        rect = intersect_scissor(rect, clip);
    }
    rect.extent.width != 0 && rect.extent.height != 0
}

#[cfg(test)]
mod damage_culling_tests {
    use super::*;
    #[test]
    fn rounded_coverage_preserves_weighted_additive_compositing() {
        assert_eq!(placement_blend(BlendMode::Opaque, true), BlendMode::Alpha);
        assert_eq!(placement_blend(BlendMode::Alpha, true), BlendMode::Alpha);
        assert_eq!(placement_blend(BlendMode::Add, true), BlendMode::Add);
        assert_eq!(placement_blend(BlendMode::Opaque, false), BlendMode::Opaque);
    }

    #[test]
    fn placement_culling_preserves_partial_and_clipped_intersections() {
        let mut p = VulkanCompositePlacement {
            scene_index: 0,
            target: RectI {
                x: 100,
                y: 100,
                width: 200,
                height: 200,
            },
            clip: None,
            rounded_clips: [None; 2],
        };
        let damage = RectI {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        assert!(!placement_intersects(p, damage));
        p.target.x = 99;
        p.target.y = 99;
        assert!(placement_intersects(p, damage));
        p.clip = Some(RectI {
            x: 150,
            y: 150,
            width: 20,
            height: 20,
        });
        assert!(!placement_intersects(p, damage));
    }
}
