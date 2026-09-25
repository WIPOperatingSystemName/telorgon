//! Allocation-free clock mapping shared by audio, MIDI and video owners.
//! Observations must pair the same instant in two clock domains, not packet arrival times.
//! Keep one mapper per source incarnation; use a new epoch after reconnect/renegotiation.

#[derive(Clone, Copy, Debug)]
pub struct ClockObservation {
    pub epoch: u64,
    pub source_ns: i64,
    pub reference_ns: i64,
    pub discontinuity: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockUpdate {
    Initialized,
    Tracking,
    Discontinuity,
    Unchanged,
}

#[derive(Clone, Copy, Debug)]
struct Anchor {
    epoch: u64,
    source: i64,
    reference: i64,
}

/// Maps source timestamps to a shared monotonic reference. This value owns no thread,
/// resource or queue. Mutate it on one processing thread; all operations are bounded.
/// Drift is limited to ±1000 ppm; larger changes re-anchor and report discontinuity.
#[derive(Clone, Debug, Default)]
pub struct ClockMapper {
    anchor: Option<Anchor>,
    last: Option<ClockObservation>,
    rate_anchor: Option<ClockObservation>,
    slope: f64,
    phase_ns: f64,
    discontinuities: u64,
}
impl ClockMapper {
    pub fn new() -> Self {
        Self {
            slope: 1.0,
            ..Self::default()
        }
    }
    pub fn reset(&mut self) {
        *self = Self::new();
    }
    pub fn discontinuities(&self) -> u64 {
        self.discontinuities
    }
    fn anchor(&mut self, sample: ClockObservation) {
        self.anchor = Some(Anchor {
            epoch: sample.epoch,
            source: sample.source_ns,
            reference: sample.reference_ns,
        });
        self.last = Some(sample);
        self.rate_anchor = Some(sample);
        self.slope = 1.0;
        self.phase_ns = 0.0;
    }
    pub fn observe(&mut self, sample: ClockObservation) -> ClockUpdate {
        let Some(last) = self.last else {
            self.anchor(sample);
            if sample.discontinuity {
                self.discontinuities = self.discontinuities.saturating_add(1);
                return ClockUpdate::Discontinuity;
            }
            return ClockUpdate::Initialized;
        };
        let mut reset = sample.discontinuity
            || sample.epoch != last.epoch
            || sample.source_ns < last.source_ns
            || sample.reference_ns < last.reference_ns;
        if !reset && sample.source_ns == last.source_ns {
            return ClockUpdate::Unchanged;
        }
        if !reset {
            let prediction = self.map(sample.epoch, sample.source_ns);
            reset = prediction.is_none_or(|predicted| {
                (i128::from(sample.reference_ns) - i128::from(predicted)).abs() > 50_000_000
            });
        }
        if reset {
            self.discontinuities = self.discontinuities.saturating_add(1);
            self.anchor(sample);
            return ClockUpdate::Discontinuity;
        }
        let rate_anchor = self.rate_anchor.unwrap();
        let source_delta = i128::from(sample.source_ns) - i128::from(rate_anchor.source_ns);
        if source_delta >= 100_000_000 {
            let reference_delta =
                i128::from(sample.reference_ns) - i128::from(rate_anchor.reference_ns);
            let measured = reference_delta as f64 / source_delta as f64;
            if !(0.999..=1.001).contains(&measured) {
                self.discontinuities = self.discontinuities.saturating_add(1);
                self.anchor(sample);
                return ClockUpdate::Discontinuity;
            }
            // Re-anchor at the old prediction before changing slope, avoiding a phase jump
            // proportional to the entire elapsed session duration.
            let predicted = self.map(sample.epoch, sample.source_ns).unwrap();
            self.anchor = Some(Anchor {
                epoch: sample.epoch,
                source: sample.source_ns,
                reference: predicted,
            });
            self.phase_ns = 0.0;
            self.slope += (measured - self.slope) * 0.125;
            self.rate_anchor = Some(sample);
        }
        let predicted = self.map(sample.epoch, sample.source_ns).unwrap();
        self.phase_ns += (i128::from(sample.reference_ns) - i128::from(predicted)) as f64 * 0.125;
        self.last = Some(sample);
        ClockUpdate::Tracking
    }
    /// None until initialized, for a stale epoch, or if timestamp arithmetic overflows.
    pub fn map(&self, epoch: u64, source_ns: i64) -> Option<i64> {
        let anchor = self.anchor?;
        if anchor.epoch != epoch {
            return None;
        }
        let delta = (i128::from(source_ns) - i128::from(anchor.source)) as f64;
        let mapped = (delta * self.slope + self.phase_ns).round();
        if !mapped.is_finite() || mapped.abs() > i64::MAX as f64 {
            return None;
        }
        i64::try_from(i128::from(anchor.reference) + mapped as i128).ok()
    }
    /// Input-consumption correction for an audio resampler: positive consumes more source
    /// frames per output frame. Apply only when this source is audio and reference is its
    /// output clock. Reset queued media/resampler history after Discontinuity.
    pub fn resample_correction_ppm(&self) -> Option<f64> {
        self.anchor?;
        Some(((1.0 / self.slope - 1.0) * 1_000_000.0).clamp(-1000.0, 1000.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoDeadline {
    Wait(u64),
    Present,
    Drop,
}
/// Compare timestamps already mapped to the same clock. Queueing/retirement remain with
/// the stream owner; this decision neither sleeps nor retains a frame.
pub fn video_deadline(
    presentation_ns: i64,
    now_ns: i64,
    maximum_lateness_ns: u64,
) -> VideoDeadline {
    let delta = i128::from(presentation_ns) - i128::from(now_ns);
    if delta > 0 {
        VideoDeadline::Wait(delta.min(u64::MAX as i128) as u64)
    } else if -delta > i128::from(maximum_lateness_ns) {
        VideoDeadline::Drop
    } else {
        VideoDeadline::Present
    }
}
