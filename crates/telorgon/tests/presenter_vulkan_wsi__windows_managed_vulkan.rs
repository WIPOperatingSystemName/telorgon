#![cfg(feature = "application-vulkan")]
#![cfg(target_os = "windows")]

use std::time::{Duration, Instant};

use telorgon::core::{ColorRgba8, RectF, SizeF, SizeI};
use telorgon::layout::{ClipId, SpatialId};
use telorgon::presenter_vulkan_wsi::{
    AcquireOutcome, PresentDisposition, PresenterState, VulkanWinitPresenter, VulkanWinitSurface,
    required_instance_extensions,
};
use telorgon::render::{
    BatchKey, BlendMode, BoxInstance, DrawItem, PipelineKind, PrimitiveKind, RenderBackend,
    RenderRequest, RenderScene, RenderSpatialNode, TargetLoad, TargetStore,
};
use telorgon::renderer_vulkan::{
    AdapterReport, DeviceSelection, VulkanConfig, VulkanDevice, VulkanInstance, VulkanScene,
};
use telorgon::scene::NodeId;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::windows::EventLoopBuilderExtWindows;
use winit::window::{Window, WindowAttributes, WindowId};

const CASE_ID: &str = "managed.windows-vulkan.distinct-resize-suspend-recreate-shutdown";
const REQUIRED_PRESENTED_FRAMES: usize = 6;

#[test]
#[ignore = "opens a short-lived Windows Vulkan qualification window; run explicitly with TELORGON_TEST_MODE=developer-hardware"]
fn managed_windows_vulkan_presents_and_recovers() {
    assert_eq!(
        std::env::var("TELORGON_TEST_MODE").as_deref(),
        Ok("developer-hardware"),
        "managed presentation evidence must be selected explicitly"
    );
    let mut event_loop_builder = EventLoop::builder();
    event_loop_builder.with_any_thread(true);
    let event_loop = event_loop_builder
        .build()
        .expect("create Windows qualification event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let extensions = required_instance_extensions(&event_loop.owned_display_handle())
        .expect("query Windows Vulkan surface extensions");
    let config = VulkanConfig {
        enable_validation: true,
        ..VulkanConfig::default()
    };
    let instance = VulkanInstance::load(&config, &extensions)
        .expect("load Vulkan 1.3 with validation for presentation");
    let mut harness = ManagedHarness::new(instance, config);

    event_loop
        .run_app(&mut harness)
        .expect("run managed Vulkan qualification loop");
    assert!(harness.failure.is_none(), "{:?}", harness.failure);
    assert_eq!(harness.presented_frames, REQUIRED_PRESENTED_FRAMES);
    assert!(harness.resize_observed, "resize event was not observed");
    assert!(harness.suspend_resume_completed);
    assert!(harness.surface_replaced);
    assert!(harness.shutdown_completed);
    assert_eq!(
        harness.instance.diagnostics().error_count(),
        0,
        "validation messages: {:#?}",
        harness.instance.diagnostics().messages()
    );
    assert_eq!(
        harness.instance.diagnostics().warning_count(),
        0,
        "validation messages: {:#?}",
        harness.instance.diagnostics().messages()
    );
    eprintln!(
        "TELORGON_EVIDENCE case={CASE_ID} layer=E5 outcome=pass presented_frames={} resize=true suspend_resume=true surface_recreate=true shutdown=true validation_errors=0",
        harness.presented_frames
    );
}

struct ManagedHarness {
    instance: VulkanInstance,
    config: VulkanConfig,
    started: Instant,
    window: Option<Window>,
    device: Option<VulkanDevice>,
    presenter: Option<VulkanWinitPresenter>,
    source: RenderScene,
    scene: Option<VulkanScene>,
    node: NodeId,
    presented_frames: usize,
    resize_requested: bool,
    resize_observed: bool,
    suspend_resume_completed: bool,
    surface_replaced: bool,
    shutdown_completed: bool,
    failure: Option<String>,
}

impl ManagedHarness {
    fn new(instance: VulkanInstance, config: VulkanConfig) -> Self {
        Self {
            instance,
            config,
            started: Instant::now(),
            window: None,
            device: None,
            presenter: None,
            source: RenderScene::default(),
            scene: None,
            node: NodeId::new(0, 1),
            presented_frames: 0,
            resize_requested: false,
            resize_observed: false,
            suspend_resume_completed: false,
            surface_replaced: false,
            shutdown_completed: false,
            failure: None,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, message: impl Into<String>) {
        self.failure = Some(message.into());
        event_loop.exit();
    }

    fn attach(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("Telorgon E5 Windows Vulkan qualification")
                    .with_inner_size(LogicalSize::new(320.0, 240.0)),
            )
            .map_err(|error| error.to_string())?;
        let surface = VulkanWinitSurface::create(&self.instance, &window, &window)
            .map_err(|error| error.to_string())?;
        let mut adapters = self
            .instance
            .adapters()
            .map_err(|error| error.to_string())?;
        adapters.sort_by_key(|adapter| std::cmp::Reverse(adapter.score));
        let device = select_present_device(&self.instance, &self.config, &surface, &adapters)?;
        let scene = device.create_scene().map_err(|error| error.to_string())?;
        let extent = window_extent(&window);
        let presenter =
            VulkanWinitPresenter::new(surface, &device, extent, self.config.frames_in_flight)
                .map_err(|error| error.to_string())?;
        self.initialize_source(extent);
        window.request_redraw();
        self.window = Some(window);
        self.device = Some(device);
        self.presenter = Some(presenter);
        self.scene = Some(scene);
        Ok(())
    }

    fn initialize_source(&mut self, extent: SizeI) {
        self.source.extent = SizeF {
            width: extent.width as f32,
            height: extent.height as f32,
        };
        self.source.spatial_nodes.upsert(
            self.node,
            RenderSpatialNode {
                id: SpatialId(0),
                transform: telorgon::core::Affine2D::IDENTITY,
            },
        );
        self.source.set_draw_order(vec![DrawItem {
            kind: PrimitiveKind::Box,
            index: 0,
            batch: BatchKey {
                pipeline: PipelineKind::AnalyticBox,
                resource: 0,
                clip: ClipId(0),
                blend: BlendMode::Alpha,
                target: 0,
            },
        }]);
    }

    fn update_box(&mut self, extent: SizeI) {
        let colors = [
            ColorRgba8::rgba(235, 64, 52, 255),
            ColorRgba8::rgba(52, 168, 83, 255),
            ColorRgba8::rgba(66, 133, 244, 255),
        ];
        self.source.extent = SizeF {
            width: extent.width as f32,
            height: extent.height as f32,
        };
        self.source.boxes.upsert(
            self.node,
            BoxInstance {
                node: self.node,
                rect: RectF {
                    x: 24.0,
                    y: 24.0,
                    width: (extent.width as f32 - 48.0).max(1.0),
                    height: (extent.height as f32 - 48.0).max(1.0),
                },
                view_bounds: RectF {
                    x: 24.0,
                    y: 24.0,
                    width: (extent.width as f32 - 48.0).max(1.0),
                    height: (extent.height as f32 - 48.0).max(1.0),
                },
                background: Some(colors[self.presented_frames % colors.len()]),
                border: Default::default(),
                outline: Default::default(),
                corner_radii: Default::default(),
                shadows: Default::default(),
                opacity: 1.0,
                clip: ClipId(0),
                spatial: SpatialId(0),
            },
        );
    }

    fn redraw(&mut self) -> Result<bool, String> {
        let window = self.window.as_ref().ok_or("window is unavailable")?;
        let extent = window_extent(window);
        let device = self
            .device
            .as_ref()
            .cloned()
            .ok_or("device is unavailable")?;
        self.update_box(extent);
        if let Some(delta) = self.source.take_delta() {
            device
                .apply_scene_delta(self.scene.as_mut().ok_or("scene is unavailable")?, &delta)
                .map_err(|error| error.to_string())?;
        }
        let Some(mut frame) = device
            .try_begin_owned_frame()
            .map_err(|error| error.to_string())?
        else {
            return Ok(false);
        };
        let step =
            {
                let outcome = self
                    .presenter
                    .as_mut()
                    .ok_or("presenter is unavailable")?
                    .acquire(&device, &frame)
                    .map_err(|error| error.to_string())?;
                match outcome {
                    AcquireOutcome::Ready(acquired) => {
                        let target = acquired.target();
                        device
                            .render(
                                self.scene.as_mut().ok_or("scene is unavailable")?,
                                &mut frame.context_mut(),
                                &target,
                                &RenderRequest {
                                    force: true,
                                    load: TargetLoad::Clear(ColorRgba8::rgba(16, 18, 24, 255)),
                                    store: TargetStore::Store,
                                    region: None,
                                },
                            )
                            .map_err(|error| error.to_string())?;
                        let recorded = frame.finish().map_err(|error| error.to_string())?;
                        let presented = acquired
                            .submit_and_present(&device, recorded)
                            .map_err(|error| error.to_string())?;
                        match presented.disposition {
                            PresentDisposition::Presented
                            | PresentDisposition::PresentedSuboptimal => RedrawStep::Presented,
                            PresentDisposition::NeedsReconfigure
                            | PresentDisposition::SurfaceLost => RedrawStep::Recover,
                        }
                    }
                    AcquireOutcome::Suspended | AcquireOutcome::NotReady => RedrawStep::Retry,
                    AcquireOutcome::NeedsReconfigure => RedrawStep::Recover,
                }
            };
        match step {
            RedrawStep::Presented => self.presented_frames += 1,
            RedrawStep::Recover => self.recover_surface(&device, extent)?,
            RedrawStep::Retry => return Ok(false),
        }
        self.advance_recovery_steps(&device)?;
        Ok(self.presented_frames >= REQUIRED_PRESENTED_FRAMES)
    }

    fn recover_surface(&mut self, device: &VulkanDevice, extent: SizeI) -> Result<(), String> {
        let presenter = self.presenter.as_mut().ok_or("presenter is unavailable")?;
        if presenter.recovery().state() == PresenterState::SurfaceLost {
            let window = self.window.as_ref().ok_or("window is unavailable")?;
            let surface = VulkanWinitSurface::create(&self.instance, window, window)
                .map_err(|error| error.to_string())?;
            presenter
                .replace_surface(device, surface, extent)
                .map_err(|error| error.to_string())
        } else {
            presenter
                .resume(device, extent)
                .map_err(|error| error.to_string())
        }
    }

    fn advance_recovery_steps(&mut self, device: &VulkanDevice) -> Result<(), String> {
        if self.presented_frames >= 2 && !self.resize_requested {
            self.resize_requested = true;
            let _ = self
                .window
                .as_ref()
                .ok_or("window is unavailable")?
                .request_inner_size(LogicalSize::new(420.0, 300.0));
        }
        if self.presented_frames >= 3 && !self.suspend_resume_completed {
            let presenter = self.presenter.as_mut().ok_or("presenter is unavailable")?;
            presenter.resize(SizeI {
                width: 0,
                height: 0,
            });
            presenter.suspend().map_err(|error| error.to_string())?;
            if presenter.recovery().state() != PresenterState::Suspended {
                return Err("presenter did not enter suspended state".to_owned());
            }
            presenter
                .resume(
                    device,
                    window_extent(self.window.as_ref().ok_or("window is unavailable")?),
                )
                .map_err(|error| error.to_string())?;
            self.suspend_resume_completed = true;
        }
        if self.presented_frames >= 4 && !self.surface_replaced {
            let window = self.window.as_ref().ok_or("window is unavailable")?;
            let surface = VulkanWinitSurface::create(&self.instance, window, window)
                .map_err(|error| error.to_string())?;
            self.presenter
                .as_mut()
                .ok_or("presenter is unavailable")?
                .replace_surface(device, surface, window_extent(window))
                .map_err(|error| error.to_string())?;
            self.surface_replaced = true;
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), String> {
        let device = self.device.as_ref().ok_or("device is unavailable")?;
        let presenter = self.presenter.as_mut().ok_or("presenter is unavailable")?;
        presenter
            .shutdown(device)
            .map_err(|error| error.to_string())?;
        if presenter.recovery().state() != PresenterState::Shutdown {
            return Err("presenter did not enter shutdown state".to_owned());
        }
        self.shutdown_completed = true;
        Ok(())
    }
}

enum RedrawStep {
    Presented,
    Recover,
    Retry,
}

impl ApplicationHandler for ManagedHarness {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(error) = self.attach(event_loop)
        {
            self.fail(event_loop, error);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(Window::id) != Some(window_id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => self.fail(event_loop, "qualification window was closed"),
            WindowEvent::Resized(size) => {
                self.resize_observed |= self.resize_requested;
                if let Some(presenter) = self.presenter.as_mut() {
                    presenter.resize(SizeI {
                        width: size.width as i32,
                        height: size.height as i32,
                    });
                }
                if size.width > 0
                    && size.height > 0
                    && let Some(window) = self.window.as_ref()
                {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => match self.redraw() {
                Ok(true) => match self.shutdown() {
                    Ok(()) => event_loop.exit(),
                    Err(error) => self.fail(event_loop, error),
                },
                Ok(false) => {
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
                Err(error) => self.fail(event_loop, error),
            },
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.started.elapsed() > Duration::from_secs(20) {
            self.fail(event_loop, format!("{CASE_ID} timed out"));
        } else if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn select_present_device(
    instance: &VulkanInstance,
    config: &VulkanConfig,
    surface: &VulkanWinitSurface,
    adapters: &[AdapterReport],
) -> Result<VulkanDevice, String> {
    let mut failures = Vec::new();
    for adapter in adapters.iter().filter(|adapter| adapter.supported) {
        let selection = DeviceSelection {
            adapter_index: adapter.index,
        };
        match VulkanDevice::create_owned(
            instance.clone(),
            config,
            &selection,
            Some(surface.presentation_requirement()),
        ) {
            Ok(device) => return Ok(device),
            Err(error) => failures.push(format!("{}: {error}", adapter.name)),
        }
    }
    Err(format!(
        "no non-CPU Vulkan adapter can present: {}",
        failures.join("; ")
    ))
}

fn window_extent(window: &Window) -> SizeI {
    let size = window.inner_size();
    SizeI {
        width: size.width.max(1) as i32,
        height: size.height.max(1) as i32,
    }
}
