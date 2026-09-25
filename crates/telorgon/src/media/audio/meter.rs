//! Opt-in meters over samples already owned by an audio stream; no device acquisition.
use super::{AudioShared, AudioState, AudioTiming};
use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChannelLevel {
    pub peak: f32,
    pub rms: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioLevels {
    pub channels: usize,
    pub levels: [ChannelLevel; 64],
    pub frames: u64,
    pub monotonic_ns: i64,
    pub format_generation: u64,
}
/// Poll from a UI/control worker. Returns each publication at most once per handle, never
/// older levels while paused/stopped. Last-handle drop disables sample accumulation.
/// Publication is at most 20 Hz of processed audio; large quanta may publish less often.
pub struct AudioMeter {
    pub(crate) shared: Arc<AudioShared>,
    seen: u64,
}
impl AudioMeter {
    pub(crate) fn new(shared: Arc<AudioShared>) -> Self {
        shared.meter.listeners.fetch_add(1, Ordering::AcqRel);
        let seen = shared.meter.sequence.load(Ordering::SeqCst);
        Self { shared, seen }
    }
    pub fn read(&mut self) -> Option<AudioLevels> {
        let epoch = self.shared.format_epoch.load(Ordering::Acquire);
        if self.shared.stop.load(Ordering::Acquire) || epoch & 1 == 0 {
            return None;
        }
        let state = self.shared.state.try_lock().ok()?;
        if !matches!(*state, AudioState::Streaming | AudioState::Draining) {
            return None;
        }
        let meter = &self.shared.meter;
        for _ in 0..4 {
            let before = meter.sequence.load(Ordering::SeqCst);
            if before == 0 || before == self.seen {
                return None;
            }
            if before & 1 != 0 {
                continue;
            }
            let channels = meter.channels.load(Ordering::SeqCst) as usize;
            let frames = meter.frames.load(Ordering::SeqCst);
            let monotonic_ns = meter.now.load(Ordering::SeqCst) as i64;
            let format_generation = meter.format_generation.load(Ordering::SeqCst);
            let mut levels = [ChannelLevel::default(); 64];
            for (index, level) in levels.iter_mut().enumerate().take(channels.min(64)) {
                level.peak = f32::from_bits(meter.peak[index].load(Ordering::SeqCst));
                level.rms = f32::from_bits(meter.rms[index].load(Ordering::SeqCst));
            }
            if meter.sequence.load(Ordering::SeqCst) != before {
                continue;
            }
            if format_generation != epoch >> 1
                || self.shared.format_epoch.load(Ordering::Acquire) != epoch
            {
                return None;
            }
            self.seen = before;
            return Some(AudioLevels {
                channels: channels.min(64),
                levels,
                frames,
                monotonic_ns,
                format_generation,
            });
        }
        None
    }
}
impl Clone for AudioMeter {
    fn clone(&self) -> Self {
        Self::new(self.shared.clone())
    }
}
impl Drop for AudioMeter {
    fn drop(&mut self) {
        self.shared.meter.listeners.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct MeterPublication {
    listeners: AtomicUsize,
    sequence: AtomicU64,
    channels: AtomicU32,
    frames: AtomicU64,
    now: AtomicU64,
    format_generation: AtomicU64,
    peak: [AtomicU32; 64],
    rms: [AtomicU32; 64],
}
impl Default for MeterPublication {
    fn default() -> Self {
        Self {
            listeners: AtomicUsize::new(0),
            sequence: AtomicU64::new(0),
            channels: AtomicU32::new(0),
            frames: AtomicU64::new(0),
            now: AtomicU64::new(0),
            format_generation: AtomicU64::new(0),
            peak: std::array::from_fn(|_| AtomicU32::new(0)),
            rms: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
}
pub(crate) struct MeterAccumulator {
    peak: [f32; 64],
    square: [f64; 64],
    frames: u64,
}
impl Default for MeterAccumulator {
    fn default() -> Self {
        Self {
            peak: [0.0; 64],
            square: [0.0; 64],
            frames: 0,
        }
    }
}
impl MeterAccumulator {
    fn reset(&mut self) {
        self.peak.fill(0.0);
        self.square.fill(0.0);
        self.frames = 0;
    }
    /// Single producer; only atomics/fixed storage are touched on the audio processing path.
    pub fn process(
        &mut self,
        publication: &MeterPublication,
        samples: &[f32],
        channels: usize,
        rate: u32,
        timing: AudioTiming,
    ) {
        if publication.listeners.load(Ordering::Acquire) == 0 {
            if self.frames != 0 {
                self.reset();
            }
            return;
        }
        if timing.discontinuity {
            self.reset();
        }
        for frame in samples.chunks_exact(channels) {
            for (index, sample) in frame.iter().enumerate() {
                self.peak[index] = self.peak[index].max(sample.abs());
                self.square[index] += f64::from(*sample) * f64::from(*sample);
            }
        }
        self.frames += (samples.len() / channels) as u64;
        if self.frames < u64::from(rate.div_ceil(20)) {
            return;
        }
        let before = publication.sequence.load(Ordering::SeqCst);
        let Some(next) = before.checked_add(2) else {
            self.reset();
            return;
        };
        publication.sequence.store(before + 1, Ordering::SeqCst);
        publication
            .channels
            .store(channels as u32, Ordering::SeqCst);
        publication.frames.store(self.frames, Ordering::SeqCst);
        publication
            .now
            .store(timing.monotonic_ns as u64, Ordering::SeqCst);
        publication
            .format_generation
            .store(timing.format_generation, Ordering::SeqCst);
        for index in 0..channels {
            publication.peak[index].store(self.peak[index].to_bits(), Ordering::SeqCst);
            publication.rms[index].store(
                ((self.square[index] / self.frames as f64).sqrt() as f32).to_bits(),
                Ordering::SeqCst,
            );
        }
        publication.sequence.store(next, Ordering::SeqCst);
        self.reset();
    }
}
