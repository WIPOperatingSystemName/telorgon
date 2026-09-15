use super::*;

/// Signal handlers only set an atomic and write to an eventfd. request_exit() itself acquires
/// locks, so it must run on the owner thread after the event loop wakes.
pub(super) struct TerminationSignals {
    requested: Arc<AtomicBool>,
    registrations: Vec<signal_hook_registry::SigId>,
}
impl TerminationSignals {
    pub(super) fn new(notifier: EventNotifier) -> AppResult<Self> {
        let mut signals = Self {
            requested: Arc::new(AtomicBool::new(false)),
            registrations: Vec::new(),
        };
        for signal in [libc::SIGTERM, libc::SIGINT] {
            let requested = signals.requested.clone();
            let notifier = notifier.clone();
            // SAFETY: AtomicBool::store and write(2) are the only operations in the handler.
            let registration = unsafe {
                signal_hook_registry::register(signal, move || {
                    requested.store(true, Ordering::Release);
                    notifier.notify();
                })
            }
            .map_err(app_error)?;
            signals.registrations.push(registration);
        }
        Ok(signals)
    }
    pub(super) fn take_requested(&self) -> bool {
        self.requested.swap(false, Ordering::AcqRel)
    }
}
impl Drop for TerminationSignals {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..) {
            signal_hook_registry::unregister(registration);
        }
    }
}

pub(super) struct InputReadyState {
    pub(super) ready: AtomicBool,
    #[cfg(feature = "profiler")]
    pub(super) callback_time_us: AtomicU64,
}

impl InputReadyState {
    pub(super) const fn new(ready: bool) -> Self {
        Self {
            ready: AtomicBool::new(ready),
            #[cfg(feature = "profiler")]
            callback_time_us: AtomicU64::new(0),
        }
    }
}

#[derive(Clone)]
pub(super) struct EventNotifier {
    fd: Arc<OwnedFd>,
}

impl EventNotifier {
    pub(super) fn new(context: &str) -> AppResult<Self> {
        let raw = unsafe {
            crate::platform_linux::ffi::eventfd(
                0,
                crate::platform_linux::ffi::EFD_CLOEXEC | crate::platform_linux::ffi::EFD_NONBLOCK,
            )
        };
        if raw < 0 {
            Err(AppError::new(format!("failed to create {context} eventfd")))
        } else {
            Ok(Self {
                fd: Arc::new(unsafe { OwnedFd::from_raw_fd(raw) }),
            })
        }
    }

    pub(super) fn event_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    pub(super) fn notify(&self) {
        let value = 1_u64;
        let _ = unsafe {
            crate::platform_linux::ffi::write(
                self.fd.as_raw_fd(),
                std::ptr::from_ref(&value).cast(),
                std::mem::size_of::<u64>(),
            )
        };
    }

    pub(super) fn drain(&self) {
        let mut value = 0_u64;
        loop {
            let read = unsafe {
                crate::platform_linux::ffi::read(
                    self.fd.as_raw_fd(),
                    std::ptr::from_mut(&mut value).cast(),
                    std::mem::size_of::<u64>(),
                )
            };
            if read != std::mem::size_of::<u64>() as isize {
                break;
            }
        }
    }
}

pub(super) unsafe extern "C" fn mark_external_fd_ready(
    _fd: i32,
    _mask: u32,
    data: *mut std::ffi::c_void,
) -> i32 {
    let Some(ready) = std::ptr::NonNull::new(data.cast::<AtomicBool>()) else {
        return 0;
    };
    unsafe { ready.as_ref() }.store(true, Ordering::Release);
    0
}

pub(super) unsafe extern "C" fn mark_input_fd_ready(
    _fd: i32,
    _mask: u32,
    data: *mut std::ffi::c_void,
) -> i32 {
    let Some(state) = std::ptr::NonNull::new(data.cast::<InputReadyState>()) else {
        return 0;
    };
    let state = unsafe { state.as_ref() };
    #[cfg(feature = "profiler")]
    if crate::profiler::pointer_move_events_enabled()
        && let Some(now_us) = crate::platform_linux::monotonic_time_microseconds()
    {
        state.callback_time_us.store(now_us, Ordering::Release);
    }
    state.ready.store(true, Ordering::Release);
    0
}

#[cfg(feature = "profiler")]
#[derive(Clone, Copy)]
pub(super) enum PointerCursorPath {
    CompositedDamage,
    Hidden,
    Deferred,
    Unchanged,
}

#[cfg(feature = "profiler")]
pub(super) struct PointerBatchProbe {
    pub(super) enabled: bool,
    dispatch_started: Option<Instant>,
    dispatch_duration_ns: u64,
    handler_started: Option<Instant>,
    events: u64,
    newest_event_us: Option<u64>,
}

#[cfg(feature = "profiler")]
impl PointerBatchProbe {
    pub(super) fn begin() -> Self {
        let enabled = crate::profiler::input_recording_enabled(
            crate::profiler::InputRecordingSource::PointerMotion,
        );
        Self {
            enabled,
            dispatch_started: enabled.then(Instant::now),
            dispatch_duration_ns: 0,
            handler_started: None,
            events: 0,
            newest_event_us: None,
        }
    }

    pub(super) fn dispatch_completed(&mut self) {
        if let Some(started) = self.dispatch_started {
            self.dispatch_duration_ns = duration_ns(started.elapsed());
            self.handler_started = Some(Instant::now());
        }
    }

    pub(super) fn observe_motion(&mut self, event_time_us: u64) {
        if !self.enabled {
            return;
        }
        self.events = self.events.saturating_add(1);
        self.newest_event_us = Some(
            self.newest_event_us
                .map_or(event_time_us, |newest| newest.max(event_time_us)),
        );
    }

    pub(super) fn newest_event_us(&self) -> Option<u64> {
        self.enabled.then_some(self.newest_event_us).flatten()
    }

    pub(super) fn has_motion(&self) -> bool {
        self.enabled && self.events != 0
    }

    pub(super) fn finish(&self, path: PointerCursorPath) {
        if !self.enabled || self.events == 0 {
            return;
        }
        crate::profiler::record_instant_value(
            "input.libinput.pointer_motion.pipeline.events_per_batch",
            self.events,
        );
        crate::profiler::record_instant_value(
            "input.libinput.pointer_motion.pipeline.dispatch_duration_ns",
            self.dispatch_duration_ns,
        );
        crate::profiler::record_instant_value(
            "input.libinput.pointer_motion.pipeline.batch_handler_duration_ns",
            self.handler_started
                .map_or(0, |started| duration_ns(started.elapsed())),
        );
        crate::profiler::record_instant(match path {
            PointerCursorPath::CompositedDamage => {
                "input.libinput.pointer_motion.pipeline.path.composited_damage"
            }
            PointerCursorPath::Hidden => "input.libinput.pointer_motion.pipeline.path.hidden",
            PointerCursorPath::Deferred => "input.libinput.pointer_motion.pipeline.path.deferred",
            PointerCursorPath::Unchanged => {
                "input.libinput.pointer_motion.pipeline.path.position_unchanged"
            }
        });
    }
}

#[cfg(feature = "profiler")]
fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(feature = "profiler")]
pub(super) fn record_pointer_event_latency(label: &'static str, event_time_us: u64) {
    if !crate::profiler::input_recording_enabled(
        crate::profiler::InputRecordingSource::PointerMotion,
    ) {
        return;
    }
    if let Some(now_us) = crate::platform_linux::monotonic_time_microseconds() {
        crate::profiler::record_instant_value(
            label,
            now_us.saturating_sub(event_time_us).saturating_mul(1_000),
        );
    }
}
