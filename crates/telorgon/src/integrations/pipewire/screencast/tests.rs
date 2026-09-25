use super::*;
use crate::integrations::pipewire::{
    ObjectKind,
    graph::{Feedback, Graph, LinkState},
};
use std::{
    num::NonZeroU32,
    time::{Duration, Instant},
};

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < deadline, "screen transport timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn ready(source: &VideoStream) -> Option<u32> {
    match source.status() {
        StreamStatus::Ready { node_id } => Some(node_id),
        StreamStatus::Failed(error) => panic!("screen producer failed: {error}"),
        _ => None,
    }
}
fn layout(width: u32, height: u32) -> CaptureLayout {
    CaptureLayout::rgba8(
        NonZeroU32::new(width).unwrap(),
        NonZeroU32::new(height).unwrap(),
        width * 4,
    )
    .unwrap()
}

#[test]
#[ignore = "requires test_pipewire.py --screen-unit; synthetic pixels only"]
fn node_precedes_consumer_and_repeated_pixels_survive_resize_and_shutdown() {
    for pixel in [
        video::PixelFormat::Rgba8,
        video::PixelFormat::Bgra8,
        video::PixelFormat::Rgbx8,
        video::PixelFormat::Bgrx8,
    ] {
        exercise_transport(None, pixel);
    }
}
#[test]
#[ignore = "requires test_pipewire.py --screen-unit; synthetic pixels only"]
fn gpu_enabled_screen_transport_falls_back_to_owned_shared_memory() {
    for pixel in [
        video::PixelFormat::Rgba8,
        video::PixelFormat::Bgra8,
        video::PixelFormat::Rgbx8,
        video::PixelFormat::Bgrx8,
    ] {
        exercise_transport(Some(Box::new(CpuFallback)), pixel);
    }
}
struct CpuFallback;
// SAFETY: this fixture never creates or exposes a GPU allocation. A CPU peer must select shm.
unsafe impl video::VideoGpuProducer for CpuFallback {
    fn formats(&self) -> Vec<video::VideoDmaBufFormat> {
        vec![video::VideoDmaBufFormat {
            pixel: video::PixelFormat::Rgba8,
            modifier: 0,
            planes: 1,
        }]
    }
    fn allocate(
        &mut self,
        _: video::VideoFormat,
        _: u64,
        _: usize,
    ) -> Result<Box<dyn video::VideoGpuOutputBuffer>, crate::integrations::pipewire::MediaError>
    {
        Err(crate::integrations::pipewire::MediaError::Unsupported(
            "synthetic fixture has no GPU",
        ))
    }
}
fn cursor_metadata(visible: bool) -> FrameMetadata {
    FrameMetadata {
        cursor: Some(video::VideoCursor {
            id: 7,
            x: if visible { 3 } else { 0 },
            y: if visible { 4 } else { 0 },
            hotspot_x: 1,
            hotspot_y: 1,
            visible: Some(visible),
            bitmap: visible.then(|| video::CursorBitmap {
                width: 2,
                height: 2,
                rgba: [80, 40, 20, 128].repeat(4),
            }),
        }),
        ..Default::default()
    }
}
fn screen_pixels(format: video::VideoFormat, seed: u8) -> Vec<u8> {
    let mut pixels =
        [seed, seed + 1, seed + 2, 255].repeat((format.width * format.height) as usize);
    crate::graphics::bridges::video_cpu::rgba_to_packed(&mut pixels, format.pixel).unwrap();
    pixels
}
fn expected_row(format: video::VideoFormat, seed: u8) -> Vec<u8> {
    match format.pixel {
        video::PixelFormat::Rgba8 | video::PixelFormat::Rgbx8 => [seed, seed + 1, seed + 2, 255],
        video::PixelFormat::Bgra8 | video::PixelFormat::Bgrx8 => [seed + 2, seed + 1, seed, 255],
        _ => unreachable!(),
    }
    .repeat(format.width as usize)
}
fn exercise_transport(
    producer: Option<Box<dyn video::VideoGpuProducer>>,
    pixel: video::PixelFormat,
) {
    let small = video::VideoFormat {
        pixel,
        ..video::VideoFormat::rgba(32, 24, 120)
    };
    let large = video::VideoFormat {
        pixel,
        ..video::VideoFormat::rgba(64, 48, 120)
    };
    let remote = std::env::var("TELORGON_TEST_REMOTE").expect("use private harness");
    assert!(remote.starts_with("telorgon-test-"));
    assert_eq!(std::env::var("PIPEWIRE_REMOTE").unwrap(), remote);
    let mut source =
        VideoStream::start(901, layout(32, 24), 120, Arc::new(|| {}), producer).unwrap();
    // Portal Start must return a node before a client can connect to that node.
    until(|| ready(&source).is_some());
    let node_id = ready(&source).unwrap();
    assert!(
        source
            .publish(screen_pixels(small, 0x35), small, cursor_metadata(true))
            .unwrap()
            .is_none()
    );
    let mut connection =
        Connection::connect(ConnectionConfig::default(), Remote::Named(remote)).unwrap();
    let h = connection.handle();
    until(|| h.state() == ConnectionState::Ready);
    let mut config = VideoConfig::producer(small);
    config.direction = video::VideoDirection::Capture;
    config.formats.push(large);
    let sink = video::VideoStream::open(h.clone(), config).unwrap();
    let port = |node: u32, direction: &str| {
        h.snapshot()
            .objects_of_kind(ObjectKind::Port)
            .find(|p| {
                p.properties.get("node.id") == Some(&node.to_string())
                    && p.properties
                        .get("port.direction")
                        .is_some_and(|d| d == direction)
            })
            .map(|p| p.handle)
    };
    until(|| {
        port(node_id, "out").is_some() && sink.node().is_some_and(|n| port(n.id(), "in").is_some())
    });
    let link = Graph::new(h.clone())
        .link(
            port(node_id, "out").unwrap(),
            port(sink.node().unwrap().id(), "in").unwrap(),
            Feedback::Reject,
        )
        .unwrap();
    until(|| {
        source.status();
        assert!(
            !matches!(link.state(), LinkState::Failed(_)),
            "{:?}",
            link.state()
        );
        link.state() == LinkState::Active
    });
    let mut received = Vec::new();
    until(|| {
        source.status();
        if let Some(frame) = sink.receive().unwrap() {
            received.push(frame);
        }
        received.len() == 3
    });
    // One submission generates multiple paced frames, with fresh clock timestamps.
    for frame in &received {
        assert_eq!(
            frame.plane(0).unwrap().row(0).unwrap(),
            expected_row(small, 0x35)
        );
    }
    for frame in &received {
        let cursor = frame.metadata().cursor.as_ref().unwrap();
        assert_eq!(
            (
                cursor.id,
                cursor.x,
                cursor.y,
                cursor.hotspot_x,
                cursor.hotspot_y
            ),
            (7, 3, 4, 1, 1)
        );
        assert_eq!(cursor.visible, Some(true));
        assert_eq!(
            cursor.bitmap.as_ref().unwrap().rgba,
            [80, 40, 20, 128].repeat(4)
        );
    }
    assert!(received[2].metadata().timestamp_ns > received[0].metadata().timestamp_ns);
    let held = received.remove(0);
    drop(received);
    let generation = source
        .state
        .borrow()
        .video
        .as_ref()
        .unwrap()
        .negotiated_format()
        .unwrap()
        .1;
    source.resize(layout(64, 48), 120).unwrap();
    until(|| ready(&source).is_some());
    assert_eq!(ready(&source), Some(node_id));
    assert!(
        source
            .state
            .borrow()
            .video
            .as_ref()
            .unwrap()
            .native_generation_retired(generation)
    );
    assert!(
        source
            .publish(screen_pixels(large, 0x76), large, cursor_metadata(false))
            .is_ok()
    );
    let mut resized = None;
    until(|| {
        source.status();
        if let Some(frame) = sink.receive().unwrap() {
            if frame.format() == large
                && frame.plane(0).unwrap().row(0).unwrap() == expected_row(large, 0x76)
            {
                resized = Some(frame);
            }
        }
        resized.is_some()
    });
    let hidden = resized
        .as_ref()
        .unwrap()
        .metadata()
        .cursor
        .as_ref()
        .unwrap();
    assert_eq!(hidden.visible, Some(false));
    assert!(hidden.bitmap.is_none());
    assert_eq!(
        held.plane(0).unwrap().row(0).unwrap(),
        expected_row(small, 0x35)
    );
    // Replacing the retained frame eventually returns the old allocation for shell reuse.
    assert!(
        source
            .publish(screen_pixels(large, 0x91), large, Default::default())
            .is_ok()
    );
    let mut spare = None;
    until(|| {
        spare = source.take_spare();
        spare.is_some()
    });
    assert_eq!(spare.unwrap(), screen_pixels(large, 0x76));
    if pixel == video::PixelFormat::Rgba8 {
        // Equal dimensions do not imply equal byte order. A peer can select a different
        // advertised format without changing the approved source or its node identity.
        let changed = video::VideoFormat {
            pixel: video::PixelFormat::Bgra8,
            ..large
        };
        let request = sink.reconfigure(vec![changed]).unwrap();
        until(|| {
            source.status();
            matches!(request.state(), RequestState::Complete(_))
                && source.negotiation().is_some_and(|n| n.format == changed)
        });
        assert_eq!(request.state(), RequestState::Complete(Ok(())));
        assert_eq!(ready(&source), Some(node_id));
        assert!(
            source
                .publish(screen_pixels(changed, 0xa0), changed, Default::default())
                .is_ok()
        );
        until(|| {
            source.status();
            sink.receive().unwrap().is_some_and(|frame| {
                frame.format() == changed
                    && frame.plane(0).unwrap().row(0).unwrap() == expected_row(changed, 0xa0)
            })
        });
    }
    source.stop();
    until(|| source.retired());
    assert_eq!(source.status(), StreamStatus::Stopped);
    assert_eq!(
        held.plane(0).unwrap().row(0).unwrap(),
        expected_row(small, 0x35)
    );
    assert_eq!(
        resized.unwrap().plane(0).unwrap().row(0).unwrap(),
        expected_row(large, 0x76)
    );
    drop(link);
    sink.stop();
    connection.shutdown().unwrap();
}
