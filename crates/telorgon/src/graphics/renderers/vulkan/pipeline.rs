use std::collections::HashMap;
use std::ffi::CStr;

use crate::graphics::render::RenderResult;
use crate::graphics::render::{BlendMode, PipelineKind};
use ash::vk;

use crate::graphics::renderers::vulkan::descriptor::DescriptorLayouts;
use crate::graphics::renderers::vulkan::error::{internal, vk_error};
use crate::graphics::renderers::vulkan::shader::ShaderModules;

pub(crate) struct PipelineCache {
    device: ash::Device,
    pipelines: HashMap<(vk::Format, PipelineKind, BlendMode), vk::Pipeline>,
}

impl PipelineCache {
    pub(crate) fn new(device: &ash::Device) -> Self {
        Self {
            device: device.clone(),
            pipelines: HashMap::new(),
        }
    }

    pub(crate) fn get_or_create(
        &mut self,
        format: vk::Format,
        kind: PipelineKind,
        blend_mode: BlendMode,
        layouts: &DescriptorLayouts,
    ) -> RenderResult<vk::Pipeline> {
        if let Some(pipeline) = self.pipelines.get(&(format, kind, blend_mode)) {
            return Ok(*pipeline);
        }
        let shaders = ShaderModules::load(&self.device, kind)?;
        let entry: &CStr = c"main";
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(shaders.vertex)
                .name(entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(shaders.fragment)
                .name(entry),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(blend_mode != BlendMode::Opaque)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(if blend_mode == BlendMode::Add {
                vk::BlendFactor::ONE
            } else {
                vk::BlendFactor::ONE_MINUS_SRC_ALPHA
            })
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(if blend_mode == BlendMode::Add {
                vk::BlendFactor::ONE
            } else {
                vk::BlendFactor::ONE_MINUS_SRC_ALPHA
            })
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA);
        let blend_attachments = [blend_attachment];
        let blend =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let color_formats = [format];
        let mut rendering =
            vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&color_formats);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(layouts.pipeline)
            .push_next(&mut rendering);
        let pipeline = unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
        }
        .map_err(|(_, result)| vk_error("failed to create Vulkan graphics pipeline", result))?
        .into_iter()
        .next()
        .ok_or_else(|| internal("Vulkan returned no graphics pipeline"))?;
        self.pipelines.insert((format, kind, blend_mode), pipeline);
        Ok(pipeline)
    }
}

impl Drop for PipelineCache {
    fn drop(&mut self) {
        for pipeline in self.pipelines.values().copied() {
            unsafe { self.device.destroy_pipeline(pipeline, None) };
        }
    }
}
