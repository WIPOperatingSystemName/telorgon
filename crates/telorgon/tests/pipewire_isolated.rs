#![cfg(all(target_os = "linux", feature = "desktop-audio-linux"))]
//! Run only with tools/sdk/test_pipewire.py; never connects to the user's default remote.
use std::time::{Duration, Instant};
use telorgon::{integrations::pipewire::*, services::audio::*};
#[track_caller]
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for isolated server"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[track_caller]
fn complete(request: Request) {
    until(|| matches!(request.state(), RequestState::Complete(_)));
    assert_eq!(request.state(), RequestState::Complete(Ok(())));
}
#[test]
#[ignore = "requires the explicit isolated synthetic server harness"]
fn registry_controls_shutdown_reconnection_and_portal_transport() {
    let remote = std::env::var("TELORGON_TEST_REMOTE").expect("use tools/sdk/test_pipewire.py");
    assert!(remote.starts_with("telorgon-test-"));
    let mut connection =
        Connection::connect(ConnectionConfig::default(), Remote::Named(remote.clone())).unwrap();
    let handle = connection.handle();
    let controls = AudioControls::new(handle.clone()).unwrap();
    let mut last_print = Instant::now();
    until(|| {
        if last_print.elapsed() > Duration::from_secs(2) {
            eprintln!("snapshot: {:#?}", handle.snapshot());
            last_print = Instant::now();
        }
        if matches!(handle.snapshot().state, ConnectionState::Failed(_)) {
            panic!("connection failed: {:?}", handle.snapshot());
        }
        handle.snapshot().state == ConnectionState::Ready
            && controls.nodes().iter().any(|n| {
                n.name == "telorgon.test.sink" && n.can_set_volume && n.channel_volumes.len() == 2
            })
    });
    let node = controls
        .nodes()
        .into_iter()
        .find(|n| n.name == "telorgon.test.sink")
        .unwrap();
    complete(
        controls
            .set_channels(
                node.handle,
                &[Gain::linear(0.2).unwrap(), Gain::linear(0.4).unwrap()],
                Amplification::Forbid,
            )
            .unwrap(),
    );
    complete(controls.set_mute(node.handle, true).unwrap());
    assert!(
        controls
            .nodes()
            .into_iter()
            .find(|n| n.handle == node.handle)
            .unwrap()
            .mute
            .unwrap()
    );
    complete(
        controls
            .set_volume(
                node.handle,
                Gain::linear(0.8).unwrap(),
                Amplification::Forbid,
            )
            .unwrap(),
    );
    let n = controls
        .nodes()
        .into_iter()
        .find(|n| n.handle == node.handle)
        .unwrap();
    assert!((n.channel_volumes[0].value() - 0.4).abs() < 0.0001);
    assert!((n.channel_volumes[1].value() - 0.8).abs() < 0.0001);
    complete(handle.barrier().unwrap());
    connection.shutdown().unwrap();
    assert!(handle.snapshot().objects.is_empty());
    assert!(matches!(
        controls.set_mute(node.handle, false),
        Err(MediaError::Disconnected)
    ));
    connection.reconnect(Remote::Named(remote.clone())).unwrap();
    let next = connection.handle();
    until(|| next.snapshot().state == ConnectionState::Ready);
    assert_eq!(
        next.snapshot().resolve(node.handle).unwrap_err(),
        MediaError::StaleHandle
    );
    connection.shutdown().unwrap();
    // An already connected socket verifies FD ownership transport. Authorization itself is
    // provided by a portal in production; this test makes no claim about portal permission UI.
    let path = std::path::Path::new(&std::env::var("PIPEWIRE_RUNTIME_DIR").unwrap()).join(remote);
    let socket = std::os::unix::net::UnixStream::connect(path).unwrap();
    let mut restricted =
        Connection::connect(ConnectionConfig::default(), Remote::Portal(socket.into())).unwrap();
    assert!(matches!(
        AudioControls::new(restricted.handle()),
        Err(MediaError::PermissionDenied)
    ));
    until(|| restricted.handle().snapshot().state == ConnectionState::Ready);
    restricted.shutdown().unwrap();
}

#[cfg(feature = "audio-linux")]
#[test]
#[ignore = "requires the explicit isolated synthetic server harness"]
fn pcm_playback_capture_and_owned_links() {
    use telorgon::integrations::pipewire::graph::{Feedback, Graph};
    use telorgon::media::audio::*;
    let remote = std::env::var("TELORGON_TEST_REMOTE").expect("use test harness");
    assert!(remote.starts_with("telorgon-test-"));
    let mut connection =
        Connection::connect(ConnectionConfig::default(), Remote::Named(remote)).unwrap();
    let h = connection.handle();
    until(|| h.snapshot().state == ConnectionState::Ready);
    let sink = h
        .snapshot()
        .objects
        .values()
        .find(|o| {
            o.properties
                .get("node.name")
                .is_some_and(|n| n == "telorgon.test.sink")
        })
        .unwrap()
        .handle;
    let mut playback = AudioStream::buffered(
        h.clone(),
        AudioConfig {
            name: "telorgon.test.playback".into(),
            target: AudioTarget::Node(sink),
            ..Default::default()
        },
    )
    .unwrap();
    playback.write(&vec![0.25; 8192]).unwrap();
    let mut last_print = Instant::now();
    until(|| {
        if last_print.elapsed() > Duration::from_secs(2) {
            eprintln!("stream {:?}; nodes {:?}", playback.state(), h.snapshot());
            last_print = Instant::now();
        }
        h.snapshot().objects.values().any(|o| {
            o.kind == ObjectKind::Port
                && o.properties
                    .get("port.direction")
                    .is_some_and(|s| s == "out")
                && o.properties
                    .get("port.name")
                    .is_some_and(|s| s.starts_with("output"))
        })
    });
    let snapshot = h.snapshot();
    let node = snapshot
        .objects
        .values()
        .find(|o| {
            o.properties
                .get("node.name")
                .is_some_and(|n| n == "telorgon.test.playback")
        })
        .unwrap()
        .handle;
    let ports = |id: u32, direction: &str| {
        snapshot
            .objects
            .values()
            .filter(|o| {
                o.kind == ObjectKind::Port
                    && o.properties.get("node.id") == Some(&id.to_string())
                    && o.properties
                        .get("port.direction")
                        .is_some_and(|s| s == direction)
            })
            .map(|o| o.handle)
            .collect::<Vec<_>>()
    };
    let out = ports(node.id(), "out");
    let input = ports(sink.id(), "in");
    assert_eq!(out.len(), input.len());
    let graph = Graph::new(h.clone());
    let links = out
        .into_iter()
        .zip(input)
        .map(|(a, b)| graph.link(a, b, Feedback::Reject).unwrap())
        .collect::<Vec<_>>();
    let mut last = Instant::now();
    until(|| {
        if last.elapsed() > Duration::from_secs(2) {
            eprintln!(
                "processing state {:?}, diag {:?}, links {:?}",
                playback.state(),
                playback.diagnostics(),
                links.iter().map(|l| l.state()).collect::<Vec<_>>()
            );
            last = Instant::now();
        }
        playback.diagnostics().frames > 0
    });
    let mut capture = AudioStream::buffered(
        h.clone(),
        AudioConfig {
            name: "telorgon.test.capture".into(),
            direction: AudioDirection::Capture,
            target: AudioTarget::Application(node),
            ..Default::default()
        },
    )
    .unwrap();
    until(|| {
        h.snapshot().objects.values().any(|o| {
            o.kind == ObjectKind::Port
                && o.properties
                    .get("port.name")
                    .is_some_and(|s| s.starts_with("input"))
        })
    });
    let snapshot = h.snapshot();
    let capture_node = snapshot
        .objects
        .values()
        .find(|o| {
            o.properties
                .get("node.name")
                .is_some_and(|n| n == "telorgon.test.capture")
        })
        .unwrap()
        .handle;
    let ports_for = |id: u32, direction: &str| {
        snapshot
            .objects
            .values()
            .filter(|o| {
                o.kind == ObjectKind::Port
                    && o.properties.get("node.id") == Some(&id.to_string())
                    && o.properties
                        .get("port.direction")
                        .is_some_and(|s| s == direction)
            })
            .map(|o| o.handle)
            .collect::<Vec<_>>()
    };
    let capture_links = ports_for(node.id(), "out")
        .into_iter()
        .zip(ports_for(capture_node.id(), "in"))
        .map(|(a, b)| graph.link(a, b, Feedback::Reject).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(capture_links.len(), 2);
    let mut recorded = vec![0.0; 8192];
    until(|| {
        let _ = playback.write(&vec![0.25; 2048]);
        let frames = capture.read(&mut recorded).unwrap();
        recorded[..frames * 2]
            .iter()
            .any(|s| (*s - 0.25).abs() < 0.001)
    });
    drop(capture_links);
    capture.stop();
    eprintln!("pause");
    let pause = playback.pause().unwrap();
    until(|| matches!(pause.state(), RequestState::Complete(_)));
    assert_eq!(
        pause.state(),
        RequestState::Complete(Ok(())),
        "playback {:?}; connection {:?}",
        playback.state(),
        h.state()
    );
    eprintln!("resume");
    complete(playback.resume().unwrap());
    eprintln!("flush");
    complete(playback.flush().unwrap());
    playback.write(&vec![0.1; 4096]).unwrap();
    eprintln!("drain");
    complete(playback.drain().unwrap());
    assert!(playback.diagnostics().frames >= 2048);
    drop(links);
    playback.stop();
    until(|| matches!(playback.state(), AudioState::Stopped));
    complete(h.barrier().unwrap());
    assert_eq!(h.snapshot().diagnostics.protocol_errors, 0);
    connection.shutdown().unwrap();
}

#[cfg(feature = "audio-linux")]
#[test]
#[ignore = "requires the explicit isolated synthetic server harness"]
fn multiport_filter_processes_virtual_source_and_sink_with_quantum_changes() {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };
    use telorgon::integrations::pipewire::graph::{Feedback, Graph};
    use telorgon::media::audio::*;
    let remote = std::env::var("TELORGON_TEST_REMOTE").expect("use test harness");
    assert!(remote.starts_with("telorgon-test-"));
    let mut connection =
        Connection::connect(ConnectionConfig::default(), Remote::Named(remote)).unwrap();
    let h = connection.handle();
    until(|| h.state() == ConnectionState::Ready);
    let source = AudioStream::realtime(
        h.clone(),
        AudioConfig {
            name: "telorgon.test.virtual-source".into(),
            role: AudioRole::VirtualSource,
            ..Default::default()
        },
        |cycle: AudioCycle<'_>| {
            for frame in cycle.samples.chunks_exact_mut(2) {
                frame[0] = 0.4;
                frame[1] = -0.2;
            }
        },
    )
    .unwrap();
    let mut sink = AudioStream::buffered(
        h.clone(),
        AudioConfig {
            name: "telorgon.test.virtual-sink".into(),
            role: AudioRole::VirtualSink,
            direction: AudioDirection::Capture,
            ..Default::default()
        },
    )
    .unwrap();
    let quantum = Arc::new(AtomicU32::new(0));
    let rate = Arc::new(AtomicU32::new(0));
    let q = quantum.clone();
    let r = rate.clone();
    let filter = AudioFilter::open(
        h.clone(),
        FilterConfig::stereo("telorgon.test.filter"),
        move |mut cycle: FilterCycle<'_>| {
            q.store(cycle.clock.frames as u32, Ordering::Relaxed);
            r.store(cycle.clock.rate_denom, Ordering::Relaxed);
            for channel in 0..2 {
                for (input, output) in cycle
                    .inputs
                    .channel(channel)
                    .unwrap()
                    .iter()
                    .zip(cycle.outputs.channel(channel).unwrap())
                {
                    *output = input * 0.5;
                }
            }
        },
    )
    .unwrap();
    let node = |name: &str| {
        h.snapshot()
            .objects
            .values()
            .find(|o| o.properties.get("node.name").is_some_and(|n| n == name))
            .map(|o| o.handle)
    };
    let ports = |id: u32, direction: &str| {
        h.snapshot()
            .objects
            .values()
            .filter(|o| {
                o.kind == ObjectKind::Port
                    && o.properties.get("node.id") == Some(&id.to_string())
                    && o.properties
                        .get("port.direction")
                        .is_some_and(|s| s == direction)
            })
            .map(|o| o.handle)
            .collect::<Vec<_>>()
    };
    until(|| {
        node("telorgon.test.virtual-source").is_some_and(|n| ports(n.id(), "out").len() == 2)
            && node("telorgon.test.virtual-sink").is_some_and(|n| ports(n.id(), "in").len() == 2)
            && filter
                .node()
                .is_some_and(|n| ports(n.id(), "out").len() == 2 && ports(n.id(), "in").len() == 2)
    });
    let graph = Graph::new(h.clone());
    let source_node = node("telorgon.test.virtual-source").unwrap();
    let sink_node = node("telorgon.test.virtual-sink").unwrap();
    let filter_node = filter.node().unwrap();
    let mut links = Vec::new();
    for (a, b) in ports(source_node.id(), "out")
        .into_iter()
        .zip(ports(filter_node.id(), "in"))
    {
        links.push(graph.link(a, b, Feedback::Reject).unwrap());
    }
    for (a, b) in ports(filter_node.id(), "out")
        .into_iter()
        .zip(ports(sink_node.id(), "in"))
    {
        links.push(graph.link(a, b, Feedback::Reject).unwrap());
    }
    let mut samples = vec![0.0; 4096];
    until(|| {
        let frames = sink.read(&mut samples).unwrap();
        samples[..frames * 2]
            .chunks_exact(2)
            .any(|s| (s[0] - 0.2).abs() < 0.001 && (s[1] + 0.1).abs() < 0.001)
    });
    assert!(filter.frames_processed() > 0);
    let status = std::process::Command::new("pw-metadata")
        .args(["-n", "settings", "0", "clock.force-quantum", "128"])
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    until(|| quantum.load(Ordering::Relaxed) == 128);
    let status = std::process::Command::new("pw-metadata")
        .args(["-n", "settings", "0", "clock.force-rate", "44100"])
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    until(|| rate.load(Ordering::Relaxed) == 44100);
    assert!(filter.discontinuities() > 1);
    complete(filter.set_active(false).unwrap());
    complete(filter.set_active(true).unwrap());
    drop(links);
    source.stop();
    sink.stop();
    filter.stop();
    until(|| filter.state() == FilterState::Stopped);
    connection.shutdown().unwrap();
}

#[cfg(feature = "midi-linux")]
#[test]
#[ignore = "requires the explicit isolated synthetic server harness"]
fn midi_messages_preserve_graph_timestamps_and_queue_bounds() {
    use telorgon::integrations::pipewire::graph::{Feedback, Graph, LinkState};
    use telorgon::media::midi::*;
    let remote = std::env::var("TELORGON_TEST_REMOTE").expect("use test harness");
    assert!(remote.starts_with("telorgon-test-"));
    let mut connection =
        Connection::connect(ConnectionConfig::default(), Remote::Named(remote)).unwrap();
    let h = connection.handle();
    until(|| h.state() == ConnectionState::Ready);
    for representation in [MidiRepresentation::Midi1, MidiRepresentation::Ump] {
        let mut config = MidiConfig::new(
            "telorgon.test.midi-source",
            MidiDirection::Output,
            representation,
        );
        config.queue_events = 2;
        let mut source = MidiStream::open(h.clone(), config).unwrap();
        let mut input_config = MidiConfig::new(
            "telorgon.test.midi-sink",
            MidiDirection::Input,
            representation,
        );
        input_config.queue_events = 1;
        let mut sink = MidiStream::open(h.clone(), input_config).unwrap();
        let port = |node: Option<ObjectHandle>| {
            node.and_then(|node| {
                discover(&h)
                    .into_iter()
                    .find(|p| p.node == Some(node))
                    .map(|p| p.handle)
            })
        };
        until(|| port(source.node()).is_some() && port(sink.node()).is_some());
        let link = Graph::new(h.clone())
            .link(
                port(source.node()).unwrap(),
                port(sink.node()).unwrap(),
                Feedback::Reject,
            )
            .unwrap();
        until(|| link.state() == LinkState::Active && source.clock().is_some());
        let packet = match representation {
            MidiRepresentation::Midi1 => MidiPacket::midi1(&[0x90, 60, 100]).unwrap(),
            MidiRepresentation::Ump => MidiPacket::ump_words(&[0x40903c00, 0xffff0000]).unwrap(),
        };
        let mut time = source.clock().unwrap();
        time.position += 12000 + 17;
        source
            .send(MidiEvent {
                time,
                packet: packet.clone(),
            })
            .unwrap();
        let mut received = None;
        until(|| {
            received = sink.receive().unwrap();
            received.is_some()
        });
        let event = received.unwrap();
        assert_eq!(event.packet, packet);
        assert_eq!(event.time.position, time.position);
        assert_eq!(event.time.clock_id, time.clock_id);
        assert_eq!(source.diagnostics().late, 0);
        assert_eq!(sink.diagnostics().malformed, 0);
        let mut batch_time = source.clock().unwrap();
        batch_time.position += 12000;
        for _ in 0..2 {
            source
                .send(MidiEvent {
                    time: batch_time,
                    packet: packet.clone(),
                })
                .unwrap();
        }
        until(|| source.diagnostics().delivered >= 3 && sink.diagnostics().overflows > 0);
        assert_eq!(sink.receive().unwrap().unwrap().packet, packet);
        assert!(sink.receive().unwrap().is_none());
        // Full-size SysEx is a single owned message; never split or borrow native storage.
        if representation == MidiRepresentation::Midi1 {
            let mut bytes = [0; MAX_SYSEX_BYTES];
            bytes[0] = 0xf0;
            bytes[MAX_SYSEX_BYTES - 1] = 0xf7;
            let sysex = MidiPacket::midi1(&bytes).unwrap();
            let mut time = source.clock().unwrap();
            time.position += 12000;
            source
                .send(MidiEvent {
                    time,
                    packet: sysex.clone(),
                })
                .unwrap();
            let mut received = None;
            until(|| {
                received = sink.receive().unwrap();
                received.is_some()
            });
            assert_eq!(received.unwrap().packet, sysex);
        }
        // A far-future event retains the head; the native side can hold at most one extra
        // event beyond the configured queue. No overwrite or unbounded staging is allowed.
        let mut future = source.clock().unwrap();
        future.position += 1_000_000;
        let mut accepted = 0;
        let mut rejected = None;
        for _ in 0..16 {
            match source.send(MidiEvent {
                time: future,
                packet: packet.clone(),
            }) {
                Ok(()) => accepted += 1,
                Err(error) => {
                    rejected = Some(error);
                    break;
                }
            }
        }
        assert!((2..=3).contains(&accepted));
        let (error, event) = rejected.expect("bounded output queue");
        assert_eq!(error, MediaError::QueueFull);
        assert_eq!(event.packet, packet);
        assert_eq!(source.diagnostics().overflows, 1);
        drop(link);
        source.stop();
        sink.stop();
        until(|| source.state() == MidiState::Stopped && sink.state() == MidiState::Stopped);
    }
    complete(h.barrier().unwrap());
    assert_eq!(h.snapshot().diagnostics.protocol_errors, 0);
    connection.shutdown().unwrap();
}
