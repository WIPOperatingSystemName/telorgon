//! Coherent snapshots published by the single audio processing thread.
use super::{AudioShared, AudioState, AudioTiming};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug)]
pub struct AudioClockSnapshot {
    pub timing: AudioTiming,
    pub sample_rate: u32,
    pub quantum_frames: u32,
    /// Cumulative discontinuity count; compare successive snapshots, since a reader may
    /// skip the particular cycle that carried `timing.discontinuity`.
    pub discontinuities: u64,
}
impl AudioClockSnapshot {
    /// Project an application sample-frame position into this snapshot's host
    /// CLOCK_MONOTONIC domain. Fractional nanoseconds round toward the future.
    /// This is graph processing time; device and application queue delays are not
    /// included. Refresh after format/discontinuity changes or clock drift.
    pub fn frame_time_ns(self, frame_position: u64) -> Option<i64> {
        if self.sample_rate == 0 || self.timing.format_generation == 0 {
            return None;
        }
        let frames = i128::from(frame_position) - i128::from(self.timing.frame_position);
        let numerator = frames * 1_000_000_000;
        let denominator = i128::from(self.sample_rate);
        let delta =
            numerator.div_euclid(denominator) + i128::from(numerator.rem_euclid(denominator) != 0);
        i64::try_from(i128::from(self.timing.monotonic_ns) + delta).ok()
    }

    /// Delay reported by PipeWire in the graph clock's units. This excludes application
    /// buffers not yet handed to PipeWire; it is an estimate, not a presentation receipt.
    pub fn delay_ns(self) -> Option<i64> {
        if self.timing.rate_num == 0 || self.timing.rate_denom == 0 {
            return None;
        }
        let delay =
            i128::from(self.timing.delay_ticks) * i128::from(self.timing.rate_num) * 1_000_000_000
                / i128::from(self.timing.rate_denom);
        i64::try_from(delay).ok()
    }
    /// Estimated audio latency including frames still held in the application's own queue.
    /// Pass only frames not included in PipeWire's delay to avoid counting them twice.
    pub fn presentation_delay_ns(self, application_queued_frames: u64) -> Option<u64> {
        if self.sample_rate == 0 {
            return None;
        }
        let queued =
            i128::from(application_queued_frames) * 1_000_000_000 / i128::from(self.sample_rate);
        u64::try_from((i128::from(self.delay_ns()?) + queued).max(0)).ok()
    }
    pub fn is_recent(self, monotonic_now_ns: i64, maximum_age_ns: u64) -> bool {
        let age = i128::from(monotonic_now_ns) - i128::from(self.timing.monotonic_ns);
        age >= 0 && age <= i128::from(maximum_age_ns)
    }
}
/// Read-only clock handle. It grants no stream control and does not keep a stopped native
/// stream running. Control-thread reads are bounded and return None during publication,
/// before valid timing, or while paused/stopped/unavailable.
#[derive(Clone)]
pub struct AudioClock {
    pub(crate) shared: Arc<AudioShared>,
}
impl AudioClock {
    pub fn snapshot(&self) -> Option<AudioClockSnapshot> {
        let epoch = self.shared.format_epoch.load(Ordering::Acquire);
        if self.shared.stop.load(Ordering::Acquire) || epoch & 1 == 0 {
            return None;
        }
        let state = self.shared.state.try_lock().ok()?;
        if !matches!(*state, AudioState::Streaming | AudioState::Draining) {
            return None;
        }
        let snapshot = self.shared.clock.read()?;
        (snapshot.timing.format_generation == epoch >> 1
            && self.shared.format_epoch.load(Ordering::Acquire) == epoch)
            .then_some(snapshot)
    }
}

pub(crate) struct ClockPublication {
    sequence: AtomicU64,
    fields: [AtomicU64; 11],
}
impl Default for ClockPublication {
    fn default() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            fields: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}
impl ClockPublication {
    /// Single writer: only RealtimeData::process calls this. Every payload access is atomic;
    /// SeqCst keeps the odd marker before payload writes and the even marker after them.
    pub fn publish(&self, snapshot: AudioClockSnapshot) {
        let sequence = self.sequence.load(Ordering::SeqCst);
        let Some(next) = sequence.checked_add(2) else {
            return;
        };
        self.sequence.store(sequence + 1, Ordering::SeqCst);
        let t = snapshot.timing;
        let values = [
            t.monotonic_ns as u64,
            t.graph_ticks,
            u64::from(t.rate_num),
            u64::from(t.rate_denom),
            t.delay_ticks as u64,
            t.frame_position,
            u64::from(t.discontinuity),
            u64::from(snapshot.sample_rate),
            u64::from(snapshot.quantum_frames),
            snapshot.discontinuities,
            t.format_generation,
        ];
        for (field, value) in self.fields.iter().zip(values) {
            field.store(value, Ordering::SeqCst);
        }
        self.sequence.store(next, Ordering::SeqCst);
    }
    fn read(&self) -> Option<AudioClockSnapshot> {
        for _ in 0..4 {
            let before = self.sequence.load(Ordering::SeqCst);
            if before == 0 {
                return None;
            }
            if before & 1 != 0 {
                continue;
            }
            let v: [u64; 11] = std::array::from_fn(|i| self.fields[i].load(Ordering::SeqCst));
            if self.sequence.load(Ordering::SeqCst) != before {
                continue;
            }
            return Some(AudioClockSnapshot {
                timing: AudioTiming {
                    monotonic_ns: v[0] as i64,
                    graph_ticks: v[1],
                    rate_num: v[2] as u32,
                    rate_denom: v[3] as u32,
                    delay_ticks: v[4] as i64,
                    frame_position: v[5],
                    discontinuity: v[6] != 0,
                    format_generation: v[10],
                },
                sample_rate: v[7] as u32,
                quantum_frames: v[8] as u32,
                discontinuities: v[9],
            });
        }
        None
    }
}
