#![cfg(all(
    target_os = "linux",
    feature = "video-linux",
    feature = "embedded-vulkan"
))]
use std::time::Duration;
use telorgon::{
    core::{ColorRgba8, RectF, RectI, SizeF, SizeI},
    layout::{ClipId, SpatialId},
    media::video::*,
    render::*,
    renderer_vulkan::*,
    scene::NodeId,
};

#[test]
#[ignore = "requires explicit developer-hardware mode and Vulkan DMA-BUF/modifier/sync-file support"]
fn completed_video_export_copy_import_preserves_pixels_and_retained_ownership() {
    assert_eq!(
        std::env::var("TELORGON_TEST_MODE").as_deref(),
        Ok("developer-hardware")
    );
    let config = VulkanConfig {
        enable_validation: true,
        ..Default::default()
    };
    let instance = VulkanInstance::load(&config, &[]).unwrap();
    let selection = DeviceSelection::best(&instance.adapters().unwrap()).expect("Vulkan adapter");
    let device = VulkanDevice::create_owned(instance, &config, &selection, None).unwrap();
    let mut source = RenderScene::default();
    source.extent = SizeF {
        width: 32.0,
        height: 32.0,
    };
    source.background = ColorRgba8::rgba(255, 0, 0, 255);
    source.damage.add(
        RectF {
            x: 0.0,
            y: 0.0,
            width: 32.0,
            height: 32.0,
        },
        source.extent,
    );
    let mut source_scene = device.create_scene().unwrap();
    device
        .apply_scene_delta(&mut source_scene, &source.take_delta().unwrap())
        .unwrap();
    let mut exporter = VulkanVideoRenderer::new(
        device.clone(),
        VideoFormat::rgba(32, 32, 30),
        16 * 1024 * 1024,
    )
    .unwrap();
    let original = exporter
        .render(
            &mut source_scene,
            FrameMetadata {
                timestamp_ns: Some(123456),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(exporter.allocated_frame_bytes() > 0);
    // Dropping the source owner must not revoke its exported frame's pixels.
    drop(source_scene);
    drop(exporter);
    let mut transfer = VulkanVideoTransfer::new(device.clone()).unwrap();
    // SAFETY: no concurrent imports or writes. The synchronous copy finishes every read
    // before returning, and original remains owned throughout the call.
    let planes = unsafe { original.planes() };
    let copied = transfer
        .copy_to_owned(
            BorrowedGpuVideoFrame {
                format: original.format(),
                modifier: original.modifier(),
                planes: &planes,
                metadata: original.metadata(),
            },
            16 * 1024 * 1024,
        )
        .unwrap();
    assert_eq!(copied.metadata().timestamp_ns, Some(123456));
    drop(planes);
    drop(original);
    // Exercise fixed native-output storage twice before sampling the independently copied
    // result. No PipeWire peer sees this fixture; this qualifies Vulkan buffer reuse only.
    let output_format = VideoGpuProducer::formats(&transfer)
        .into_iter()
        .find(|f| f.pixel == copied.format().pixel)
        .unwrap();
    let mut output = VideoGpuProducer::allocate(
        &mut transfer,
        copied.format(),
        output_format.modifier,
        16 * 1024 * 1024,
    )
    .unwrap();
    for _ in 0..2 {
        // SAFETY: this test owns the output exclusively; no consumer or pending read exists.
        unsafe { output.copy_from(&copied) }.unwrap();
    }
    use std::os::fd::AsFd;
    let output_planes: Vec<_> = output
        .planes()
        .iter()
        .map(|p| BorrowedVideoDmaBufPlane {
            fd: p.fd.as_fd(),
            offset: p.offset,
            stride: p.stride,
            allocation_size: p.allocation_size,
        })
        .collect();
    let copied = transfer
        .copy_to_owned(
            BorrowedGpuVideoFrame {
                format: copied.format(),
                modifier: output_format.modifier,
                planes: &output_planes,
                metadata: copied.metadata(),
            },
            16 * 1024 * 1024,
        )
        .unwrap();
    drop(output_planes);
    drop(output);
    drop(transfer);

    assert_red(&device, copied);
}

fn assert_red(device: &VulkanDevice, copied: GpuVideoFrame) {
    let image = ImageId(702);
    let lease = device
        .import_video_frame(copied.clone(), 1, ImageAlphaMode::Premultiplied)
        .unwrap();
    assert!(
        device
            .import_video_frame(copied.clone(), 2, ImageAlphaMode::Premultiplied)
            .is_err()
    );
    let mut scene = device.create_scene().unwrap();
    scene.bind_external_image(image, lease).unwrap();
    device
        .apply_scene_delta(&mut scene, &image_scene(image))
        .unwrap();
    drop(copied); // The renderer lease is now the final allocation owner.
    let target = OffscreenVulkanTarget::new(
        &device,
        SizeI {
            width: 32,
            height: 32,
        },
    )
    .unwrap();
    let mut recording = device.begin_owned_frame().unwrap();
    let pending = {
        let mut context = recording.context_mut();
        device
            .render(
                &mut scene,
                &mut context,
                &target.target(),
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .unwrap();
        context
            .record_readback(
                &target.target(),
                &ReadbackRequest {
                    region: RectI {
                        x: 0,
                        y: 0,
                        width: 32,
                        height: 32,
                    },
                    format: ReadbackFormat::Rgba8,
                },
            )
            .unwrap()
    };
    let receipt = recording.finish().unwrap().submit().unwrap();
    let pixels = pending
        .bind_to_submission(receipt)
        .unwrap()
        .wait(Duration::from_secs(10))
        .unwrap();
    for pixel in pixels.pixels.chunks_exact(4) {
        assert_eq!(pixel, &[255, 0, 0, 255]);
    }
    assert!(scene.remove_external_image(image));
    assert_eq!(
        device.diagnostics().error_count(),
        0,
        "{:?}",
        device.diagnostics().messages()
    );
}

fn image_scene(image: ImageId) -> RenderSceneDelta {
    let mut scene = RenderScene::default();
    scene.extent = SizeF {
        width: 32.0,
        height: 32.0,
    };
    let node = NodeId::new(42, 1);
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: 32.0,
        height: 32.0,
    };
    scene.images.upsert(
        node,
        ImageInstance {
            node,
            image,
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
            resource: image.0,
            clip: ClipId(0),
            blend: BlendMode::Alpha,
            target: 0,
        },
    }]);
    scene.take_delta().unwrap()
}

#[test]
#[ignore = "requires developer-hardware mode and the --gpu-video private PipeWire harness"]
fn gpu_video_crosses_native_pipewire_and_outlives_both_streams() {
    use telorgon::integrations::pipewire::{
        Connection, ConnectionConfig, ConnectionState, ObjectKind, Remote,
        graph::{Feedback, Graph, LinkState},
    };
    assert_eq!(
        std::env::var("TELORGON_TEST_MODE").as_deref(),
        Ok("developer-hardware")
    );
    let remote = std::env::var("TELORGON_TEST_REMOTE").expect("use private GPU video harness");
    assert!(remote.starts_with("telorgon-test-"));
    let config = VulkanConfig {
        enable_validation: true,
        ..Default::default()
    };
    let instance = VulkanInstance::load(&config, &[]).unwrap();
    let selection = DeviceSelection::best(&instance.adapters().unwrap()).expect("Vulkan adapter");
    let device = VulkanDevice::create_owned(instance, &config, &selection, None).unwrap();
    let format = VideoFormat::rgba(32, 32, 60);
    let mut scene = RenderScene::default();
    scene.extent = SizeF {
        width: 32.0,
        height: 32.0,
    };
    scene.background = ColorRgba8::rgba(255, 0, 0, 255);
    scene.damage.add(
        RectF {
            x: 0.0,
            y: 0.0,
            width: 32.0,
            height: 32.0,
        },
        scene.extent,
    );
    let mut native_scene = device.create_scene().unwrap();
    device
        .apply_scene_delta(&mut native_scene, &scene.take_delta().unwrap())
        .unwrap();
    let mut renderer = VulkanVideoRenderer::new(device.clone(), format, 16 * 1024 * 1024).unwrap();
    let original = renderer
        .render(&mut native_scene, FrameMetadata::default())
        .unwrap();
    let mut connection =
        Connection::connect(ConnectionConfig::default(), Remote::Named(remote)).unwrap();
    let handle = connection.handle();
    until(|| handle.state() == ConnectionState::Ready);
    let source = VideoStream::open_gpu_producer(
        handle.clone(),
        VideoConfig::producer(format),
        VulkanVideoTransfer::new(device.clone()).unwrap(),
    )
    .unwrap();
    let mut config = VideoConfig::producer(format);
    config.direction = VideoDirection::Capture;
    let sink = VideoStream::open_gpu(
        handle.clone(),
        config,
        VulkanVideoTransfer::new(device.clone()).unwrap(),
    )
    .unwrap();
    let port = |stream: &VideoStream, direction: &str| {
        stream.node().and_then(|node| {
            handle
                .snapshot()
                .objects_of_kind(ObjectKind::Port)
                .find(|p| {
                    p.properties.get("node.id") == Some(&node.id().to_string())
                        && p.properties
                            .get("port.direction")
                            .is_some_and(|d| d == direction)
                })
                .map(|p| p.handle)
        })
    };
    until(|| port(&source, "out").is_some() && port(&sink, "in").is_some());
    let link = Graph::new(handle.clone())
        .link(
            port(&source, "out").unwrap(),
            port(&sink, "in").unwrap(),
            Feedback::Reject,
        )
        .unwrap();
    until(|| {
        assert!(
            !matches!(link.state(), LinkState::Failed(_)),
            "{:?}",
            link.state()
        );
        link.state() == LinkState::Active
    });
    let mut retained = None;
    until(|| {
        assert!(
            !matches!(source.state(), VideoState::Failed(_)),
            "{:?}",
            source.state()
        );
        assert!(
            !matches!(sink.state(), VideoState::Failed(_)),
            "{:?}",
            sink.state()
        );
        if source.state() == VideoState::Streaming {
            source.submit_gpu(original.clone()).unwrap();
        }
        match sink.receive_frame().unwrap() {
            Some(VideoFrame::Gpu(frame)) => retained = Some(frame),
            Some(VideoFrame::Cpu(_)) => {
                panic!("GPU qualification requires actual DMA-BUF transport")
            }
            None => {}
        }
        retained.is_some()
    });
    assert!(matches!(
        source.negotiation().unwrap().transport,
        VideoTransport::DmaBuf(_)
    ));
    assert!(source.diagnostics().allocated_frame_bytes > 0);
    assert_eq!(handle.snapshot().diagnostics.protocol_errors, 0);
    drop(link);
    source.stop();
    sink.stop();
    connection.shutdown().unwrap();
    drop(original);
    drop(renderer);
    drop(native_scene);
    assert_red(&device, retained.unwrap());
}
fn until(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "GPU qualification timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "requires explicit developer-hardware mode; synthetic GPU scenes only"]
fn asynchronous_video_export_checks_submission_and_retires_cancelled_targets() {
    for pixel in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
        asynchronous_export_format(pixel);
    }
}
fn asynchronous_export_format(pixel: PixelFormat) {
    assert_eq!(
        std::env::var("TELORGON_TEST_MODE").as_deref(),
        Ok("developer-hardware")
    );
    let config = VulkanConfig {
        enable_validation: true,
        ..Default::default()
    };
    let instance = VulkanInstance::load(&config, &[]).unwrap();
    let selection = DeviceSelection::best(&instance.adapters().unwrap()).unwrap();
    let device = VulkanDevice::create_owned(instance, &config, &selection, None).unwrap();
    let mut source = RenderScene::default();
    source.extent = SizeF {
        width: 32.0,
        height: 32.0,
    };
    source.background = ColorRgba8::rgba(255, 0, 0, 255);
    source.damage.add(
        RectF {
            x: 0.0,
            y: 0.0,
            width: 32.0,
            height: 32.0,
        },
        source.extent,
    );
    let mut scene = device.create_scene().unwrap();
    device
        .apply_scene_delta(&mut scene, &source.take_delta().unwrap())
        .unwrap();
    let mut exporter = VulkanVideoRenderer::new(
        device.clone(),
        VideoFormat {
            pixel,
            ..VideoFormat::rgba(32, 32, 30)
        },
        16 * 1024 * 1024,
    )
    .unwrap();
    // Cancelling delivery before submission keeps the target pinned by recorded commands.
    let mut recording = device.begin_owned_frame().unwrap();
    let cancelled = exporter
        .record(&mut scene, &mut recording.context_mut(), Default::default())
        .unwrap();
    assert!(exporter.allocated_frame_bytes() > 0);
    drop(cancelled);
    assert!(exporter.allocated_frame_bytes() > 0);
    drop(recording);
    assert_eq!(exporter.allocated_frame_bytes(), 0);

    // Dropping an unobserved receipt transfers ownership to deferred submission retirement.
    let mut recording = device.begin_owned_frame().unwrap();
    let cancelled = exporter
        .record(&mut scene, &mut recording.context_mut(), Default::default())
        .unwrap();
    let receipt = recording.finish().unwrap().submit().unwrap();
    drop(cancelled);
    drop(receipt);
    assert!(exporter.allocated_frame_bytes() > 0);
    let advance = device.begin_owned_frame().unwrap();
    let mut advance = advance.finish().unwrap().submit().unwrap();
    advance.wait(Duration::from_secs(10)).unwrap();
    drop(advance);
    assert_eq!(exporter.allocated_frame_bytes(), 0);

    let mut recording = device.begin_owned_frame().unwrap();
    let mut pending = exporter
        .record(&mut scene, &mut recording.context_mut(), Default::default())
        .unwrap();
    let mut receipt = recording.finish().unwrap().submit().unwrap();
    let unrelated = device.begin_owned_frame().unwrap();
    let mut other = unrelated.finish().unwrap().submit().unwrap();
    other.wait(Duration::from_secs(10)).unwrap();
    assert!(pending.try_complete(&mut other).is_err());
    receipt.wait(Duration::from_secs(10)).unwrap();
    let frame = pending.try_complete(&mut receipt).unwrap().unwrap();
    assert!(pending.try_complete(&mut receipt).is_err());
    drop(receipt);
    drop(other);
    drop(pending);
    assert!(exporter.allocated_frame_bytes() > 0);
    assert_red(&device, frame);
    assert_eq!(exporter.allocated_frame_bytes(), 0);

    // The same asynchronous operation renders approved composition placements. An imported
    // red image supplies scene pixels, so this checks composition rather than only clear color.
    let red = exporter.render(&mut scene, Default::default()).unwrap();
    let mut composite = device.create_scene().unwrap();
    composite
        .bind_external_image(
            ImageId(703),
            device
                .import_video_frame(red, 1, ImageAlphaMode::Premultiplied)
                .unwrap(),
        )
        .unwrap();
    device
        .apply_scene_delta(&mut composite, &image_scene(ImageId(703)))
        .unwrap();
    let mut recording = device.begin_owned_frame().unwrap();
    let mut pending = exporter
        .record_composite(
            &mut [VulkanCompositeScene {
                scene: &mut composite,
            }],
            &[VulkanCompositePlacement {
                scene_index: 0,
                target: RectI {
                    x: 0,
                    y: 0,
                    width: 32,
                    height: 32,
                },
                clip: None,
                rounded_clips: [None; 2],
            }],
            &mut recording.context_mut(),
            Default::default(),
        )
        .unwrap();
    let mut receipt = recording.finish().unwrap().submit().unwrap();
    receipt.wait(Duration::from_secs(10)).unwrap();
    let frame = pending.try_complete(&mut receipt).unwrap().unwrap();
    drop(pending);
    drop(receipt);
    assert!(composite.remove_external_image(ImageId(703)));
    drop(composite);
    assert_red(&device, frame);
    assert_eq!(exporter.allocated_frame_bytes(), 0);
}
