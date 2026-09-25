//! Bounded preview diagnostics. No client titles or pixels are recorded.
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fmt,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

pub(super) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("TELORGON_PREVIEW_DEBUG").as_deref() == Ok("1"))
}

pub(super) fn event(args: fmt::Arguments<'_>) {
    static LINES: AtomicUsize = AtomicUsize::new(0);
    if !enabled() {
        return;
    }
    match LINES.fetch_add(1, Ordering::Relaxed) {
        0..2048 => eprintln!("telorgon-preview: {args}"),
        2048 => {
            eprintln!("telorgon-preview: diagnostic limit reached; restart to record another run")
        }
        _ => {}
    }
}

#[derive(Default)]
struct Stats {
    since: Option<Instant>,
    stages: BTreeMap<&'static str, (u64, u128, u128)>,
}

thread_local! {
    static STATS: RefCell<Stats> = RefCell::new(Stats::default());
}

/// Owner-thread CPU durations, distinct from the existing asynchronous GPU timestamp stats.
pub(super) struct Span {
    label: &'static str,
    start: Option<Instant>,
}

pub(super) fn span(label: &'static str) -> Span {
    Span {
        label,
        start: enabled().then(Instant::now),
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(start) = self.start else { return };
        let elapsed = start.elapsed().as_micros();
        STATS.with_borrow_mut(|stats| {
            let since = *stats.since.get_or_insert(start);
            let stage = stats.stages.entry(self.label).or_default();
            stage.0 += 1;
            stage.1 += elapsed;
            stage.2 = stage.2.max(elapsed);
            if elapsed >= 8_000 {
                event(format_args!(
                    "slow_cpu stage={} elapsed_us={elapsed}",
                    self.label
                ));
            }
            if since.elapsed().as_secs() >= 2 {
                for (label, (calls, total, max)) in &stats.stages {
                    event(format_args!(
                        "cpu stage={label} calls={calls} avg_us={} max_us={max}",
                        total / u128::from(*calls)
                    ));
                }
                stats.stages.clear();
                stats.since = Some(Instant::now());
            }
        });
    }
}

/// Tracks owner-observed ready-to-submit delay without logging every blocked event-loop turn.
#[derive(Default)]
pub(super) struct PresentationTrace {
    ready: BTreeMap<usize, (Instant, bool, bool)>,
}

impl PresentationTrace {
    pub(super) fn ready(&mut self, slot: usize) {
        if enabled() {
            self.ready.insert(slot, (Instant::now(), false, false));
        }
    }

    pub(super) fn blocked(&mut self, primary_commit: bool) {
        for (_, primary, cursor) in self.ready.values_mut() {
            *primary |= primary_commit;
            *cursor |= !primary_commit;
        }
    }

    pub(super) fn submitted(&mut self, slot: usize) {
        if let Some((start, primary, cursor)) = self.ready.remove(&slot) {
            let wait_us = start.elapsed().as_micros();
            if wait_us >= 2_000 {
                event(format_args!("presentation slot={slot} ready_to_submit_us={wait_us} blocked_by_primary={primary} blocked_by_cursor={cursor}"));
            }
        }
    }

    pub(super) fn discard(&mut self, slot: usize) {
        self.ready.remove(&slot);
    }
}

/// Trace the selected widget through animation, physical scaling, and snapshot grouping.
/// Stable frames are omitted; a changed rendered rectangle or fade is always recorded.
pub(super) fn tile_frames(
    frame: &super::scene::ShellFrame,
    widgets: &[super::widgets::WidgetLayer],
    windows: &BTreeMap<crate::integrations::wayland::compositor::WaylandSurfaceId, super::client::ClientWindow>,
    now: u64,
) {
    if !enabled() { return; }
    thread_local! {
        static LAST: RefCell<BTreeMap<u32, String>> = RefCell::new(BTreeMap::new());
    }
    for widget in widgets.iter().filter(|w| w.spec.tiling.is_some()) {
        let placements: Vec<_> = frame.placements.iter().filter(|p| matches!(p.key,
            super::scene::ShellLayerKey::Widget(id) | super::scene::ShellLayerKey::TilePreview(id) if id == widget.id))
            .map(|p| {
                let extent = match p.scene {
                    super::scene::ShellSceneKey::Motion(id) => frame.motion.outputs.iter().find(|o| o.id == id).map(|o| o.extent),
                    _ => None,
                };
                // Snapshot identities change each fade frame; compare geometry instead.
                (p.key, p.target, p.clip, p.rounded_clips, extent)
            }).collect();
        let owner = widget.tile_preview_owner;
        let csd = owner.and_then(|id| windows.get(&id)).map(|w| !w.server_decorated);
        let message = format!("widget={} owner={owner:?} client_decorated={csd:?} selected={:?} sampled={:?} opacity={:.4} physical={placements:?}",
            widget.id, widget.tile_preview, widget.sampled, widget.opacity);
        LAST.with_borrow_mut(|last| {
            if last.get(&widget.id) != Some(&message) {
                if !placements.is_empty() || last.contains_key(&widget.id) {
                    event(format_args!("tile_frame now_ns={now} {message}"));
                }
                last.insert(widget.id, message);
            }
        });
    }
}
