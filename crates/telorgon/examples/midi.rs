//! cargo run -p telorgon --no-default-features --features midi-linux --example midi -- list
//! Use `listen OUTPUT_PORT_ID` to print input events, or `send INPUT_PORT_ID` to play C4.
//! Explicit routing only. Does not open hardware until the user selects a port.
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
    fn wait(mut ready: impl FnMut() -> bool) -> Result<(), Box<dyn std::error::Error>> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready() {
            if Instant::now() >= deadline {
                return Err("MIDI operation timed out".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
        let h = connection.handle();
        wait(|| h.state() == ConnectionState::Ready)?;
        let barrier = h.barrier()?;
        wait(|| matches!(barrier.state(), RequestState::Complete(_)))?;
        if let RequestState::Complete(result) = barrier.state() {
            result?;
        }
        if args.is_empty() || args[0] == "list" {
            for port in discover(&h) {
                println!(
                    "{} {:?} {:?}: {}",
                    port.handle.id(),
                    port.direction,
                    port.representation,
                    port.name
                );
            }
            connection.shutdown()?;
            return Ok(());
        }
        let sending = match args[0].as_str() {
            "send" => true,
            "listen" => false,
            _ => {
                return Err("usage: midi list | listen OUTPUT_PORT_ID | send INPUT_PORT_ID".into());
            }
        };
        let id: u32 = args.get(1).ok_or("missing port ID")?.parse()?;
        let remote = discover(&h)
            .into_iter()
            .find(|p| p.handle.id() == id)
            .ok_or("MIDI port not found")?;
        let direction = if sending {
            MidiDirection::Output
        } else {
            MidiDirection::Input
        };
        if remote.direction == direction {
            return Err("selected port has the wrong direction".into());
        }
        let mut stream = MidiStream::open(
            h.clone(),
            MidiConfig::new("telorgon.example.midi", direction, remote.representation),
        )?;
        let own_port = || {
            stream.node().and_then(|node| {
                discover(&h)
                    .into_iter()
                    .find(|p| p.node == Some(node))
                    .map(|p| p.handle)
            })
        };
        wait(|| own_port().is_some())?;
        let (output, input) = if sending {
            (own_port().unwrap(), remote.handle)
        } else {
            (remote.handle, own_port().unwrap())
        };
        let link = Graph::new(h.clone()).link(output, input, Feedback::Reject)?;
        wait(|| link.state() == LinkState::Active && stream.clock().is_some())?;
        if sending {
            let now = stream.clock().ok_or("clock unavailable")?;
            for (delay_ms, status) in [(100, 0x90u8), (600, 0x80u8)] {
                let packet = match remote.representation {
                    MidiRepresentation::Midi1 => MidiPacket::midi1(&[status, 60, 100])?,
                    MidiRepresentation::Ump => MidiPacket::ump_words(&[0x20000000
                        | ((status as u32) << 16)
                        | (60 << 8)
                        | 100])?,
                };
                let time = now.after(Duration::from_millis(delay_ms))?;
                stream
                    .send(MidiEvent { time, packet })
                    .map_err(|(error, _)| error)?;
            }
            wait(|| stream.diagnostics().delivered >= 2)?;
        } else {
            println!("Listening for 10 seconds; positions are graph samples, not wall-clock time.");
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                while let Some(event) = stream.receive()? {
                    println!(
                        "clock {} @ {}: {:02x?}",
                        event.time.clock_id,
                        event.time.position,
                        event.packet.bytes()
                    );
                }
                if matches!(stream.state(), MidiState::Failed(_) | MidiState::Stopped) {
                    return Err("MIDI stream disconnected".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        println!("{:?}", stream.diagnostics());
        drop(link);
        stream.stop();
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
