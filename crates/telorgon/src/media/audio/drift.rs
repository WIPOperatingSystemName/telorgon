//! Relative drift between two audio clocks sharing the same monotonic reference.
use super::AudioClockSnapshot;
use crate::{
    integrations::pipewire::MediaError,
    media::timing::{ClockMapper, ClockObservation, ClockUpdate},
};

#[derive(Clone, Copy, Debug)]
pub struct DriftAdjustment {
    /// Positive consumes more source frames per output frame; bounded to ±1000 ppm.
    pub correction_ppm: f64,
    /// Resampler history was discarded. The caller must also discard its queued media.
    pub discontinuity: bool,
}
#[derive(Default)]
struct Domain {
    mapper: ClockMapper,
    last: Option<(u64, u64, i64, u64)>,
}
impl Domain {
    fn observe(&mut self, snapshot: AudioClockSnapshot) -> Result<bool, MediaError> {
        let timing = snapshot.timing;
        let key = (
            timing.format_generation,
            timing.graph_ticks,
            timing.monotonic_ns,
            snapshot.discontinuities,
        );
        // Snapshots can be polled repeatedly; a repeated discontinuity flag describes
        // one past processing cycle, not a fresh event on every caller poll.
        if self.last == Some(key) {
            return Ok(false);
        }
        let changed = self
            .last
            .is_some_and(|last| last.0 != key.0 || last.3 != key.3);
        let observation: ClockObservation = timing
            .clock_observation(timing.format_generation, changed)
            .ok_or(MediaError::InvalidArgument("audio clock rate"))?;
        let update = self.mapper.observe(observation);
        self.last = Some(key);
        Ok(matches!(
            update,
            ClockUpdate::Initialized | ClockUpdate::Discontinuity
        ))
    }
}
#[derive(Default)]
pub(crate) struct DriftTracker {
    input: Domain,
    output: Domain,
}
impl DriftTracker {
    pub fn update(
        &mut self,
        input: AudioClockSnapshot,
        output: AudioClockSnapshot,
        now_ns: i64,
    ) -> Result<DriftAdjustment, MediaError> {
        for snapshot in [input, output] {
            // Tolerate three long quanta, while bounding how long obsolete clocks survive.
            let age = (u64::from(snapshot.quantum_frames) * 3_000_000_000
                / u64::from(snapshot.sample_rate.max(1)))
            .clamp(250_000_000, 10_000_000_000);
            if snapshot.sample_rate == 0 || !snapshot.is_recent(now_ns, age) {
                return Err(MediaError::NotReady);
            }
        }
        let input_reset = self.input.observe(input)?;
        let output_reset = self.output.observe(output)?;
        let input_ppm = self
            .input
            .mapper
            .resample_correction_ppm()
            .ok_or(MediaError::NotReady)?;
        let output_ppm = self
            .output
            .mapper
            .resample_correction_ppm()
            .ok_or(MediaError::NotReady)?;
        // Both mappers map graph time to monotonic time, so divide their clock speeds
        // to obtain source samples consumed per destination sample.
        let correction = ((1.0 + input_ppm / 1_000_000.0) / (1.0 + output_ppm / 1_000_000.0) - 1.0)
            * 1_000_000.0;
        if !correction.is_finite() || correction.abs() > 1000.0 {
            return Err(MediaError::Unsupported(
                "relative audio clock drift exceeds 1000 ppm",
            ));
        }
        Ok(DriftAdjustment {
            correction_ppm: correction,
            discontinuity: input_reset || output_reset,
        })
    }
}
