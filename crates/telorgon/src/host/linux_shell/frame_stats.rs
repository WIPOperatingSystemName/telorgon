//! Opt-in owner-loop observations, not hardware presentation timestamps or GPU timings.
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Surface {
    revision: Option<u64>,
    updates: u64,
    seen: bool,
}
impl Surface {
    fn observe(&mut self, revision: u64) {
        self.seen = true;
        if self.revision != Some(revision) {
            self.revision = Some(revision);
            self.updates += 1;
        }
    }
}

#[derive(Default)]
struct Timings {
    samples: u64,
    total: Duration,
    max: Duration,
}
impl Timings {
    fn observe(&mut self, duration: Duration) {
        self.samples += 1;
        self.total += duration;
        self.max = self.max.max(duration);
    }
    fn merge(&mut self, other: Self) {
        self.samples += other.samples;
        self.total += other.total;
        self.max = self.max.max(other.max);
    }
    fn avg_ms(&self) -> f64 {
        self.total.as_secs_f64() * 1000.0 / self.samples.max(1) as f64
    }
    fn max_ms(&self) -> f64 {
        self.max.as_secs_f64() * 1000.0
    }
}

/// Measures event age using libinput's CLOCK_MONOTONIC domain; never mixes it with uptime-relative
/// Wayland callback timestamps. Device lifecycle events with timestamp zero have no age sample.
pub(super) struct InputBatch {
    started: Instant,
    observed_us: Option<u64>,
    oldest_us: Option<u64>,
    events: u64,
    queue_age: Timings,
}
impl InputBatch {
    pub(super) fn new(observed_us: Option<u64>) -> Self {
        Self {
            started: Instant::now(),
            observed_us,
            oldest_us: None,
            events: 0,
            queue_age: Timings::default(),
        }
    }
    pub(super) fn observe(&mut self, event_us: u64) {
        self.events += 1;
        if let Some(observed) = self.observed_us
            && event_us != 0
            && event_us <= observed
        {
            self.queue_age
                .observe(Duration::from_micros(observed - event_us));
            self.oldest_us = Some(
                self.oldest_us
                    .map_or(event_us, |oldest| oldest.min(event_us)),
            );
        }
    }
    pub(super) fn trace_values(&self, flushed_us: Option<u64>) -> [u64; 4] {
        [
            self.events,
            self.queue_age.samples,
            self.queue_age.max.as_micros().min(u128::from(u64::MAX)) as u64,
            self.oldest_age(flushed_us).map_or(u64::MAX, |age| {
                age.as_micros().min(u128::from(u64::MAX)) as u64
            }),
        ]
    }
    pub(super) fn probe(&self, flushed_us: Option<u64>, context: impl Fn() -> String) {
        if !super::stall_probe::enabled() { return; }
        let cpu = self.started.elapsed();
        super::stall_probe::observe("input_batch_cpu", cpu, &context);
        if self.queue_age.samples > 0 {
            super::stall_probe::observe("input_queue", self.queue_age.max, &context);
        }
        if let Some(age) = self.oldest_age(flushed_us) {
            super::stall_probe::observe("input_to_flush", age, context);
        }
    }
    fn oldest_age(&self, flushed_us: Option<u64>) -> Option<Duration> {
        let (oldest, flushed) = (self.oldest_us?, flushed_us?);
        flushed.checked_sub(oldest).map(Duration::from_micros)
    }
}

pub(super) struct FrameStats {
    since: Instant,
    refresh_hz: f64,
    flips: u64,
    last_flip: Option<Instant>,
    gap_max: Duration,
    renders: u64,
    cpu_total: Duration,
    cpu_max: Duration,
    surfaces: BTreeMap<u32, Surface>,
    owner_cpu: Timings,
    input_events: u64,
    input_queue_age: Timings,
    input_batch_cpu: Timings,
    input_oldest_to_flush: Timings,
    x11_shm_commits: u64,
    x11_dmabuf_commits: u64,
}
impl FrameStats {
    pub(super) fn from_env(refresh_millihertz: u32) -> Option<Self> {
        if std::env::var("TELORGON_FRAME_STATS").as_deref() != Ok("1") {
            return None;
        }
        let refresh_hz = f64::from(refresh_millihertz) / 1000.0;
        eprintln!(
            "telorgon-frame-stats: enabled refresh_hz={refresh_hz:.3} debug_assertions={} (primary flip observations; idle gaps are normal)",
            cfg!(debug_assertions)
        );
        Some(Self {
            since: Instant::now(),
            refresh_hz,
            flips: 0,
            last_flip: None,
            gap_max: Duration::ZERO,
            renders: 0,
            cpu_total: Duration::ZERO,
            cpu_max: Duration::ZERO,
            surfaces: BTreeMap::new(),
            owner_cpu: Timings::default(),
            input_events: 0,
            input_queue_age: Timings::default(),
            input_batch_cpu: Timings::default(),
            input_oldest_to_flush: Timings::default(),
            x11_shm_commits: 0,
            x11_dmabuf_commits: 0,
        })
    }
    pub(super) fn presented(&mut self, revisions: &[(u32, u64)]) {
        let now = Instant::now();
        if let Some(previous) = self.last_flip.replace(now) {
            self.gap_max = self.gap_max.max(now.duration_since(previous));
        }
        self.flips += 1;
        for &(surface, revision) in revisions {
            // Keep observation storage bounded even with rapidly created windows.
            if self.surfaces.len() < 256 || self.surfaces.contains_key(&surface) {
                self.surfaces.entry(surface).or_default().observe(revision);
            }
        }
    }
    pub(super) fn rendered(&mut self, duration: Duration) {
        self.renders += 1;
        self.cpu_total += duration;
        self.cpu_max = self.cpu_max.max(duration);
    }
    pub(super) fn owner_turn(&mut self, duration: Duration) {
        self.owner_cpu.observe(duration);
    }
    pub(super) fn input_flushed(&mut self, batch: InputBatch, flushed_us: Option<u64>) {
        self.input_batch_cpu.observe(batch.started.elapsed());
        if let Some(age) = batch.oldest_age(flushed_us) {
            self.input_oldest_to_flush.observe(age);
        }
        self.input_events += batch.events;
        self.input_queue_age.merge(batch.queue_age);
    }
    pub(super) fn x11_commit(&mut self, dma_buf: bool) {
        if dma_buf {
            self.x11_dmabuf_commits += 1;
        } else {
            self.x11_shm_commits += 1;
        }
    }
    pub(super) fn report(&mut self) {
        let now = Instant::now();
        let seconds = now.duration_since(self.since).as_secs_f64();
        if seconds < 2.0 {
            return;
        }
        eprintln!(
            "telorgon-frame-stats: seconds={seconds:.2} refresh_hz={:.3} primary_fps={:.1} render_fps={:.1} max_primary_gap_ms={:.2} render_cpu_avg_ms={:.2} render_cpu_max_ms={:.2}",
            self.refresh_hz,
            self.flips as f64 / seconds,
            self.renders as f64 / seconds,
            self.gap_max.as_secs_f64() * 1000.0,
            self.cpu_total.as_secs_f64() * 1000.0 / self.renders.max(1) as f64,
            self.cpu_max.as_secs_f64() * 1000.0
        );
        eprintln!(
            "telorgon-frame-stats: owner_active_avg_ms={:.2} owner_active_max_ms={:.2} input_events={} input_age_samples={} input_queue_avg_ms={:.2} input_queue_max_ms={:.2} input_batch_cpu_max_ms={:.2} input_flush_samples={} input_oldest_to_flush_max_ms={:.2} x11_shm_commits={} x11_dmabuf_commits={}",
            self.owner_cpu.avg_ms(),
            self.owner_cpu.max_ms(),
            self.input_events,
            self.input_queue_age.samples,
            self.input_queue_age.avg_ms(),
            self.input_queue_age.max_ms(),
            self.input_batch_cpu.max_ms(),
            self.input_oldest_to_flush.samples,
            self.input_oldest_to_flush.max_ms(),
            self.x11_shm_commits,
            self.x11_dmabuf_commits
        );
        self.surfaces.retain(|id, surface| {
            eprintln!(
                "telorgon-frame-stats: surface={id} new_content_fps={:.1}",
                surface.updates as f64 / seconds
            );
            let active = surface.seen;
            surface.seen = false;
            surface.updates = 0;
            active
        });
        self.since = now;
        self.flips = 0;
        self.renders = 0;
        self.gap_max = Duration::ZERO;
        self.cpu_total = Duration::ZERO;
        self.cpu_max = Duration::ZERO;
        self.owner_cpu = Timings::default();
        self.input_events = 0;
        self.input_queue_age = Timings::default();
        self.input_batch_cpu = Timings::default();
        self.input_oldest_to_flush = Timings::default();
        self.x11_shm_commits = 0;
        self.x11_dmabuf_commits = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_age_excludes_missing_and_future_timestamps_and_tracks_oldest() {
        let mut batch = InputBatch::new(Some(1_000_000));
        for time in [999_000, 990_000, 0, 1_000_100] {
            batch.observe(time);
        }
        assert_eq!(batch.events, 4);
        assert_eq!(batch.queue_age.samples, 2);
        assert_eq!(batch.queue_age.avg_ms(), 5.5);
        assert_eq!(batch.queue_age.max_ms(), 10.0);
        assert_eq!(
            batch.oldest_age(Some(1_002_000)),
            Some(Duration::from_millis(12))
        );
        assert_eq!(batch.oldest_age(Some(980_000)), None);
        assert_eq!(batch.oldest_age(None), None);
        let mut no_clock = InputBatch::new(None);
        no_clock.observe(900_000);
        assert_eq!(no_clock.queue_age.samples, 0);
        assert_eq!(no_clock.oldest_age(Some(1_002_000)), None);
    }
    #[test]
    fn timing_batches_preserve_sample_weight_and_maximum() {
        let mut first = Timings::default();
        first.observe(Duration::from_millis(3));
        let mut second = Timings::default();
        second.observe(Duration::from_millis(1));
        second.observe(Duration::from_millis(2));
        first.merge(second);
        assert_eq!(first.samples, 3);
        assert_eq!(first.avg_ms(), 2.0);
        assert_eq!(first.max_ms(), 3.0);
        assert_eq!(Timings::default().avg_ms(), 0.0);
    }
    #[test]
    fn unchanged_content_is_not_counted_as_a_new_client_frame() {
        let mut surface = Surface::default();
        for revision in [1, 1, 2, 2, 2, 4] {
            surface.observe(revision);
        }
        assert_eq!(surface.updates, 3);
        surface.updates = 0;
        surface.observe(4);
        assert_eq!(surface.updates, 0);
        surface.observe(5);
        assert_eq!(surface.updates, 1);
    }
}
