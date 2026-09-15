#![cfg(all(
    feature = "application-software",
    any(
        feature = "application-vulkan-windows",
        feature = "shell-wayland-linux",
        feature = "embedded-vulkan"
    )
))]

use telorgon::core::{ColorRgba8, RectF, RectI, SizeF, SizeI};
use telorgon::layout::{ClipId, SpatialId};
use telorgon::render::{
    BatchKey, BlendMode, BoxInstance, DrawItem, PrimitiveKind, RenderBackend, RenderRequest,
    RenderScene, RenderSpatialNode, TargetLoad, TargetStore,
};
use telorgon::renderer_vulkan::interop;
use telorgon::renderer_vulkan::{
    DeviceSelection, HostCompletionDomain, HostedAllocationPolicy, HostedDeviceExtensions,
    HostedDeviceFeatures, HostedFrameDescriptor, HostedImageUse, HostedTargetDescriptor,
    HostedVulkanDeviceDescriptor, OffscreenVulkanTarget, VulkanConfig, VulkanDevice,
    VulkanInstance,
};
use telorgon::scene::NodeId;

#[test]
#[ignore = "requires TELORGON_TEST_MODE=developer-hardware and a non-CPU Vulkan 1.3 adapter"]
fn host_records_two_views_without_telorgon_submission_or_command_buffer_ownership() {
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
    let queue_family = interop::graphics_queue_family(&owner);
    let queue = interop::graphics_queue(&owner);
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
    let target = OffscreenVulkanTarget::new(
        &owner,
        SizeI {
            width: 128,
            height: 64,
        },
    )
    .expect("create host-owned target");
    let native = interop::borrowed_target_parts(&target.target());

    let mut left = hosted.create_scene().expect("create left hosted scene");
    let mut right = hosted.create_scene().expect("create right hosted scene");
    hosted
        .apply_scene_delta(&mut left, &box_scene(ColorRgba8::rgba(240, 40, 30, 255)))
        .expect("upload left view model");
    hosted
        .apply_scene_delta(&mut right, &box_scene(ColorRgba8::rgba(30, 80, 240, 255)))
        .expect("upload right view model");

    let command_pool = unsafe {
        raw.create_command_pool(
            &ash::vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family)
                .flags(ash::vk::CommandPoolCreateFlags::TRANSIENT),
            None,
        )
    }
    .expect("host creates command pool");
    let command_buffer = unsafe {
        raw.allocate_command_buffers(
            &ash::vk::CommandBufferAllocateInfo::default()
                .command_pool(command_pool)
                .level(ash::vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
    }
    .expect("host allocates command buffer")[0];
    unsafe {
        raw.begin_command_buffer(
            command_buffer,
            &ash::vk::CommandBufferBeginInfo::default()
                .flags(ash::vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )
    }
    .expect("host begins command buffer");

    let left_receipt = record_region(
        &hosted,
        &mut left,
        command_buffer,
        native,
        RectI {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        },
        HostedImageUse::Undefined,
        HostedImageUse::ColorAttachment,
    );
    let right_receipt = record_region(
        &hosted,
        &mut right,
        command_buffer,
        native,
        RectI {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        },
        HostedImageUse::ColorAttachment,
        HostedImageUse::ColorAttachment,
    );
    assert_eq!(left_receipt.stats().command_buffers_begun, 0);
    assert_eq!(left_receipt.stats().command_buffers_ended, 0);
    assert_eq!(left_receipt.stats().submissions, 0);
    assert_eq!(right_receipt.stats().presentations, 0);

    unsafe { raw.end_command_buffer(command_buffer) }.expect("host ends command buffer");
    let fence = unsafe { raw.create_fence(&ash::vk::FenceCreateInfo::default(), None) }
        .expect("host creates fence");
    let command = [ash::vk::CommandBufferSubmitInfo::default().command_buffer(command_buffer)];
    let submits = [ash::vk::SubmitInfo2::default().command_buffer_infos(&command)];
    unsafe { raw.queue_submit2(queue, &submits, fence) }.expect("host submits command buffer");
    let point = domain.point(1).expect("declare host submission");
    hosted
        .commit_hosted(left_receipt, point)
        .expect("commit left resource pins");
    hosted
        .commit_hosted(right_receipt, point)
        .expect("commit right resource pins");
    unsafe { raw.wait_for_fences(&[fence], true, 10_000_000_000) }
        .expect("host waits for its fence");
    let maintenance = hosted
        .advance_host_completion(&domain, 1)
        .expect("retire hosted resources");
    assert_eq!(maintenance.released_frames, 2);
    assert_eq!(maintenance.quarantined_frames, 0);
    assert_eq!(domain.contract_violations(), 0);
    unsafe {
        raw.destroy_fence(fence, None);
        raw.destroy_command_pool(command_pool, None);
    }
    let validation_errors = instance.diagnostics().error_count();
    assert_eq!(
        validation_errors,
        0,
        "{:#?}",
        instance.diagnostics().messages()
    );
    println!(
        "TELORGON_EVIDENCE case=hosted.vulkan.command-only-two-view-subregions layer=E6 outcome=pass views=2 submissions_by_telorgon=0 command_begin_end_by_telorgon=0 target_subregions=true completion_receipts=2 validation_errors={validation_errors}"
    );
}

fn record_region(
    device: &VulkanDevice,
    scene: &mut telorgon::renderer_vulkan::VulkanScene,
    command_buffer: ash::vk::CommandBuffer,
    native: interop::BorrowedVulkanTargetParts<'_>,
    region: RectI,
    initial_use: HostedImageUse,
    final_use: HostedImageUse,
) -> telorgon::renderer_vulkan::HostedFrameReceipt {
    let target = unsafe {
        HostedTargetDescriptor::new(
            native.image,
            native.view,
            native.format,
            native.extent,
            region,
            ash::vk::ImageUsageFlags::COLOR_ATTACHMENT | ash::vk::ImageUsageFlags::TRANSFER_SRC,
            interop::graphics_queue_family(device),
            initial_use,
            final_use,
            telorgon::render::ColorSpace::Linear,
            telorgon::render::AlphaMode::Premultiplied,
        )
    };
    let descriptor = unsafe { HostedFrameDescriptor::new(command_buffer, target) };
    let mut frame =
        unsafe { device.begin_hosted_frame(descriptor) }.expect("begin hosted interval");
    let stats = {
        let (mut context, target) = frame.context_and_target();
        device
            .render(
                scene,
                &mut context,
                &target,
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(6, 8, 12, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .expect("record hosted view")
    };
    assert!(stats.recorded);
    frame.finish().expect("finish hosted interval")
}

fn box_scene(color: ColorRgba8) -> telorgon::render::RenderSceneDelta {
    let node = NodeId::new(0, 1);
    let mut source = RenderScene::default();
    source.extent = SizeF {
        width: 64.0,
        height: 64.0,
    };
    source.spatial_nodes.upsert(
        node,
        RenderSpatialNode {
            id: SpatialId(0),
            transform: telorgon::core::Affine2D::IDENTITY,
        },
    );
    source.boxes.upsert(
        node,
        BoxInstance {
            node,
            rect: RectF {
                x: 8.0,
                y: 8.0,
                width: 48.0,
                height: 48.0,
            },
            view_bounds: RectF {
                x: 8.0,
                y: 8.0,
                width: 48.0,
                height: 48.0,
            },
            background: Some(color),
            border: Default::default(),
            outline: Default::default(),
            corner_radii: Default::default(),
            shadows: Default::default(),
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(0),
        },
    );
    source.set_draw_order(vec![DrawItem {
        kind: PrimitiveKind::Box,
        index: 0,
        batch: BatchKey {
            pipeline: telorgon::render::PipelineKind::AnalyticBox,
            resource: 0,
            clip: ClipId(0),
            blend: BlendMode::Alpha,
            target: 0,
        },
    }]);
    source.take_delta().expect("box scene delta")
}
