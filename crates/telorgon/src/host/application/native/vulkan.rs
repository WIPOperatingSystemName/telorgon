use std::sync::Arc;
use std::time::Duration;

use crate::graphics::bridges::vulkan_dxgi::VulkanDxgiBridge;
use crate::foundation::SizeI;
use crate::graphics::presentation::wsi::{
    VulkanPresentModePreference, VulkanWinitPresenter, VulkanWinitSurface,
    required_instance_extensions,
};
use crate::graphics::render::RenderBackend;
use crate::graphics::renderers::vulkan::{
    AdapterReport, DeviceSelection, VulkanConfig, VulkanDevice, VulkanInstance,
    VulkanLiveResizeMode,
};
use winit::event_loop::{EventLoop, EventLoopProxy};
use winit::window::Window;

use super::HostEvent;
use super::resize::{ResizeUpdate, SurfaceCommitPolicy, SurfaceResizeAction};
use super::vulkan_pipeline::VulkanPresentationPipeline;
use super::vulkan_worker::{VulkanRenderWorker, VulkanWork, VulkanWorkerInit};
use super::winit_host::{NativePresentation, PreparedPresentationFrame, PresentationAction};
#[cfg(not(feature = "application-software"))]
use super::winit_host::{create_managed_event_loop, run_composed_managed};
#[cfg(not(feature = "application-software"))]
use crate::host::application::ReadyGuiApplication;
use crate::host::application::{AppError, AppResult};

#[cfg(not(feature = "application-software"))]
pub fn run_gui_vulkan(application: ReadyGuiApplication) -> AppResult<()> {
    let (driver, options, renderer, assets, pointer) = application.into_parts()?;
    if renderer == crate::host::application::Renderer::Software {
        return Err(AppError::new(
            "this build does not include the managed software renderer",
        ));
    }
    let event_loop =
        create_managed_event_loop(crate::host::application::profiler::ProfileTarget::Gui)?;
    let presentation = create_vulkan_presentation(event_loop.event_loop())?;
    run_composed_managed(event_loop, driver, options, assets, pointer, presentation)
}

pub(crate) fn create_vulkan_presentation(
    event_loop: &EventLoop<HostEvent>,
) -> AppResult<VulkanPresentation> {
    let display = event_loop.owned_display_handle();
    let extensions =
        required_instance_extensions(&display).map_err(|error| AppError::new(error.to_string()))?;
    let config = VulkanConfig::default();
    let instance = VulkanInstance::load(&config, &extensions)
        .map_err(|error| AppError::new(error.to_string()))?;
    Ok(VulkanPresentation::new(
        instance,
        config,
        event_loop.create_proxy(),
    ))
}

pub(crate) struct VulkanPresentation {
    instance: Option<VulkanInstance>,
    config: VulkanConfig,
    proxy: EventLoopProxy<HostEvent>,
    worker: Option<VulkanRenderWorker>,
    pending_resize: Option<ResizeUpdate>,
    metrics_revision: u64,
    scene_epoch: u64,
    force_frame: bool,
}

impl VulkanPresentation {
    pub(crate) fn new(
        instance: VulkanInstance,
        config: VulkanConfig,
        proxy: EventLoopProxy<HostEvent>,
    ) -> Self {
        Self {
            instance: Some(instance),
            config,
            proxy,
            worker: None,
            pending_resize: None,
            metrics_revision: 0,
            scene_epoch: 0,
            force_frame: true,
        }
    }

    fn extent(window: &Window) -> SizeI {
        let size = window.inner_size();
        SizeI {
            width: size.width as i32,
            height: size.height as i32,
        }
    }

    fn worker(&self) -> Result<&VulkanRenderWorker, String> {
        self.worker
            .as_ref()
            .ok_or_else(|| "Vulkan presentation worker is unavailable".to_owned())
    }
}

impl NativePresentation for VulkanPresentation {
    fn attach(&mut self, window: Arc<Window>) -> Result<(), String> {
        let instance = self
            .instance
            .as_ref()
            .ok_or_else(|| "Vulkan instance is unavailable".to_owned())?;
        let surface = VulkanWinitSurface::create(instance, &*window, &*window)
            .map_err(|error| format!("failed to create Vulkan surface: {error}"))?;
        let mut adapters = instance
            .adapters()
            .map_err(|error| format!("failed to enumerate Vulkan adapters: {error}"))?;
        adapters.sort_by_key(|adapter| std::cmp::Reverse(adapter.score));
        let device = select_present_device(instance, &self.config, &surface, &adapters)?;
        let scene = device
            .create_scene()
            .map_err(|error| format!("failed to create Vulkan scene: {error}"))?;
        let extent = Self::extent(&window);
        let present_mode = if self.config.prefer_mailbox_present {
            VulkanPresentModePreference::MailboxWithFifoFallback
        } else {
            VulkanPresentModePreference::Fifo
        };
        let presenter = if self.config.enable_dxgi_presenter && device.capabilities().dxgi_interop {
            let presenter =
                VulkanDxgiBridge::new(&*window, &device, extent, self.config.frames_in_flight)
                    .map_err(|error| format!("failed to create DXGI presenter: {error}"))?;
            drop(surface);
            #[cfg(feature = "profiler")]
            crate::runtime::instrumentation::instant!("presenter.dxgi_hwnd");
            VulkanPresentationPipeline::Dxgi(Box::new(presenter))
        } else {
            #[cfg(feature = "profiler")]
            crate::runtime::instrumentation::record_diagnostic(
                "presentation.dxgi_interop_unavailable",
                crate::runtime::instrumentation::DiagnosticSeverity::Warning,
                1,
            );
            VulkanPresentationPipeline::Wsi(Box::new(
                VulkanWinitPresenter::new_with_present_mode(
                    surface,
                    &device,
                    extent,
                    self.config.frames_in_flight,
                    present_mode,
                )
                .map_err(|error| format!("failed to create Vulkan presenter: {error}"))?,
            ))
        };
        let instance = self
            .instance
            .take()
            .expect("Vulkan instance was checked above");
        let worker = VulkanRenderWorker::spawn(
            VulkanWorkerInit {
                instance,
                device,
                presenter,
                scene,
                window,
            },
            self.proxy.clone(),
        )?;
        self.worker = Some(worker);
        #[cfg(feature = "profiler")]
        crate::runtime::instrumentation::instant!("renderer.vulkan");
        self.pending_resize = Some(ResizeUpdate {
            generation: 0,
            metrics_revision: 0,
            phase: super::resize::ResizeInteractionPhase::Stable,
            extent,
            surface: SurfaceResizeAction::Commit,
        });
        self.metrics_revision = 0;
        self.force_frame = true;
        Ok(())
    }

    fn resize_policy(&self) -> SurfaceCommitPolicy {
        surface_commit_policy(self.config.live_resize_mode)
    }

    fn resize(&mut self, update: ResizeUpdate) -> Result<(), String> {
        self.metrics_revision = update.metrics_revision;
        self.pending_resize = Some(update);
        self.force_frame = update.surface != SurfaceResizeAction::Suspend;
        if update.surface == SurfaceResizeAction::Suspend {
            let resize = self.pending_resize.take();
            self.worker()?.submit(VulkanWork {
                resize,
                deltas: Vec::new(),
                snapshot: super::vulkan_worker::PresentationSnapshot {
                    metrics_revision: self.metrics_revision,
                    scene_epoch: self.scene_epoch,
                },
                force_present: false,
                frame_interval: Duration::from_millis(16),
                #[cfg(feature = "profiler")]
                profile_frame: crate::runtime::instrumentation::current_frame_id(),
                #[cfg(feature = "profiler")]
                profile_view: crate::runtime::instrumentation::current_view_id(),
            })?;
        }
        Ok(())
    }

    fn suspend(&mut self) -> Result<(), String> {
        self.force_frame = true;
        self.worker()?.suspend()
    }

    fn present(&mut self, frame: PreparedPresentationFrame) -> Result<PresentationAction, String> {
        self.worker()?.poll_error()?;
        let frame_revision = frame.metrics.revision.get();
        if frame_revision != self.metrics_revision {
            return Err(format!(
                "presentation metrics revision {frame_revision} does not match active surface revision {}",
                self.metrics_revision
            ));
        }
        self.scene_epoch = frame.scene_epoch;
        let should_submit = self.force_frame
            || frame.force_present
            || self.pending_resize.is_some()
            || frame.changed
            || !frame.deltas.is_empty();
        if !should_submit {
            return Ok(PresentationAction::Idle);
        }
        let work = VulkanWork {
            resize: self.pending_resize.take(),
            deltas: frame.deltas,
            snapshot: super::vulkan_worker::PresentationSnapshot {
                metrics_revision: self.metrics_revision,
                scene_epoch: self.scene_epoch,
            },
            force_present: self.force_frame || frame.force_present || frame.changed,
            frame_interval: frame.frame_interval,
            #[cfg(feature = "profiler")]
            profile_frame: crate::runtime::instrumentation::current_frame_id(),
            #[cfg(feature = "profiler")]
            profile_view: crate::runtime::instrumentation::current_view_id(),
        };
        {
            #[cfg(feature = "profiler")]
            let _span = crate::runtime::instrumentation::span!("worker.submit");
            self.worker()?.submit(work)?;
        }
        self.force_frame = false;
        Ok(PresentationAction::Submitted)
    }

    fn poll(&mut self) -> Result<(), String> {
        self.worker()?.poll_error()
    }

    fn synchronize_resize(
        &mut self,
        metrics_revision: u64,
        timeout: Duration,
    ) -> Result<bool, String> {
        if self.config.live_resize_mode != VulkanLiveResizeMode::Responsive {
            return Ok(false);
        }
        self.worker()?
            .wait_for_presented_resize(metrics_revision, timeout)
    }

    fn shutdown(&mut self) -> Result<(), String> {
        self.pending_resize = None;
        self.metrics_revision = 0;
        self.scene_epoch = 0;
        self.force_frame = false;
        if let Some(mut worker) = self.worker.take() {
            worker.shutdown()
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responsive_live_resize_commits_each_surface_extent() {
        assert_eq!(
            surface_commit_policy(VulkanLiveResizeMode::Responsive),
            SurfaceCommitPolicy::Responsive
        );
    }

    #[test]
    fn scaled_preview_fallback_remains_explicit() {
        assert_eq!(
            surface_commit_policy(VulkanLiveResizeMode::DeferredScaledPreview),
            SurfaceCommitPolicy::DeferredScaledPreview
        );
    }
}

const fn surface_commit_policy(mode: VulkanLiveResizeMode) -> SurfaceCommitPolicy {
    match mode {
        VulkanLiveResizeMode::Responsive => SurfaceCommitPolicy::Responsive,
        VulkanLiveResizeMode::DeferredScaledPreview => SurfaceCommitPolicy::DeferredScaledPreview,
    }
}

fn select_present_device(
    instance: &VulkanInstance,
    config: &VulkanConfig,
    surface: &VulkanWinitSurface,
    adapters: &[AdapterReport],
) -> Result<VulkanDevice, String> {
    let mut failures = Vec::new();
    let mut vulkan_wsi_fallback = None;
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
            Ok(device) if !config.enable_dxgi_presenter || device.capabilities().dxgi_interop => {
                return Ok(device);
            }
            Ok(device) => {
                vulkan_wsi_fallback.get_or_insert(device);
            }
            Err(error) => failures.push(format!("{}: {error}", adapter.name)),
        }
    }
    if let Some(device) = vulkan_wsi_fallback {
        return Ok(device);
    }
    Err(format!(
        "no Vulkan adapter can present to this Windows surface: {}",
        failures.join("; ")
    ))
}
