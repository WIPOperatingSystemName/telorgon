//! cargo run -p telorgon --no-default-features --features video-linux,embedded-vulkan --example video_gpu_source
//! Publishes a synthetic virtual camera for 30 seconds. Renders on an explicitly created
//! Vulkan device when DMA-BUF is negotiated; otherwise produces shared-memory pixels.
//! Does not read any camera or screen. Select this node in a PipeWire video consumer.
//! Append --float16 for capability-gated linear float16 DMA-BUF, with float16 CPU fallback.
//! The generated scene is SDR; choosing float16 alone does not create HDR mastering metadata.
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};
    use telorgon::{
        core::{ColorRgba8, RectF, SizeF},
        integrations::pipewire::*,
        media::video::*,
        render::{RenderBackend, RenderScene},
        renderer_vulkan::*,
    };
    let args: Vec<_> = std::env::args().skip(1).collect();
    let float16 = match args.as_slice() {
        [] => false,
        [flag] if flag == "--float16" => true,
        _ => return Err("expected optional --float16".into()),
    };
    let mut format = VideoFormat::rgba(640, 480, 30);
    if float16 {
        format.pixel = PixelFormat::RgbaF16;
        format.color = Colorimetry::LINEAR_BT709;
    }
    let gpu = (|| -> Result<_, Box<dyn std::error::Error>> {
        let config = VulkanConfig::default();
        let instance = VulkanInstance::load(&config, &[])?;
        let selection = DeviceSelection::best(&instance.adapters()?).ok_or("no Vulkan adapter")?;
        let device = VulkanDevice::create_owned(instance, &config, &selection, None)?;
        let renderer = VulkanVideoRenderer::new(device.clone(), format, 64 * 1024 * 1024)?;
        let allocator = VulkanVideoTransfer::new(device.clone())?;
        let scene = device.create_scene()?;
        Ok((device, renderer, allocator, scene))
    })();
    let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while connection.handle().state() != ConnectionState::Ready {
        if Instant::now() >= deadline {
            return Err("PipeWire connection timeout".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut config = VideoConfig::producer(format);
    config.name = "Telorgon GPU synthetic camera".into();
    config.virtual_camera = true;
    let (stream, mut graphics) = match gpu {
        Ok((device, renderer, allocator, scene)) => (
            VideoStream::open_gpu_producer(connection.handle(), config, allocator)?,
            Some((device, renderer, scene)),
        ),
        Err(error) => {
            eprintln!("GPU unavailable ({error}); using shared memory.");
            (VideoStream::open(connection.handle(), config)?, None)
        }
    };
    let mut source = RenderScene::default();
    source.extent = SizeF {
        width: format.width as f32,
        height: format.height as f32,
    };
    let mut announced = false;
    let mut phase = 0u8;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let VideoState::Failed(error) = stream.state() {
            return Err(error.into());
        }
        if !announced && let Some(node) = stream.node() {
            println!(
                "Synthetic video node {}. Select it in a consumer; exits after 30 seconds.",
                node.id()
            );
            announced = true;
        }
        if stream.state() == VideoState::Streaming
            && let Some(negotiated) = stream.negotiation()
        {
            let color = [phase, 100, 210, 255];
            match negotiated.transport {
                VideoTransport::SharedMemory => {
                    let pixels = if float16 {
                        let mut pixel = [0u8; 8];
                        for (channel, bytes) in
                            color[..3].iter().zip(pixel[..6].chunks_exact_mut(2))
                        {
                            bytes.copy_from_slice(&linear_half(*channel));
                        }
                        pixel[6..].copy_from_slice(&0x3c00u16.to_le_bytes()); // opaque 1.0
                        pixel.repeat(format.width as usize * format.height as usize)
                    } else {
                        color.repeat(format.width as usize * format.height as usize)
                    };
                    let frame = CpuVideoFrame::packed(format, pixels, FrameMetadata::default())?;
                    let _ = stream.submit(frame);
                }
                VideoTransport::DmaBuf(_) => {
                    let (device, renderer, scene) =
                        graphics.as_mut().ok_or("GPU transport without device")?;
                    source.background = ColorRgba8::rgba(color[0], color[1], color[2], 255);
                    source.damage.add(
                        RectF {
                            x: 0.0,
                            y: 0.0,
                            width: source.extent.width,
                            height: source.extent.height,
                        },
                        source.extent,
                    );
                    if let Some(delta) = source.take_delta() {
                        device.apply_scene_delta(scene, &delta)?;
                    }
                    match renderer.render(scene, FrameMetadata::default()) {
                        Ok(frame) => {
                            let _ = stream.submit_gpu(frame);
                        }
                        Err(MediaError::ResourceLimit(_)) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
            }
            phase = phase.wrapping_add(3);
        }
        std::thread::sleep(format.rate.period().unwrap());
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

#[cfg(target_os = "linux")]
fn linear_half(channel: u8) -> [u8; 2] {
    if channel == 0 {
        return [0, 0];
    }
    let srgb = channel as f32 / 255.0;
    let linear = if srgb <= 0.04045 {
        srgb / 12.92
    } else {
        ((srgb + 0.055) / 1.055).powf(2.4)
    };
    // Nonzero 8-bit sRGB channels map to finite, normal, positive half floats. This
    // narrow example conversion is round-to-nearest-even, not a general f32 encoder.
    let bits = linear.to_bits();
    let rounded = bits + 0x0fff + ((bits >> 13) & 1);
    (((rounded >> 13) - (112 << 10)) as u16).to_le_bytes()
}
