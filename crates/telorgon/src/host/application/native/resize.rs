#[cfg(target_os = "windows")]
use std::cell::RefCell;
#[cfg(target_os = "windows")]
use std::collections::HashMap;
#[cfg(target_os = "windows")]
use std::collections::HashSet;
#[cfg(target_os = "windows")]
use std::rc::Rc;
use std::sync::Arc;
#[cfg(target_os = "windows")]
use std::sync::Mutex;

use crate::foundation::SizeI;
use winit::event_loop::EventLoopProxy;
use winit::window::Window;

use super::HostEvent;

#[cfg(target_os = "windows")]
type LiveResizeHandler = Rc<dyn Fn(SizeI, std::time::Instant, bool, bool)>;

#[cfg(target_os = "windows")]
thread_local! {
    /// Thread-affine callbacks used to escape Win32's modal move/size loop.
    ///
    /// These deliberately do not live behind `PlatformResizeSignals`' mutexes: the managed host
    /// is UI-thread-affine, and requiring the callback to be `Send` would falsely imply that it is
    /// safe to run application/runtime work from another thread.
    static LIVE_RESIZE_HANDLERS: RefCell<HashMap<usize, LiveResizeHandler>> =
        RefCell::new(HashMap::new());
    /// HWNDs with one coalesced post-native resize barrier message in the thread queue.
    static LIVE_RESIZE_BARRIERS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    /// Last exact client extent observed for each registered HWND.
    static LIVE_RESIZE_EXTENTS: RefCell<HashMap<usize, SizeI>> = RefCell::new(HashMap::new());
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeResizeTiming {
    BeforeNativeCommit,
    AfterNativeCommit,
}

#[cfg(target_os = "windows")]
const fn native_resize_timing(previous: SizeI, current: SizeI) -> NativeResizeTiming {
    if current.width > previous.width || current.height > previous.height {
        // A smaller old buffer would leave an uncovered band when DWM enlarges its target. Prepare
        // the exact larger buffer first. For a mixed-axis corner transition, prioritizing the
        // expanding axis avoids an uncovered region after the native commit; the shrinking axis is
        // clipped as soon as the target transaction completes.
        NativeResizeTiming::BeforeNativeCommit
    } else {
        // Preserve the old larger buffer until DWM has contracted its target, then let DWM clip it.
        NativeResizeTiming::AfterNativeCommit
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResizeInteractionPhase {
    Stable,
    Started,
    Updating,
    Ended,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SurfaceCommitPolicy {
    Responsive,
    // This remains part of the platform-neutral protocol even in software-only builds, where the
    // sole presenter always uses responsive commits.
    #[allow(dead_code)]
    DeferredScaledPreview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SurfaceResizeAction {
    KeepCurrent,
    Commit,
    CommitAfterPreview,
    Suspend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResizeUpdate {
    pub(super) generation: u64,
    pub(super) metrics_revision: u64,
    pub(super) phase: ResizeInteractionPhase,
    pub(super) extent: SizeI,
    pub(super) surface: SurfaceResizeAction,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResizeSignalSnapshot {
    generation: u64,
    active: bool,
}

impl ResizeSignalSnapshot {
    #[cfg(test)]
    const fn new(generation: u64, active: bool) -> Self {
        Self { generation, active }
    }

    pub(super) const fn is_active(self) -> bool {
        self.active
    }
}

#[derive(Debug, Default)]
pub(super) struct LiveResizeCoordinator {
    active_generation: Option<u64>,
    latest_extent: Option<SizeI>,
    latest_metrics_revision: u64,
    next_metrics_revision: u64,
}

impl LiveResizeCoordinator {
    pub(super) const fn latest_extent(&self) -> Option<SizeI> {
        self.latest_extent
    }

    pub(super) const fn latest_metrics_revision(&self) -> u64 {
        self.latest_metrics_revision
    }

    pub(super) fn observe(
        &mut self,
        extent: SizeI,
        signal: Option<ResizeSignalSnapshot>,
        policy: SurfaceCommitPolicy,
    ) -> ResizeUpdate {
        let metrics_revision = self.observe_metrics(extent);
        self.latest_extent = Some(extent);
        if extent.width <= 0 || extent.height <= 0 {
            let generation = self.active_generation.take().unwrap_or_default();
            return ResizeUpdate {
                generation,
                metrics_revision,
                phase: ResizeInteractionPhase::Cancelled,
                extent,
                surface: SurfaceResizeAction::Suspend,
            };
        }

        if let Some(signal) = signal {
            if signal.active {
                let phase = if self.active_generation == Some(signal.generation) {
                    ResizeInteractionPhase::Updating
                } else {
                    self.active_generation = Some(signal.generation);
                    ResizeInteractionPhase::Started
                };
                return ResizeUpdate {
                    generation: signal.generation,
                    metrics_revision,
                    phase,
                    extent,
                    surface: surface_action(policy, true),
                };
            }
            if self.active_generation == Some(signal.generation) {
                self.active_generation = None;
                return ResizeUpdate {
                    generation: signal.generation,
                    metrics_revision,
                    phase: ResizeInteractionPhase::Ended,
                    extent,
                    surface: final_surface_action(policy),
                };
            }
        }

        ResizeUpdate {
            generation: signal.map_or(0, |signal| signal.generation),
            metrics_revision,
            phase: ResizeInteractionPhase::Stable,
            extent,
            surface: SurfaceResizeAction::Commit,
        }
    }

    pub(super) fn needs_finalization(&self, signal: Option<ResizeSignalSnapshot>) -> bool {
        self.active_generation.is_some_and(|generation| {
            signal.is_some_and(|signal| !signal.active && signal.generation == generation)
        })
    }

    pub(super) fn finalize(
        &mut self,
        signal: Option<ResizeSignalSnapshot>,
        policy: SurfaceCommitPolicy,
    ) -> Option<ResizeUpdate> {
        if !self.needs_finalization(signal) {
            return None;
        }
        let generation = self.active_generation.take()?;
        let extent = self.latest_extent?;
        Some(ResizeUpdate {
            generation,
            metrics_revision: self.latest_metrics_revision,
            phase: ResizeInteractionPhase::Ended,
            extent,
            surface: if extent.width <= 0 || extent.height <= 0 {
                SurfaceResizeAction::Suspend
            } else {
                final_surface_action(policy)
            },
        })
    }

    pub(super) fn cancel(&mut self) {
        self.active_generation = None;
    }

    fn observe_metrics(&mut self, extent: SizeI) -> u64 {
        if self.latest_extent == Some(extent) && self.latest_metrics_revision != 0 {
            return self.latest_metrics_revision;
        }
        self.next_metrics_revision = self.next_metrics_revision.saturating_add(1).max(1);
        self.latest_metrics_revision = self.next_metrics_revision;
        self.latest_metrics_revision
    }
}

const fn surface_action(policy: SurfaceCommitPolicy, interactive: bool) -> SurfaceResizeAction {
    match (policy, interactive) {
        (SurfaceCommitPolicy::DeferredScaledPreview, true) => SurfaceResizeAction::KeepCurrent,
        _ => SurfaceResizeAction::Commit,
    }
}

const fn final_surface_action(policy: SurfaceCommitPolicy) -> SurfaceResizeAction {
    match policy {
        SurfaceCommitPolicy::Responsive => SurfaceResizeAction::Commit,
        SurfaceCommitPolicy::DeferredScaledPreview => SurfaceResizeAction::CommitAfterPreview,
    }
}

#[derive(Debug, Default)]
pub(super) struct PlatformResizeSignals {
    #[cfg(target_os = "windows")]
    windows: Mutex<HashMap<usize, ResizeSignalSnapshot>>,
    #[cfg(target_os = "windows")]
    subclasses: Mutex<HashMap<usize, usize>>,
    #[cfg(target_os = "windows")]
    wake: Mutex<Option<EventLoopProxy<HostEvent>>>,
}

impl PlatformResizeSignals {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(super) fn set_event_loop_proxy(&self, proxy: EventLoopProxy<HostEvent>) {
        #[cfg(target_os = "windows")]
        {
            *self
                .wake
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(proxy);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = proxy;
    }

    /// Installs a thread-affine Win32 subclass that observes the native modal sizing loop.
    ///
    /// Winit's message hook only sees messages removed by its outer `PeekMessage` loop. The
    /// enter/exit sizing messages are delivered through the window procedure and can run inside
    /// `DefWindowProc`'s nested loop, so observing them at the window procedure is required.
    pub(super) fn register_window(self: &Arc<Self>, window: &Window) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::UI::Shell::SetWindowSubclass;

            let handle = windows_handle(window)
                .ok_or_else(|| "native Windows window did not expose a Win32 handle".to_owned())?;
            if self
                .subclasses
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains_key(&handle)
            {
                return Ok(());
            }

            let signals = Arc::into_raw(Arc::clone(self)) as usize;
            // SAFETY: registration happens on the window thread. `signals` owns one strong Arc
            // reference until `unregister_window` removes this exact subclass and reclaims it.
            let installed = unsafe {
                SetWindowSubclass(
                    handle as windows_sys::Win32::Foundation::HWND,
                    Some(resize_subclass_proc),
                    RESIZE_SUBCLASS_ID,
                    signals,
                )
            };
            if installed == 0 {
                // SAFETY: SetWindowSubclass failed, so no callback can retain this raw reference.
                unsafe { drop(Arc::from_raw(signals as *const Self)) };
                return Err("failed to install the Win32 live-resize observer".to_owned());
            }

            self.windows
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entry(handle)
                .or_default();
            let size = window.inner_size();
            LIVE_RESIZE_EXTENTS.with(|extents| {
                extents.borrow_mut().insert(
                    handle,
                    SizeI {
                        width: size.width as i32,
                        height: size.height as i32,
                    },
                );
            });
            self.subclasses
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(handle, signals);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = window;
        Ok(())
    }

    pub(super) fn unregister_window(&self, window: &Window) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::UI::Shell::RemoveWindowSubclass;
            use windows_sys::Win32::UI::WindowsAndMessaging::KillTimer;

            let Some(handle) = windows_handle(window) else {
                return Ok(());
            };
            LIVE_RESIZE_HANDLERS.with(|handlers| {
                handlers.borrow_mut().remove(&handle);
            });
            LIVE_RESIZE_BARRIERS.with(|barriers| {
                barriers.borrow_mut().remove(&handle);
            });
            LIVE_RESIZE_EXTENTS.with(|extents| {
                extents.borrow_mut().remove(&handle);
            });
            let Some(signals) = self
                .subclasses
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&handle)
                .copied()
            else {
                return Ok(());
            };

            // SAFETY: this timer belongs to this HWND and killing a missing timer is harmless.
            unsafe {
                KillTimer(
                    handle as windows_sys::Win32::Foundation::HWND,
                    RESIZE_REDRAW_TIMER_ID,
                )
            };

            // SAFETY: this is called on the same window thread that installed the subclass, while
            // the Winit window is still alive. The callback and subclass id exactly match install.
            let removed = unsafe {
                RemoveWindowSubclass(
                    handle as windows_sys::Win32::Foundation::HWND,
                    Some(resize_subclass_proc),
                    RESIZE_SUBCLASS_ID,
                )
            };
            if removed == 0 {
                // Retain the Arc reference if the callback may still be installed; leaking one
                // reference is safer than allowing a native callback to dereference freed state.
                return Err("failed to remove the Win32 live-resize observer".to_owned());
            }

            self.subclasses
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&handle);
            self.windows
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&handle);
            // SAFETY: successful removal guarantees this callback will no longer use its ref data,
            // and this raw pointer represents the one Arc reference created during registration.
            unsafe { drop(Arc::from_raw(signals as *const Self)) };
        }
        #[cfg(not(target_os = "windows"))]
        let _ = window;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    pub(super) fn set_live_resize_handler(
        &self,
        window: &Window,
        handler: LiveResizeHandler,
    ) -> Result<(), String> {
        let handle = windows_handle(window)
            .ok_or_else(|| "native Windows window did not expose a Win32 handle".to_owned())?;
        LIVE_RESIZE_HANDLERS.with(|handlers| {
            handlers.borrow_mut().insert(handle, handler);
        });
        Ok(())
    }

    pub(super) fn snapshot(&self, window: &Window) -> Option<ResizeSignalSnapshot> {
        #[cfg(target_os = "windows")]
        {
            let handle = windows_handle(window)?;
            return self
                .windows
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&handle)
                .copied();
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = window;
            None
        }
    }

    #[cfg(target_os = "windows")]
    fn observe_windows_message(&self, handle: usize, message: u32) -> Option<ResizeSignalSnapshot> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE};

        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match message {
            WM_ENTERSIZEMOVE => {
                let state = windows.entry(handle).or_default();
                state.generation = state.generation.saturating_add(1).max(1);
                state.active = true;
                Some(*state)
            }
            WM_EXITSIZEMOVE => {
                let state = windows.get_mut(&handle)?;
                state.active = false;
                Some(*state)
            }
            _ => None,
        }
    }

    #[cfg(target_os = "windows")]
    fn wake_event_loop(&self, event: HostEvent) {
        if let Some(proxy) = self
            .wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            let _ = proxy.send_event(event);
        }
    }

    #[cfg(target_os = "windows")]
    fn dispatch_windows_live_resize(
        &self,
        window: windows_sys::Win32::Foundation::HWND,
        synchronize_present: bool,
        repeat_extent: bool,
    ) {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

        let handle = window as usize;
        let active = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&handle)
            .is_some_and(|signal| signal.active);
        if !active {
            return;
        }

        let mut rect = RECT::default();
        // SAFETY: the subclass callback supplies a live HWND and `rect` is writable for the call.
        if unsafe { GetClientRect(window, &mut rect) } == 0 {
            return;
        }
        let extent = SizeI {
            width: rect.right.saturating_sub(rect.left),
            height: rect.bottom.saturating_sub(rect.top),
        };
        let handler = LIVE_RESIZE_HANDLERS.with(|handlers| handlers.borrow().get(&handle).cloned());
        if let Some(handler) = handler {
            handler(
                extent,
                std::time::Instant::now(),
                synchronize_present,
                repeat_extent,
            );
        }
    }

    #[cfg(target_os = "windows")]
    fn post_windows_live_resize_barrier(&self, window: windows_sys::Win32::Foundation::HWND) {
        use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

        let handle = window as usize;
        let inserted = LIVE_RESIZE_BARRIERS.with(|barriers| barriers.borrow_mut().insert(handle));
        if !inserted {
            return;
        }
        // PostMessage queues work after the current sent-message stack unwinds. That distinction is
        // required here: WM_SIZE can be nested inside DefWindowProc(WM_WINDOWPOSCHANGED), and DWM
        // cannot commit the new target while that native transaction is still on the stack.
        if unsafe { PostMessageW(window, TELORGON_RESIZE_BARRIER_MESSAGE, 0, 0) } == 0 {
            LIVE_RESIZE_BARRIERS.with(|barriers| {
                barriers.borrow_mut().remove(&handle);
            });
            #[cfg(feature = "profiler")]
            crate::runtime::instrumentation::instant!("responsiveness.resize.barrier_post_failed");
        } else {
            #[cfg(feature = "profiler")]
            crate::runtime::instrumentation::instant!("responsiveness.resize.barrier_posted");
        }
    }

    #[cfg(target_os = "windows")]
    fn observe_windows_resize_timing(
        &self,
        window: windows_sys::Win32::Foundation::HWND,
    ) -> Option<NativeResizeTiming> {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

        let mut rect = RECT::default();
        if unsafe { GetClientRect(window, &mut rect) } == 0 {
            return None;
        }
        let current = SizeI {
            width: rect.right.saturating_sub(rect.left),
            height: rect.bottom.saturating_sub(rect.top),
        };
        let handle = window as usize;
        let timing = LIVE_RESIZE_EXTENTS.with(|extents| {
            let mut extents = extents.borrow_mut();
            let previous = extents.insert(handle, current).unwrap_or(current);
            native_resize_timing(previous, current)
        });
        let active = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&handle)
            .is_some_and(|signal| signal.active);
        active.then_some(timing)
    }
}

#[cfg(target_os = "windows")]
const RESIZE_SUBCLASS_ID: usize = 0x4c49_5448_5253_5a45;
#[cfg(target_os = "windows")]
const RESIZE_REDRAW_TIMER_ID: usize = 0x4c49_5448_5254_4d52;
#[cfg(target_os = "windows")]
const RESIZE_REDRAW_TIMER_INTERVAL_MS: u32 = 16;
/// Private HWND message used only by the installed Telorgon subclass.
#[cfg(target_os = "windows")]
const TELORGON_RESIZE_BARRIER_MESSAGE: u32 =
    windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 0x04c1;

#[cfg(target_os = "windows")]
const fn is_resize_redraw_timer(message: u32, wparam: usize) -> bool {
    message == windows_sys::Win32::UI::WindowsAndMessaging::WM_TIMER
        && wparam == RESIZE_REDRAW_TIMER_ID
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn resize_subclass_proc(
    window: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
    _subclass_id: usize,
    signals: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::Shell::DefSubclassProc;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        KillTimer, SetTimer, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_SIZE,
    };

    // SAFETY: registration stores a strong Arc reference in `signals`, and unregistering first
    // removes the native callback before releasing that reference.
    let signals = unsafe { &*(signals as *const PlatformResizeSignals) };
    if message == TELORGON_RESIZE_BARRIER_MESSAGE {
        LIVE_RESIZE_BARRIERS.with(|barriers| {
            barriers.borrow_mut().remove(&(window as usize));
        });
        #[cfg(feature = "profiler")]
        crate::runtime::instrumentation::instant!("responsiveness.resize.barrier_dispatched");
        signals.dispatch_windows_live_resize(window, true, true);
        return 0;
    }
    if is_resize_redraw_timer(message, wparam) {
        // Windows runs a nested modal loop while moving or sizing a window. Winit can buffer the
        // corresponding application callbacks until that loop unwinds, so drive the thread-affine
        // managed host directly rather than posting another event into the same blocked loop.
        signals.dispatch_windows_live_resize(window, false, false);
        return 0;
    }
    if let Some(signal) = signals.observe_windows_message(window as usize, message) {
        let observed_at = std::time::Instant::now();
        if message == WM_ENTERSIZEMOVE {
            // SAFETY: the timer is scoped to this live HWND and is removed on exit/unregister.
            let _timer = unsafe {
                SetTimer(
                    window,
                    RESIZE_REDRAW_TIMER_ID,
                    RESIZE_REDRAW_TIMER_INTERVAL_MS,
                    None,
                )
            };
            #[cfg(feature = "profiler")]
            if _timer == 0 {
                crate::runtime::instrumentation::instant!("responsiveness.resize.paint_timer_failed");
            }
        } else if message == WM_EXITSIZEMOVE {
            // SAFETY: the timer, if present, belongs to this live HWND.
            unsafe { KillTimer(window, RESIZE_REDRAW_TIMER_ID) };
        }
        #[cfg(feature = "profiler")]
        if signal.is_active() {
            crate::runtime::instrumentation::instant!("responsiveness.resize.native_started");
        } else {
            crate::runtime::instrumentation::instant!("responsiveness.resize.native_ended");
        }
        signals.wake_event_loop(HostEvent::ResizeSignalChanged {
            signal,
            observed_at,
        });
    }
    if message == WM_SIZE {
        let timing = signals.observe_windows_resize_timing(window);
        if timing == Some(NativeResizeTiming::BeforeNativeCommit) {
            // On expansion, prepare and present the exact larger buffer before DWM exposes the
            // larger target. Scaling-none clips that buffer against the still-smaller old target.
            signals.dispatch_windows_live_resize(window, true, false);
        }
        // Forward the nested notification. DefWindowProc can emit WM_SIZE while handling
        // WM_WINDOWPOSCHANGED, and the HWND/DWM target cannot commit until that outer transaction
        // returns. Contractions intentionally retain their old larger buffer across this boundary.
        let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };
        if timing.is_some() {
            // Both directions receive a post-transaction pass. For contraction this performs the
            // delayed exact resize; for expansion it re-synchronizes presentation and DwmFlush
            // after the DWM target has caught up with the already-prepared larger buffer.
            signals.post_windows_live_resize_barrier(window);
        }
        return result;
    }
    // SAFETY: every subclass callback must forward unhandled processing through DefSubclassProc.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

#[cfg(target_os = "windows")]
fn windows_handle(window: &Window) -> Option<usize> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as usize),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: SizeI = SizeI {
        width: 800,
        height: 600,
    };
    const LATEST: SizeI = SizeI {
        width: 1280,
        height: 720,
    };

    #[cfg(target_os = "windows")]
    #[test]
    fn outward_resize_prepares_before_native_commit_and_inward_resize_waits() {
        assert_eq!(
            native_resize_timing(FIRST, LATEST),
            NativeResizeTiming::BeforeNativeCommit
        );
        assert_eq!(
            native_resize_timing(LATEST, FIRST),
            NativeResizeTiming::AfterNativeCommit
        );
        assert_eq!(
            native_resize_timing(
                FIRST,
                SizeI {
                    width: FIRST.width + 1,
                    height: FIRST.height - 1,
                },
            ),
            NativeResizeTiming::BeforeNativeCommit
        );
    }

    #[test]
    fn deferred_live_resize_previews_then_commits_the_latest_extent_once() {
        let mut coordinator = LiveResizeCoordinator::default();
        let started = coordinator.observe(
            FIRST,
            Some(ResizeSignalSnapshot::new(7, true)),
            SurfaceCommitPolicy::DeferredScaledPreview,
        );
        assert_eq!(started.phase, ResizeInteractionPhase::Started);
        assert_eq!(started.surface, SurfaceResizeAction::KeepCurrent);
        let updating = coordinator.observe(
            LATEST,
            Some(ResizeSignalSnapshot::new(7, true)),
            SurfaceCommitPolicy::DeferredScaledPreview,
        );
        assert_eq!(updating.phase, ResizeInteractionPhase::Updating);
        assert_eq!(updating.surface, SurfaceResizeAction::KeepCurrent);
        assert!(updating.metrics_revision > started.metrics_revision);

        let ended = coordinator
            .finalize(
                Some(ResizeSignalSnapshot::new(7, false)),
                SurfaceCommitPolicy::DeferredScaledPreview,
            )
            .unwrap();
        assert_eq!(ended.phase, ResizeInteractionPhase::Ended);
        assert_eq!(ended.extent, LATEST);
        assert_eq!(ended.surface, SurfaceResizeAction::CommitAfterPreview);
        assert_eq!(ended.metrics_revision, updating.metrics_revision);
        assert!(
            coordinator
                .finalize(
                    Some(ResizeSignalSnapshot::new(7, false)),
                    SurfaceCommitPolicy::DeferredScaledPreview,
                )
                .is_none()
        );
    }

    #[test]
    fn stale_end_signal_cannot_finalize_a_newer_resize() {
        let mut coordinator = LiveResizeCoordinator::default();
        coordinator.observe(
            FIRST,
            Some(ResizeSignalSnapshot::new(9, true)),
            SurfaceCommitPolicy::DeferredScaledPreview,
        );
        assert!(!coordinator.needs_finalization(Some(ResizeSignalSnapshot::new(8, false))));
    }

    #[test]
    fn frame_paced_policy_commits_intermediate_extents() {
        let mut coordinator = LiveResizeCoordinator::default();
        let update = coordinator.observe(
            FIRST,
            Some(ResizeSignalSnapshot::new(3, true)),
            SurfaceCommitPolicy::Responsive,
        );
        assert_eq!(update.surface, SurfaceResizeAction::Commit);

        let ended = coordinator
            .finalize(
                Some(ResizeSignalSnapshot::new(3, false)),
                SurfaceCommitPolicy::Responsive,
            )
            .unwrap();
        assert_eq!(ended.surface, SurfaceResizeAction::Commit);
    }

    #[test]
    fn zero_extent_suspends_and_cancels_live_resize() {
        let mut coordinator = LiveResizeCoordinator::default();
        coordinator.observe(
            FIRST,
            Some(ResizeSignalSnapshot::new(2, true)),
            SurfaceCommitPolicy::DeferredScaledPreview,
        );
        let update = coordinator.observe(
            SizeI {
                width: 0,
                height: 600,
            },
            Some(ResizeSignalSnapshot::new(2, true)),
            SurfaceCommitPolicy::DeferredScaledPreview,
        );
        assert_eq!(update.phase, ResizeInteractionPhase::Cancelled);
        assert_eq!(update.surface, SurfaceResizeAction::Suspend);
        assert!(!coordinator.needs_finalization(Some(ResizeSignalSnapshot::new(2, false))));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn win32_messages_define_exact_resize_generations() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_TIMER,
        };

        let signals = PlatformResizeSignals::default();
        assert_eq!(
            signals.observe_windows_message(41, WM_ENTERSIZEMOVE),
            Some(ResizeSignalSnapshot::new(1, true))
        );
        assert_eq!(
            signals.windows.lock().unwrap().get(&41).copied(),
            Some(ResizeSignalSnapshot::new(1, true))
        );
        assert_eq!(
            signals.observe_windows_message(41, WM_EXITSIZEMOVE),
            Some(ResizeSignalSnapshot::new(1, false))
        );
        assert_eq!(
            signals.windows.lock().unwrap().get(&41).copied(),
            Some(ResizeSignalSnapshot::new(1, false))
        );
        assert_eq!(
            signals.observe_windows_message(41, WM_ENTERSIZEMOVE),
            Some(ResizeSignalSnapshot::new(2, true))
        );
        assert_eq!(
            signals.windows.lock().unwrap().get(&41).copied(),
            Some(ResizeSignalSnapshot::new(2, true))
        );
        assert!(is_resize_redraw_timer(WM_TIMER, RESIZE_REDRAW_TIMER_ID));
        assert!(!is_resize_redraw_timer(
            WM_TIMER,
            RESIZE_REDRAW_TIMER_ID + 1
        ));
    }
}
