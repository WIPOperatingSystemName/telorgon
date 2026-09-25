#![cfg(all(target_os = "linux", feature = "video-linux"))]
//! Private synthetic streams only; never opens a system camera or desktop source.
use std::time::{Duration, Instant};
use telorgon::{
    integrations::pipewire::{
        graph::{Feedback, Graph, LinkState},
        *,
    },
    media::video::*,
};
#[track_caller]
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < deadline, "isolated video timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn connection() -> Connection {
    let remote = std::env::var("TELORGON_TEST_REMOTE").expect("use test_pipewire.py");
    assert!(remote.starts_with("telorgon-test-"));
    let connection =
        Connection::connect(ConnectionConfig::default(), Remote::Named(remote)).unwrap();
    until(|| connection.handle().state() == ConnectionState::Ready);
    connection
}
fn link(
    h: &ConnectionHandle,
    source: &VideoStream,
    sink: &VideoStream,
) -> telorgon::integrations::pipewire::graph::LinkLease {
    let port = |node: Option<ObjectHandle>, direction: &str| {
        node.and_then(|node| {
            h.snapshot()
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
    until(|| port(source.node(), "out").is_some() && port(sink.node(), "in").is_some());
    let lease = Graph::new(h.clone())
        .link(
            port(source.node(), "out").unwrap(),
            port(sink.node(), "in").unwrap(),
            Feedback::Reject,
        )
        .unwrap();
    until(|| {
        assert!(
            !matches!(lease.state(), LinkState::Failed(_)),
            "{:?}",
            lease.state()
        );
        lease.state() == LinkState::Active
    });
    lease
}
fn capture(
    source: &mut VideoStream,
    sink: &mut VideoStream,
    frame: CpuVideoFrame,
) -> CpuVideoFrame {
    let mut result = None;
    let mut printed = Instant::now();
    until(|| {
        assert!(
            !matches!(source.state(), VideoState::Failed(_)),
            "source {:?}",
            source.state()
        );
        assert!(
            !matches!(sink.state(), VideoState::Failed(_)),
            "sink {:?}",
            sink.state()
        );
        let sent = source.submit(frame.clone());
        if printed.elapsed() > Duration::from_secs(2) {
            eprintln!(
                "source {:?} {:?} format {:?}; sink {:?} {:?} format {:?}; send {:?}",
                source.state(),
                source.diagnostics(),
                source.negotiated_format(),
                sink.state(),
                sink.diagnostics(),
                sink.negotiated_format(),
                sent.as_ref().err().map(|e| &e.0)
            );
            printed = Instant::now();
        }
        result = sink.receive().unwrap();
        result.is_some()
    });
    result.unwrap()
}
#[test]
#[ignore = "requires the isolated synthetic server harness"]
fn video_frames_preserve_metadata_and_old_leases_across_resize() {
    let mut connection = connection();
    let h = connection.handle();
    let large = VideoFormat::rgba(64, 48, 120);
    let small = VideoFormat::rgba(32, 24, 120);
    let mut config = VideoConfig::producer(large);
    config.name = "telorgon.test.video-source".into();
    config.virtual_camera = true;
    let mut source = VideoStream::open(h.clone(), config.clone()).unwrap();
    config.name = "telorgon.test.video-sink".into();
    config.direction = VideoDirection::Capture;
    config.virtual_camera = false;
    config.formats = vec![large, small];
    config.buffer_frames = 3;
    config.memory_budget = (large.byte_len().unwrap()
        + MAX_CURSOR_BYTES
        + MAX_DAMAGE_RECTS * std::mem::size_of::<VideoRect>())
        * 3;
    let mut sink = VideoStream::open(h.clone(), config).unwrap();
    let lease = link(&h, &source, &sink);
    until(|| source.state() == VideoState::Streaming && sink.state() == VideoState::Streaming);
    let metadata = FrameMetadata {
        timestamp_ns: Some(12_345_678),
        crop: Some(VideoRect {
            x: 4,
            y: 6,
            width: 30,
            height: 16,
        }),
        transform: VideoTransform::Rotate90,
        damage: vec![VideoRect {
            x: 4,
            y: 6,
            width: 8,
            height: 8,
        }],
        cursor: Some(VideoCursor {
            id: 1,
            x: 9,
            y: 10,
            hotspot_x: 1,
            hotspot_y: 1,
            visible: Some(true),
            bitmap: Some(CursorBitmap {
                width: 2,
                height: 2,
                rgba: vec![0x7f; 16],
            }),
        }),
        ..Default::default()
    };
    let frame =
        CpuVideoFrame::packed(large, vec![0x55; large.byte_len().unwrap()], metadata).unwrap();
    let held = capture(&mut source, &mut sink, frame);
    assert_eq!(held.plane(0).unwrap().row(0).unwrap(), &[0x55; 256]);
    assert_eq!(held.metadata().timestamp_ns, Some(12_345_678));
    assert_eq!(
        held.metadata().crop,
        Some(VideoRect {
            x: 4,
            y: 6,
            width: 30,
            height: 16
        })
    );
    assert_eq!(held.metadata().transform, VideoTransform::Rotate90);
    assert_eq!(held.metadata().damage.len(), 1);
    assert_eq!(
        held.metadata()
            .cursor
            .as_ref()
            .unwrap()
            .bitmap
            .as_ref()
            .unwrap()
            .rgba,
        vec![0x7f; 16]
    );
    let start_count = source.diagnostics().frames;
    let deadline = Instant::now() + Duration::from_millis(300);
    let cadence_frame = CpuVideoFrame::packed(
        large,
        vec![0x55; large.byte_len().unwrap()],
        FrameMetadata::default(),
    )
    .unwrap();
    while Instant::now() < deadline {
        let _ = source.submit(cadence_frame.clone());
        while sink.receive().unwrap().is_some() {}
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        source.diagnostics().frames - start_count >= 20,
        "120 FPS driver did not exceed 60 FPS: {:?}",
        source.diagnostics()
    );
    let before = held.generation();
    let request = source.reconfigure(vec![small]).unwrap();
    until(|| {
        assert!(
            !matches!(source.state(), VideoState::Failed(_)),
            "{:?}",
            source.state()
        );
        matches!(request.state(), RequestState::Complete(_))
    });
    assert_eq!(request.state(), RequestState::Complete(Ok(())));
    until(|| {
        sink.negotiated_format()
            .is_some_and(|(f, g)| f == small && g > before)
    });
    assert_eq!(sink.state(), VideoState::Renegotiating);
    assert_eq!(held.format(), large);
    assert_eq!(held.plane(0).unwrap().row(47).unwrap(), &[0x55; 256]);
    drop(held);
    until(|| sink.state() == VideoState::Streaming);
    let frame = CpuVideoFrame::packed(
        small,
        vec![0xaa; small.byte_len().unwrap()],
        FrameMetadata::default(),
    )
    .unwrap();
    let frame = capture(&mut source, &mut sink, frame);
    assert!(frame.generation() > before);
    assert_eq!(frame.plane(0).unwrap().row(0).unwrap(), &[0xaa; 128]);
    let pause = source.set_active(false).unwrap();
    until(|| matches!(pause.state(), RequestState::Complete(_)));
    assert_eq!(pause.state(), RequestState::Complete(Ok(())));
    let resume = source.set_active(true).unwrap();
    until(|| matches!(resume.state(), RequestState::Complete(_)));
    assert_eq!(resume.state(), RequestState::Complete(Ok(())));
    drop(lease);
    source.stop();
    sink.stop();
    until(|| source.state() == VideoState::Stopped && sink.state() == VideoState::Stopped);
    assert_eq!(frame.plane(0).unwrap().row(0).unwrap(), &[0xaa; 128]);
    let barrier = h.barrier().unwrap();
    until(|| matches!(barrier.state(), RequestState::Complete(_)));
    assert_eq!(h.snapshot().diagnostics.protocol_errors, 0);
    connection.shutdown().unwrap();
}
#[test]
#[ignore = "requires the isolated synthetic server harness"]
fn nv12_multiplane_pixels_survive_native_transport() {
    let mut connection = connection();
    let h = connection.handle();
    let format = VideoFormat {
        pixel: PixelFormat::Nv12,
        color: Colorimetry::BT709,
        ..VideoFormat::rgba(32, 24, 30)
    };
    let mut config = VideoConfig::producer(format);
    config.name = "telorgon.test.nv12-source".into();
    let mut source = VideoStream::open(h.clone(), config.clone()).unwrap();
    config.name = "telorgon.test.nv12-sink".into();
    config.direction = VideoDirection::Capture;
    let mut sink = VideoStream::open(h.clone(), config).unwrap();
    let lease = link(&h, &source, &sink);
    let frame = CpuVideoFrame::from_planes(
        format,
        vec![
            CpuPlane {
                bytes: vec![64; 32 * 24],
                offset: 0,
                stride: 32,
            },
            CpuPlane {
                bytes: vec![128; 32 * 12],
                offset: 0,
                stride: 32,
            },
        ],
        FrameMetadata::default(),
    )
    .unwrap();
    let captured = capture(&mut source, &mut sink, frame);
    assert_eq!(captured.format().color, Colorimetry::BT709);
    assert_eq!(captured.plane(0).unwrap().row(23).unwrap(), &[64; 32]);
    assert_eq!(captured.plane(1).unwrap().row(11).unwrap(), &[128; 32]);
    assert_eq!(sink.diagnostics().malformed, 0);
    drop(lease);
    source.stop();
    sink.stop();
    let barrier = h.barrier().unwrap();
    until(|| matches!(barrier.state(), RequestState::Complete(_)));
    assert_eq!(h.snapshot().diagnostics.protocol_errors, 0);
    connection.shutdown().unwrap();
}

#[test]
#[ignore = "requires isolated PipeWire; use tools/sdk/test_pipewire.py video_isolated"]
fn capture_ranges_negotiate_and_preview_wakes_and_releases_on_stop() {
    use telorgon::{
        graphics::bridges::video_cpu::VideoImageOptions, host::application::video::VideoPreview,
        ui::ImageId,
    };
    let mut connection = connection();
    let handle = connection.handle();
    let format = VideoFormat::rgba(40, 24, 90);
    let source = VideoStream::open(handle.clone(), VideoConfig::producer(format)).unwrap();
    let mut config = VideoConfig::producer(VideoFormat::rgba(64, 48, 30));
    config.direction = VideoDirection::Capture;
    config.capture_range = Some(VideoCaptureRange {
        min_size: [2, 2],
        max_size: [128, 128],
        min_rate: FrameRate::hz(0),
        max_rate: FrameRate::hz(120),
    });
    let sink = VideoStream::open(handle.clone(), config).unwrap();
    let lease = link(&handle, &source, &sink);
    until(|| sink.negotiated_format().is_some());
    assert_eq!(sink.negotiated_format().unwrap().0, format);
    let mut preview = VideoPreview::start(sink, ImageId(74), VideoImageOptions::default()).unwrap();
    let signal = preview.signal();
    let pixels = [3u8, 70, 180, 255].repeat(40 * 24);
    until(|| {
        if source.state() == VideoState::Streaming {
            let _ = source.submit(
                CpuVideoFrame::packed(format, pixels.clone(), FrameMetadata::default()).unwrap(),
            );
        }
        signal.snapshot().image.is_some()
    });
    let resource = signal.snapshot().image.clone().unwrap();
    assert_eq!(&*resource.pixels, &pixels);
    assert_eq!(resource.extent.width, 40);
    preview.shutdown();
    assert!(signal.snapshot().image.is_none());
    assert_eq!(&*resource.pixels, &pixels);
    drop(lease);
    source.stop();
    connection.shutdown().unwrap();
}

#[test]
#[ignore = "requires isolated PipeWire; use tools/sdk/test_pipewire.py video_isolated"]
fn gpu_capture_negotiates_shared_memory_fallback_without_invoking_gpu() {
    struct UnusedGpu;
    // SAFETY: this fixture only offers capabilities; the CPU-only peer must never invoke a
    // GPU transfer. A panic occurs before any resource access if that invariant is violated.
    unsafe impl VideoGpuTransfer for UnusedGpu {
        fn formats(&self) -> Vec<VideoDmaBufFormat> {
            vec![VideoDmaBufFormat {
                pixel: PixelFormat::Rgba8,
                modifier: 0,
                planes: 1,
            }]
        }
        fn copy_to_owned(
            &mut self,
            _: BorrowedGpuVideoFrame<'_>,
            _: usize,
        ) -> Result<GpuVideoFrame, MediaError> {
            panic!("GPU invoked for shared-memory capture")
        }
    }
    let mut connection = connection();
    let handle = connection.handle();
    let format = VideoFormat::rgba(32, 24, 30);
    let source = VideoStream::open(handle.clone(), VideoConfig::producer(format)).unwrap();
    let mut config = VideoConfig::producer(format);
    config.direction = VideoDirection::Capture;
    let sink = VideoStream::open_gpu(handle.clone(), config, UnusedGpu).unwrap();
    let lease = link(&handle, &source, &sink);
    let pixels = [13u8, 22, 80, 255].repeat(32 * 24);
    let mut captured = None;
    until(|| {
        if source.state() == VideoState::Streaming {
            let _ = source.submit(
                CpuVideoFrame::packed(format, pixels.clone(), FrameMetadata::default()).unwrap(),
            );
        }
        captured = sink.receive_frame().unwrap();
        captured.is_some()
    });
    let VideoFrame::Cpu(frame) = captured.unwrap() else {
        panic!("CPU fallback was not selected")
    };
    assert_eq!(frame.plane(0).unwrap().row(0).unwrap(), &pixels[..32 * 4]);
    drop(lease);
    source.stop();
    sink.stop();
    connection.shutdown().unwrap();
}

#[test]
#[ignore = "requires isolated PipeWire; use tools/sdk/test_pipewire.py video_isolated"]
fn gpu_producer_allocates_shared_memory_for_cpu_consumer() {
    gpu_producer_fallback(false);
}
#[test]
#[ignore = "requires isolated PipeWire; use tools/sdk/test_pipewire.py video_isolated"]
fn failed_gpu_test_allocation_renegotiates_shared_memory() {
    gpu_producer_fallback(true);
}
fn gpu_producer_fallback(gpu_consumer: bool) {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct NoAllocator(Arc<AtomicUsize>);
    // SAFETY: negative fixture never produces any allocation or performs GPU operations.
    unsafe impl VideoGpuProducer for NoAllocator {
        fn formats(&self) -> Vec<VideoDmaBufFormat> {
            vec![VideoDmaBufFormat {
                pixel: PixelFormat::Rgba8,
                modifier: 0,
                planes: 1,
            }]
        }
        fn allocate(
            &mut self,
            _: VideoFormat,
            _: u64,
            _: usize,
        ) -> Result<Box<dyn VideoGpuOutputBuffer>, MediaError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(MediaError::Unsupported("intentional allocation failure"))
        }
    }
    struct NoTransfer;
    // SAFETY: fixture only tests negotiation; no GPU storage or access is provided.
    unsafe impl VideoGpuTransfer for NoTransfer {
        fn formats(&self) -> Vec<VideoDmaBufFormat> {
            vec![VideoDmaBufFormat {
                pixel: PixelFormat::Rgba8,
                modifier: 0,
                planes: 1,
            }]
        }
        fn copy_to_owned(
            &mut self,
            _: BorrowedGpuVideoFrame<'_>,
            _: usize,
        ) -> Result<GpuVideoFrame, MediaError> {
            panic!("failed allocator must select shared-memory fallback");
        }
    }
    let mut connection = connection();
    let handle = connection.handle();
    let format = VideoFormat::rgba(32, 24, 120);
    let attempts = Arc::new(AtomicUsize::new(0));
    let source = VideoStream::open_gpu_producer(
        handle.clone(),
        VideoConfig::producer(format),
        NoAllocator(attempts.clone()),
    )
    .unwrap();
    let mut config = VideoConfig::producer(format);
    config.direction = VideoDirection::Capture;
    let sink = if gpu_consumer {
        VideoStream::open_gpu(handle.clone(), config, NoTransfer).unwrap()
    } else {
        VideoStream::open(handle.clone(), config).unwrap()
    };
    let lease = link(&handle, &source, &sink);
    let pixels = [7u8, 88, 200, 255].repeat(32 * 24);
    let mut captured = None;
    until(|| {
        assert!(
            !matches!(source.state(), VideoState::Failed(_)),
            "source {:?}",
            source.state()
        );
        assert!(
            !matches!(sink.state(), VideoState::Failed(_)),
            "sink {:?}",
            sink.state()
        );
        if source.state() == VideoState::Streaming {
            let _ = source.submit(
                CpuVideoFrame::packed(format, pixels.clone(), FrameMetadata::default()).unwrap(),
            );
        }
        captured = sink.receive().unwrap();
        captured.is_some()
    });
    assert_eq!(
        source.negotiation().unwrap().transport,
        VideoTransport::SharedMemory
    );
    assert!(source.diagnostics().allocated_frame_bytes > 0);
    assert_eq!(attempts.load(Ordering::Relaxed) > 0, gpu_consumer);
    source.stop();
    let retained = captured.unwrap();
    assert_eq!(
        retained.plane(0).unwrap().row(0).unwrap(),
        &pixels[..32 * 4]
    );
    drop(lease);
    sink.stop();
    connection.shutdown().unwrap();
}

#[test]
#[ignore = "requires the isolated synthetic server harness"]
fn incremental_damage_becomes_conservative_after_producer_and_receiver_queue_loss() {
    let mut connection = connection();
    let h = connection.handle();
    let format = VideoFormat::rgba(16, 16, 30);
    let mut config = VideoConfig::producer(format);
    config.buffer_frames = 2;
    config.repeat_last_frame = true;
    let source = VideoStream::open(h.clone(), config).unwrap();
    let mut config = VideoConfig::producer(format);
    config.direction = VideoDirection::Capture;
    config.buffer_frames = 3;
    let sink = VideoStream::open(h.clone(), config).unwrap();
    let lease = link(&h, &source, &sink);
    until(|| source.state() == VideoState::Streaming && sink.state() == VideoState::Streaming);
    let damage = VideoRect {
        x: 2,
        y: 3,
        width: 4,
        height: 5,
    };
    let frame = |value| {
        CpuVideoFrame::packed(
            format,
            vec![value; format.byte_len().unwrap()],
            FrameMetadata {
                damage: vec![damage],
                ..Default::default()
            },
        )
        .unwrap()
    };
    source.submit(frame(77)).unwrap();
    // Once continuity is established, a paced producer can preserve partial source damage.
    until(|| {
        sink.receive()
            .unwrap()
            .is_some_and(|f| !f.metadata().discontinuity && f.metadata().damage == vec![damage])
    });
    let pause = source.set_active(false).unwrap();
    until(|| matches!(pause.state(), RequestState::Complete(_)));
    assert_eq!(pause.state(), RequestState::Complete(Ok(())));
    while sink.receive().unwrap().is_some() {}
    let before = source.diagnostics().dropped;
    source.submit(frame(80)).unwrap();
    source.submit(frame(88)).unwrap();
    assert_eq!(source.diagnostics().dropped, before + 1);
    let resume = source.set_active(true).unwrap();
    until(|| matches!(resume.state(), RequestState::Complete(_)));
    assert_eq!(resume.state(), RequestState::Complete(Ok(())));
    let mut delivered = None;
    until(|| {
        if let Some(f) = sink.receive().unwrap() {
            if f.plane(0).unwrap().row(0).unwrap()[0] == 88 {
                delivered = Some(f);
            }
        }
        delivered.is_some()
    });
    let delivered = delivered.unwrap();
    assert!(
        delivered.metadata().discontinuity
            || delivered.metadata().damage
                == vec![VideoRect {
                    x: 0,
                    y: 0,
                    width: 16,
                    height: 16
                }]
    );
    drop(delivered);
    while sink.receive().unwrap().is_some() {}
    let before = sink.diagnostics().dropped;
    // Let the bounded receiver queue evict a predecessor. Its next immutable frame must
    // explicitly invalidate partial damage, even if the native sequence itself had no gap.
    until(|| sink.diagnostics().dropped > before);
    let backlogged = sink.receive().unwrap().unwrap();
    assert!(backlogged.metadata().discontinuity);
    assert_eq!(backlogged.plane(0).unwrap().row(0).unwrap(), &[88; 64]);
    drop(lease);
    source.stop();
    sink.stop();
    connection.shutdown().unwrap();
}
