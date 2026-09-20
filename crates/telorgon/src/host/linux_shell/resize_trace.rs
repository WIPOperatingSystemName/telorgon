//! Opt-in, bounded metadata tracing; does not alter frame or buffer ownership.
use std::{
    collections::BTreeMap,
    fmt,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

struct Capture {
    id: u64,
    started: Instant,
    deadline: Instant,
    events: usize,
}
#[derive(Default)]
struct Trace {
    next_id: u64,
    captures: BTreeMap<u32, Capture>,
}
fn state() -> Option<&'static Mutex<Trace>> {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    static STATE: OnceLock<Mutex<Trace>> = OnceLock::new();
    ENABLED
        .get_or_init(|| std::env::var("TELORGON_X11_RESIZE_TRACE").as_deref() == Ok("1"))
        .then(|| STATE.get_or_init(|| Mutex::new(Trace::default())))
}
pub(super) fn begin(surface: u32, details: fmt::Arguments<'_>) {
    let Some(state) = state() else { return };
    let Ok(mut trace) = state.lock() else { return };
    let now = Instant::now();
    trace.captures.retain(|_, c| c.deadline > now);
    if trace.captures.len() >= 64 && !trace.captures.contains_key(&surface) {
        return;
    }
    trace.next_id = trace.next_id.wrapping_add(1);
    let id = trace.next_id;
    trace.captures.insert(
        surface,
        Capture {
            id,
            started: now,
            deadline: now + Duration::from_secs(5),
            events: 0,
        },
    );
    drop(trace);
    event(surface, "begin", details);
}
pub(super) fn reveal(surface: u32, details: fmt::Arguments<'_>) {
    if let Some(state) = state()
        && let Ok(mut trace) = state.lock()
        && let Some(capture) = trace.captures.get_mut(&surface)
    {
        capture.deadline = Instant::now() + Duration::from_secs(1);
    }
    event(surface, "reveal", details);
}
pub(super) fn event(surface: u32, stage: &str, details: fmt::Arguments<'_>) {
    let Some(state) = state() else { return };
    let Ok(mut trace) = state.lock() else { return };
    let Some(capture) = trace.captures.get_mut(&surface) else {
        return;
    };
    if Instant::now() > capture.deadline || capture.events >= 1000 {
        return;
    }
    capture.events += 1;
    eprintln!(
        "telorgon-resize-trace: resize={} surface={} event={} us={} stage={} {}",
        capture.id,
        surface,
        capture.events,
        capture.started.elapsed().as_micros(),
        stage,
        details
    );
    if capture.events == 1000 {
        eprintln!(
            "telorgon-resize-trace: resize={} surface={} stage=limit reached=1000",
            capture.id, surface
        );
    }
}
