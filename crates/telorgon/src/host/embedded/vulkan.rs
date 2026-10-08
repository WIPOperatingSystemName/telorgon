//! Host-driven, window-system-free Telorgon views over command-only Vulkan recording.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::host::application::{AppRuntime, Component, PlatformInput, PreparedFrame};
use crate::foundation::{PointF, RectI, SizeF};
use crate::graphics::render::{RenderBackend, RenderRequest, RenderStats};
use crate::graphics::renderers::vulkan::{
    HostedFrameDescriptor, HostedFrameReceipt, HostedVulkanDeviceDescriptor, VulkanConfig,
    VulkanDevice, VulkanScene,
};

static NEXT_HOST_ID: AtomicU64 = AtomicU64::new(1);

/// Explicit host-owned profiler source for embedded integrations. It creates no service thread,
/// listener, browser, executor, or automatic polling; the host decides when and where to drain.
#[cfg(feature = "instrumentation")]
pub struct EmbeddedProfiler {
    _session: crate::runtime::instrumentation::Session,
    collector: crate::runtime::instrumentation::Collector,
}

#[cfg(feature = "instrumentation")]
impl EmbeddedProfiler {
    pub fn new(
        config: crate::runtime::instrumentation::SessionConfig,
    ) -> Result<Self, crate::runtime::instrumentation::SessionStartError> {
        let (session, collector) = crate::runtime::instrumentation::Session::start(config)?;
        Ok(Self {
            _session: session,
            collector,
        })
    }

    pub fn drain_into(&mut self, events: &mut Vec<crate::runtime::instrumentation::Event>) {
        self.collector.drain_into(events);
    }

    #[must_use]
    pub fn lanes(&self) -> Vec<crate::runtime::instrumentation::LaneInfo> {
        self.collector.lanes()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UiHostError {
    #[error("{0}")]
    Runtime(#[from] crate::host::application::AppError),
    #[error("{0}")]
    Render(#[from] crate::graphics::render::RenderError),
    #[error("UI view handle does not belong to this host")]
    ForeignView,
    #[error("prepared UI view token is stale or was already consumed")]
    StalePreparation,
}

pub type UiHostResult<T> = Result<T, UiHostError>;

#[derive(Clone)]
pub struct UiDevice {
    renderer: VulkanDevice,
}

impl UiDevice {
    pub fn new(renderer: VulkanDevice) -> Self {
        Self { renderer }
    }

    /// Creates a shared UI device over host-owned Vulkan objects.
    ///
    /// # Safety
    ///
    /// This inherits every native lifetime, enabled-feature, external-synchronization, and
    /// allocation guarantee required by [`VulkanDevice::from_hosted`].
    pub unsafe fn from_hosted(
        descriptor: HostedVulkanDeviceDescriptor<'_>,
        config: &VulkanConfig,
    ) -> UiHostResult<Self> {
        Ok(Self::new(unsafe {
            VulkanDevice::from_hosted(descriptor, config)?
        }))
    }

    pub fn renderer(&self) -> &VulkanDevice {
        &self.renderer
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UiViewId {
    host_id: u64,
    value: u64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PreparedUiView {
    host_id: u64,
    view: UiViewId,
    serial: u64,
    pub changed: bool,
    pub scene_epoch: u64,
    #[cfg(feature = "instrumentation")]
    profile_frame: Option<crate::runtime::instrumentation::ProfileFrameId>,
}

impl PreparedUiView {
    pub fn view(self) -> UiViewId {
        self.view
    }
}

pub enum HostedViewRecord {
    Unchanged {
        scene_epoch: u64,
    },
    Recorded {
        scene_epoch: u64,
        render: RenderStats,
        receipt: Box<HostedFrameReceipt>,
    },
}

impl HostedViewRecord {
    pub fn recorded(&self) -> bool {
        matches!(self, Self::Recorded { .. })
    }
}

struct UiView<C: Component> {
    runtime: AppRuntime<C>,
    scene: VulkanScene,
    prepare_serial: u64,
    prepared: bool,
}

/// Owns independent component/view state while sharing one renderer device and its caches.
pub struct UiHost<C: Component> {
    id: u64,
    next_view: u64,
    device: UiDevice,
    views: BTreeMap<UiViewId, UiView<C>>,
}

impl<C: Component> UiHost<C> {
    pub fn new(device: UiDevice) -> Self {
        Self {
            id: NEXT_HOST_ID.fetch_add(1, Ordering::Relaxed),
            next_view: 1,
            device,
            views: BTreeMap::new(),
        }
    }

    pub fn device(&self) -> &UiDevice {
        &self.device
    }

    pub fn create_view(&mut self, component: C) -> UiHostResult<UiViewId> {
        let id = UiViewId {
            host_id: self.id,
            value: self.next_view,
        };
        self.next_view = self
            .next_view
            .checked_add(1)
            .ok_or(UiHostError::ForeignView)?;
        self.views.insert(
            id,
            UiView {
                runtime: AppRuntime::new(component)?,
                scene: self.device.renderer.create_scene()?,
                prepare_serial: 0,
                prepared: false,
            },
        );
        Ok(id)
    }

    pub fn remove_view(&mut self, view: UiViewId) -> UiHostResult<()> {
        self.ensure_view(view)?;
        self.views.remove(&view).ok_or(UiHostError::ForeignView)?;
        Ok(())
    }

    pub fn runtime(&self, view: UiViewId) -> UiHostResult<&AppRuntime<C>> {
        self.ensure_view(view)?;
        Ok(&self.views.get(&view).expect("view was validated").runtime)
    }

    pub fn runtime_mut(&mut self, view: UiViewId) -> UiHostResult<&mut AppRuntime<C>> {
        self.ensure_view(view)?;
        Ok(&mut self
            .views
            .get_mut(&view)
            .expect("view was validated")
            .runtime)
    }

    pub fn queue_input(
        &mut self,
        view: UiViewId,
        input: impl Into<PlatformInput>,
    ) -> UiHostResult<()> {
        self.runtime_mut(view)?.queue_input(input);
        Ok(())
    }

    pub fn flush_input(
        &mut self,
        view: UiViewId,
        timestamp: crate::foundation::MonotonicInstant,
    ) -> UiHostResult<()> {
        self.runtime_mut(view)?.flush_input(timestamp);
        Ok(())
    }

    /// Maps a host target-space point into a view's local coordinates.
    pub fn map_target_point(region: RectI, point: PointF) -> Option<PointF> {
        let x = point.x - region.x as f32;
        let y = point.y - region.y as f32;
        (x >= 0.0 && y >= 0.0 && x < region.width as f32 && y < region.height as f32)
            .then_some(PointF { x, y })
    }

    /// Maps target pixels through a host-declared logical view extent.
    pub fn map_target_point_scaled(
        region: RectI,
        logical_extent: SizeF,
        point: PointF,
    ) -> Option<PointF> {
        if region.width <= 0
            || region.height <= 0
            || !logical_extent.width.is_finite()
            || !logical_extent.height.is_finite()
            || logical_extent.width <= 0.0
            || logical_extent.height <= 0.0
        {
            return None;
        }
        let local = Self::map_target_point(region, point)?;
        Some(PointF {
            x: local.x * logical_extent.width / region.width as f32,
            y: local.y * logical_extent.height / region.height as f32,
        })
    }

    /// Runs component work and scene compilation without allocating or recording GPU commands.
    pub fn prepare_view(
        &mut self,
        view: UiViewId,
        now: crate::foundation::MonotonicInstant,
        force: bool,
    ) -> UiHostResult<PreparedUiView> {
        #[cfg(feature = "instrumentation")]
        let profile_frame = crate::runtime::instrumentation::start_frame("embedded.frame");
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("embedded.prepare_view");
        self.ensure_view(view)?;
        let state = self.views.get_mut(&view).expect("view was validated");
        let PreparedFrame {
            changed,
            scene_epoch,
            ..
        } = state.runtime.prepare_frame(now, force)?;
        while let Some(delta) = state.runtime.pop_scene_delta() {
            self.device
                .renderer
                .apply_scene_delta(&mut state.scene, &delta)?;
        }
        state.prepare_serial = state.prepare_serial.wrapping_add(1).max(1);
        state.prepared = true;
        Ok(PreparedUiView {
            host_id: self.id,
            view,
            serial: state.prepare_serial,
            changed,
            scene_epoch,
            #[cfg(feature = "instrumentation")]
            profile_frame: profile_frame.id(),
        })
    }

    /// Records one prepared view into a host-provided Vulkan target and returns resource pins.
    ///
    /// An unchanged view with `request.force == false` returns before starting a hosted frame, so
    /// it creates no descriptor pool, staging buffer, Vulkan command, submission, or receipt.
    ///
    /// # Safety
    ///
    /// This inherits the native command-buffer and target guarantees of
    /// [`VulkanDevice::begin_hosted_frame`]. If recording returns an error, the host must discard
    /// the command-buffer contents from this interval rather than submit them.
    pub unsafe fn record_view<'host>(
        &mut self,
        prepared: PreparedUiView,
        descriptor: HostedFrameDescriptor<'host>,
        request: &RenderRequest,
    ) -> UiHostResult<HostedViewRecord> {
        #[cfg(feature = "instrumentation")]
        let _frame_scope = crate::runtime::instrumentation::enter_frame(prepared.profile_frame);
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("embedded.record_view");
        if prepared.host_id != self.id || prepared.view.host_id != self.id {
            return Err(UiHostError::ForeignView);
        }
        let state = self
            .views
            .get_mut(&prepared.view)
            .ok_or(UiHostError::ForeignView)?;
        if !state.prepared || state.prepare_serial != prepared.serial {
            return Err(UiHostError::StalePreparation);
        }
        state.prepared = false;
        if !prepared.changed && !request.force {
            return Ok(HostedViewRecord::Unchanged {
                scene_epoch: prepared.scene_epoch,
            });
        }
        let mut frame = unsafe { self.device.renderer.begin_hosted_frame(descriptor)? };
        let render_result = {
            let (mut context, target) = frame.context_and_target();
            self.device
                .renderer
                .render(&mut state.scene, &mut context, &target, request)
        };
        let render = match render_result {
            Ok(stats) => stats,
            Err(error) => {
                frame.abort()?;
                return Err(error.into());
            }
        };
        let receipt = frame.finish()?;
        Ok(HostedViewRecord::Recorded {
            scene_epoch: prepared.scene_epoch,
            render,
            receipt: Box::new(receipt),
        })
    }

    fn ensure_view(&self, view: UiViewId) -> UiHostResult<()> {
        if view.host_id != self.id || !self.views.contains_key(&view) {
            Err(UiHostError::ForeignView)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_points_map_to_local_view_coordinates() {
        let region = RectI {
            x: 20,
            y: 30,
            width: 100,
            height: 50,
        };
        assert_eq!(
            UiHost::<TestComponent>::map_target_point(region, PointF { x: 25.0, y: 39.0 }),
            Some(PointF { x: 5.0, y: 9.0 })
        );
        assert_eq!(
            UiHost::<TestComponent>::map_target_point(region, PointF { x: 120.0, y: 39.0 }),
            None
        );
        assert_eq!(
            UiHost::<TestComponent>::map_target_point_scaled(
                region,
                SizeF {
                    width: 50.0,
                    height: 25.0,
                },
                PointF { x: 70.0, y: 55.0 },
            ),
            Some(PointF { x: 25.0, y: 12.5 })
        );
    }

    struct TestComponent;

    impl Component for TestComponent {
        type State = ();
        type Action = ();

        fn create(&self, _context: &mut crate::host::application::CreateContext<'_>) -> Self::State {
        }

        fn mount(
            &self,
            _state: &Self::State,
            ui: &mut crate::host::application::Ui<'_, '_, Self::Action>,
        ) -> crate::ui::UiRoot {
            ui.foundation().root(
                crate::ui::BoxStyle::default(),
                crate::ui::LayoutStyle::default(),
                |_| {},
            )
        }

        fn action(
            &self,
            _state: &mut Self::State,
            _action: Self::Action,
            _context: &mut crate::host::application::UpdateContext<'_, Self>,
        ) {
        }
    }
}
