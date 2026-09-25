//! Run with --no-default-features --features audio-linux --example audio_record -- ARGS
//! list | node ID SECONDS NEW_DIRECTORY | system ID SECONDS NEW_DIRECTORY |
//! application APP_ID SECONDS NEW_DIRECTORY
//! Recording starts only with an explicit target. Application mode follows changing output
//! streams, preserving each incarnation in a separate WAV plus timestamp CSV (no mixing).
#[cfg(target_os = "linux")]
#[path = "support/pcm_wave.rs"]
mod pcm_wave;
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        collections::BTreeMap,
        fs::OpenOptions,
        io::Write,
        time::{Duration, Instant},
    };
    use telorgon::{integrations::pipewire::*, media::audio::*};
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("list");
    if !matches!(mode, "list" | "node" | "system" | "application") {
        return Err("use list, node, system or application".into());
    }
    let recording = mode != "list";
    let mut seconds = 0;
    if recording {
        if args.len() != 4 {
            return Err("provide TARGET SECONDS NEW_DIRECTORY".into());
        }
        seconds = args[2].parse::<u64>()?;
        if !(1..=600).contains(&seconds) {
            return Err("duration must be 1..600 seconds".into());
        }
    }
    let node = if matches!(mode, "node" | "system") {
        Some(args[1].parse::<u32>()?)
    } else {
        None
    };
    let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
    let handle = connection.handle();
    let deadline = Instant::now() + Duration::from_secs(5);
    while handle.state() != ConnectionState::Ready {
        if Instant::now() >= deadline {
            return Err("PipeWire readiness timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if !recording {
        for object in handle
            .snapshot()
            .objects
            .values()
            .filter(|object| object.handle.id() > 0 && object.kind == ObjectKind::Node)
        {
            println!(
                "{} {:?} {} app={}",
                object.handle.id(),
                object.media_class(),
                object
                    .properties
                    .get("node.name")
                    .map(String::as_str)
                    .unwrap_or(""),
                object
                    .properties
                    .get("application.id")
                    .map(String::as_str)
                    .unwrap_or("")
            );
        }
        connection.shutdown()?;
        return Ok(());
    }
    let directory = std::path::Path::new(&args[3]);
    std::fs::create_dir(directory)?;
    struct Track {
        stream: AudioStream,
        wave: pcm_wave::Wave,
        timing: std::fs::File,
    }
    impl Track {
        fn drain(
            &mut self,
            samples: &mut [f32],
            limit: usize,
        ) -> Result<(), Box<dyn std::error::Error>> {
            for _ in 0..limit {
                let Some(block) = self.stream.read_timestamped(samples)? else {
                    break;
                };
                self.wave.write(&samples[..block.frames * 2])?;
                writeln!(
                    self.timing,
                    "{},{},{},{},{},{}",
                    block.frame_position().ok_or("frame position overflow")?,
                    block.frames,
                    block.timing.graph_ticks,
                    block.timing.monotonic_ns,
                    block.offset_frames,
                    block.discontinuity
                )?;
            }
            Ok(())
        }
    }
    let mut tracks = BTreeMap::<ObjectHandle, Track>::new();
    let mut created = 0;
    let mut selected_node: Option<ObjectHandle> = None;
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut scan = Instant::now();
    let mut samples = vec![0.0; 8192 * 2];
    while Instant::now() < end {
        if handle.state() != ConnectionState::Ready {
            return Err("PipeWire disconnected".into());
        }
        if Instant::now() >= scan {
            let snapshot = handle.snapshot();
            let targets: Vec<_> = snapshot
                .objects
                .values()
                .filter(|object| object.kind == ObjectKind::Node)
                .filter(|object| match mode {
                    "node" => {
                        Some(object.handle.id()) == node
                            && object.media_class() == Some("Audio/Source")
                    }
                    "system" => {
                        Some(object.handle.id()) == node
                            && object.media_class() == Some("Audio/Sink")
                    }
                    _ => {
                        object.media_class() == Some("Stream/Output/Audio")
                            && object.properties.get("application.id") == Some(&args[1])
                    }
                })
                .filter(|object| {
                    mode == "application"
                        || selected_node.is_none_or(|selected| selected == object.handle)
                })
                .map(|object| object.handle)
                .collect();
            let removed: Vec<_> = tracks
                .keys()
                .filter(|id| !targets.contains(id))
                .copied()
                .collect();
            for id in removed {
                if let Some(mut track) = tracks.remove(&id) {
                    track.stream.stop();
                    track.drain(&mut samples, 8192)?;
                    track.wave.finish()?;
                    track.timing.flush()?;
                }
            }
            for id in targets {
                if mode != "application" {
                    selected_node = Some(id);
                }
                if tracks.contains_key(&id) {
                    continue;
                }
                if tracks.len() >= 8 || created >= 64 {
                    return Err("recording limit: 8 live / 64 total tracks".into());
                }
                let target = match mode {
                    "system" => AudioTarget::SystemOutput(id),
                    "application" => AudioTarget::Application(id),
                    _ => AudioTarget::Node(id),
                };
                let stream = AudioStream::timestamped_capture(
                    handle.clone(),
                    AudioConfig {
                        direction: AudioDirection::Capture,
                        target,
                        ..Default::default()
                    },
                )?;
                let name = format!("track-{created:03}-node-{}", id.id());
                created += 1;
                let wave =
                    pcm_wave::Wave::create(&directory.join(format!("{name}.wav")), 48000, 2)?;
                let mut timing = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(directory.join(format!("{name}.csv")))?;
                writeln!(
                    timing,
                    "frame_position,frames,graph_ticks,monotonic_ns,offset_frames,discontinuity"
                )?;
                tracks.insert(
                    id,
                    Track {
                        stream,
                        wave,
                        timing,
                    },
                );
            }
            scan = Instant::now() + Duration::from_millis(100);
        }
        for (id, track) in &mut tracks {
            if let AudioState::Failed(error) = track.stream.state() {
                if handle.snapshot().resolve(*id).is_err() {
                    continue;
                }
                return Err(error.into());
            }
            track.drain(&mut samples, 16)?;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    for track in tracks.values() {
        track.stream.stop();
    }
    connection.shutdown()?;
    for mut track in tracks.into_values() {
        track.drain(&mut samples, 8192)?;
        track.wave.finish()?;
        track.timing.flush()?;
    }
    if created == 0 {
        return Err("no matching capture source appeared".into());
    }
    println!(
        "Recorded {created} tracks. CSV discontinuities identify dropped source intervals; WAV files contain delivered samples only."
    );
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This example requires Linux PipeWire.");
}
