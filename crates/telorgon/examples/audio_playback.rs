//! cargo run -p telorgon --no-default-features --features audio-linux --example audio_playback -- sound.wav
//! Plays only the explicitly supplied file. Optional application-owned gain/mute never
//! creates desktop controls. GUI owners use the same SoundPlayback and stop it on unmount.
#[cfg(target_os = "linux")]
mod linux {
    use std::time::{Duration, Instant};
    use telorgon::{integrations::pipewire::*, media::audio::*};
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let path = std::env::args_os()
            .nth(1)
            .ok_or("provide a PCM WAVE file")?;
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > 64 * 1024 * 1024 {
            return Err("sound exceeds 64 MiB".into());
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(file, 64 * 1024 * 1024 + 1),
            &mut bytes,
        )?;
        let asset = SoundAsset::decode_wav(&bytes)?;
        let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
        let h = connection.handle();
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.state() != ConnectionState::Ready {
            if Instant::now() >= deadline {
                return Err(format!("connection: {:?}", h.state()).into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let playback = asset.prepare(h, AudioTarget::Default, false)?;
        playback.stream().set_gain(0.5)?;
        let request = playback.stream().resume()?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let RequestState::Complete(result) = request.state() {
                result?;
                break;
            }
            if Instant::now() >= deadline {
                return Err("playback start timed out".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let seconds = asset.samples().len() as f64 / asset.channels() as f64 / asset.rate() as f64;
        let deadline = Instant::now() + Duration::from_secs_f64(seconds + 5.0);
        while !playback.is_source_exhausted() {
            match playback.stream().state() {
                AudioState::Failed(error) => return Err(error.into()),
                AudioState::Stopped => {
                    return Err("playback stopped before the source ended".into());
                }
                _ => {}
            }
            if Instant::now() >= deadline {
                return Err("playback made insufficient progress".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let latency =
            playback.stream().diagnostics().latency_frames.max(0) as f64 / asset.rate() as f64;
        std::thread::sleep(Duration::from_secs_f64((latency + 0.05).min(5.0)));
        drop(playback);
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
