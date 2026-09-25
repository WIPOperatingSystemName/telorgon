//! --no-default-features --features audio-linux --example audio_routing -- list
//! --no-default-features --features audio-linux --example audio_routing -- SECONDS OUT:IN [OUT:IN ...]
//! Routes only explicitly selected mono-f32 DSP ports. Feedback is rejected. Creation and
//! activation are observed asynchronously; all owned links/filter are removed on exit.
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};
    use telorgon::{integrations::pipewire::*, media::audio::*};
    let args: Vec<_> = std::env::args().skip(1).collect();
    let list = args.first().is_none_or(|arg| arg == "list");
    let seconds = if list { 0 } else { args[0].parse::<u64>()? };
    if !list && (!(1..=600).contains(&seconds) || !(2..=65).contains(&args.len())) {
        return Err("provide 1..600 seconds and 1..64 OUT:IN port pairs".into());
    }
    let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
    let handle = connection.handle();
    let deadline = Instant::now() + Duration::from_secs(5);
    while handle.state() != ConnectionState::Ready {
        if Instant::now() >= deadline {
            return Err("connection readiness timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let snapshot = handle.snapshot();
    if list {
        for port in snapshot
            .objects
            .values()
            .filter(|object| object.kind == ObjectKind::Port)
        {
            if port.properties.get("format.dsp").map(String::as_str)
                != Some("32 bit float mono audio")
            {
                continue;
            }
            println!(
                "{} {} {} node={}",
                port.handle.id(),
                port.properties
                    .get("port.direction")
                    .map(String::as_str)
                    .unwrap_or("?"),
                port.properties
                    .get("port.name")
                    .map(String::as_str)
                    .unwrap_or("?"),
                port.properties
                    .get("node.id")
                    .map(String::as_str)
                    .unwrap_or("?")
            );
        }
        connection.shutdown()?;
        return Ok(());
    }
    let mut routes = Vec::new();
    for pair in &args[1..] {
        let (output, input) = pair.split_once(':').ok_or("port pairs must be OUT:IN")?;
        let output = snapshot
            .objects
            .get(&output.parse::<u32>()?)
            .ok_or("source port disappeared")?
            .handle;
        let input = snapshot
            .objects
            .get(&input.parse::<u32>()?)
            .ok_or("destination port disappeared")?
            .handle;
        routes.push((output, input));
    }
    let mut loopback = AudioLoopback::open(handle, LoopbackConfig::new(routes))?;
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut previous = LoopbackState::Connecting;
    while Instant::now() < end {
        loopback.poll();
        let state = loopback.state();
        if state != previous {
            println!("{state:?}");
            previous = state.clone();
        }
        if let LoopbackState::Failed(error) = state {
            return Err(error.into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    println!("Processed {} graph frames", loopback.frames_processed());
    loopback.stop();
    connection.shutdown()?;
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This example requires Linux PipeWire.");
}
