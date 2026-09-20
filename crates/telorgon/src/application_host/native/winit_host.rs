use std::collections::BTreeMap;
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(target_os = "windows")]
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use crate::core::{MonotonicInstant, PointF, SizeF, SizeI};
use crate::input::{ButtonState, InputEvent, KeyEvent, Modifiers, PhysicalKey, PointerButton};
use crate::platform::{PendingHostFacts, PostTurnSchedule, RemainingWork, ViewId};
use crate::platform_winit::{
    ViewRegistry, WinitClockObservation, WinitWakeIntent, interpret_schedule,
};
use crate::presentation::{SurfaceMetrics, SurfaceRevision};
use crate::render::{AlphaMode, ColorSpace, RenderSceneDelta};
use crate::{AssetBundle, AssetMediaCache};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, Size};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::PhysicalKey as WinitPhysicalKey;
use winit::window::{
    CursorIcon, CustomCursor, ResizeDirection, Window, WindowAttributes, WindowId,
};

use super::HostEvent;
use super::resize::{
    LiveResizeCoordinator, PlatformResizeSignals, ResizeUpdate, SurfaceCommitPolicy,
    SurfaceResizeAction,
};
#[cfg(all(
    feature = "application-software",
    not(all(feature = "application-vulkan-windows", target_os = "windows"))
))]
use super::software::SoftwarePresentation;
#[cfg(not(all(feature = "application-vulkan-windows", target_os = "windows")))]
use crate::application_host::ReadyGuiApplication;
use crate::application_host::{
    AppError, AppResult, AppRuntimeCore, Command, ComponentDriver, CompositionDriver,
    PlatformInput, WindowOptions,
};

pub(crate) struct ManagedEventLoop {
    event_loop: EventLoop<HostEvent>,
    resize_signals: Arc<PlatformResizeSignals>,
    _profiler: crate::application_host::profiler::ManagedProfiler,
}

impl ManagedEventLoop {
    pub(crate) fn event_loop(&self) -> &EventLoop<HostEvent> {
        &self.event_loop
    }
}

pub(crate) fn create_managed_event_loop(
    profile_target: crate::application_host::profiler::ProfileTarget,
) -> AppResult<ManagedEventLoop> {
    let profiler = crate::application_host::profiler::ManagedProfiler::start(profile_target)?;
    let resize_signals = PlatformResizeSignals::new();
    let event_loop = EventLoop::<HostEvent>::with_user_event()
        .build()
        .map_err(|error| AppError::new(error.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    resize_signals.set_event_loop_proxy(event_loop.create_proxy());
    Ok(ManagedEventLoop {
        event_loop,
        resize_signals,
        _profiler: profiler,
    })
}

#[cfg(feature = "application-software")]
#[cfg(not(all(feature = "application-vulkan-windows", target_os = "windows")))]
pub fn run_gui_software(application: ReadyGuiApplication) -> AppResult<()> {
    let (driver, options, renderer, assets, pointer) = application.into_parts()?;
    if renderer == crate::application_host::Renderer::Vulkan {
        return Err(AppError::new(
            "this build does not include the Vulkan managed renderer",
        ));
    }
    let event_loop =
        create_managed_event_loop(crate::application_host::profiler::ProfileTarget::Gui)?;
    let software = SoftwarePresentation::new(event_loop.event_loop().owned_display_handle())
        .map_err(AppError::new)?;
    run_composed_managed(event_loop, driver, options, assets, pointer, software)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresentationAction {
    Idle,
    Submitted,
}

/// Runtime-prepared input to a managed renderer/presenter assembly.
pub(crate) struct PreparedPresentationFrame {
    pub(crate) changed: bool,
    pub(crate) scene_epoch: u64,
    pub(crate) metrics: SurfaceMetrics,
    pub(crate) deltas: Vec<RenderSceneDelta>,
    pub(crate) frame_interval: Duration,
    pub(crate) force_present: bool,
}

pub(crate) trait NativePresentation {
    fn attach(&mut self, window: Arc<Window>) -> Result<(), String>;
    fn resume(&mut self, _window: Arc<Window>) -> Result<(), String> {
        Ok(())
    }
    fn resize_policy(&self) -> SurfaceCommitPolicy {
        SurfaceCommitPolicy::Responsive
    }
    fn resize(&mut self, update: ResizeUpdate) -> Result<(), String>;
    fn suspend(&mut self) -> Result<(), String>;
    fn present(&mut self, frame: PreparedPresentationFrame) -> Result<PresentationAction, String>;
    fn poll(&mut self) -> Result<(), String> {
        Ok(())
    }
    /// Waits for native presentation to finish a frame carrying `metrics_revision`.
    ///
    /// The Windows resize path uses this as a bounded `WM_SIZE` barrier. Synchronous presenters
    /// already satisfy it, while worker-backed presenters override it.
    fn synchronize_resize(
        &mut self,
        _metrics_revision: u64,
        _timeout: Duration,
    ) -> Result<bool, String> {
        Ok(true)
    }
    fn shutdown(&mut self) -> Result<(), String>;
}

pub(crate) fn run_composed_managed<P>(
    event_loop: ManagedEventLoop,
    mut driver: CompositionDriver,
    options: WindowOptions,
    assets: AssetBundle,
    pointer: crate::PointerConfiguration,
    presentation: P,
) -> AppResult<()>
where
    P: NativePresentation + 'static,
{
    let proxy = event_loop.event_loop.create_proxy();
    driver.set_wake(move || {
        let _ = proxy.send_event(HostEvent::RuntimeWake);
    });
    run_managed_source(
        event_loop,
        CompositionSource {
            driver,
            assets,
            pointer,
        },
        options,
        presentation,
    )
}

fn run_managed_source<S, P>(
    event_loop: ManagedEventLoop,
    source: S,
    options: WindowOptions,
    presentation: P,
) -> AppResult<()>
where
    S: NativeRuntimeSource + 'static,
    P: NativePresentation + 'static,
{
    let proxy = event_loop.event_loop.create_proxy();
    let _exit_request = crate::application_host::exit::HostExit::register(move || {
        let _ = proxy.send_event(HostEvent::ExitRequested);
    });
    #[cfg(feature = "profiler")]
    let _profile_view = crate::profiler::enter_view(Some(crate::profiler::ProfileViewId::PRIMARY));
    #[cfg(feature = "profiler")]
    let _ = crate::profiler::register_view(
        crate::profiler::ProfileViewId::PRIMARY,
        "Application window",
    );
    #[cfg(target_os = "windows")]
    {
        let host = Rc::new(RefCell::new(NativeHost::new(
            source,
            options,
            presentation,
            Arc::clone(&event_loop.resize_signals),
            event_loop.event_loop.create_proxy(),
        )));
        let mut application = NativeHostApplication::new(Rc::clone(&host));
        event_loop
            .event_loop
            .run_app(&mut application)
            .map_err(|error| AppError::new(error.to_string()))?;
        let failure = host.borrow_mut().failure.take();
        return if let Some(error) = failure {
            Err(AppError::new(error))
        } else {
            Ok(())
        };
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut host = NativeHost::new(
            source,
            options,
            presentation,
            Arc::clone(&event_loop.resize_signals),
            event_loop.event_loop.create_proxy(),
        );
        event_loop
            .event_loop
            .run_app(&mut host)
            .map_err(|error| AppError::new(error.to_string()))?;
        if let Some(error) = host.failure {
            Err(AppError::new(error))
        } else {
            Ok(())
        }
    }
}

trait NativeRuntimeSource {
    type Driver: ComponentDriver;

    fn managed_pointer(&self) -> AppResult<ManagedPointer>;
    fn window_icon(
        &self,
        profile: &crate::AppIconProfile,
    ) -> AppResult<Option<winit::window::Icon>>;
    fn mount(self, extent: SizeI) -> AppResult<AppRuntimeCore<Self::Driver>>;
    fn close(runtime: &mut AppRuntimeCore<Self::Driver>) -> AppResult<()>;
}

struct CompositionSource {
    driver: CompositionDriver,
    assets: AssetBundle,
    pointer: crate::PointerConfiguration,
}

impl NativeRuntimeSource for CompositionSource {
    type Driver = CompositionDriver;

    fn managed_pointer(&self) -> AppResult<ManagedPointer> {
        ManagedPointer::new(self.assets, self.pointer.clone())
    }

    fn window_icon(
        &self,
        profile: &crate::AppIconProfile,
    ) -> AppResult<Option<winit::window::Icon>> {
        let Some(icon) = profile.preferred(64) else {
            return Ok(None);
        };
        let mut media =
            AssetMediaCache::new(self.assets).map_err(|error| AppError::new(error.to_string()))?;
        let size = Some(
            crate::AssetRasterSize::new(64, 64)
                .map_err(|error| AppError::new(error.to_string()))?,
        );
        let decoded = match icon.tint_color() {
            Some(tint) => media.tinted_icon(icon.source(), size, tint),
            None => media.icon(icon.source(), size),
        }
        .map_err(|error| AppError::new(error.to_string()))?;
        let rgba = straight_alpha_rgba(&decoded);
        winit::window::Icon::from_rgba(
            rgba,
            decoded.extent.width as u32,
            decoded.extent.height as u32,
        )
        .map(Some)
        .map_err(|error| AppError::new(format!("invalid native window icon: {error}")))
    }

    fn mount(self, extent: SizeI) -> AppResult<AppRuntimeCore<Self::Driver>> {
        let mut runtime = AppRuntimeCore::from_composition_driver(self.driver, extent)?;
        runtime.register_fonts(self.assets)?;
        let mut media =
            AssetMediaCache::new(self.assets).map_err(|error| AppError::new(error.to_string()))?;
        for resource in media
            .preload_render_resources()
            .map_err(|error| AppError::new(error.to_string()))?
        {
            runtime.set_image_resource(resource)?;
        }
        Ok(runtime)
    }

    fn close(runtime: &mut AppRuntimeCore<Self::Driver>) -> AppResult<()> {
        runtime.close_composition()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ManagedCursorKey {
    asset: crate::CursorAsset,
    size: u16,
    hotspot_x: u16,
    hotspot_y: u16,
    tint: Option<u32>,
}

#[derive(Clone)]
struct ManagedCursorFrame {
    cursor: CustomCursor,
    duration: Duration,
}

struct ManagedPointerAnimation {
    frames: Vec<ManagedCursorFrame>,
    index: usize,
    next_frame_at: Instant,
}

struct ManagedPointer {
    configuration: crate::PointerConfiguration,
    theme: Option<crate::PointerTheme>,
    media: AssetMediaCache,
    cursors: BTreeMap<ManagedCursorKey, CustomCursor>,
    current_request: Option<crate::PointerRequest>,
    animation: Option<ManagedPointerAnimation>,
}

impl ManagedPointer {
    fn new(bundle: AssetBundle, configuration: crate::PointerConfiguration) -> AppResult<Self> {
        let theme = configuration
            .load_theme(bundle)
            .map_err(|error| AppError::new(error.to_string()))?;
        let media =
            AssetMediaCache::new(bundle).map_err(|error| AppError::new(error.to_string()))?;
        Ok(Self {
            configuration,
            theme,
            media,
            cursors: BTreeMap::new(),
            current_request: None,
            animation: None,
        })
    }

    fn apply(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: &Window,
        request: crate::PointerRequest,
        now: Instant,
    ) -> AppResult<()> {
        if self.current_request == Some(request) {
            self.advance_animation(window, now);
            return Ok(());
        }

        self.animation = None;
        self.current_request = Some(request);
        match crate::resolve_pointer(
            request,
            self.configuration.client_cursor_mode(),
            self.configuration.pointer_overrides(),
            self.theme.as_ref(),
        ) {
            crate::PointerResolution::Hidden => window.set_cursor_visible(false),
            crate::PointerResolution::ClientSurface => {
                window.set_cursor_visible(true);
                window.set_cursor(CursorIcon::Default);
            }
            crate::PointerResolution::System(icon) => {
                window.set_cursor_visible(true);
                window.set_cursor(winit_cursor_icon(icon));
            }
            crate::PointerResolution::Graphic(graphic) => {
                let graphic = graphic.clone();
                let frames = self.custom_frames(event_loop, &graphic)?;
                let first = frames
                    .first()
                    .expect("pointer graphics always contain at least one frame");
                window.set_cursor_visible(true);
                window.set_cursor(first.cursor.clone());
                if frames.len() > 1 {
                    self.animation = Some(ManagedPointerAnimation {
                        next_frame_at: now + first.duration,
                        frames,
                        index: 0,
                    });
                }
            }
        }
        Ok(())
    }

    fn custom_frames(
        &mut self,
        event_loop: &ActiveEventLoop,
        graphic: &crate::CursorGraphic,
    ) -> AppResult<Vec<ManagedCursorFrame>> {
        let size = graphic
            .logical_size()
            .or_else(|| {
                self.theme
                    .as_ref()
                    .and_then(crate::PointerTheme::logical_size)
            })
            .unwrap_or(32);
        let hotspot = graphic.pointer_hotspot();
        if hotspot.x >= size || hotspot.y >= size {
            return Err(AppError::new(format!(
                "custom pointer hotspot ({}, {}) is outside its {}px image",
                hotspot.x, hotspot.y, size
            )));
        }
        let raster_size = crate::AssetRasterSize::new(u32::from(size), u32::from(size))
            .map_err(|error| AppError::new(error.to_string()))?;
        let mut frames = Vec::with_capacity(graphic.frames().len());
        for frame in graphic.frames() {
            let key = ManagedCursorKey {
                asset: frame.asset,
                size,
                hotspot_x: hotspot.x,
                hotspot_y: hotspot.y,
                tint: graphic.tint_color().map(crate::ColorRgba8::to_ne_u32),
            };
            let cursor = if let Some(cursor) = self.cursors.get(&key) {
                cursor.clone()
            } else {
                let decoded = match graphic.tint_color() {
                    Some(tint) => self
                        .media
                        .tinted_cursor(frame.asset, Some(raster_size), tint),
                    None => self.media.cursor(frame.asset, Some(raster_size)),
                }
                .map_err(|error| AppError::new(error.to_string()))?;
                let source = CustomCursor::from_rgba(
                    straight_alpha_rgba(&decoded),
                    u16::try_from(decoded.extent.width)
                        .map_err(|_| AppError::new("custom pointer width exceeds u16"))?,
                    u16::try_from(decoded.extent.height)
                        .map_err(|_| AppError::new("custom pointer height exceeds u16"))?,
                    hotspot.x,
                    hotspot.y,
                )
                .map_err(|error| AppError::new(format!("invalid custom pointer image: {error}")))?;
                let cursor = event_loop.create_custom_cursor(source);
                self.cursors.insert(key, cursor.clone());
                cursor
            };
            frames.push(ManagedCursorFrame {
                cursor,
                duration: Duration::from_millis(
                    frame
                        .duration_ms
                        .map_or(0, std::num::NonZeroU32::get)
                        .into(),
                ),
            });
        }
        Ok(frames)
    }

    fn advance_animation(&mut self, window: &Window, now: Instant) {
        let Some(animation) = self.animation.as_mut() else {
            return;
        };
        let mut advanced = 0;
        while now >= animation.next_frame_at && advanced < animation.frames.len() {
            animation.index = (animation.index + 1) % animation.frames.len();
            let frame = &animation.frames[animation.index];
            window.set_cursor(frame.cursor.clone());
            animation.next_frame_at += frame.duration;
            advanced += 1;
        }
        if now >= animation.next_frame_at {
            animation.next_frame_at = now + animation.frames[animation.index].duration;
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.animation
            .as_ref()
            .map(|animation| animation.next_frame_at)
    }
}

fn straight_alpha_rgba(image: &crate::DecodedAssetImage) -> Vec<u8> {
    let mut rgba = image.pixels_rgba8.to_vec();
    if image.alpha_mode == crate::render::ImageAlphaMode::Premultiplied {
        for pixel in rgba.chunks_exact_mut(4) {
            let alpha = u16::from(pixel[3]);
            if alpha == 0 {
                pixel[..3].fill(0);
            } else {
                for channel in &mut pixel[..3] {
                    *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
        }
    }
    rgba
}

struct NativeHost<S: NativeRuntimeSource, P: NativePresentation> {
    pending_source: Option<S>,
    options: WindowOptions,
    runtime: Option<AppRuntimeCore<S::Driver>>,
    presentation: P,
    window: Option<Arc<Window>>,
    started: Instant,
    pending_resize: PendingResize,
    live_resize: LiveResizeCoordinator,
    resize_signals: Arc<PlatformResizeSignals>,
    event_proxy: EventLoopProxy<HostEvent>,
    views: ViewRegistry,
    view: Option<ViewId>,
    redraw: RedrawDemand,
    frame_pacer: FramePacer,
    drawable: bool,
    occluded: bool,
    suspended: bool,
    host_wake_pending: bool,
    cursor_position: PointF,
    pointer: Option<ManagedPointer>,
    diagnostics: NativeHostDiagnostics,
    failure: Option<String>,
}

/// Keeps managed application state interior-mutable so the Windows native resize subclass can
/// execute a frame while `DefWindowProc` owns its nested move/size loop.
#[cfg(target_os = "windows")]
struct NativeHostApplication<S: NativeRuntimeSource, P: NativePresentation> {
    host: Rc<RefCell<NativeHost<S, P>>>,
    live_resize_handler_installed: bool,
}

#[cfg(target_os = "windows")]
impl<S: NativeRuntimeSource, P: NativePresentation> NativeHostApplication<S, P> {
    fn new(host: Rc<RefCell<NativeHost<S, P>>>) -> Self {
        Self {
            host,
            live_resize_handler_installed: false,
        }
    }

    fn install_live_resize_handler(&mut self) -> Result<(), String>
    where
        S: 'static,
        P: 'static,
    {
        if self.live_resize_handler_installed {
            return Ok(());
        }
        let (signals, window) = {
            let host = self.host.borrow();
            let Some(window) = host.window.clone() else {
                return Ok(());
            };
            (Arc::clone(&host.resize_signals), window)
        };
        let host: Weak<RefCell<NativeHost<S, P>>> = Rc::downgrade(&self.host);
        signals.set_live_resize_handler(
            &window,
            Rc::new(
                move |extent, observed_at, synchronize_present, repeat_extent| {
                    let Some(host) = host.upgrade() else {
                        return;
                    };
                    // A nested native callback can occur while a normal Winit callback is active.
                    // Skipping that one tick is safe; the next WM_SIZE/timer tick carries the latest
                    // extent and avoids a RefCell panic or re-entrant runtime turn.
                    let Ok(mut host) = host.try_borrow_mut() else {
                        return;
                    };
                    host.native_live_resize_tick(
                        extent,
                        observed_at,
                        synchronize_present,
                        repeat_extent,
                    );
                },
            ),
        )?;
        self.live_resize_handler_installed = true;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum RedrawReason {
    Runtime = 0,
    Input = 1,
    Command = 2,
    ExternalWake = 3,
    Animation = 4,
    Timer = 5,
    Startup = 6,
    Resize = 7,
    Expose = 8,
    Recovery = 9,
    OperatingSystem = 10,
    PointerMove = 11,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RedrawSource {
    NativeCallback,
    SynchronousResize,
    SynchronousResizeBarrier,
}

impl RedrawReason {
    const COUNT: usize = 12;

    const fn bit(self) -> u16 {
        1 << self as u8
    }

    const fn forces_present(self) -> bool {
        matches!(
            self,
            Self::Startup | Self::Resize | Self::Expose | Self::Recovery | Self::OperatingSystem
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RedrawReasons(u16);

impl RedrawReasons {
    fn insert(&mut self, reason: RedrawReason) -> bool {
        let bit = reason.bit();
        let inserted = self.0 & bit == 0;
        self.0 |= bit;
        inserted
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn force_present(self) -> bool {
        [
            RedrawReason::Startup,
            RedrawReason::Resize,
            RedrawReason::Expose,
            RedrawReason::Recovery,
            RedrawReason::OperatingSystem,
        ]
        .into_iter()
        .any(|reason| reason.forces_present() && self.0 & reason.bit() != 0)
    }

    #[cfg(any(feature = "profiler", test))]
    fn pointer_move_only(self) -> bool {
        let allowed = RedrawReason::PointerMove.bit() | RedrawReason::Runtime.bit();
        self.0 & RedrawReason::PointerMove.bit() != 0 && self.0 & !allowed == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RedrawDemand {
    reasons: RedrawReasons,
    native_request_pending: bool,
}

impl RedrawDemand {
    fn mark(&mut self, reason: RedrawReason) -> bool {
        self.reasons.insert(reason)
    }

    const fn has_demand(&self) -> bool {
        !self.reasons.is_empty()
    }

    fn requires_immediate_presentation(&self) -> bool {
        self.reasons.force_present()
    }

    fn queue_native_request(&mut self) -> bool {
        if self.native_request_pending {
            false
        } else {
            self.native_request_pending = true;
            true
        }
    }

    fn native_callback_started(&mut self) -> bool {
        std::mem::take(&mut self.native_request_pending)
    }

    fn cancel_native_request(&mut self) {
        self.native_request_pending = false;
    }

    fn take_reasons(&mut self) -> RedrawReasons {
        std::mem::take(&mut self.reasons)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeHostDiagnostics {
    native_pointer_moves: u64,
    input_turns: u64,
    clean_input_turns: u64,
    redraw_requests: u64,
    redraw_requests_suppressed: u64,
    redraw_callbacks: u64,
    presentations_idle: u64,
    presentations_submitted: u64,
    redraw_reasons: [u64; RedrawReason::COUNT],
}

impl Default for NativeHostDiagnostics {
    fn default() -> Self {
        Self {
            native_pointer_moves: 0,
            input_turns: 0,
            clean_input_turns: 0,
            redraw_requests: 0,
            redraw_requests_suppressed: 0,
            redraw_callbacks: 0,
            presentations_idle: 0,
            presentations_submitted: 0,
            redraw_reasons: [0; RedrawReason::COUNT],
        }
    }
}

#[derive(Clone, Debug, Default)]
struct PendingResize {
    extent: Option<SizeI>,
    last_applied_extent: Option<SizeI>,
    last_applied_at: Option<Instant>,
}

/// Caps application-driven frame preparation without delaying runtime/input turns.
///
/// Deadlines advance from the most recent frame that actually started. A late callback therefore
/// schedules its successor from the late time instead of issuing catch-up frames.
#[derive(Clone, Debug, Default)]
struct FramePacer {
    last_started_at: Option<Instant>,
}

impl FramePacer {
    fn throttle_deadline(&self, now: Instant, interval: Duration) -> Option<Instant> {
        let deadline = self.last_started_at?.checked_add(interval)?;
        (now < deadline).then_some(deadline)
    }

    fn frame_started(&mut self, now: Instant) {
        self.last_started_at = Some(now);
    }
}

impl PendingResize {
    fn queue(&mut self, extent: SizeI) -> bool {
        if self.extent == Some(extent)
            || (self.extent.is_none() && self.last_applied_extent == Some(extent))
        {
            return false;
        }
        self.extent = Some(extent);
        true
    }

    fn queue_for_barrier(&mut self, extent: SizeI) -> bool {
        if self.extent == Some(extent) {
            return false;
        }
        self.extent = Some(extent);
        true
    }

    fn is_due(&self, now: Instant, interval: Duration) -> bool {
        self.extent.is_some()
            && self.last_applied_at.is_none_or(|last_applied_at| {
                now.saturating_duration_since(last_applied_at) >= interval
            })
    }

    fn next_due_at(&self, interval: Duration) -> Option<Instant> {
        self.extent?;
        self.last_applied_at
            .and_then(|last_applied_at| last_applied_at.checked_add(interval))
    }

    fn take(&mut self, now: Instant) -> Option<SizeI> {
        let extent = self.extent.take()?;
        self.last_applied_extent = Some(extent);
        self.last_applied_at = Some(now);
        Some(extent)
    }

    fn is_pending(&self) -> bool {
        self.extent.is_some()
    }
}

const DEFAULT_REFRESH_MILLIHERTZ: u32 = 60_000;

fn frame_interval_for_refresh_rate(refresh_millihertz: Option<u32>) -> Duration {
    let refresh_millihertz = refresh_millihertz
        .filter(|refresh_millihertz| *refresh_millihertz > 0)
        .unwrap_or(DEFAULT_REFRESH_MILLIHERTZ);
    Duration::from_nanos(1_000_000_000_000_u64.div_ceil(u64::from(refresh_millihertz)))
}

impl<S: NativeRuntimeSource, P: NativePresentation> NativeHost<S, P> {
    fn new(
        source: S,
        options: WindowOptions,
        presentation: P,
        resize_signals: Arc<PlatformResizeSignals>,
        event_proxy: EventLoopProxy<HostEvent>,
    ) -> Self {
        Self {
            pending_source: Some(source),
            options,
            runtime: None,
            presentation,
            window: None,
            started: Instant::now(),
            pending_resize: PendingResize::default(),
            live_resize: LiveResizeCoordinator::default(),
            resize_signals,
            event_proxy,
            views: ViewRegistry::new(NonZeroU16::MIN)
                .expect("one managed Winit view is within the adapter bound"),
            view: None,
            redraw: RedrawDemand::default(),
            frame_pacer: FramePacer::default(),
            drawable: false,
            occluded: false,
            suspended: false,
            host_wake_pending: false,
            cursor_position: PointF::default(),
            pointer: None,
            diagnostics: NativeHostDiagnostics::default(),
            failure: None,
        }
    }

    fn timestamp_at(&self, now: Instant) -> MonotonicInstant {
        MonotonicInstant::from_nanos(
            now.saturating_duration_since(self.started)
                .as_nanos()
                .min(u64::MAX as u128) as u64,
        )
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, message: impl Into<String>) {
        self.record_failure(message);
        event_loop.exit();
    }

    fn record_failure(&mut self, message: impl Into<String>) {
        let message = message.into();
        eprintln!("telorgon-app: {message}");
        self.failure = Some(message);
    }

    fn poll_presentation(&mut self, event_loop: &ActiveEventLoop) -> bool {
        match self.presentation.poll() {
            Ok(()) => true,
            Err(error) => {
                self.fail(event_loop, error.to_string());
                false
            }
        }
    }

    fn mark_redraw(&mut self, reason: RedrawReason) {
        if self.redraw.mark(reason) {
            self.diagnostics.redraw_reasons[reason as usize] =
                self.diagnostics.redraw_reasons[reason as usize].saturating_add(1);
        }
    }

    fn custom_chrome_action(&self) -> Option<crate::WindowAction> {
        if self.options.decorations != crate::application_host::WindowDecorationMode::Hidden {
            return None;
        }
        let runtime = self.runtime.as_ref()?;
        let snapshot = crate::WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).ok()?;
        match snapshot.hit_test(self.cursor_position.x, self.cursor_position.y) {
            Some(crate::WindowChromeRole::DragRegion) => Some(crate::WindowAction::BeginMove),
            Some(crate::WindowChromeRole::Action(action)) => Some(action),
            _ => None,
        }
    }

    fn pointer_request(&self) -> crate::PointerRequest {
        let Some(runtime) = self.runtime.as_ref() else {
            return crate::PointerRequest::Semantic(crate::PointerIcon::Default);
        };
        if self.options.decorations == crate::application_host::WindowDecorationMode::Hidden
            && let Ok(snapshot) =
                crate::WindowChromeSnapshot::derive(runtime.ui(), runtime.layout())
            && let Some(region) =
                snapshot.hit_test_region(self.cursor_position.x, self.cursor_position.y)
        {
            if let Some(request) = runtime.ui().pointer_requests.get(region.node).copied() {
                return request;
            }
            match region.role {
                crate::WindowChromeRole::DragRegion => {
                    return crate::PointerRequest::Semantic(crate::PointerIcon::Move);
                }
                crate::WindowChromeRole::Action(crate::WindowAction::BeginResize(edge)) => {
                    return crate::PointerRequest::Semantic(resize_pointer_icon(edge));
                }
                crate::WindowChromeRole::Action(crate::WindowAction::BeginMove) => {
                    return crate::PointerRequest::Semantic(crate::PointerIcon::Move);
                }
                crate::WindowChromeRole::Action(_) => {
                    return crate::PointerRequest::Semantic(crate::PointerIcon::Pointer);
                }
                _ => {}
            }
        }

        runtime
            .ui()
            .pointer_requests
            .iter()
            .filter_map(|(node, request)| {
                runtime.layout().computed(node).and_then(|computed| {
                    (computed.visible_rect.contains(self.cursor_position)
                        && computed.border_rect.contains(self.cursor_position))
                    .then_some((*request, computed.border_rect.area()))
                })
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(request, _)| request)
            .unwrap_or(crate::PointerRequest::Semantic(crate::PointerIcon::Default))
    }

    fn refresh_pointer(&mut self, event_loop: &ActiveEventLoop, now: Instant) -> bool {
        let request = self.pointer_request();
        let Some(window) = self.window.clone() else {
            return true;
        };
        let Some(pointer) = self.pointer.as_mut() else {
            return true;
        };
        if let Err(error) = pointer.apply(event_loop, &window, request, now) {
            self.fail(
                event_loop,
                format!("failed to apply managed pointer theme: {error}"),
            );
            return false;
        }
        true
    }

    fn apply_custom_chrome_action(
        &mut self,
        event_loop: &ActiveEventLoop,
        action: crate::WindowAction,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let result = match action {
            crate::WindowAction::Close => {
                if let Some(runtime) = self.runtime.as_mut()
                    && let Err(error) = S::close(runtime)
                {
                    self.fail(event_loop, format!("component close failed: {error}"));
                    return;
                }
                self.flush_commands();
                event_loop.exit();
                return;
            }
            crate::WindowAction::Minimize => {
                window.set_minimized(true);
                Ok(())
            }
            crate::WindowAction::ToggleMaximize => {
                window.set_maximized(!window.is_maximized());
                Ok(())
            }
            crate::WindowAction::BeginMove => window.drag_window(),
            crate::WindowAction::BeginResize(edge) => {
                window.drag_resize_window(winit_resize_direction(edge))
            }
            crate::WindowAction::ShowSystemMenu => Ok(()),
        };
        if let Err(error) = result {
            self.fail(
                event_loop,
                format!("custom window-frame action failed: {error}"),
            );
        }
    }

    fn redraw_eligible(&self) -> bool {
        self.drawable && !self.occluded && !self.suspended
    }

    fn request_redraw_once(&mut self) {
        if !self.redraw_eligible() || !self.redraw.has_demand() {
            return;
        }
        if !self.redraw.queue_native_request() {
            self.diagnostics.redraw_requests_suppressed = self
                .diagnostics
                .redraw_requests_suppressed
                .saturating_add(1);
            return;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
            self.diagnostics.redraw_requests = self.diagnostics.redraw_requests.saturating_add(1);
        } else {
            self.redraw.cancel_native_request();
        }
    }

    fn emit_host_diagnostics(&self) {
        #[cfg(feature = "profiler")]
        {
            crate::profiler::counter!(
                "host.pointer_moves.received",
                self.diagnostics.native_pointer_moves
            );
            crate::profiler::counter!("host.input_turns", self.diagnostics.input_turns);
            crate::profiler::counter!("host.input_turns.clean", self.diagnostics.clean_input_turns);
            crate::profiler::counter!("host.redraw_requests", self.diagnostics.redraw_requests);
            crate::profiler::counter!(
                "host.redraw_requests.suppressed",
                self.diagnostics.redraw_requests_suppressed
            );
            crate::profiler::counter!("host.redraw_callbacks", self.diagnostics.redraw_callbacks);
            crate::profiler::counter!(
                "host.presentations.idle",
                self.diagnostics.presentations_idle
            );
            crate::profiler::counter!(
                "host.presentations.submitted",
                self.diagnostics.presentations_submitted
            );
            crate::profiler::counter!(
                "host.redraw_reason.runtime",
                self.diagnostics.redraw_reasons[RedrawReason::Runtime as usize]
            );
            crate::profiler::counter!(
                "host.redraw_reason.input",
                self.diagnostics.redraw_reasons[RedrawReason::Input as usize]
            );
            crate::profiler::counter!(
                "host.redraw_reason.command",
                self.diagnostics.redraw_reasons[RedrawReason::Command as usize]
            );
            crate::profiler::counter!(
                "host.redraw_reason.external_wake",
                self.diagnostics.redraw_reasons[RedrawReason::ExternalWake as usize]
            );
            crate::profiler::counter!(
                "host.redraw_reason.animation",
                self.diagnostics.redraw_reasons[RedrawReason::Animation as usize]
            );
            crate::profiler::counter!(
                "host.redraw_reason.timer",
                self.diagnostics.redraw_reasons[RedrawReason::Timer as usize]
            );
            crate::profiler::counter!(
                "host.redraw_reason.startup",
                self.diagnostics.redraw_reasons[RedrawReason::Startup as usize]
            );
            crate::profiler::counter!(
                "host.redraw_reason.resize",
                self.diagnostics.redraw_reasons[RedrawReason::Resize as usize]
            );
            crate::profiler::counter!(
                "host.redraw_reason.expose",
                self.diagnostics.redraw_reasons[RedrawReason::Expose as usize]
            );
            crate::profiler::counter!(
                "host.redraw_reason.recovery",
                self.diagnostics.redraw_reasons[RedrawReason::Recovery as usize]
            );
            crate::profiler::counter!(
                "host.redraw_reason.os",
                self.diagnostics.redraw_reasons[RedrawReason::OperatingSystem as usize]
            );
            crate::profiler::counter!(
                "host.redraw_reason.pointer_move",
                self.diagnostics.redraw_reasons[RedrawReason::PointerMove as usize]
            );
        }
    }

    fn refresh_frame_interval(&self) -> Duration {
        let refresh_millihertz = self
            .window
            .as_ref()
            .and_then(|window| window.current_monitor())
            .and_then(|monitor| monitor.refresh_rate_millihertz());
        frame_interval_for_refresh_rate(refresh_millihertz)
    }

    fn resize_signal(&self) -> Option<super::resize::ResizeSignalSnapshot> {
        self.window
            .as_deref()
            .and_then(|window| self.resize_signals.snapshot(window))
    }

    fn finalize_live_resize(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let signal = self.resize_signal();
        if !self.live_resize.needs_finalization(signal) {
            return true;
        }
        if self.pending_resize.is_pending() {
            self.mark_redraw(RedrawReason::Resize);
            self.redraw(event_loop, RedrawSource::SynchronousResize);
            return self.failure.is_none();
        }
        let policy = self.presentation.resize_policy();
        let Some(update) = self.live_resize.finalize(signal, policy) else {
            return true;
        };
        if let Err(error) = self.presentation.resize(update) {
            self.fail(event_loop, error);
            return false;
        }
        self.mark_redraw(RedrawReason::Resize);
        true
    }

    fn complete_live_resize_release(
        &mut self,
        event_loop: &ActiveEventLoop,
        signal: super::resize::ResizeSignalSnapshot,
        _observed_at: Instant,
    ) -> bool {
        // A native request made during the modal sizing loop may never deliver its callback until
        // well after WM_EXITSIZEMOVE. The synchronous release frame satisfies that request, so it
        // must no longer suppress subsequent animation or input redraws.
        self.redraw.cancel_native_request();
        // The direct Win32 stream is authoritative during the modal transaction. Re-read the
        // client size at release so a skipped nested callback or delayed Winit event cannot make
        // the final transaction commit an intermediate extent.
        if let Some(window) = self.window.as_ref() {
            let size = window.inner_size();
            let current_extent = SizeI {
                width: size.width as i32,
                height: size.height as i32,
            };
            if self.pending_resize.queue(current_extent) {
                self.mark_redraw(RedrawReason::Resize);
            }
        }
        #[cfg(feature = "profiler")]
        let submissions_before = self.diagnostics.presentations_submitted;

        if self.pending_resize.is_pending() {
            self.mark_redraw(RedrawReason::Resize);
            self.redraw(event_loop, RedrawSource::SynchronousResize);
        } else {
            let policy = self.presentation.resize_policy();
            if let Some(update) = self.live_resize.finalize(Some(signal), policy) {
                if let Err(error) = self.presentation.resize(update) {
                    self.fail(event_loop, error);
                    return false;
                }
            }
            // Resize release is a mandatory frame boundary even when the final WM_SIZE was already
            // consumed. This publishes the committed surface and resumes animation immediately.
            self.mark_redraw(RedrawReason::Resize);
            self.redraw(event_loop, RedrawSource::SynchronousResize);
        }
        if self.failure.is_some() {
            return false;
        }

        #[cfg(feature = "profiler")]
        if self.diagnostics.presentations_submitted > submissions_before {
            crate::profiler::instant!("responsiveness.resize.final_frame_submitted");
            crate::profiler::counter!(
                "responsiveness.resize.release_to_submit_ms",
                _observed_at.elapsed().as_secs_f64() * 1_000.0
            );
        }

        self.apply_post_turn_schedule(event_loop, Instant::now())
    }

    fn flush_commands(&mut self) {
        #[cfg(feature = "profiler")]
        let _span = crate::profiler::span!("commands.flush");
        loop {
            let command = self
                .runtime
                .as_mut()
                .and_then(|runtime| runtime.pop_command());
            let Some(command) = command else { break };
            match command {
                Command::RequestFrame => self.mark_redraw(RedrawReason::Command),
            }
        }
    }

    fn process_runtime_turn(&mut self, timestamp: MonotonicInstant) {
        let pending = self
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.has_pending_runtime_turn(timestamp));
        if !pending {
            return;
        }
        let outcome = {
            #[cfg(feature = "profiler")]
            let _span = crate::profiler::span!("input.flush");
            self.runtime
                .as_mut()
                .expect("pending runtime work requires a mounted runtime")
                .flush_input(timestamp)
        };
        self.diagnostics.input_turns = self.diagnostics.input_turns.saturating_add(1);
        if outcome.processed_work() && !outcome.frame_became_needed() {
            self.diagnostics.clean_input_turns =
                self.diagnostics.clean_input_turns.saturating_add(1);
        }
        if outcome.frame_became_needed() {
            if outcome.timers_processed != 0 {
                self.mark_redraw(RedrawReason::Timer);
            } else if outcome.pointer_move_only_frame_became_needed() {
                self.mark_redraw(RedrawReason::PointerMove);
            } else if outcome.events_dispatched != 0 {
                self.mark_redraw(RedrawReason::Input);
            } else {
                self.mark_redraw(RedrawReason::ExternalWake);
            }
        }
    }

    fn mark_runtime_frame_demand(&mut self) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        if runtime.needs_frame() {
            let reason = if runtime.animation_active() {
                RedrawReason::Animation
            } else {
                RedrawReason::Runtime
            };
            self.mark_redraw(reason);
        }
    }

    fn apply_post_turn_schedule(
        &mut self,
        event_loop: &ActiveEventLoop,
        native_now: Instant,
    ) -> bool {
        let timestamp = self.timestamp_at(native_now);
        #[cfg(feature = "profiler")]
        let _pointer_profile_suppression = (!crate::profiler::pointer_move_events_enabled()
            && self.runtime.as_ref().is_some_and(|runtime| {
                runtime.pending_runtime_turn_is_pointer_move_only(timestamp)
            }))
        .then(crate::profiler::suppress_current_thread);
        self.process_runtime_turn(timestamp);
        self.flush_commands();
        self.mark_runtime_frame_demand();
        if !self.refresh_pointer(event_loop, native_now) {
            return false;
        }

        let runtime_pending = self
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.has_pending_runtime_turn(timestamp));
        let next_deadline = self
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.next_deadline());
        let live_resize_active = cfg!(target_os = "windows")
            && self
                .resize_signal()
                .is_some_and(super::resize::ResizeSignalSnapshot::is_active);
        let redraw_eligible = self.redraw_eligible();
        let redraw_demanded = self.redraw.has_demand();
        let frame_interval = (live_resize_active || (redraw_eligible && redraw_demanded))
            .then(|| self.refresh_frame_interval());
        let resize_throttled = live_resize_active
            && self.pending_resize.is_pending()
            && !self.pending_resize.is_due(
                native_now,
                frame_interval.expect("live resize needs a frame interval"),
            );
        let redraw_pacing_deadline = if redraw_eligible
            && redraw_demanded
            && !self.redraw.requires_immediate_presentation()
            && !resize_throttled
        {
            self.frame_pacer.throttle_deadline(
                native_now,
                frame_interval.expect("redraw demand needs a frame interval"),
            )
        } else {
            None
        };
        let redraw_throttled = resize_throttled || redraw_pacing_deadline.is_some();
        let redraw_view = (redraw_eligible && redraw_demanded && !redraw_throttled)
            .then_some(self.view)
            .flatten();
        let redraw_views = redraw_view.as_slice();
        let schedule = match PostTurnSchedule::new(
            RemainingWork::new(runtime_pending, false, false, false),
            redraw_views,
            next_deadline,
            PendingHostFacts::new(self.host_wake_pending, false),
        ) {
            Ok(schedule) => schedule,
            Err(error) => {
                self.fail(
                    event_loop,
                    format!("failed to publish post-turn schedule: {error}"),
                );
                return false;
            }
        };
        let plan = match interpret_schedule(
            &schedule,
            &self.views,
            WinitClockObservation::new(timestamp, native_now),
        ) {
            Ok(plan) => plan,
            Err(error) => {
                self.fail(
                    event_loop,
                    format!("failed to interpret Winit schedule: {error}"),
                );
                return false;
            }
        };

        if plan.wake_intent() == WinitWakeIntent::RequestWake {
            if self.event_proxy.send_event(HostEvent::RuntimeWake).is_err() {
                self.fail(
                    event_loop,
                    "failed to request the next managed runtime turn",
                );
                return false;
            }
            self.host_wake_pending = true;
        }

        let mut control_flow = plan.control_flow();
        if live_resize_active
            && let Some(resize_deadline) = self
                .pending_resize
                .next_due_at(frame_interval.expect("live resize needs a frame interval"))
        {
            control_flow = earlier_wait_deadline(control_flow, resize_deadline);
        }
        if let Some(redraw_deadline) = redraw_pacing_deadline {
            control_flow = earlier_wait_deadline(control_flow, redraw_deadline);
        }
        if let Some(pointer_deadline) = self
            .pointer
            .as_ref()
            .and_then(ManagedPointer::next_deadline)
        {
            control_flow = earlier_wait_deadline(control_flow, pointer_deadline);
        }
        event_loop.set_control_flow(control_flow);

        if !plan.redraw_targets().is_empty() {
            debug_assert_eq!(plan.redraw_targets().len(), 1);
            self.request_redraw_once();
        }
        self.emit_host_diagnostics();
        true
    }

    #[cfg(target_os = "windows")]
    fn native_live_resize_tick(
        &mut self,
        extent: SizeI,
        observed_at: Instant,
        synchronize_present: bool,
        repeat_extent: bool,
    ) {
        if self.failure.is_some()
            || !self
                .resize_signal()
                .is_some_and(super::resize::ResizeSignalSnapshot::is_active)
        {
            return;
        }

        self.drawable = extent.width > 0 && extent.height > 0;
        let queued = if repeat_extent {
            self.pending_resize.queue_for_barrier(extent)
        } else {
            self.pending_resize.queue(extent)
        };
        if queued {
            self.mark_redraw(RedrawReason::Resize);
        }
        let frame_interval = self.refresh_frame_interval();
        let resize_due =
            synchronize_present || self.pending_resize.is_due(observed_at, frame_interval);
        let redraw_due = synchronize_present
            || (self.redraw.has_demand()
                && self
                    .frame_pacer
                    .throttle_deadline(observed_at, frame_interval)
                    .is_none());
        if (resize_due || redraw_due)
            && let Err(error) = self.try_redraw(if synchronize_present {
                RedrawSource::SynchronousResizeBarrier
            } else {
                RedrawSource::SynchronousResize
            })
        {
            self.record_failure(error);
            // The proxy message may be buffered until the native loop exits, but it guarantees
            // the ordinary host path observes the failure and terminates afterward.
            let _ = self.event_proxy.send_event(HostEvent::RuntimeWake);
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop, source: RedrawSource) {
        if let Err(error) = self.try_redraw(source) {
            self.fail(event_loop, error);
        }
    }

    fn try_redraw(&mut self, source: RedrawSource) -> Result<(), String> {
        let requested_callback = if source == RedrawSource::NativeCallback {
            self.diagnostics.redraw_callbacks = self.diagnostics.redraw_callbacks.saturating_add(1);
            self.redraw.native_callback_started()
        } else {
            false
        };
        self.presentation.poll()?;
        let mut resize_barrier_revision = None;
        if let Some(extent) = self.pending_resize.take(Instant::now()) {
            self.drawable = extent.width > 0 && extent.height > 0;
            let signal = self.resize_signal();
            let policy = self.presentation.resize_policy();
            let update = self.live_resize.observe(extent, signal, policy);
            #[cfg(feature = "profiler")]
            match update.phase {
                super::resize::ResizeInteractionPhase::Stable => {
                    crate::profiler::instant!("responsiveness.resize.stable");
                }
                super::resize::ResizeInteractionPhase::Started => {
                    crate::profiler::instant!("responsiveness.resize.started");
                }
                super::resize::ResizeInteractionPhase::Updating => {
                    crate::profiler::instant!("responsiveness.resize.updating");
                }
                super::resize::ResizeInteractionPhase::Ended => {
                    crate::profiler::instant!("responsiveness.resize.ended");
                }
                super::resize::ResizeInteractionPhase::Cancelled => {
                    crate::profiler::instant!("responsiveness.resize.cancelled");
                }
            }
            self.presentation.resize(update)?;
            resize_barrier_revision = resize_revision_to_synchronize(source, update);
            self.mark_redraw(RedrawReason::Resize);
            if self.drawable
                && let Some(runtime) = self.runtime.as_mut()
            {
                runtime.queue_input(PlatformInput::Resize(SizeF {
                    width: extent.width as f32,
                    height: extent.height as f32,
                }));
            }
        }
        if !self.redraw_eligible() {
            self.redraw.cancel_native_request();
            self.flush_commands();
            return Ok(());
        }
        let frame_started_at = Instant::now();
        self.frame_pacer.frame_started(frame_started_at);
        #[cfg(feature = "profiler")]
        let _frame = crate::profiler::start_frame("frame.total");
        let timestamp = self.timestamp_at(frame_started_at);
        let frame_interval = self.refresh_frame_interval();
        self.process_runtime_turn(timestamp);
        self.flush_commands();
        self.mark_runtime_frame_demand();
        if (source == RedrawSource::NativeCallback && !requested_callback)
            || !self.redraw.has_demand()
        {
            self.mark_redraw(RedrawReason::OperatingSystem);
        }
        let reasons = self.redraw.take_reasons();
        let force_present = reasons.force_present();
        #[cfg(feature = "profiler")]
        crate::profiler::counter!(
            "frame.trigger.pointer_move_only",
            u8::from(reasons.pointer_move_only())
        );
        let result = if let Some(runtime) = self.runtime.as_mut() {
            #[cfg(feature = "profiler")]
            crate::profiler::counter!("frame.refresh_interval_ns", frame_interval.as_nanos());
            let prepared = runtime
                .prepare_frame(timestamp, false)
                .map_err(|error| format!("frame preparation failed: {error}"))?;
            let mut deltas = Vec::new();
            {
                #[cfg(feature = "profiler")]
                let _span = crate::profiler::span!("transport.coalesce");
                while let Some(delta) = runtime.pop_scene_delta() {
                    deltas.push(delta);
                }
            }
            let physical_extent = self
                .live_resize
                .latest_extent()
                .or_else(|| {
                    self.window.as_ref().map(|window| {
                        let size = window.inner_size();
                        SizeI {
                            width: size.width as i32,
                            height: size.height as i32,
                        }
                    })
                })
                .unwrap_or_default();
            let frame = PreparedPresentationFrame {
                changed: prepared.changed,
                scene_epoch: prepared.scene_epoch,
                metrics: SurfaceMetrics {
                    revision: SurfaceRevision::new(self.live_resize.latest_metrics_revision()),
                    logical_extent: runtime.extent(),
                    physical_extent,
                    scale_factor: self
                        .window
                        .as_ref()
                        .map_or(1.0, |window| window.scale_factor()),
                    color_space: ColorSpace::Srgb,
                    alpha_mode: AlphaMode::Opaque,
                }
                .validate()
                .map_err(|error| error.to_string())?,
                deltas,
                frame_interval,
                force_present,
            };
            {
                #[cfg(feature = "profiler")]
                let _span = crate::profiler::span!("presentation.render");
                self.presentation.present(frame)
            }
        } else {
            return Ok(());
        };
        match result {
            Ok(PresentationAction::Idle) => {
                self.diagnostics.presentations_idle =
                    self.diagnostics.presentations_idle.saturating_add(1);
            }
            Ok(PresentationAction::Submitted) => {
                self.diagnostics.presentations_submitted =
                    self.diagnostics.presentations_submitted.saturating_add(1);
            }
            Err(error) => return Err(error),
        }
        if let Some(metrics_revision) = resize_barrier_revision {
            let synchronized = self
                .presentation
                .synchronize_resize(metrics_revision, WINDOW_RESIZE_PRESENT_TIMEOUT)?;
            #[cfg(not(any(target_os = "windows", feature = "profiler")))]
            let _ = synchronized;
            #[cfg(target_os = "windows")]
            if synchronized {
                flush_windows_compositor();
            }
            #[cfg(feature = "profiler")]
            if !synchronized {
                crate::profiler::instant!("responsiveness.resize.present_barrier_timeout");
            }
        }
        self.flush_commands();
        Ok(())
    }
}

#[cfg(target_os = "windows")]
const WINDOW_RESIZE_PRESENT_TIMEOUT: Duration = Duration::from_millis(100);

#[cfg(not(target_os = "windows"))]
const WINDOW_RESIZE_PRESENT_TIMEOUT: Duration = Duration::ZERO;

#[cfg(target_os = "windows")]
fn flush_windows_compositor() {
    // A correctly-sized present followed by DwmFlush keeps DWM from exposing the old-size backing
    // surface after this WM_SIZE callback returns.
    let result = unsafe { windows_sys::Win32::Graphics::Dwm::DwmFlush() };
    #[cfg(feature = "profiler")]
    if result < 0 {
        crate::profiler::instant!("responsiveness.resize.dwm_flush_failed");
    }
    #[cfg(not(feature = "profiler"))]
    let _ = result;
}

impl<S, P> ApplicationHandler<HostEvent> for NativeHost<S, P>
where
    S: NativeRuntimeSource + 'static,
    P: NativePresentation + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.suspended = false;
        if let Some(window) = self.window.clone() {
            if let Err(error) = self.presentation.resume(Arc::clone(&window)) {
                self.fail(event_loop, error);
                return;
            }
            let size = window.inner_size();
            self.drawable = size.width > 0 && size.height > 0;
            let _ = self.pending_resize.queue(SizeI {
                width: size.width as i32,
                height: size.height as i32,
            });
            self.mark_redraw(RedrawReason::Recovery);
            return;
        }
        let Some(source) = self.pending_source.take() else {
            self.fail(
                event_loop,
                "native application cannot be resumed after its state was lost",
            );
            return;
        };
        let options = &self.options;
        let window_icon = match source.window_icon(&options.icon) {
            Ok(icon) => icon,
            Err(error) => {
                self.fail(event_loop, error.to_string());
                return;
            }
        };
        let pointer = match source.managed_pointer() {
            Ok(pointer) => pointer,
            Err(error) => {
                self.fail(event_loop, error.to_string());
                return;
            }
        };
        let mut attributes = WindowAttributes::default()
            .with_title(options.title.clone())
            .with_decorations(
                options.decorations == crate::application_host::WindowDecorationMode::System,
            )
            .with_window_icon(window_icon)
            .with_inner_size(Size::Logical(LogicalSize::new(
                f64::from(options.size.width.max(1)),
                f64::from(options.size.height.max(1)),
            )));
        if let Some(minimum) = options.min_size {
            attributes = attributes.with_min_inner_size(Size::Logical(LogicalSize::new(
                f64::from(minimum.width.max(1)),
                f64::from(minimum.height.max(1)),
            )));
        }
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, format!("window creation failed: {error}"));
                return;
            }
        };
        if let Err(error) = self.resize_signals.register_window(&window) {
            self.fail(event_loop, error);
            return;
        }
        let registration = match self.views.register(window.id()) {
            Ok(registration) => registration,
            Err(error) => {
                self.fail(
                    event_loop,
                    format!("failed to register managed view: {error}"),
                );
                return;
            }
        };
        self.view = Some(registration.view);
        self.window = Some(Arc::clone(&window));
        self.pointer = Some(pointer);
        if let Err(error) = self.presentation.attach(Arc::clone(&window)) {
            self.fail(event_loop, error);
            return;
        }
        let mut runtime = match source.mount(options.size) {
            Ok(runtime) => runtime,
            Err(error) => {
                self.fail(event_loop, format!("application mount failed: {error}"));
                return;
            }
        };
        let size = window.inner_size();
        self.drawable = size.width > 0 && size.height > 0;
        runtime.queue_input(PlatformInput::Resize(SizeF {
            width: size.width.max(1) as f32,
            height: size.height.max(1) as f32,
        }));
        self.runtime = Some(runtime);
        self.mark_redraw(RedrawReason::Startup);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: HostEvent) {
        if self.failure.is_some() {
            event_loop.exit();
            return;
        }
        match event {
            HostEvent::ExitRequested => event_loop.exit(),
            HostEvent::RuntimeWake => {
                self.host_wake_pending = false;
                self.poll_presentation(event_loop);
            }
            #[cfg(all(feature = "application-vulkan-windows", target_os = "windows"))]
            HostEvent::PresentationWake => {
                self.poll_presentation(event_loop);
            }
            HostEvent::ResizeSignalChanged {
                signal,
                observed_at,
            } => {
                if !self.poll_presentation(event_loop) || signal.is_active() {
                    return;
                }
                let _ = self.complete_live_resize_release(event_loop, signal, observed_at);
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .window
            .as_ref()
            .is_none_or(|window| window.id() != window_id)
        {
            return;
        }
        if !self.poll_presentation(event_loop) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                if let Some(runtime) = self.runtime.as_mut()
                    && let Err(error) = S::close(runtime)
                {
                    self.fail(event_loop, format!("component close failed: {error}"));
                    return;
                }
                self.flush_commands();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                let event_extent = SizeI {
                    width: size.width as i32,
                    height: size.height as i32,
                };
                let current_extent = self.window.as_ref().map_or(event_extent, |window| {
                    let current = window.inner_size();
                    SizeI {
                        width: current.width as i32,
                        height: current.height as i32,
                    }
                });
                let live_resize_active = self
                    .resize_signal()
                    .is_some_and(super::resize::ResizeSignalSnapshot::is_active);
                if !should_accept_winit_resize(live_resize_active, event_extent, current_extent) {
                    #[cfg(feature = "profiler")]
                    crate::profiler::instant!("responsiveness.resize.stale_winit_event_rejected");
                    return;
                }
                self.drawable = event_extent.width > 0 && event_extent.height > 0;
                let queued = self.pending_resize.queue(event_extent);
                if queued {
                    self.mark_redraw(RedrawReason::Resize);
                }
                if !self.drawable {
                    // Zero-sized surfaces must be suspended even if the operating system stops
                    // delivering paint callbacks while the window is minimized. The next nonzero
                    // resize updates `drawable` before scheduling its recovery frame.
                    self.redraw(event_loop, RedrawSource::SynchronousResize);
                    return;
                }
                let frame_interval = self.refresh_frame_interval();
                let resize_frame_due = self.pending_resize.is_due(Instant::now(), frame_interval);
                if should_apply_resize_synchronously(live_resize_active, resize_frame_due) {
                    // CPU-side input/layout/scene preparation is safe in WM_SIZE. Vulkan WSI work
                    // is only published to the presentation worker and never runs here. Outside
                    // the native sizing loop, defer so startup and programmatic resize bursts can
                    // collapse to their final extent before rendering.
                    self.redraw(event_loop, RedrawSource::SynchronousResize);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                #[cfg(feature = "profiler")]
                record_gui_input(
                    crate::profiler::InputRecordingSource::PointerMotion,
                    "input.gui.pointer_motion",
                );
                self.diagnostics.native_pointer_moves =
                    self.diagnostics.native_pointer_moves.saturating_add(1);
                self.cursor_position = PointF {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                if let Some(runtime) = self.runtime.as_mut() {
                    runtime.queue_input(InputEvent::mouse_moved(self.cursor_position));
                }
            }
            WindowEvent::CursorLeft { .. } => {
                #[cfg(feature = "profiler")]
                record_gui_input(
                    crate::profiler::InputRecordingSource::PointerMotion,
                    "input.gui.pointer_motion.leave",
                );
                if let Some(runtime) = self.runtime.as_mut() {
                    runtime.queue_input(InputEvent::mouse_moved(PointF { x: -1.0, y: -1.0 }));
                }
                self.cursor_position = PointF { x: -1.0, y: -1.0 };
            }
            WindowEvent::MouseInput { state, button, .. } => {
                #[cfg(feature = "profiler")]
                record_gui_input(
                    crate::profiler::InputRecordingSource::PointerButton,
                    "input.gui.pointer_button",
                );
                let custom_action = (state == ElementState::Pressed
                    && button == winit::event::MouseButton::Left)
                    .then(|| self.custom_chrome_action())
                    .flatten();
                if let Some(runtime) = self.runtime.as_mut() {
                    let button = mouse_button(button);
                    let state = match state {
                        ElementState::Pressed => ButtonState::Pressed,
                        ElementState::Released => ButtonState::Released,
                    };
                    runtime.queue_input(InputEvent::mouse_button(button, state));
                }
                if let Some(action) = custom_action {
                    self.apply_custom_chrome_action(event_loop, action);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                #[cfg(feature = "profiler")]
                record_gui_input(
                    crate::profiler::InputRecordingSource::Scroll,
                    "input.gui.scroll",
                );
                let delta = match delta {
                    MouseScrollDelta::LineDelta(x, y) => PointF {
                        x: x * 24.0,
                        y: y * 24.0,
                    },
                    MouseScrollDelta::PixelDelta(position) => PointF {
                        x: position.x as f32,
                        y: position.y as f32,
                    },
                };
                if let Some(runtime) = self.runtime.as_mut() {
                    runtime.queue_input(InputEvent::mouse_scroll(delta));
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                #[cfg(feature = "profiler")]
                record_gui_input(
                    crate::profiler::InputRecordingSource::Keyboard,
                    "input.gui.keyboard",
                );
                if let WinitPhysicalKey::Code(code) = event.physical_key
                    && let Some(runtime) = self.runtime.as_mut()
                {
                    runtime.queue_input(InputEvent::Key(KeyEvent {
                        physical_key: PhysicalKey::new(code as u32),
                        state: match event.state {
                            ElementState::Pressed => ButtonState::Pressed,
                            ElementState::Released => ButtonState::Released,
                        },
                        repeat: event.repeat,
                        modifiers: Modifiers::empty(),
                        ..KeyEvent::new(PhysicalKey::new(code as u32), ButtonState::Pressed)
                    }));
                }
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop, RedrawSource::NativeCallback),
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                if occluded {
                    self.redraw.cancel_native_request();
                } else {
                    self.mark_redraw(RedrawReason::Expose);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.poll_presentation(event_loop) {
            return;
        }
        if !self.finalize_live_resize(event_loop) {
            return;
        }
        if cfg!(target_os = "windows")
            && self
                .resize_signal()
                .is_some_and(super::resize::ResizeSignalSnapshot::is_active)
        {
            let frame_interval = self.refresh_frame_interval();
            if self.pending_resize.is_due(Instant::now(), frame_interval) {
                self.mark_redraw(RedrawReason::Resize);
                self.redraw(event_loop, RedrawSource::SynchronousResize);
                if self.failure.is_some() {
                    return;
                }
            }
        }
        let _ = self.apply_post_turn_schedule(event_loop, Instant::now());
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.suspended = true;
        self.redraw.cancel_native_request();
        self.live_resize.cancel();
        if let Err(error) = self.presentation.suspend() {
            self.fail(event_loop, error);
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Err(error) = self.presentation.shutdown() {
            eprintln!("telorgon-app: {error}");
            self.failure.get_or_insert(error);
        }
        if let Some(window) = self.window.as_deref()
            && let Err(error) = self.resize_signals.unregister_window(window)
        {
            eprintln!("telorgon-app: {error}");
            self.failure.get_or_insert(error);
        }
        if let (Some(view), Some(window)) = (self.view.take(), self.window.as_deref())
            && let Err(error) = self.views.retire(view, window.id())
        {
            eprintln!("telorgon-app: failed to retire managed view: {error}");
            self.failure.get_or_insert(error.to_string());
        }
    }
}

#[cfg(target_os = "windows")]
impl<S, P> ApplicationHandler<HostEvent> for NativeHostApplication<S, P>
where
    S: NativeRuntimeSource + 'static,
    P: NativePresentation + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.host.borrow_mut().resumed(event_loop);
        let host_ready = { self.host.borrow().failure.is_none() };
        if host_ready && let Err(error) = self.install_live_resize_handler() {
            self.host.borrow_mut().fail(event_loop, error);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: HostEvent) {
        self.host.borrow_mut().user_event(event_loop, event);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.host
            .borrow_mut()
            .window_event(event_loop, window_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.host.borrow_mut().about_to_wait(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.host.borrow_mut().suspended(event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.host.borrow_mut().exiting(event_loop);
        self.live_resize_handler_installed = false;
    }
}

#[cfg(feature = "profiler")]
fn record_gui_input(source: crate::profiler::InputRecordingSource, label: &'static str) {
    if crate::profiler::input_recording_enabled(source) {
        crate::profiler::record_instant(label);
    }
}

fn mouse_button(button: MouseButton) -> PointerButton {
    match button {
        MouseButton::Left => PointerButton::PRIMARY,
        MouseButton::Right => PointerButton::SECONDARY,
        MouseButton::Middle => PointerButton::MIDDLE,
        MouseButton::Back => PointerButton::BACK,
        MouseButton::Forward => PointerButton::FORWARD,
        MouseButton::Other(value) => PointerButton::new(value),
    }
}

fn winit_resize_direction(edge: crate::WindowResizeEdge) -> ResizeDirection {
    match edge {
        crate::WindowResizeEdge::Top => ResizeDirection::North,
        crate::WindowResizeEdge::TopRight => ResizeDirection::NorthEast,
        crate::WindowResizeEdge::Right => ResizeDirection::East,
        crate::WindowResizeEdge::BottomRight => ResizeDirection::SouthEast,
        crate::WindowResizeEdge::Bottom => ResizeDirection::South,
        crate::WindowResizeEdge::BottomLeft => ResizeDirection::SouthWest,
        crate::WindowResizeEdge::Left => ResizeDirection::West,
        crate::WindowResizeEdge::TopLeft => ResizeDirection::NorthWest,
    }
}

fn resize_pointer_icon(edge: crate::WindowResizeEdge) -> crate::PointerIcon {
    match edge {
        crate::WindowResizeEdge::Top => crate::PointerIcon::NResize,
        crate::WindowResizeEdge::TopRight => crate::PointerIcon::NeResize,
        crate::WindowResizeEdge::Right => crate::PointerIcon::EResize,
        crate::WindowResizeEdge::BottomRight => crate::PointerIcon::SeResize,
        crate::WindowResizeEdge::Bottom => crate::PointerIcon::SResize,
        crate::WindowResizeEdge::BottomLeft => crate::PointerIcon::SwResize,
        crate::WindowResizeEdge::Left => crate::PointerIcon::WResize,
        crate::WindowResizeEdge::TopLeft => crate::PointerIcon::NwResize,
    }
}

fn winit_cursor_icon(icon: crate::PointerIcon) -> CursorIcon {
    match icon {
        crate::PointerIcon::Default => CursorIcon::Default,
        crate::PointerIcon::ContextMenu => CursorIcon::ContextMenu,
        crate::PointerIcon::Help => CursorIcon::Help,
        crate::PointerIcon::Pointer => CursorIcon::Pointer,
        crate::PointerIcon::Progress => CursorIcon::Progress,
        crate::PointerIcon::Wait => CursorIcon::Wait,
        crate::PointerIcon::Cell => CursorIcon::Cell,
        crate::PointerIcon::Crosshair => CursorIcon::Crosshair,
        crate::PointerIcon::Text => CursorIcon::Text,
        crate::PointerIcon::VerticalText => CursorIcon::VerticalText,
        crate::PointerIcon::Alias => CursorIcon::Alias,
        crate::PointerIcon::Copy => CursorIcon::Copy,
        crate::PointerIcon::Move => CursorIcon::Move,
        crate::PointerIcon::NoDrop => CursorIcon::NoDrop,
        crate::PointerIcon::NotAllowed => CursorIcon::NotAllowed,
        crate::PointerIcon::Grab => CursorIcon::Grab,
        crate::PointerIcon::Grabbing => CursorIcon::Grabbing,
        crate::PointerIcon::EResize => CursorIcon::EResize,
        crate::PointerIcon::NResize => CursorIcon::NResize,
        crate::PointerIcon::NeResize => CursorIcon::NeResize,
        crate::PointerIcon::NwResize => CursorIcon::NwResize,
        crate::PointerIcon::SResize => CursorIcon::SResize,
        crate::PointerIcon::SeResize => CursorIcon::SeResize,
        crate::PointerIcon::SwResize => CursorIcon::SwResize,
        crate::PointerIcon::WResize => CursorIcon::WResize,
        crate::PointerIcon::EwResize => CursorIcon::EwResize,
        crate::PointerIcon::NsResize => CursorIcon::NsResize,
        crate::PointerIcon::NeswResize => CursorIcon::NeswResize,
        crate::PointerIcon::NwseResize => CursorIcon::NwseResize,
        crate::PointerIcon::ColResize => CursorIcon::ColResize,
        crate::PointerIcon::RowResize => CursorIcon::RowResize,
        crate::PointerIcon::AllScroll => CursorIcon::AllScroll,
        crate::PointerIcon::ZoomIn => CursorIcon::ZoomIn,
        crate::PointerIcon::ZoomOut => CursorIcon::ZoomOut,
        crate::PointerIcon::DndAsk => CursorIcon::DndAsk,
        crate::PointerIcon::AllResize => CursorIcon::AllResize,
    }
}

fn earlier_wait_deadline(control_flow: ControlFlow, deadline: Instant) -> ControlFlow {
    match control_flow {
        ControlFlow::Poll => ControlFlow::Poll,
        ControlFlow::Wait => ControlFlow::WaitUntil(deadline),
        ControlFlow::WaitUntil(existing) => ControlFlow::WaitUntil(if deadline < existing {
            deadline
        } else {
            existing
        }),
    }
}

const fn should_apply_resize_synchronously(
    live_resize_active: bool,
    resize_frame_due: bool,
) -> bool {
    cfg!(target_os = "windows") && live_resize_active && resize_frame_due
}

fn should_accept_winit_resize(
    live_resize_active: bool,
    event_extent: SizeI,
    current_extent: SizeI,
) -> bool {
    !cfg!(target_os = "windows") || (!live_resize_active && event_extent == current_extent)
}

fn resize_revision_to_synchronize(source: RedrawSource, update: ResizeUpdate) -> Option<u64> {
    (source == RedrawSource::SynchronousResizeBarrier
        && update.surface == SurfaceResizeAction::Commit)
        .then_some(update.metrics_revision)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_primary_mouse_buttons_to_stable_codes() {
        assert_eq!(mouse_button(MouseButton::Left), PointerButton::PRIMARY);
        assert_eq!(mouse_button(MouseButton::Right), PointerButton::SECONDARY);
        assert_eq!(mouse_button(MouseButton::Middle), PointerButton::MIDDLE);
    }

    #[test]
    fn interactive_resize_bursts_apply_only_the_latest_extent_per_frame() {
        let now = Instant::now();
        let mut pending = PendingResize::default();
        let _ = pending.queue(SizeI {
            width: 640,
            height: 480,
        });
        let _ = pending.queue(SizeI {
            width: 960,
            height: 600,
        });
        let _ = pending.queue(SizeI {
            width: 1280,
            height: 720,
        });
        assert_eq!(
            pending.take(now),
            Some(SizeI {
                width: 1280,
                height: 720,
            })
        );
        assert_eq!(pending.take(now), None);
    }

    #[test]
    fn live_resize_frames_are_refresh_paced_without_dropping_the_final_extent() {
        let started = Instant::now();
        let interval = Duration::from_millis(16);
        let mut pending = PendingResize::default();
        let _ = pending.queue(SizeI {
            width: 800,
            height: 600,
        });
        assert!(pending.is_due(started, interval));
        assert!(pending.take(started).is_some());
        let _ = pending.queue(SizeI {
            width: 900,
            height: 700,
        });
        assert!(!pending.is_due(started + Duration::from_millis(8), interval));
        assert_eq!(pending.next_due_at(interval), Some(started + interval));
        let _ = pending.queue(SizeI {
            width: 1000,
            height: 800,
        });
        assert!(pending.is_due(started + interval, interval));
        assert_eq!(
            pending.take(started + interval),
            Some(SizeI {
                width: 1000,
                height: 800,
            })
        );
        assert_eq!(pending.next_due_at(interval), None);
    }

    #[test]
    fn native_resize_timer_does_not_requeue_an_already_applied_extent() {
        let now = Instant::now();
        let extent = SizeI {
            width: 1000,
            height: 700,
        };
        let mut pending = PendingResize::default();
        assert!(pending.queue(extent));
        assert_eq!(pending.take(now), Some(extent));
        assert!(!pending.queue(extent));
        assert!(!pending.is_pending());

        let changed = SizeI {
            width: 1001,
            height: 700,
        };
        assert!(pending.queue(changed));
        assert!(pending.is_pending());
    }

    #[test]
    fn post_native_barrier_can_repeat_an_expansion_extent() {
        let now = Instant::now();
        let extent = SizeI {
            width: 1200,
            height: 800,
        };
        let mut pending = PendingResize::default();
        assert!(pending.queue(extent));
        assert_eq!(pending.take(now), Some(extent));
        assert!(!pending.queue(extent));
        assert!(pending.queue_for_barrier(extent));
        assert_eq!(pending.take(now), Some(extent));
    }

    #[test]
    fn monitor_refresh_rate_defines_the_managed_frame_interval() {
        assert_eq!(
            frame_interval_for_refresh_rate(Some(60_000)),
            Duration::from_nanos(16_666_667)
        );
        assert_eq!(
            frame_interval_for_refresh_rate(Some(120_000)),
            Duration::from_nanos(8_333_334)
        );
        assert_eq!(
            frame_interval_for_refresh_rate(Some(144_000)),
            Duration::from_nanos(6_944_445)
        );
        assert_eq!(
            frame_interval_for_refresh_rate(None),
            frame_interval_for_refresh_rate(Some(0))
        );
    }

    #[test]
    fn frame_pacer_waits_for_refresh_and_never_issues_catch_up_frames() {
        let started = Instant::now();
        let interval = Duration::from_millis(16);
        let mut pacer = FramePacer::default();
        assert_eq!(pacer.throttle_deadline(started, interval), None);

        pacer.frame_started(started);
        assert_eq!(
            pacer.throttle_deadline(started + Duration::from_millis(8), interval),
            Some(started + interval)
        );
        assert_eq!(pacer.throttle_deadline(started + interval, interval), None);

        let late = started + Duration::from_millis(100);
        pacer.frame_started(late);
        assert_eq!(
            pacer.throttle_deadline(late + Duration::from_millis(1), interval),
            Some(late + interval)
        );
    }

    #[test]
    fn only_active_windows_resize_frames_are_applied_synchronously() {
        assert!(!should_apply_resize_synchronously(false, true));
        assert!(!should_apply_resize_synchronously(true, false));
        assert_eq!(
            should_apply_resize_synchronously(true, true),
            cfg!(target_os = "windows")
        );
    }

    #[test]
    fn windows_live_resize_rejects_the_parallel_or_stale_winit_stream() {
        let current = SizeI {
            width: 1001,
            height: 700,
        };
        let stale = SizeI {
            width: 980,
            height: 700,
        };
        assert_eq!(
            should_accept_winit_resize(true, current, current),
            !cfg!(target_os = "windows")
        );
        assert_eq!(
            should_accept_winit_resize(false, stale, current),
            !cfg!(target_os = "windows")
        );
        assert!(should_accept_winit_resize(false, current, current));
    }

    #[test]
    fn exact_wm_size_barrier_accepts_only_responsive_resize_commits() {
        let update = ResizeUpdate {
            generation: 4,
            metrics_revision: 17,
            phase: super::super::resize::ResizeInteractionPhase::Updating,
            extent: SizeI {
                width: 1001,
                height: 700,
            },
            surface: SurfaceResizeAction::Commit,
        };
        assert_eq!(
            resize_revision_to_synchronize(RedrawSource::SynchronousResizeBarrier, update),
            Some(17)
        );
        assert_eq!(
            resize_revision_to_synchronize(RedrawSource::SynchronousResize, update),
            None
        );
        assert_eq!(
            resize_revision_to_synchronize(
                RedrawSource::SynchronousResizeBarrier,
                ResizeUpdate {
                    surface: SurfaceResizeAction::KeepCurrent,
                    ..update
                }
            ),
            None
        );
    }

    #[test]
    fn redraw_demand_merges_reasons_and_deduplicates_native_requests() {
        let mut demand = RedrawDemand::default();
        assert!(demand.mark(RedrawReason::Input));
        assert!(!demand.mark(RedrawReason::Input));
        assert!(demand.mark(RedrawReason::Animation));
        assert!(demand.has_demand());
        assert!(demand.queue_native_request());
        assert!(!demand.queue_native_request());

        assert!(demand.native_callback_started());
        assert!(!demand.native_callback_started());
        assert!(demand.queue_native_request());
        let reasons = demand.take_reasons();
        assert!(!reasons.is_empty());
        assert!(!reasons.force_present());
        assert!(!demand.has_demand());
    }

    #[test]
    fn resize_release_retires_a_stale_native_redraw_request() {
        let mut demand = RedrawDemand::default();
        demand.mark(RedrawReason::Resize);
        assert!(demand.queue_native_request());

        // Live-resize frames run synchronously and consume demand without receiving the native
        // callback that normally retires this request.
        let live_resize_reasons = demand.take_reasons();
        assert!(live_resize_reasons.force_present());
        demand.mark(RedrawReason::Resize);
        assert!(!demand.queue_native_request());

        // WM_EXITSIZEMOVE retires the stale request before its mandatory synchronous frame.
        demand.cancel_native_request();
        let release_reasons = demand.take_reasons();
        assert!(release_reasons.force_present());
        demand.mark(RedrawReason::Animation);
        assert!(demand.queue_native_request());
    }

    #[test]
    fn ordinary_runtime_reasons_share_pacing_while_lifecycle_presentation_bypasses_it() {
        for reason in [
            RedrawReason::Runtime,
            RedrawReason::Input,
            RedrawReason::Command,
            RedrawReason::ExternalWake,
            RedrawReason::Animation,
            RedrawReason::Timer,
            RedrawReason::PointerMove,
        ] {
            let mut demand = RedrawDemand::default();
            demand.mark(reason);
            assert!(!demand.requires_immediate_presentation());
        }

        let mut demand = RedrawDemand::default();
        demand.mark(RedrawReason::Expose);
        assert!(demand.requires_immediate_presentation());
    }

    #[test]
    fn expose_and_recovery_reasons_force_presentation() {
        let mut reasons = RedrawReasons::default();
        reasons.insert(RedrawReason::Expose);
        assert!(reasons.force_present());

        let mut reasons = RedrawReasons::default();
        reasons.insert(RedrawReason::Recovery);
        assert!(reasons.force_present());
    }

    #[test]
    fn pointer_move_only_frames_allow_the_generic_runtime_reason_but_no_other_trigger() {
        let mut reasons = RedrawReasons::default();
        reasons.insert(RedrawReason::PointerMove);
        reasons.insert(RedrawReason::Runtime);
        assert!(reasons.pointer_move_only());

        reasons.insert(RedrawReason::Animation);
        assert!(!reasons.pointer_move_only());

        let mut reasons = RedrawReasons::default();
        reasons.insert(RedrawReason::Input);
        assert!(!reasons.pointer_move_only());
    }

    #[test]
    fn resize_deadline_only_shortens_waiting_control_flow() {
        let now = Instant::now();
        let later = now + Duration::from_millis(20);
        assert_eq!(
            earlier_wait_deadline(ControlFlow::Poll, now),
            ControlFlow::Poll
        );
        assert_eq!(
            earlier_wait_deadline(ControlFlow::Wait, later),
            ControlFlow::WaitUntil(later)
        );
        assert_eq!(
            earlier_wait_deadline(ControlFlow::WaitUntil(later), now),
            ControlFlow::WaitUntil(now)
        );
        assert_eq!(
            earlier_wait_deadline(ControlFlow::WaitUntil(now), later),
            ControlFlow::WaitUntil(now)
        );
    }
}
