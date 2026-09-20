use ash::vk;

use crate::graphics::renderers::vulkan::driver_workarounds::DriverWorkarounds;
use crate::graphics::renderers::vulkan::target::VulkanImageState;

pub(crate) fn before_storage_upload(
    buffer: vk::Buffer,
    previously_initialized: bool,
) -> vk::BufferMemoryBarrier2<'static> {
    vk::BufferMemoryBarrier2::default()
        .src_stage_mask(if previously_initialized {
            vk::PipelineStageFlags2::VERTEX_SHADER | vk::PipelineStageFlags2::FRAGMENT_SHADER
        } else {
            vk::PipelineStageFlags2::NONE
        })
        .src_access_mask(if previously_initialized {
            vk::AccessFlags2::SHADER_STORAGE_READ
        } else {
            vk::AccessFlags2::NONE
        })
        .dst_stage_mask(vk::PipelineStageFlags2::COPY)
        .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
        .buffer(buffer)
        .offset(0)
        .size(vk::WHOLE_SIZE)
}

pub(crate) fn after_storage_upload(
    buffer: vk::Buffer,
    workarounds: DriverWorkarounds,
) -> vk::BufferMemoryBarrier2<'static> {
    vk::BufferMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::COPY)
        .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
        .dst_stage_mask(
            vk::PipelineStageFlags2::VERTEX_SHADER | vk::PipelineStageFlags2::FRAGMENT_SHADER,
        )
        .dst_access_mask(workarounds.geometry_upload_read_access())
        .buffer(buffer)
        .offset(0)
        .size(vk::WHOLE_SIZE)
}

pub(crate) fn color_subresource() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

pub(crate) fn fragment_sampled_state() -> VulkanImageState {
    VulkanImageState {
        layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        stage: vk::PipelineStageFlags2::FRAGMENT_SHADER,
        access: vk::AccessFlags2::SHADER_SAMPLED_READ,
    }
}

pub(crate) fn image_transition(
    image: vk::Image,
    initial: VulkanImageState,
    final_state: VulkanImageState,
) -> vk::ImageMemoryBarrier2<'static> {
    vk::ImageMemoryBarrier2::default()
        .src_stage_mask(initial.stage)
        .src_access_mask(initial.access)
        .dst_stage_mask(final_state.stage)
        .dst_access_mask(final_state.access)
        .old_layout(initial.layout)
        .new_layout(final_state.layout)
        .image(image)
        .subresource_range(color_subresource())
}

pub(crate) fn target_to_color(
    image: vk::Image,
    initial: VulkanImageState,
) -> vk::ImageMemoryBarrier2<'static> {
    image_transition(image, initial, VulkanImageState::COLOR_ATTACHMENT)
}

pub(crate) fn color_to_final(
    image: vk::Image,
    final_state: VulkanImageState,
) -> vk::ImageMemoryBarrier2<'static> {
    image_transition(image, VulkanImageState::COLOR_ATTACHMENT, final_state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ash::vk::Handle;

    #[test]
    fn upload_completion_changes_only_the_v3dv_destination_access() {
        let buffer = vk::Buffer::from_raw(17);
        let defaults = vk::BufferMemoryBarrier2::default();
        for (driver, access) in [
            (vk::DriverId::MESA_V3DV, vk::AccessFlags2::SHADER_READ),
            (
                vk::DriverId::MESA_RADV,
                vk::AccessFlags2::SHADER_STORAGE_READ,
            ),
            (
                vk::DriverId::NVIDIA_PROPRIETARY,
                vk::AccessFlags2::SHADER_STORAGE_READ,
            ),
            (
                vk::DriverId::from_raw(0),
                vk::AccessFlags2::SHADER_STORAGE_READ,
            ),
        ] {
            let barrier = after_storage_upload(buffer, DriverWorkarounds::for_driver(driver));
            assert_eq!(barrier.s_type, defaults.s_type);
            assert!(barrier.p_next.is_null());
            assert_eq!(barrier.src_stage_mask, vk::PipelineStageFlags2::COPY);
            assert_eq!(barrier.src_access_mask, vk::AccessFlags2::TRANSFER_WRITE);
            assert_eq!(
                barrier.dst_stage_mask,
                vk::PipelineStageFlags2::VERTEX_SHADER | vk::PipelineStageFlags2::FRAGMENT_SHADER,
            );
            assert_eq!(barrier.dst_access_mask, access);
            assert_eq!(
                barrier.src_queue_family_index,
                defaults.src_queue_family_index
            );
            assert_eq!(
                barrier.dst_queue_family_index,
                defaults.dst_queue_family_index
            );
            assert_eq!(barrier.buffer, buffer);
            assert_eq!(barrier.offset, 0);
            assert_eq!(barrier.size, vk::WHOLE_SIZE);
        }
    }

    #[test]
    fn storage_upload_keeps_initialization_and_overwrite_dependencies() {
        let buffer = vk::Buffer::from_raw(23);
        for initialized in [false, true] {
            let barrier = before_storage_upload(buffer, initialized);
            assert_eq!(
                barrier.src_stage_mask,
                if initialized {
                    vk::PipelineStageFlags2::VERTEX_SHADER
                        | vk::PipelineStageFlags2::FRAGMENT_SHADER
                } else {
                    vk::PipelineStageFlags2::NONE
                },
            );
            assert_eq!(
                barrier.src_access_mask,
                if initialized {
                    vk::AccessFlags2::SHADER_STORAGE_READ
                } else {
                    vk::AccessFlags2::NONE
                },
            );
            assert_eq!(barrier.dst_stage_mask, vk::PipelineStageFlags2::COPY);
            assert_eq!(barrier.dst_access_mask, vk::AccessFlags2::TRANSFER_WRITE);
            assert_eq!(barrier.buffer, buffer);
            assert_eq!(barrier.offset, 0);
            assert_eq!(barrier.size, vk::WHOLE_SIZE);
        }
    }

    #[test]
    fn fragment_sampling_keeps_its_precise_image_access() {
        let sampled = fragment_sampled_state();
        assert_eq!(sampled.stage, vk::PipelineStageFlags2::FRAGMENT_SHADER);
        assert_eq!(sampled.access, vk::AccessFlags2::SHADER_SAMPLED_READ);
        assert_eq!(sampled.layout, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
    }
}
