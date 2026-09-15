#![cfg(all(
    feature = "application-software",
    any(
        feature = "application-vulkan-windows",
        feature = "shell-wayland-linux",
        feature = "embedded-vulkan"
    )
))]
#![cfg(target_os = "linux")]

use std::ffi::CStr;
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};

use ash::vk;
use telorgon::core::{RectF, RectI, SizeF, SizeI};
use telorgon::layout::{ClipId, SpatialId};
use telorgon::render::{
    AlphaMode, BatchKey, BlendMode, ColorSpace, DrawItem, ImageId, ImageInstance, PipelineKind,
    PrimitiveKind, RenderBackend, RenderRequest, RenderScene, TargetLoad, TargetStore,
};
use telorgon::renderer_vulkan::interop;
use telorgon::renderer_vulkan::{
    DRM_FORMAT_ABGR8888, DeviceSelection, HostCompletionDomain, HostedAllocationPolicy,
    HostedDeviceExtensions, HostedDeviceFeatures, HostedFrameDescriptor, HostedImageUse,
    HostedTargetDescriptor, HostedVulkanDeviceDescriptor, OffscreenVulkanTarget, VulkanConfig,
    VulkanDevice, VulkanDmaBufImport, VulkanDmaBufPlane, VulkanExternalImageImport,
    VulkanExternalImageOrigin, VulkanInstance,
};
use telorgon::scene::NodeId;

#[test]
#[ignore = "requires Linux, TELORGON_TEST_MODE=developer-hardware, and DMA-BUF/modifier/sync-FD Vulkan support"]
fn dma_buf_is_imported_sampled_and_released_with_sync_fds() {
    run_dma_buf_test(false);
}

#[cfg(feature = "shell-wayland-linux")]
#[test]
#[ignore = "requires Linux, TELORGON_TEST_MODE=developer-hardware, and DMA-BUF/modifier/sync-FD Vulkan support"]
fn cached_dma_buf_rearms_acquire_and_release_across_three_generations() {
    run_dma_buf_test(true);
}

fn run_dma_buf_test(cached: bool) {
    assert_eq!(
        std::env::var("TELORGON_TEST_MODE").as_deref(),
        Ok("developer-hardware"),
        "hardware test must be selected explicitly"
    );
    let config = VulkanConfig {
        enable_validation: true,
        ..VulkanConfig::default()
    };
    let instance = VulkanInstance::load(&config, &[]).expect("load Vulkan 1.3 with validation");
    let adapters = instance.adapters().expect("enumerate Vulkan adapters");
    let selection = DeviceSelection::best(&adapters)
        .unwrap_or_else(|| panic!("no eligible non-CPU Vulkan adapter; reports: {adapters:#?}"));
    let physical_devices = unsafe { interop::raw_instance(&instance).enumerate_physical_devices() }
        .expect("enumerate physical devices");
    let physical_device = physical_devices[selection.adapter_index];
    let queue_family = adapters[selection.adapter_index].graphics_queue_families[0];
    require_extensions(&instance, physical_device);
    let raw = create_external_device(&instance, physical_device, queue_family);
    let queue = unsafe { raw.get_device_queue(queue_family, 0) };
    let domain = HostCompletionDomain::new();
    let hosted = unsafe {
        VulkanDevice::from_hosted(
            HostedVulkanDeviceDescriptor {
                instance: &instance,
                physical_device,
                device: &raw,
                graphics_queue: queue,
                graphics_queue_family: queue_family,
                features: HostedDeviceFeatures {
                    dynamic_rendering: true,
                    synchronization2: true,
                    shader_demote_to_helper_invocation: true,
                },
                extensions: HostedDeviceExtensions {
                    external_memory_fd: true,
                    external_memory_dma_buf: true,
                    image_drm_format_modifier: true,
                    external_semaphore_fd: true,
                    queue_family_foreign: true,
                },
                allocation_policy: HostedAllocationPolicy::TelorgonManaged,
                completion_domain: domain.clone(),
            },
            &config,
        )
    }
    .expect("wrap external-FD-enabled host device");
    hosted
        .external_image_capabilities()
        .require(VulkanExternalImageImport::LinuxDmaBuf)
        .expect("Linux DMA-BUF capability is complete");

    let source_usage = vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
    let negotiated = hosted
        .dma_buf_import_capabilities(source_usage)
        .expect("query Linux DMA-BUF format/modifier capabilities")
        .into_iter()
        .find(|capability| {
            capability.drm_fourcc == DRM_FORMAT_ABGR8888
                && capability.exportable()
                && (!cached || capability.format == vk::Format::R8G8B8A8_SRGB)
        })
        .expect("adapter has no jointly importable/exportable ABGR8888 DMA-BUF tuple");

    let extent = vk::Extent2D {
        width: 32,
        height: 32,
    };
    let command_pool = unsafe {
        raw.create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
            None,
        )
    }
    .expect("create host command pool");
    let commands = unsafe {
        raw.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(3),
        )
    }
    .expect("allocate producer, Telorgon, and readback command buffers");

    let source = create_exported_source(
        &instance,
        &raw,
        physical_device,
        queue_family,
        queue,
        commands[0],
        SourceImageConfig {
            extent,
            format: negotiated.format,
            modifier: negotiated.drm_modifier,
            usage: source_usage,
        },
    );
    #[cfg(feature = "shell-wayland-linux")]
    let mut importer = telorgon::compositor_render::DmaBufImporter::new(&hosted).unwrap();
    let mut scene = hosted.create_scene().expect("create hosted scene");
    let image_id = ImageId(if cached { u32::MAX } else { 41 });
    let mut previous_import = None;
    for generation in 1..=if cached { 3 } else { 1 } {
        if generation > 1 {
            unsafe {
                raw.reset_command_buffer(commands[1], vk::CommandBufferResetFlags::empty())
                    .unwrap();
                raw.reset_command_buffer(commands[2], vk::CommandBufferResetFlags::empty())
                    .unwrap();
            }
        }
        scene.remove_external_image(image_id);
        let target = OffscreenVulkanTarget::new(
            &hosted,
            SizeI {
                width: extent.width as i32,
                height: extent.height as i32,
            },
        )
        .expect("create hosted output target");
        let target_parts = interop::borrowed_target_parts(&target.target());
        if cached {
            #[cfg(feature = "shell-wayland-linux")]
            {
                use telorgon::compositor_wayland::{
                    DmaBufDescriptor, DmaBufFlags, DmaBufImage, DmaBufPlane, WaylandBufferId,
                };
                importer
                    .import_and_bind(
                        &hosted,
                        &mut scene,
                        WaylandBufferId::from_raw(41).unwrap(),
                        generation,
                        DmaBufImage {
                            descriptor: DmaBufDescriptor::new(
                                SizeI {
                                    width: extent.width as i32,
                                    height: extent.height as i32,
                                },
                                negotiated.drm_fourcc,
                                DmaBufFlags::default(),
                                vec![DmaBufPlane {
                                    index: 0,
                                    fd_token: 1,
                                    offset: source.layout.offset as u32,
                                    stride: source.layout.row_pitch as u32,
                                    modifier: source.modifier,
                                }],
                            )
                            .unwrap(),
                            planes: vec![source.dma_buf.try_clone().unwrap()],
                        },
                        // The producer contents stay unchanged; each commit imports a fresh duplicate
                        // of its completed producer fence into the cached acquire semaphore.
                        Some(source.acquire_sync_fd.try_clone().unwrap()),
                        vec![RectI {
                            x: 0,
                            y: 0,
                            width: extent.width as i32,
                            height: extent.height as i32,
                        }],
                    )
                    .expect("bind a cached DMA-BUF generation");
            }
            #[cfg(not(feature = "shell-wayland-linux"))]
            unreachable!("cache bridge requires the desktop feature");
        } else {
            let lease = unsafe {
                hosted.import_dma_buf(VulkanDmaBufImport {
                    planes: vec![VulkanDmaBufPlane {
                        memory: source.dma_buf.try_clone().unwrap(),
                        memory_index: 0,
                        offset: source.layout.offset,
                        size: source.layout.size,
                        row_pitch: source.layout.row_pitch as u32,
                        allocation_size: source.allocation_size,
                    }],
                    drm_fourcc: negotiated.drm_fourcc,
                    drm_modifier: source.modifier,
                    format: negotiated.format,
                    extent,
                    usage: negotiated.usage,
                    content_version: 1,
                    lease_generation: 1,
                    color_encoding: negotiated.color_encoding,
                    alpha_mode: negotiated.alpha_mode,
                    origin: VulkanExternalImageOrigin::TopLeft,
                    initial_use: HostedImageUse::General,
                    final_use: HostedImageUse::General,
                    acquire: Some(source.acquire_sync_fd.try_clone().unwrap()),
                    damage: vec![RectI {
                        x: 0,
                        y: 0,
                        width: extent.width as i32,
                        height: extent.height as i32,
                    }],
                    protected: false,
                })
            }
            .expect("import DMA-BUF and acquire sync FD");
            scene
                .bind_external_image(image_id, lease)
                .expect("bind imported DMA-BUF lease");
        }
        let update = hosted
            .apply_scene_delta(
                &mut scene,
                &external_image_scene(extent, image_id, generation),
            )
            .expect("upload external-image scene records");
        assert_eq!(
            update.epoch, generation,
            "each cached generation must update the retained scene"
        );

        unsafe { raw.begin_command_buffer(commands[1], &vk::CommandBufferBeginInfo::default()) }
            .expect("host begins Telorgon command buffer");
        let target_descriptor = unsafe {
            HostedTargetDescriptor::new(
                target_parts.image,
                target_parts.view,
                target_parts.format,
                target_parts.extent,
                RectI {
                    x: 0,
                    y: 0,
                    width: extent.width as i32,
                    height: extent.height as i32,
                },
                vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                queue_family,
                HostedImageUse::Undefined,
                HostedImageUse::TransferSource,
                ColorSpace::Linear,
                AlphaMode::Premultiplied,
            )
        };
        let descriptor = unsafe { HostedFrameDescriptor::new(commands[1], target_descriptor) };
        let mut frame = unsafe { hosted.begin_hosted_frame(descriptor) }
            .expect("begin command-only DMA-BUF frame");
        let stats = {
            let (mut context, target_view) = frame.context_and_target();
            hosted
                .render(
                    &mut scene,
                    &mut context,
                    &target_view,
                    &RenderRequest {
                        force: true,
                        load: TargetLoad::Clear(telorgon::core::ColorRgba8::rgba(0, 0, 0, 255)),
                        store: TargetStore::Store,
                        region: None,
                    },
                )
                .expect("record DMA-BUF sample")
        };
        let receipt = frame.finish().expect("finish hosted DMA-BUF frame");
        assert_eq!(stats.upload_bytes_recorded, 0);
        assert_eq!(receipt.external_waits().len(), 1);
        assert_eq!(receipt.external_signals().len(), 1);
        let import_handles = (
            receipt.external_image_uses()[0].image,
            receipt.external_waits()[0].semaphore,
            receipt.external_signals()[0].semaphore,
        );
        if let Some(previous) = previous_import {
            assert_eq!(
                import_handles, previous,
                "cache must retain the native image and semaphore handles"
            );
        }
        previous_import = Some(import_handles);
        unsafe { raw.end_command_buffer(commands[1]) }.expect("host ends Telorgon command buffer");

        let waits = receipt
            .external_waits()
            .iter()
            .map(|wait| {
                vk::SemaphoreSubmitInfo::default()
                    .semaphore(wait.semaphore)
                    .stage_mask(wait.stage_mask)
            })
            .collect::<Vec<_>>();
        let signals = receipt
            .external_signals()
            .iter()
            .map(|signal| {
                vk::SemaphoreSubmitInfo::default()
                    .semaphore(signal.semaphore)
                    .stage_mask(signal.stage_mask)
            })
            .collect::<Vec<_>>();
        let command_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(commands[1])];
        let submit = [vk::SubmitInfo2::default()
            .wait_semaphore_infos(&waits)
            .command_buffer_infos(&command_infos)
            .signal_semaphore_infos(&signals)];
        unsafe { raw.queue_submit2(queue, &submit, vk::Fence::null()) }
            .expect("host submits DMA-BUF consumer");
        let external_use = receipt.external_image_uses()[0];
        let release = unsafe {
            receipt
                .export_external_release_sync_fd(external_use.image, external_use.lease_generation)
        }
        .expect("export one-shot DMA-BUF release sync FD after submission");
        assert_eq!(release.content_version, generation);
        assert_eq!(release.lease_generation, generation);
        assert!(
            unsafe {
                receipt.export_external_release_sync_fd(
                    external_use.image,
                    external_use.lease_generation,
                )
            }
            .is_err(),
            "each generation may export its release only once"
        );
        let point = domain
            .point(generation)
            .expect("declare host completion point");
        hosted
            .commit_hosted(receipt, point)
            .expect("commit DMA-BUF pins after release export");

        let readback = HostReadback::new(&instance, &raw, physical_device, extent);
        let release_wait = unsafe { import_sync_fd(&instance, &raw, release.sync_fd) };
        unsafe { raw.begin_command_buffer(commands[2], &vk::CommandBufferBeginInfo::default()) }
            .expect("begin host readback command buffer");
        unsafe {
            raw.cmd_copy_image_to_buffer(
                commands[2],
                target_parts.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                readback.buffer,
                &[vk::BufferImageCopy::default()
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    })],
            );
            raw.end_command_buffer(commands[2])
        }
        .expect("record host readback copy");
        let fence = unsafe { raw.create_fence(&vk::FenceCreateInfo::default(), None) }
            .expect("create host readback fence");
        let wait_infos = [vk::SemaphoreSubmitInfo::default()
            .semaphore(release_wait)
            .stage_mask(vk::PipelineStageFlags2::TRANSFER)];
        let readback_commands =
            [vk::CommandBufferSubmitInfo::default().command_buffer(commands[2])];
        let readback_submit = [vk::SubmitInfo2::default()
            .wait_semaphore_infos(&wait_infos)
            .command_buffer_infos(&readback_commands)];
        unsafe { raw.queue_submit2(queue, &readback_submit, fence) }
            .expect("wait release sync FD and submit readback");
        unsafe { raw.wait_for_fences(&[fence], true, 10_000_000_000) }
            .expect("wait for DMA-BUF composition");
        let pixel = readback.center_pixel(&raw, extent);
        assert!(
            pixel[0] >= 245 && pixel[1] <= 5 && pixel[2] <= 5 && pixel[3] >= 245,
            "unexpected imported DMA-BUF pixel: {pixel:?}"
        );
        let maintenance = hosted
            .advance_host_completion(&domain, generation)
            .expect("retire imported DMA-BUF resources");
        assert_eq!(maintenance.released_external_images, 1);
        assert_eq!(domain.contract_violations(), 0);
        let validation_errors = instance.diagnostics().error_count();
        assert_eq!(
            validation_errors,
            0,
            "validation messages: {:#?}",
            instance.diagnostics().messages()
        );
        println!(
            "TELORGON_EVIDENCE case=vulkan.external-image.linux-dma-buf-sync-fd layer=E8 outcome=pass external_pixel_upload_bytes=0 draws=1 dma_buf_import=true acquire_sync_fd=true release_sync_fd=true foreign_queue_transfer=true completion_receipts=1 generation={generation} cached={cached} validation_errors={validation_errors}"
        );

        unsafe {
            raw.destroy_fence(fence, None);
            raw.destroy_semaphore(release_wait, None);
        }
        drop(readback);
        drop(target);
    }
    drop(scene);
    #[cfg(feature = "shell-wayland-linux")]
    drop(importer);
    drop(hosted);
    unsafe {
        raw.destroy_semaphore(source.acquire_export_semaphore, None);
        raw.destroy_image(source.image, None);
        raw.free_memory(source.memory, None);
        raw.destroy_command_pool(command_pool, None);
        raw.destroy_device(None);
    }
}

struct ExportedSource {
    image: vk::Image,
    memory: vk::DeviceMemory,
    acquire_export_semaphore: vk::Semaphore,
    dma_buf: OwnedFd,
    acquire_sync_fd: OwnedFd,
    modifier: u64,
    layout: vk::SubresourceLayout,
    allocation_size: u64,
}

#[derive(Copy, Clone)]
struct SourceImageConfig {
    extent: vk::Extent2D,
    format: vk::Format,
    modifier: u64,
    usage: vk::ImageUsageFlags,
}

fn create_exported_source(
    instance: &VulkanInstance,
    raw: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family: u32,
    queue: vk::Queue,
    command: vk::CommandBuffer,
    config: SourceImageConfig,
) -> ExportedSource {
    let modifiers = [config.modifier];
    let mut external = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut modifier_list =
        vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&modifiers);
    let image = unsafe {
        raw.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(config.format)
                .extent(vk::Extent3D {
                    width: config.extent.width,
                    height: config.extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                .usage(config.usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .push_next(&mut external)
                .push_next(&mut modifier_list),
            None,
        )
    }
    .expect("create exportable DRM-modifier source image");
    let requirements = unsafe { raw.get_image_memory_requirements(image) };
    let memory_type = choose_memory_type(instance, physical_device, requirements.memory_type_bits);
    let mut export = vk::ExportMemoryAllocateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let memory = unsafe {
        raw.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(memory_type)
                .push_next(&mut export)
                .push_next(&mut dedicated),
            None,
        )
    }
    .expect("allocate exportable DMA-BUF image memory");
    unsafe { raw.bind_image_memory(image, memory, 0) }.expect("bind exportable image memory");
    let modifier_loader =
        ash::ext::image_drm_format_modifier::Device::new(interop::raw_instance(instance), raw);
    let mut modifier_properties = vk::ImageDrmFormatModifierPropertiesEXT::default();
    unsafe {
        modifier_loader.get_image_drm_format_modifier_properties(image, &mut modifier_properties)
    }
    .expect("query chosen DRM modifier");
    assert_eq!(
        modifier_properties.drm_format_modifier, config.modifier,
        "Vulkan must select the single negotiated DRM modifier"
    );
    let layout = unsafe {
        raw.get_image_subresource_layout(
            image,
            vk::ImageSubresource {
                aspect_mask: vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
                mip_level: 0,
                array_layer: 0,
            },
        )
    };
    let memory_fd_loader =
        ash::khr::external_memory_fd::Device::new(interop::raw_instance(instance), raw);
    let dma_fd = unsafe {
        memory_fd_loader.get_memory_fd(
            &vk::MemoryGetFdInfoKHR::default()
                .memory(memory)
                .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
        )
    }
    .expect("export DMA-BUF memory FD");
    let mut semaphore_export = vk::ExportSemaphoreCreateInfo::default()
        .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let acquire_export_semaphore = unsafe {
        raw.create_semaphore(
            &vk::SemaphoreCreateInfo::default().push_next(&mut semaphore_export),
            None,
        )
    }
    .expect("create exportable acquire semaphore");
    unsafe { raw.begin_command_buffer(command, &vk::CommandBufferBeginInfo::default()) }
        .expect("begin source producer command buffer");
    let to_transfer = vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::NONE)
        .src_access_mask(vk::AccessFlags2::NONE)
        .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
        .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .image(image)
        .subresource_range(color_range());
    unsafe {
        raw.cmd_pipeline_barrier2(
            command,
            &vk::DependencyInfo::default().image_memory_barriers(&[to_transfer]),
        );
        raw.cmd_clear_color_image(
            command,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &vk::ClearColorValue {
                float32: [1.0, 0.0, 0.0, 1.0],
            },
            &[color_range()],
        );
    }
    let to_foreign = vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
        .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
        .dst_stage_mask(vk::PipelineStageFlags2::NONE)
        .dst_access_mask(vk::AccessFlags2::NONE)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::GENERAL)
        .src_queue_family_index(queue_family)
        .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
        .image(image)
        .subresource_range(color_range());
    unsafe {
        raw.cmd_pipeline_barrier2(
            command,
            &vk::DependencyInfo::default().image_memory_barriers(&[to_foreign]),
        );
        raw.end_command_buffer(command)
    }
    .expect("record DMA-BUF producer");
    let command_info = [vk::CommandBufferSubmitInfo::default().command_buffer(command)];
    let signal_info = [vk::SemaphoreSubmitInfo::default()
        .semaphore(acquire_export_semaphore)
        .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
    let submit = [vk::SubmitInfo2::default()
        .command_buffer_infos(&command_info)
        .signal_semaphore_infos(&signal_info)];
    unsafe { raw.queue_submit2(queue, &submit, vk::Fence::null()) }
        .expect("submit DMA-BUF producer");
    let semaphore_fd_loader =
        ash::khr::external_semaphore_fd::Device::new(interop::raw_instance(instance), raw);
    let acquire_fd = unsafe {
        semaphore_fd_loader.get_semaphore_fd(
            &vk::SemaphoreGetFdInfoKHR::default()
                .semaphore(acquire_export_semaphore)
                .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
        )
    }
    .expect("export acquire sync FD");
    ExportedSource {
        image,
        memory,
        acquire_export_semaphore,
        dma_buf: unsafe { OwnedFd::from_raw_fd(dma_fd) },
        acquire_sync_fd: unsafe { OwnedFd::from_raw_fd(acquire_fd) },
        modifier: modifier_properties.drm_format_modifier,
        layout,
        allocation_size: requirements.size,
    }
}

struct HostReadback {
    device: ash::Device,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    bytes: usize,
}

impl HostReadback {
    fn new(
        instance: &VulkanInstance,
        raw: &ash::Device,
        physical_device: vk::PhysicalDevice,
        extent: vk::Extent2D,
    ) -> Self {
        let bytes = extent.width as usize * extent.height as usize * 4;
        let buffer = unsafe {
            raw.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(bytes as u64)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }
        .expect("create host readback buffer");
        let requirements = unsafe { raw.get_buffer_memory_requirements(buffer) };
        let properties = unsafe {
            interop::raw_instance(instance).get_physical_device_memory_properties(physical_device)
        };
        let memory_type = (0..properties.memory_type_count)
            .find(|index| {
                requirements.memory_type_bits & (1 << index) != 0
                    && properties.memory_types[*index as usize]
                        .property_flags
                        .contains(
                            vk::MemoryPropertyFlags::HOST_VISIBLE
                                | vk::MemoryPropertyFlags::HOST_COHERENT,
                        )
            })
            .expect("host-visible coherent readback memory");
        let memory = unsafe {
            raw.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(memory_type),
                None,
            )
        }
        .expect("allocate host readback memory");
        unsafe { raw.bind_buffer_memory(buffer, memory, 0) }.expect("bind readback buffer");
        Self {
            device: raw.clone(),
            buffer,
            memory,
            bytes,
        }
    }

    fn center_pixel(&self, raw: &ash::Device, extent: vk::Extent2D) -> [u8; 4] {
        let mapped = unsafe {
            raw.map_memory(
                self.memory,
                0,
                self.bytes as u64,
                vk::MemoryMapFlags::empty(),
            )
        }
        .expect("map host readback");
        let pixels = unsafe { std::slice::from_raw_parts(mapped.cast::<u8>(), self.bytes) };
        let offset =
            ((extent.height as usize / 2) * extent.width as usize + extent.width as usize / 2) * 4;
        let pixel = pixels[offset..offset + 4].try_into().unwrap();
        unsafe { raw.unmap_memory(self.memory) };
        pixel
    }
}

impl Drop for HostReadback {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
            self.device.free_memory(self.memory, None);
        }
    }
}

unsafe fn import_sync_fd(
    instance: &VulkanInstance,
    raw: &ash::Device,
    sync_fd: OwnedFd,
) -> vk::Semaphore {
    let semaphore = unsafe { raw.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }
        .expect("create release wait semaphore");
    let fd = sync_fd.into_raw_fd();
    let loader = ash::khr::external_semaphore_fd::Device::new(interop::raw_instance(instance), raw);
    let info = vk::ImportSemaphoreFdInfoKHR::default()
        .semaphore(semaphore)
        .flags(vk::SemaphoreImportFlags::TEMPORARY)
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
        .fd(fd);
    if let Err(error) = unsafe { loader.import_semaphore_fd(&info) } {
        drop(unsafe { OwnedFd::from_raw_fd(fd) });
        unsafe { raw.destroy_semaphore(semaphore, None) };
        panic!("import release sync FD: {error:?}");
    }
    semaphore
}

fn create_external_device(
    instance: &VulkanInstance,
    physical_device: vk::PhysicalDevice,
    queue_family: u32,
) -> ash::Device {
    let priorities = [1.0];
    let queues = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family)
        .queue_priorities(&priorities)];
    let extensions = external_extension_names();
    let mut features13 = vk::PhysicalDeviceVulkan13Features::default()
        .dynamic_rendering(true)
        .synchronization2(true)
        .shader_demote_to_helper_invocation(true);
    unsafe {
        interop::raw_instance(instance).create_device(
            physical_device,
            &vk::DeviceCreateInfo::default()
                .queue_create_infos(&queues)
                .enabled_extension_names(&extensions)
                .push_next(&mut features13),
            None,
        )
    }
    .expect("create external-FD-enabled host Vulkan device")
}

fn require_extensions(instance: &VulkanInstance, physical_device: vk::PhysicalDevice) {
    let supported = unsafe {
        interop::raw_instance(instance).enumerate_device_extension_properties(physical_device)
    }
    .expect("enumerate external device extensions");
    for required in [
        ash::khr::external_memory_fd::NAME,
        ash::ext::external_memory_dma_buf::NAME,
        ash::ext::image_drm_format_modifier::NAME,
        ash::khr::external_semaphore_fd::NAME,
        ash::ext::queue_family_foreign::NAME,
    ] {
        assert!(
            supported.iter().any(|property| unsafe {
                CStr::from_ptr(property.extension_name.as_ptr()) == required
            }),
            "required Vulkan extension is unavailable: {}",
            required.to_string_lossy()
        );
    }
}

fn external_extension_names() -> [*const std::ffi::c_char; 5] {
    [
        ash::khr::external_memory_fd::NAME.as_ptr(),
        ash::ext::external_memory_dma_buf::NAME.as_ptr(),
        ash::ext::image_drm_format_modifier::NAME.as_ptr(),
        ash::khr::external_semaphore_fd::NAME.as_ptr(),
        ash::ext::queue_family_foreign::NAME.as_ptr(),
    ]
}

fn choose_memory_type(
    instance: &VulkanInstance,
    physical_device: vk::PhysicalDevice,
    bits: u32,
) -> u32 {
    let properties = unsafe {
        interop::raw_instance(instance).get_physical_device_memory_properties(physical_device)
    };
    (0..properties.memory_type_count)
        .filter(|index| bits & (1 << index) != 0)
        .find(|index| {
            properties.memory_types[*index as usize]
                .property_flags
                .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        })
        .or_else(|| (0..properties.memory_type_count).find(|index| bits & (1 << index) != 0))
        .expect("compatible source image memory type")
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

fn external_image_scene(
    extent: vk::Extent2D,
    image_id: ImageId,
    generation: u64,
) -> telorgon::render::RenderSceneDelta {
    let mut scene = RenderScene::default();
    let node = NodeId::new(41, 1);
    scene.images.upsert(
        node,
        ImageInstance {
            node,
            image: image_id,
            tint: None,
            rect: RectF {
                x: 0.0,
                y: 0.0,
                width: extent.width as f32,
                height: extent.height as f32,
            },
            view_bounds: RectF {
                x: 0.0,
                y: 0.0,
                width: extent.width as f32,
                height: extent.height as f32,
            },
            content_version: generation,
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
            resource: image_id.0,
            clip: ClipId(0),
            blend: BlendMode::Alpha,
            target: 0,
        },
    }]);
    scene.damage.add(
        RectF {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
        },
        SizeF {
            width: extent.width as f32,
            height: extent.height as f32,
        },
    );
    let mut delta = scene.take_delta().expect("DMA-BUF scene delta");
    delta.epoch = generation;
    delta
}
