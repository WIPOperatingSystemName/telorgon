//! cargo run -p telorgon --no-default-features --features midi-linux --example midi_forward -- OUTPUT_PORT_ID INPUT_PORT_ID SECONDS
//! Forward unchanged MIDI packets for an explicit duration (1..600 seconds), adding
//! 20 ms of scheduling delay. Select ports with the `midi list` example. No implicit
//! reconnect or representation conversion. The software forwarding path is not a
//! PipeWire graph link: choose a route that does not feed back to its own source.
#[cfg(target_os = "linux")]
mod linux {
    use std::time::{Duration, Instant};
    use telorgon::{
        integrations::pipewire::{
            graph::{Feedback, Graph, LinkState},
            *,
        },
        media::midi::*,
    };
    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

    fn wait(mut ready: impl FnMut() -> bool) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready() {
            if Instant::now() >= deadline {
                return Err("MIDI setup timed out".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
    fn port(connection: &ConnectionHandle, stream: &MidiStream) -> Option<ObjectHandle> {
        let node = stream.node()?;
        discover(connection)
            .into_iter()
            .find(|port| port.node == Some(node))
            .map(|port| port.handle)
    }
    pub fn run() -> Result<()> {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        if args.len() != 3 {
            return Err("usage: midi_forward OUTPUT_PORT_ID INPUT_PORT_ID SECONDS".into());
        }
        let source_id: u32 = args[0].parse()?;
        let destination_id: u32 = args[1].parse()?;
        let seconds: u64 = args[2].parse()?;
        if !(1..=600).contains(&seconds) {
            return Err("duration must be 1..600 seconds".into());
        }
        let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
        let handle = connection.handle();
        wait(|| handle.state() == ConnectionState::Ready)?;
        let barrier = handle.barrier()?;
        wait(|| matches!(barrier.state(), RequestState::Complete(_)))?;
        if let RequestState::Complete(result) = barrier.state() {
            result?;
        }
        let ports = discover(&handle);
        let source = ports
            .iter()
            .find(|p| p.handle.id() == source_id && p.direction == MidiDirection::Output)
            .ok_or("source output port not found")?;
        let destination = ports
            .iter()
            .find(|p| p.handle.id() == destination_id && p.direction == MidiDirection::Input)
            .ok_or("destination input port not found")?;
        if source.representation != destination.representation {
            return Err("representations must match".into());
        }
        if source.node.is_some() && source.node == destination.node {
            return Err("this example rejects forwarding back into the source node".into());
        }
        let mut input = MidiStream::open(
            handle.clone(),
            MidiConfig::new(
                "telorgon.forward.input",
                MidiDirection::Input,
                source.representation,
            ),
        )?;
        let mut output = MidiStream::open(
            handle.clone(),
            MidiConfig::new(
                "telorgon.forward.output",
                MidiDirection::Output,
                source.representation,
            ),
        )?;
        wait(|| port(&handle, &input).is_some() && port(&handle, &output).is_some())?;
        let graph = Graph::new(handle.clone());
        let incoming = graph.link(
            source.handle,
            port(&handle, &input).ok_or("input disappeared")?,
            Feedback::Reject,
        )?;
        let outgoing = graph.link(
            port(&handle, &output).ok_or("output disappeared")?,
            destination.handle,
            Feedback::Reject,
        )?;
        wait(|| {
            incoming.state() == LinkState::Active
                && outgoing.state() == LinkState::Active
                && input.clock().is_some()
                && output.clock().is_some()
        })?;
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut pending = None;
        let mut forwarded = 0u64;
        let mut discarded = 0u64;
        while Instant::now() < deadline {
            for stream in [&input, &output] {
                match stream.state() {
                    MidiState::Failed(error) => return Err(error.into()),
                    MidiState::Stopped => return Err("MIDI stream stopped".into()),
                    _ => {}
                }
            }
            if handle.snapshot().resolve(source.handle).is_err()
                || handle.snapshot().resolve(destination.handle).is_err()
            {
                return Err("selected MIDI port was removed".into());
            }
            if let (Some(source_clock), Some(destination_clock)) = (input.clock(), output.clock()) {
                // At most one retained event outside the two bounded native queues.
                // Rebase only on dequeue; queue pressure must not continually postpone it.
                for _ in 0..64 {
                    if pending.is_none() {
                        let Some(mut event) = input.receive()? else {
                            break;
                        };
                        if event.time.generation != source_clock.generation {
                            discarded += 1;
                            continue;
                        }
                        event.time = event
                            .time
                            .rebase(destination_clock, Duration::from_millis(20))?;
                        pending = Some(event);
                    }
                    let event = pending.take().expect("pending event");
                    match output.send(event) {
                        Ok(()) => forwarded += 1,
                        Err((MediaError::QueueFull | MediaError::NotReady, event)) => {
                            pending = Some(event);
                            break;
                        }
                        Err((MediaError::StaleHandle, _)) => discarded += 1,
                        Err((error, _)) => return Err(error.into()),
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Acceptance and native delivery are reported separately; stopping can discard
        // packets still queued at the requested duration boundary.
        println!(
            "accepted={forwarded}, discarded={discarded}, pending={}",
            pending.is_some()
        );
        println!(
            "input={:?}, output={:?}",
            input.diagnostics(),
            output.diagnostics()
        );
        drop(incoming);
        drop(outgoing);
        input.stop();
        output.stop();
        connection.shutdown()?;
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    linux::run()
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This example requires Linux.");
}
