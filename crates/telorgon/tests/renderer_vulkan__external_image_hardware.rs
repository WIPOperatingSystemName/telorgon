#![cfg(all(
    feature = "application-software",
    any(
        feature = "application-vulkan-windows",
        feature = "shell-wayland-linux",
        feature = "embedded-vulkan"
    )
))]

use std::time::Duration;

use ash::vk;
use telorgon::core::{ColorRgba8, RectF, RectI, SizeF, SizeI};
use telorgon::layout::{ClipId, SpatialId};
use telorgon::render::{
    AlphaMode, BatchKey, BlendMode, ColorSpace, DrawItem, ImageAlphaMode, ImageColorEncoding,
    ImageId, ImageInstance, PipelineKind, PrimitiveKind, ReadbackFormat, ReadbackRequest,
    RenderBackend, RenderRequest, RenderScene, TargetLoad, TargetStore,
};
use telorgon::renderer_vulkan::interop;
use telorgon::renderer_vulkan::{
    DeviceSelection, HostCompletionDomain, HostedAllocationPolicy, HostedDeviceExtensions,
    HostedDeviceFeatures, HostedFrameDescriptor, HostedImageUse, HostedTargetDescriptor,
    HostedVulkanDeviceDescriptor, OffscreenVulkanTarget, VulkanConfig, VulkanDevice,
    VulkanExternalAcquire, VulkanExternalImageDescriptor, VulkanExternalImageImport,
    VulkanExternalImageOrigin, VulkanExternalRelease, VulkanInstance,
};
use telorgon::scene::NodeId;

#[test]
#[ignore = "requires TELORGON_TEST_MODE=developer-hardware and a non-CPU Vulkan 1.3 adapter"]
fn host_owned_image_is_sampled_with_real_acquire_and_release_semaphores() {
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
    let owner = VulkanDevice::create_owned(instance.clone(), &config, &selection, None)
        .expect("create host Vulkan device");
    let raw = interop::raw_device(&owner).clone();
    let queue = interop::graphics_queue(&owner);
    let queue_family = interop::graphics_queue_family(&owner);
    let domain = HostCompletionDomain::new();
    let hosted = unsafe {
        VulkanDevice::from_hosted(
            HostedVulkanDeviceDescriptor {
                instance: &instance,
                physical_device: interop::raw_physical_device(&owner),
                device: &raw,
                graphics_queue: queue,
                graphics_queue_family: queue_family,
                features: HostedDeviceFeatures {
                    dynamic_rendering: true,
                    synchronization2: true,
                    shader_demote_to_helper_invocation: true,
                },
                extensions: HostedDeviceExtensions::default(),
                allocation_policy: HostedAllocationPolicy::TelorgonManaged,
                completion_domain: domain.clone(),
            },
            &config,
        )
    }
    .expect("import host Vulkan device");
    assert!(hosted.external_image_capabilities().borrowed_same_device);
    assert!(!hosted.external_image_capabilities().linux_dma_buf);
    hosted
        .external_image_capabilities()
        .require(VulkanExternalImageImport::BorrowedSameDevice)
        .expect("same-device borrowed external images are supported");

    let extent = SizeI {
        width: 32,
        height: 32,
    };
    let source = OffscreenVulkanTarget::new(&owner, extent).expect("create host source image");
    let output = OffscreenVulkanTarget::new(&owner, extent).expect("create host output image");
    let source_parts = interop::borrowed_target_parts(&source.target());
    let output_parts = interop::borrowed_target_parts(&output.target());
    let acquire = unsafe { raw.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }
        .expect("host creates acquire semaphore");
    let release = unsafe { raw.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }
        .expect("host creates release semaphore");
    let command_pool = unsafe {
        raw.create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family)
                .flags(vk::CommandPoolCreateFlags::TRANSIENT),
            None,
        )
    }
    .expect("host creates command pool");
    let buffers = unsafe {
        raw.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(2),
        )
    }
    .expect("host allocates producer and consumer command buffers");
    let producer = buffers[0];
    let consumer = buffers[1];

    record_host_clear(&raw, producer, source_parts, [0.85, 0.05, 0.02, 1.0]);

    let lease = unsafe {
        hosted.import_external_image(VulkanExternalImageDescriptor {
            image: source_parts.image,
            view: source_parts.view,
            format: source_parts.format,
            extent: source_parts.extent,
            usage: vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            queue_family,
            content_version: 1,
            lease_generation: 1,
            color_encoding: ImageColorEncoding::Linear,
            alpha_mode: ImageAlphaMode::Opaque,
            origin: VulkanExternalImageOrigin::TopLeft,
            initial_use: HostedImageUse::ColorAttachment,
            final_use: HostedImageUse::ShaderRead,
            acquire: VulkanExternalAcquire::BinarySemaphore(acquire),
            release: VulkanExternalRelease::BinarySemaphore(release),
            damage: vec![RectI {
                x: 0,
                y: 0,
                width: extent.width,
                height: extent.height,
            }],
            protected: false,
        })
    }
    .expect("import host-owned sampled image");
    let image_id = ImageId(7);
    let mut scene = hosted.create_scene().expect("create hosted external scene");
    scene
        .bind_external_image(image_id, lease)
        .expect("bind external image to logical scene ID");
    hosted
        .apply_scene_delta(&mut scene, &external_scene(image_id, extent))
        .expect("apply external image scene");

    unsafe {
        raw.begin_command_buffer(
            consumer,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )
    }
    .expect("host begins consumer command buffer");
    let target = unsafe {
        HostedTargetDescriptor::new(
            output_parts.image,
            output_parts.view,
            output_parts.format,
            output_parts.extent,
            RectI {
                x: 0,
                y: 0,
                width: extent.width,
                height: extent.height,
            },
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
            queue_family,
            HostedImageUse::Undefined,
            HostedImageUse::ColorAttachment,
            ColorSpace::Linear,
            AlphaMode::Premultiplied,
        )
    };
    let descriptor = unsafe { HostedFrameDescriptor::new(consumer, target) };
    let mut frame = unsafe { hosted.begin_hosted_frame(descriptor) }
        .expect("begin hosted external-image interval");
    let stats = {
        let (mut context, target) = frame.context_and_target();
        hosted
            .render(
                &mut scene,
                &mut context,
                &target,
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .expect("record external image sampling")
    };
    assert_eq!(stats.draws, 1);
    let receipt = frame.finish().expect("finish external-image interval");
    assert_eq!(receipt.stats().external_image_reads, 1);
    assert_eq!(receipt.external_waits().len(), 1);
    assert_eq!(receipt.external_signals().len(), 1);
    assert_eq!(receipt.external_image_uses().len(), 1);
    assert_eq!(receipt.external_image_uses()[0].damage_rects, 1);
    unsafe { raw.end_command_buffer(consumer) }.expect("host ends consumer command buffer");

    let producer_command = [vk::CommandBufferSubmitInfo::default().command_buffer(producer)];
    let producer_signal = [vk::SemaphoreSubmitInfo::default()
        .semaphore(acquire)
        .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
    let producer_submit = [vk::SubmitInfo2::default()
        .command_buffer_infos(&producer_command)
        .signal_semaphore_infos(&producer_signal)];
    unsafe { raw.queue_submit2(queue, &producer_submit, vk::Fence::null()) }
        .expect("host submits external producer");

    let consumer_waits = receipt
        .external_waits()
        .iter()
        .map(|wait| {
            vk::SemaphoreSubmitInfo::default()
                .semaphore(wait.semaphore)
                .stage_mask(wait.stage_mask)
        })
        .collect::<Vec<_>>();
    let consumer_signals = receipt
        .external_signals()
        .iter()
        .map(|signal| {
            vk::SemaphoreSubmitInfo::default()
                .semaphore(signal.semaphore)
                .stage_mask(signal.stage_mask)
        })
        .collect::<Vec<_>>();
    let consumer_command = [vk::CommandBufferSubmitInfo::default().command_buffer(consumer)];
    let consumer_submit = [vk::SubmitInfo2::default()
        .wait_semaphore_infos(&consumer_waits)
        .command_buffer_infos(&consumer_command)
        .signal_semaphore_infos(&consumer_signals)];
    unsafe { raw.queue_submit2(queue, &consumer_submit, vk::Fence::null()) }
        .expect("host submits Telorgon consumer with returned synchronization");
    let point = domain.point(1).expect("declare host completion point");
    hosted
        .commit_hosted(receipt, point)
        .expect("commit external image resource pins");

    let release_fence = unsafe { raw.create_fence(&vk::FenceCreateInfo::default(), None) }
        .expect("host creates release-wait fence");
    let release_wait = [vk::SemaphoreSubmitInfo::default()
        .semaphore(release)
        .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
    let release_submit = [vk::SubmitInfo2::default().wait_semaphore_infos(&release_wait)];
    unsafe { raw.queue_submit2(queue, &release_submit, release_fence) }
        .expect("host submits release semaphore consumer");
    unsafe { raw.wait_for_fences(&[release_fence], true, 10_000_000_000) }
        .expect("external image release semaphore completes");
    let maintenance = hosted
        .advance_host_completion(&domain, 1)
        .expect("retire external image receipt");
    assert_eq!(maintenance.released_frames, 1);
    assert_eq!(maintenance.released_external_images, 1);
    assert_eq!(domain.contract_violations(), 0);

    let mut readback_frame = owner.begin_owned_frame().expect("begin output readback");
    let pending = {
        let mut context = readback_frame.context_mut();
        context
            .record_readback(
                &output.target(),
                &ReadbackRequest {
                    region: RectI {
                        x: 0,
                        y: 0,
                        width: extent.width,
                        height: extent.height,
                    },
                    format: ReadbackFormat::Rgba8,
                },
            )
            .expect("record composed output readback")
    };
    let submission = readback_frame
        .finish()
        .expect("finish output readback")
        .submit()
        .expect("submit output readback");
    let image = pending
        .bind_to_submission(submission)
        .expect("bind output readback")
        .wait(Duration::from_secs(10))
        .expect("wait for output readback");
    let center = ((16 * 32 + 16) * 4) as usize;
    assert!(
        image.pixels[center] > 200,
        "center pixel was {:#?}",
        &image.pixels[center..center + 4]
    );
    assert!(image.pixels[center + 1] < 30);
    assert!(image.pixels[center + 2] < 20);

    unsafe {
        raw.destroy_fence(release_fence, None);
        raw.destroy_command_pool(command_pool, None);
        raw.destroy_semaphore(acquire, None);
        raw.destroy_semaphore(release, None);
    }
    let validation_errors = instance.diagnostics().error_count();
    assert_eq!(
        validation_errors,
        0,
        "{:#?}",
        instance.diagnostics().messages()
    );
    println!(
        "TELORGON_EVIDENCE case=vulkan.external-image.same-device-acquire-sample-release layer=E8 outcome=pass external_pixel_upload_bytes=0 draws=1 acquire_wait=true release_signal=true host_submission=true completion_receipts=1 validation_errors={validation_errors}"
    );
}

fn record_host_clear(
    raw: &ash::Device,
    command_buffer: vk::CommandBuffer,
    target: interop::BorrowedVulkanTargetParts<'_>,
    color: [f32; 4],
) {
    unsafe {
        raw.begin_command_buffer(
            command_buffer,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )
    }
    .expect("host begins producer command buffer");
    let barrier = vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::NONE)
        .src_access_mask(vk::AccessFlags2::NONE)
        .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .image(target.image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    unsafe {
        raw.cmd_pipeline_barrier2(
            command_buffer,
            &vk::DependencyInfo::default().image_memory_barriers(&[barrier]),
        );
    }
    let attachment = vk::RenderingAttachmentInfo::default()
        .image_view(target.view)
        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .clear_value(vk::ClearValue {
            color: vk::ClearColorValue { float32: color },
        });
    unsafe {
        raw.cmd_begin_rendering(
            command_buffer,
            &vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent: target.extent,
                })
                .layer_count(1)
                .color_attachments(&[attachment]),
        );
        raw.cmd_end_rendering(command_buffer);
        raw.end_command_buffer(command_buffer)
    }
    .expect("host ends producer command buffer");
}

fn external_scene(image: ImageId, extent: SizeI) -> telorgon::render::RenderSceneDelta {
    let node = NodeId::new(0, 1);
    let mut source = RenderScene::default();
    source.extent = SizeF {
        width: extent.width as f32,
        height: extent.height as f32,
    };
    source.images.upsert(
        node,
        ImageInstance {
            node,
            image,
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
            content_version: 1,
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(0),
        },
    );
    source.set_draw_order(vec![DrawItem {
        kind: PrimitiveKind::Image,
        index: 0,
        batch: BatchKey {
            pipeline: PipelineKind::Image,
            resource: image.0,
            clip: ClipId(0),
            blend: BlendMode::Alpha,
            target: 0,
        },
    }]);
    source.take_delta().expect("external image scene delta")
}
