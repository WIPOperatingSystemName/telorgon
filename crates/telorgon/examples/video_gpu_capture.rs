//! cargo run -p telorgon --no-default-features --features video-linux,embedded-vulkan --example video_gpu_capture -- --node NODE_ID
//! Explicitly capture a selected PipeWire source for 15 seconds. Prints negotiated frame
//! metadata; does not open a GUI or save pixels. Use video_source for a synthetic CPU peer.
//! GPU-capable peers use completed copies; peers without DMA-BUF use shared memory.
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};
    use telorgon::{integrations::pipewire::*, media::video::*, renderer_vulkan::*};
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != "--node" {
        return Err("select an authorized source with --node NODE_ID".into());
    }
    let node_id: u32 = args[1].parse()?;
    let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while connection.handle().state() != ConnectionState::Ready {
        if Instant::now() >= deadline {
            return Err(format!("connection: {:?}", connection.handle().state()).into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let target = sources(&connection.handle())
        .into_iter()
        .find(|s| s.handle.id() == node_id)
        .ok_or("video source does not exist")?
        .handle;
    let formats = [
        PixelFormat::Rgba8,
        PixelFormat::Bgra8,
        PixelFormat::Rgbx8,
        PixelFormat::Bgrx8,
    ]
    .into_iter()
    .map(|pixel| VideoFormat {
        pixel,
        color: Colorimetry::UNKNOWN,
        ..VideoFormat::rgba(640, 480, 30)
    })
    .collect();
    let mut stream_config = VideoConfig::capture(target, formats);
    stream_config.buffer_frames = 3;
    stream_config.memory_budget = 256 * 1024 * 1024;
    stream_config.capture_range = Some(VideoCaptureRange {
        min_size: [2, 2],
        max_size: [4096, 4096],
        min_rate: FrameRate::hz(0),
        max_rate: FrameRate::hz(240),
    });
    let gpu = (|| -> Result<VulkanVideoTransfer, Box<dyn std::error::Error>> {
        let config = VulkanConfig::default();
        let instance = VulkanInstance::load(&config, &[])?;
        let selection = DeviceSelection::best(&instance.adapters()?).ok_or("no Vulkan adapter")?;
        let device = VulkanDevice::create_owned(instance, &config, &selection, None)?;
        Ok(VulkanVideoTransfer::new(device)?)
    })();
    let stream = match gpu {
        Ok(transfer) => VideoStream::open_gpu(connection.handle(), stream_config, transfer)?,
        Err(error) => {
            eprintln!("GPU unavailable ({error}); using CPU capture.");
            VideoStream::open(connection.handle(), stream_config)?
        }
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut count = 0u64;
    while Instant::now() < deadline {
        if let VideoState::Failed(error) = stream.state() {
            return Err(error.into());
        }
        if let Some(frame) = stream.receive_frame()? {
            count += 1;
            if count == 1 || count % 120 == 0 {
                match frame {
                    VideoFrame::Cpu(frame) => println!(
                        "{count}: CPU {:?}, {:?}",
                        frame.format(),
                        frame.metadata().timestamp_ns
                    ),
                    VideoFrame::Gpu(frame) => println!(
                        "{count}: GPU {:?}, modifier {:#x}, {} bytes, {:?}",
                        frame.format(),
                        frame.modifier(),
                        frame.allocation_bytes(),
                        frame.metadata().timestamp_ns
                    ),
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    println!("{:?}", stream.diagnostics());
    stream.stop();
    connection.shutdown()?;
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This example requires Linux.");
}
