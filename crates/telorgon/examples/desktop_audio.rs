//! cargo run -p telorgon --no-default-features --features desktop-audio-linux --example desktop_audio -- list
//! Mutations require an explicit node ID selected from this fresh connection's registry.
#[cfg(target_os = "linux")]
mod linux {
    use std::time::{Duration, Instant};
    use telorgon::{integrations::pipewire::*, services::audio::*};
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
        let handle = connection.handle();
        let controls = AudioControls::new(handle.clone())?;
        let events = handle.subscribe(16, || {})?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while handle.snapshot().state != ConnectionState::Ready {
            if Instant::now() > deadline {
                return Err("audio connection was not ready within five seconds".into());
            }
            while events.try_recv().is_some() {}
            std::thread::sleep(Duration::from_millis(10));
        }
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.first().is_none_or(|s| s == "list") {
            for node in controls.nodes() {
                println!(
                    "{} {} {:?} {:?}",
                    node.handle.id(),
                    node.name,
                    node.channel_volumes,
                    node.mute
                );
            }
        } else {
            let id: u32 = args.get(1).ok_or("provide node ID")?.parse()?;
            let node = controls
                .nodes()
                .into_iter()
                .find(|n| n.handle.id() == id)
                .ok_or("node disappeared")?;
            let request = match args[0].as_str() {
                "mute" => controls.set_mute(node.handle, true)?,
                "unmute" => controls.set_mute(node.handle, false)?,
                "volume" => controls.set_volume(
                    node.handle,
                    Gain::ui(args.get(2).ok_or("provide UI volume 0..1")?.parse()?)?,
                    Amplification::Forbid,
                )?,
                "default" => controls.policy().set_default(
                    telorgon::integrations::wireplumber::DefaultKind::Output,
                    node.handle,
                )?,
                _ => return Err("use list, mute ID, unmute ID, volume ID 0..1, default ID".into()),
            };
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if Instant::now() >= deadline {
                    request.cancel();
                    return Err("audio control request timed out".into());
                }
                match request.state() {
                    RequestState::Complete(result) => {
                        result?;
                        break;
                    }
                    _ => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        }
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
