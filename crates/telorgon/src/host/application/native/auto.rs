use std::sync::Arc;
use std::time::Duration;

use winit::window::Window;

use super::resize::{ResizeUpdate, SurfaceCommitPolicy};
use super::software::SoftwarePresentation;
use super::vulkan::{VulkanPresentation, create_vulkan_presentation};
use super::winit_host::{
    ManagedEventLoop, NativePresentation, PreparedPresentationFrame, PresentationAction,
    create_managed_event_loop, run_composed_managed,
};
use crate::host::application::{
    AppError, AppResult, CompositionDriver, ReadyGuiApplication, Renderer, WindowOptions,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ActiveRenderer {
    #[default]
    Pending,
    Vulkan,
    Software,
}

/// A startup-only fallback wrapper. Backend selection finishes before the component is mounted, so
/// a failed Vulkan probe cannot consume application state or scene deltas.
struct AutoPresentation {
    vulkan: Option<VulkanPresentation>,
    software: Option<SoftwarePresentation>,
    active: ActiveRenderer,
}

impl AutoPresentation {
    fn new(vulkan: VulkanPresentation, software: SoftwarePresentation) -> Self {
        Self {
            vulkan: Some(vulkan),
            software: Some(software),
            active: ActiveRenderer::Pending,
        }
    }

    fn unavailable(&self) -> String {
        "managed renderer was used before startup backend selection completed".to_owned()
    }
}

impl NativePresentation for AutoPresentation {
    fn uses_logical_coordinates(&self) -> bool {
        match self.active {
            ActiveRenderer::Vulkan => self.vulkan.as_ref().unwrap().uses_logical_coordinates(),
            ActiveRenderer::Software => true,
            ActiveRenderer::Pending => false,
        }
    }

    fn attach(&mut self, window: Arc<Window>) -> Result<(), String> {
        let vulkan_error = match self.vulkan.as_mut() {
            Some(vulkan) => match vulkan.attach(Arc::clone(&window)) {
                Ok(()) => {
                    #[cfg(feature = "profiler")]
                    crate::runtime::instrumentation::instant!("renderer.vulkan");
                    self.software = None;
                    self.active = ActiveRenderer::Vulkan;
                    return Ok(());
                }
                Err(error) => error,
            },
            None => "Vulkan renderer was not initialized".to_owned(),
        };

        self.vulkan = None;
        let software = self.software.as_mut().ok_or_else(|| {
            format!("Vulkan startup failed ({vulkan_error}) and software fallback is unavailable")
        })?;
        software.attach(window).map_err(|software_error| {
            format!(
                "Vulkan startup failed ({vulkan_error}); software fallback also failed: {software_error}"
            )
        })?;
        eprintln!(
            "telorgon-app: Vulkan startup failed ({vulkan_error}); using the software renderer"
        );
        #[cfg(feature = "profiler")]
        crate::runtime::instrumentation::instant!("renderer.software_fallback");
        self.active = ActiveRenderer::Software;
        Ok(())
    }

    fn resume(&mut self, window: Arc<Window>) -> Result<(), String> {
        match self.active {
            ActiveRenderer::Vulkan => self.vulkan.as_mut().unwrap().resume(window),
            ActiveRenderer::Software => self.software.as_mut().unwrap().resume(window),
            ActiveRenderer::Pending => Err(self.unavailable()),
        }
    }

    fn resize_policy(&self) -> SurfaceCommitPolicy {
        match self.active {
            ActiveRenderer::Vulkan => self.vulkan.as_ref().unwrap().resize_policy(),
            ActiveRenderer::Software | ActiveRenderer::Pending => SurfaceCommitPolicy::Responsive,
        }
    }

    fn resize(&mut self, update: ResizeUpdate) -> Result<(), String> {
        match self.active {
            ActiveRenderer::Vulkan => self.vulkan.as_mut().unwrap().resize(update),
            ActiveRenderer::Software => self.software.as_mut().unwrap().resize(update),
            ActiveRenderer::Pending => Err(self.unavailable()),
        }
    }

    fn suspend(&mut self) -> Result<(), String> {
        match self.active {
            ActiveRenderer::Vulkan => self.vulkan.as_mut().unwrap().suspend(),
            ActiveRenderer::Software => self.software.as_mut().unwrap().suspend(),
            ActiveRenderer::Pending => Ok(()),
        }
    }

    fn present(&mut self, frame: PreparedPresentationFrame) -> Result<PresentationAction, String> {
        match self.active {
            ActiveRenderer::Vulkan => self.vulkan.as_mut().unwrap().present(frame),
            ActiveRenderer::Software => {
                NativePresentation::present(self.software.as_mut().unwrap(), frame)
            }
            ActiveRenderer::Pending => Err(self.unavailable()),
        }
    }

    fn poll(&mut self) -> Result<(), String> {
        match self.active {
            ActiveRenderer::Vulkan => self.vulkan.as_mut().unwrap().poll(),
            ActiveRenderer::Software => self.software.as_mut().unwrap().poll(),
            ActiveRenderer::Pending => Ok(()),
        }
    }

    fn synchronize_resize(
        &mut self,
        metrics_revision: u64,
        timeout: Duration,
    ) -> Result<bool, String> {
        match self.active {
            ActiveRenderer::Vulkan => self
                .vulkan
                .as_mut()
                .unwrap()
                .synchronize_resize(metrics_revision, timeout),
            ActiveRenderer::Software => Ok(true),
            ActiveRenderer::Pending => Err(self.unavailable()),
        }
    }

    fn shutdown(&mut self) -> Result<(), String> {
        match self.active {
            ActiveRenderer::Vulkan => self.vulkan.as_mut().unwrap().shutdown(),
            ActiveRenderer::Software => self.software.as_mut().unwrap().shutdown(),
            ActiveRenderer::Pending => Ok(()),
        }
    }
}

pub fn run_gui_auto(application: ReadyGuiApplication) -> AppResult<()> {
    let (driver, options, renderer, assets, pointer) = application.into_parts()?;
    let event_loop = event_loop()?;
    run_composed(event_loop, driver, options, renderer, assets, pointer)
}

fn event_loop() -> AppResult<ManagedEventLoop> {
    create_managed_event_loop(crate::host::application::profiler::ProfileTarget::Gui)
}

fn software_presentation(event_loop: &ManagedEventLoop) -> AppResult<SoftwarePresentation> {
    SoftwarePresentation::new(event_loop.event_loop().owned_display_handle()).map_err(AppError::new)
}

fn run_composed(
    event_loop: ManagedEventLoop,
    driver: CompositionDriver,
    options: WindowOptions,
    renderer: Renderer,
    assets: crate::AssetBundle,
    pointer: crate::PointerConfiguration,
) -> AppResult<()> {
    match renderer {
        Renderer::Vulkan => {
            let presentation = create_vulkan_presentation(event_loop.event_loop())?;
            run_composed_managed(event_loop, driver, options, assets, pointer, presentation)
        }
        Renderer::Software => {
            let presentation = software_presentation(&event_loop)?;
            run_composed_managed(event_loop, driver, options, assets, pointer, presentation)
        }
        Renderer::Auto => match create_vulkan_presentation(event_loop.event_loop()) {
            Ok(vulkan) => match software_presentation(&event_loop) {
                Ok(software) => run_composed_managed(
                    event_loop,
                    driver,
                    options,
                    assets,
                    pointer,
                    AutoPresentation::new(vulkan, software),
                ),
                Err(_) => {
                    run_composed_managed(event_loop, driver, options, assets, pointer, vulkan)
                }
            },
            Err(vulkan_error) => {
                eprintln!(
                    "telorgon-app: Vulkan initialization failed ({vulkan_error}); using the software renderer"
                );
                let software = software_presentation(&event_loop)?;
                run_composed_managed(event_loop, driver, options, assets, pointer, software)
            }
        },
    }
}
