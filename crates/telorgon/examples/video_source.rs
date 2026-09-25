//! cargo run -p telorgon --no-default-features --features video-linux --example video_source
//! Publishes a synthetic 640x480 RGBA PipeWire camera for 30 seconds. Select it in a
//! PipeWire-aware consumer or link its output explicitly. Does not read any camera/screen.
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};
    use telorgon::{integrations::pipewire::*, media::video::*};
    let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
    let ready = Instant::now() + Duration::from_secs(5);
    while connection.handle().state() != ConnectionState::Ready {
        if Instant::now() > ready {
            return Err("PipeWire connection timed out".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let format = VideoFormat::rgba(640, 480, 30);
    let mut config = VideoConfig::producer(format);
    config.name = "Telorgon synthetic camera".into();
    config.virtual_camera = true;
    let stream = VideoStream::open(connection.handle(), config)?;
    let end = Instant::now() + Duration::from_secs(30);
    let mut announced = false;
    let mut phase = 0u8;
    while Instant::now() < end {
        if let VideoState::Failed(error) = stream.state() {
            return Err(error.into());
        }
        if !announced && let Some(node) = stream.node() {
            println!(
                "Synthetic camera node {}. Waiting for a consumer; exits after 30 seconds.",
                node.id()
            );
            announced = true;
        }
        if stream.state() == VideoState::Streaming
            && let Some((format, _)) = stream.negotiated_format()
        {
            let mut pixels = vec![0; format.byte_len()?];
            for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
                let x = index % format.width as usize;
                let y = index / format.width as usize;
                pixel.copy_from_slice(&[(x as u8).wrapping_add(phase), y as u8, 128, 255]);
            }
            let frame = CpuVideoFrame::packed(format, pixels, FrameMetadata::default())?;
            stream.submit(frame).map_err(|(error, _)| error)?;
            phase = phase.wrapping_add(1);
        }
        std::thread::sleep(format.rate.period().expect("fixed source cadence"));
    }
    println!("{:?}", stream.diagnostics());
    stream.stop();
    connection.shutdown()?;
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This PipeWire example requires Linux.");
}
