//! Host-driven retained UI rendering into an owned CPU framebuffer.

use crate::foundation::{MonotonicInstant, SizeI};
use crate::graphics::presentation::{PresentationError, PresentationErrorKind, SurfaceMetrics};
use crate::graphics::render::{
    DamageRegion, RenderBackend, RenderError, RenderRequest, RenderStats, RenderTargetInfo,
    TargetLoad, TargetStore,
};
use crate::graphics::renderers::software::{
    SoftwareRenderer, SoftwareScene, SoftwareSurface, SoftwareTarget,
};
use crate::host::application::{
    AppError, AppRuntime, AppRuntimeCore, ComposedAppRuntime, InputFlushOutcome, PlatformInput,
    PreparedFrame,
};
use crate::platform::contracts::ScaleFactor;
use crate::runtime::{Component, ComponentDriver, ComponentRuntimeDriver, CompositionDriver};

#[derive(Debug, thiserror::Error)]
pub enum SoftwareUiHostError {
    #[error("{0}")]
    Runtime(#[from] AppError),
    #[error("{0}")]
    Render(#[from] RenderError),
    #[error("{0}")]
    Presentation(#[from] PresentationError),
    #[error("software UI host time moved backwards")]
    TimeRegression,
}

pub type SoftwareUiHostResult<T> = Result<T, SoftwareUiHostError>;

/// Result of one explicitly advanced UI turn. A surface is supplied only when new pixels were
/// rasterized; idle and suspended turns never advertise stale damage from an earlier frame.
/// The borrow prevents advancing the host while a presenter is reading its framebuffer.
pub struct SoftwareUiFrame<'frame> {
    pub metrics: SurfaceMetrics,
    pub input: InputFlushOutcome,
    pub prepared: PreparedFrame,
    pub render: RenderStats,
    surface: Option<&'frame SoftwareSurface>,
}

impl SoftwareUiFrame<'_> {
    pub fn surface(&self) -> Option<&SoftwareSurface> {
        self.surface
    }

    pub fn pixels_rgba8(&self) -> Option<&[u8]> {
        self.surface.map(SoftwareSurface::pixels_rgba8)
    }

    pub fn damage(&self) -> Option<&DamageRegion> {
        self.surface.map(SoftwareSurface::presented_damage)
    }
}

/// One component runtime and CPU render scene, advanced solely by the caller.
///
/// This host creates no window, event loop, worker, native presentation resource, or clock.
/// Inputs use logical view coordinates; `configure` supplies logical/physical output mapping.
/// The caller presents a returned surface synchronously or copies it before the next turn.
pub struct SoftwareUiHost<D: ComponentDriver> {
    runtime: AppRuntimeCore<D>,
    renderer: SoftwareRenderer,
    scene: SoftwareScene,
    surface: SoftwareSurface,
    metrics: SurfaceMetrics,
    scene_initialized: bool,
    force_redraw: bool,
    last_advance: Option<MonotonicInstant>,
}

impl<C: Component> SoftwareUiHost<ComponentRuntimeDriver<C>> {
    pub fn new(component: C, metrics: SurfaceMetrics) -> SoftwareUiHostResult<Self> {
        validate_metrics(metrics)?;
        let runtime = AppRuntime::with_extent(component, initial_extent(metrics))?;
        Self::from_runtime(runtime, metrics)
    }
}

impl SoftwareUiHost<CompositionDriver> {
    pub fn from_composed<C: crate::authoring::compose::Component>(
        component: C,
        metrics: SurfaceMetrics,
    ) -> SoftwareUiHostResult<Self> {
        validate_metrics(metrics)?;
        let runtime =
            ComposedAppRuntime::from_composed_with_extent(component, initial_extent(metrics))?;
        Self::from_runtime(runtime, metrics)
    }
}

impl<D: ComponentDriver> SoftwareUiHost<D> {
    /// Adopts a runtime, including one whose earlier scene deltas have already been consumed.
    /// The first turn initializes rendering from a complete runtime scene snapshot.
    pub fn from_runtime(
        mut runtime: AppRuntimeCore<D>,
        metrics: SurfaceMetrics,
    ) -> SoftwareUiHostResult<Self> {
        validate_metrics(metrics)?;
        configure_runtime(&mut runtime, metrics)?;
        let renderer = SoftwareRenderer;
        let scene = renderer.create_scene()?;
        Ok(Self {
            runtime,
            renderer,
            scene,
            surface: SoftwareSurface::default(),
            metrics,
            scene_initialized: false,
            force_redraw: true,
            last_advance: None,
        })
    }

    pub fn runtime(&self) -> &AppRuntimeCore<D> {
        &self.runtime
    }

    /// Allows advanced component, theme, asset, and task configuration. Scene-delta consumption
    /// belongs to this host; callers must not drain the runtime's render transport independently.
    pub fn runtime_mut(&mut self) -> &mut AppRuntimeCore<D> {
        &mut self.runtime
    }

    pub fn into_runtime(self) -> AppRuntimeCore<D> {
        self.runtime
    }

    pub fn metrics(&self) -> SurfaceMetrics {
        self.metrics
    }

    /// Queues portable input without running user callbacks. Supply geometry changes through
    /// `configure`, rather than queuing a separate `PlatformInput::Resize` event.
    pub fn queue_input(&mut self, input: impl Into<PlatformInput>) {
        self.runtime.queue_input(input);
    }

    /// Reconfigures output without acquiring or changing a native surface. Zero physical extent
    /// suspends rasterization while retaining component state and pending repaint demand.
    pub fn configure(&mut self, metrics: SurfaceMetrics) -> SoftwareUiHostResult<bool> {
        validate_metrics(metrics)?;
        if metrics == self.metrics {
            return Ok(false);
        }
        configure_runtime(&mut self.runtime, metrics)?;
        self.metrics = metrics;
        self.force_redraw = true;
        Ok(true)
    }

    pub fn next_deadline(&self) -> Option<MonotonicInstant> {
        self.runtime.next_deadline()
    }

    pub fn animation_active(&self) -> bool {
        self.runtime.animation_active()
    }

    /// Runs queued input and component work, then renders when a drawable needs repainting.
    /// `force` also supports a presenter recovering lost contents without changing its metrics.
    pub fn advance(
        &mut self,
        now: MonotonicInstant,
        force: bool,
    ) -> SoftwareUiHostResult<SoftwareUiFrame<'_>> {
        if self.last_advance.is_some_and(|previous| now < previous) {
            return Err(SoftwareUiHostError::TimeRegression);
        }
        self.last_advance = Some(now);
        let input = self.runtime.flush_input(now);
        let force_render = force || self.force_redraw;
        let mut prepared = self.runtime.prepare_frame(now, force_render)?;
        let mut scene_updated = false;
        if !self.scene_initialized {
            // An adopted runtime may only have incremental deltas left. A complete snapshot is
            // necessary before applying later patches to a fresh renderer scene.
            while self.runtime.pop_scene_delta().is_some() {}
            let snapshot = self.runtime.scene_snapshot();
            self.renderer
                .apply_scene_delta(&mut self.scene, &snapshot)?;
            prepared.scene_epoch = snapshot.epoch;
            prepared.changed = true;
            self.scene_initialized = true;
            scene_updated = true;
        } else {
            while let Some(delta) = self.runtime.pop_scene_delta() {
                self.renderer.apply_scene_delta(&mut self.scene, &delta)?;
                scene_updated = true;
            }
        }
        let render =
            if self.metrics.drawable() && (force_render || prepared.changed || scene_updated) {
                let target = SoftwareTarget::new(RenderTargetInfo {
                    color_space: self.metrics.color_space,
                    alpha_mode: self.metrics.alpha_mode,
                    ..RenderTargetInfo::full(self.metrics.physical_extent)
                });
                let request = RenderRequest {
                    force: force_render,
                    load: TargetLoad::Clear(self.scene.background()),
                    store: TargetStore::Store,
                    region: None,
                };
                let render = self.renderer.render_logical(
                    &mut self.scene,
                    &mut self.surface.begin_frame(),
                    &target,
                    &request,
                )?;
                self.force_redraw = false;
                render
            } else {
                RenderStats {
                    epoch: prepared.scene_epoch,
                    ..RenderStats::default()
                }
            };
        Ok(SoftwareUiFrame {
            metrics: self.metrics,
            input,
            prepared,
            render,
            surface: render.recorded.then_some(&self.surface),
        })
    }
}

fn initial_extent(metrics: SurfaceMetrics) -> SizeI {
    SizeI {
        width: metrics.logical_extent.width.ceil().max(1.0) as i32,
        height: metrics.logical_extent.height.ceil().max(1.0) as i32,
    }
}

fn configure_runtime<D: ComponentDriver>(
    runtime: &mut AppRuntimeCore<D>,
    metrics: SurfaceMetrics,
) -> SoftwareUiHostResult<()> {
    if metrics.logical_extent.width > 0.0 && metrics.logical_extent.height > 0.0 {
        runtime.resize_logical(metrics.logical_extent)?;
    }
    runtime.set_raster_scale(
        ScaleFactor::new(metrics.scale_factor as f32).map_err(|error| {
            PresentationError::new(PresentationErrorKind::InvalidState, error.to_string())
        })?,
    );
    Ok(())
}

fn validate_metrics(metrics: SurfaceMetrics) -> SoftwareUiHostResult<()> {
    metrics.validate()?;
    let logical = metrics.logical_extent;
    let physical = metrics.physical_extent;
    if physical.width < 0
        || physical.height < 0
        || logical.width >= i32::MAX as f32
        || logical.height >= i32::MAX as f32
        || (metrics.drawable() && (logical.width <= 0.0 || logical.height <= 0.0))
        || !(metrics.scale_factor as f32).is_finite()
        || metrics.scale_factor as f32 <= 0.0
    {
        return Err(PresentationError::new(
            PresentationErrorKind::InvalidState,
            "software UI surface dimensions or raster scale are invalid",
        )
        .into());
    }
    if !matches!(
        metrics.color_space,
        crate::graphics::render::ColorSpace::Srgb | crate::graphics::render::ColorSpace::Linear
    ) {
        return Err(PresentationError::new(
            PresentationErrorKind::Unsupported,
            "software UI supports only linear and sRGB surfaces",
        )
        .into());
    }
    let bytes = (physical.width as usize)
        .checked_mul(physical.height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if bytes.is_none_or(|bytes| bytes > isize::MAX as usize) {
        return Err(PresentationError::new(
            PresentationErrorKind::InvalidState,
            "software UI framebuffer byte length overflows",
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "software_tests.rs"]
mod tests;
