use crate::foundation::{MonotonicInstant, RectI, SizeF, SizeI};
use crate::graphics::render::{
    ReadbackFormat, ReadbackImage, ReadbackRequest, RenderBackend, RenderRequest, RenderTargetInfo,
    TargetLoad, TargetStore,
};
use crate::graphics::renderers::software::{SoftwareRenderer, SoftwareScene, SoftwareSurface, SoftwareTarget};

use crate::host::application::{
    AppResult, AppRuntime, AppRuntimeCore, Component, ComponentDriver, ComposedAppRuntime,
    PlatformInput,
};

/// Explicit deterministic software assembly for tests, export, and headless use.
pub struct HeadlessRuntime {
    renderer: SoftwareRenderer,
    scene: SoftwareScene,
    surface: SoftwareSurface,
}

impl Default for HeadlessRuntime {
    fn default() -> Self {
        Self {
            renderer: SoftwareRenderer,
            scene: SoftwareScene::default(),
            surface: SoftwareSurface::default(),
        }
    }
}

impl HeadlessRuntime {
    pub fn run_once<C: Component>(
        &mut self,
        component: C,
        extent: SizeI,
    ) -> AppResult<ReadbackImage> {
        let extent = SizeI {
            width: extent.width.max(1),
            height: extent.height.max(1),
        };
        let runtime = AppRuntime::with_extent(component, extent)?;
        self.render_runtime(runtime, extent)
    }

    pub fn run_composed_once<C: crate::authoring::compose::Component>(
        &mut self,
        component: C,
        extent: SizeI,
    ) -> AppResult<ReadbackImage> {
        let extent = SizeI {
            width: extent.width.max(1),
            height: extent.height.max(1),
        };
        let runtime = ComposedAppRuntime::from_composed_with_extent(component, extent)?;
        self.render_runtime(runtime, extent)
    }

    fn render_runtime<D: ComponentDriver>(
        &mut self,
        mut runtime: AppRuntimeCore<D>,
        extent: SizeI,
    ) -> AppResult<ReadbackImage> {
        runtime.queue_input(PlatformInput::Resize(SizeF {
            width: extent.width as f32,
            height: extent.height as f32,
        }));
        runtime.flush_input(MonotonicInstant::ZERO);
        runtime.prepare_frame(MonotonicInstant::ZERO, true)?;
        while let Some(delta) = runtime.pop_scene_delta() {
            self.renderer.apply_scene_delta(&mut self.scene, &delta)?;
        }
        let target = SoftwareTarget::new(RenderTargetInfo::full(extent));
        let clear = self.scene.background();
        {
            let mut frame = self.surface.begin_frame();
            self.renderer.render(
                &mut self.scene,
                &mut frame,
                &target,
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(clear),
                    store: TargetStore::Store,
                    region: None,
                },
            )?;
        }
        self.surface
            .readback(&ReadbackRequest {
                region: RectI {
                    x: 0,
                    y: 0,
                    width: extent.width,
                    height: extent.height,
                },
                format: ReadbackFormat::Rgba8,
            })
            .map_err(Into::into)
    }
}
