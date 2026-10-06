use std::collections::BTreeMap;
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(target_os = "windows")]
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use crate::foundation::{MonotonicInstant, PointF, SizeF, SizeI};
use crate::graphics::presentation::{SurfaceMetrics, SurfaceRevision};
use crate::graphics::render::{AlphaMode, ColorSpace, RenderSceneDelta};
use crate::input::{ButtonState, InputEvent, PointerButton};
use crate::platform::contracts::{PendingHostFacts, PostTurnSchedule, RemainingWork, ViewId};
use crate::platform::winit::{
    ViewRegistry, WinitClockObservation, WinitWakeIntent, interpret_schedule,
};
use crate::{AssetBundle, AssetMediaCache};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, Size};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
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
    not(all(feature = "application-vulkan", any(target_os = "windows", target_os = "linux")))
))]
use super::software::SoftwarePresentation;
#[cfg(not(all(feature = "application-vulkan", any(target_os = "windows", target_os = "linux"))))]
use crate::host::application::ReadyGuiApplication;
use crate::host::application::{
    AppError, AppResult, AppRuntimeCore, Command, ComponentDriver, CompositionDriver,
    PlatformInput, WindowOptions,
};

pub(crate) struct ManagedEventLoop {
    event_loop: EventLoop<HostEvent>,
    resize_signals: Arc<PlatformResizeSignals>,
    _profiler: crate::host::application::profiler::ManagedProfiler,
}

impl ManagedEventLoop {
    pub(crate) fn event_loop(&self) -> &EventLoop<HostEvent> {
        &self.event_loop
    }
}

pub(crate) fn create_managed_event_loop(
    profile_target: crate::host::application::profiler::ProfileTarget,
) -> AppResult<ManagedEventLoop> {
    let profiler = crate::host::application::profiler::ManagedProfiler::start(profile_target)?;
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
#[cfg(not(all(feature = "application-vulkan", any(target_os = "windows", target_os = "linux"))))]
pub fn run_gui_software(application: ReadyGuiApplication) -> AppResult<()> {
    let (driver, options, renderer, assets, pointer) = application.into_parts()?;
    if renderer == crate::host::application::Renderer::Vulkan {
        return Err(AppError::new(
            "this build does not include the Vulkan managed renderer",
        ));
    }
    let event_loop =
        create_managed_event_loop(crate::host::application::profiler::ProfileTarget::Gui)?;
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
    /// Whether retained scenes use logical units rather than physical pixels.
    fn uses_logical_coordinates(&self) -> bool {
        false
    }
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
    let _exit_request = crate::host::application::exit::HostExit::register(move || {
        let _ = proxy.send_event(HostEvent::ExitRequested);
    });
    #[cfg(feature = "profiler")]
    let _profile_view = crate::runtime::instrumentation::enter_view(Some(
        crate::runtime::instrumentation::ProfileViewId::PRIMARY,
    ));
    #[cfg(feature = "profiler")]
    let _ = crate::runtime::instrumentation::register_view(
        crate::runtime::instrumentation::ProfileViewId::PRIMARY,
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
        runtime.register_assets(self.assets)?;
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
    if image.alpha_mode == crate::graphics::render::ImageAlphaMode::Premultiplied {
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
    keyboard: keyboard::NativeKeyboard,
    ime: ime::NativeIme,
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
            keyboard: keyboard::NativeKeyboard::default(),
            ime: ime::NativeIme::default(),
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
        crate::runtime::instrumentation::instant!("responsiveness.resize.dwm_flush_failed");
    }
    #[cfg(not(feature = "profiler"))]
    let _ = result;
}
mod dpi;
mod pointer;
mod scheduling;
use pointer::{mouse_button, winit_cursor_icon};
mod events;
mod keyboard;
mod ime;
mod redraw;

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
fn record_gui_input(
    source: crate::runtime::instrumentation::InputRecordingSource,
    label: &'static str,
) {
    if crate::runtime::instrumentation::input_recording_enabled(source) {
        crate::runtime::instrumentation::record_instant(label);
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
mod tests;
