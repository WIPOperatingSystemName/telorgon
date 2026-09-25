//! cargo run -p telorgon --no-default-features --features video-linux,portal-client-linux,application-software --example video_preview -- --camera
//! Use --screen for a portal-selected screen, or --node NODE_ID for an explicitly selected
//! PipeWire source (e.g. the synthetic video_source example). No source opens without a mode.
//! Append --hdr EXPOSURE_NITS PEAK_NITS to opt into HDR-to-SDR preview tone mapping.
//! Example: --node 42 --hdr 203 1000. Source color metadata must identify PQ or HLG.
#[cfg(target_os = "linux")]
mod linux {
    use std::time::{Duration, Instant};
    use telorgon::{
        app::*,
        graphics::bridges::video_cpu::{HdrToneMap, VideoImageOptions},
        host::application::video::{VideoPreview, VideoPreviewSnapshot},
        integrations::{pipewire::*, portals::client::*},
        media::video::*,
        ui::ImageId,
    };
    #[component(no_default)]
    struct Preview {
        #[input]
        frames: Signal<VideoPreviewSnapshot>,
    }
    impl Component for Preview {
        fn view(&self) -> impl View {
            let snapshot = self.watch(&self.frames);
            let mut content = column()
                .gap(8.0)
                .padding(12.0)
                .child(text(format!("{:?}", snapshot.state)));
            if let Some(resource) = &snapshot.image {
                let scale = (640.0 / resource.extent.width as f32)
                    .min(460.0 / resource.extent.height as f32);
                content = content.child(
                    Image::resource(resource.clone())
                        .width(resource.extent.width as f32 * scale)
                        .height(resource.extent.height as f32 * scale)
                        .accessible_label("Live video preview"),
                );
            }
            if let Some(error) = &snapshot.conversion_error {
                content = content.child(text(error.to_string()));
            }
            content
        }
    }
    pub fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if !matches!(
            args.first().map(String::as_str),
            Some("--camera" | "--screen" | "--node")
        ) {
            return Err("choose --camera (portal), --screen (portal), or --node NODE_ID".into());
        }
        // Validate all arguments before asking for portal permission or opening a stream.
        let option_start = if args[0] == "--node" { 2 } else { 1 };
        let node_id: Option<u32> = if args[0] == "--node" {
            Some(args.get(1).ok_or("missing NODE_ID")?.parse()?)
        } else {
            None
        };
        let hdr_tone_map = match args.get(option_start..) {
            Some([]) => None,
            Some([flag, exposure, peak]) if flag == "--hdr" => {
                Some(HdrToneMap::reinhard(exposure.parse()?, peak.parse()?)?)
            }
            _ => return Err("expected optional --hdr EXPOSURE_NITS PEAK_NITS".into()),
        };
        let mut owned_connection = None;
        let mut session = None;
        let connection = match args[0].as_str() {
            "--camera" => {
                owned_connection = Some(futures_lite::future::block_on(async {
                    PortalClient::session_bus()
                        .await?
                        .access_camera(ConnectionConfig::default(), &PortalCancellation::default())
                        .await
                })?);
                owned_connection.as_ref().unwrap().handle()
            }
            "--screen" => {
                session = Some(futures_lite::future::block_on(async {
                    PortalClient::session_bus()
                        .await?
                        .share_screen(
                            ScreenCastOptions::default(),
                            ConnectionConfig::default(),
                            &PortalCancellation::default(),
                        )
                        .await
                })?);
                session.as_ref().unwrap().connection()
            }
            _ => {
                owned_connection = Some(Connection::connect(
                    ConnectionConfig::default(),
                    Remote::Default,
                )?);
                owned_connection.as_ref().unwrap().handle()
            }
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while connection.state() != ConnectionState::Ready {
            if Instant::now() >= deadline {
                return Err(format!("connection: {:?}", connection.state()).into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let target = if let Some(session) = &session {
            session
                .sources()
                .first()
                .ok_or("portal returned no sources")?
                .resolve(&connection)?
        } else {
            let sources = sources(&connection);
            let selected = if args[0] == "--node" {
                let id = node_id.expect("validated node argument");
                sources.iter().find(|s| s.handle.id() == id)
            } else {
                sources
                    .iter()
                    .find(|s| s.camera)
                    .or_else(|| sources.first())
            };
            selected
                .ok_or("no matching authorized video source")?
                .handle
        };
        let mut pixels = vec![
            PixelFormat::Rgba8,
            PixelFormat::Bgra8,
            PixelFormat::Rgbx8,
            PixelFormat::Bgrx8,
            PixelFormat::Nv12,
            PixelFormat::I420,
            PixelFormat::Yuy2,
            PixelFormat::Rgb8,
            PixelFormat::Bgr8,
        ];
        if hdr_tone_map.is_some() {
            pixels.insert(0, PixelFormat::RgbaF16);
            pixels.insert(0, PixelFormat::P010);
        }
        let formats = pixels
            .into_iter()
            .map(|pixel| VideoFormat {
                pixel,
                color: Colorimetry::UNKNOWN,
                ..VideoFormat::rgba(640, 480, 30)
            })
            .collect();
        let mut config = VideoConfig::capture(target, formats);
        config.buffer_frames = 3;
        config.memory_budget = 256 * 1024 * 1024;
        config.capture_range = Some(VideoCaptureRange {
            min_size: [2, 2],
            max_size: [4096, 4096],
            min_rate: FrameRate::hz(0),
            max_rate: FrameRate::hz(240),
        });
        // Explicit example policy: untagged RGB screen pixels are sRGB. Untagged YUV
        // is rejected rather than guessing its matrix/range. Select a fallback suited to
        // the camera when its metadata is incomplete.
        let mut preview = VideoPreview::start(
            VideoStream::open(connection, config)?,
            ImageId(0x70000001),
            VideoImageOptions {
                fallback_color: Some(Colorimetry::SRGB),
                hdr_tone_map,
                ..Default::default()
            },
        )?;
        let result = Application::gui("org.telorgon.examples.video-preview", "Telorgon video preview")
            .renderer(Renderer::Software)
            .window(
                Window::new("Video preview")
                    .size(680, 560)
                    .content(Preview {
                        frames: preview.signal(),
                    }),
            )
            .run();
        preview.shutdown();
        if let Some(session) = session {
            futures_lite::future::block_on(session.close());
        }
        if let Some(connection) = &mut owned_connection {
            connection.shutdown()?;
        }
        result?;
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
