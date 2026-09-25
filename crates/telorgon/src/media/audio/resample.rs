//! Streaming windowed-sinc resampling with precomputed phases and bounded storage.
use crate::integrations::pipewire::MediaError;
const TAPS: usize = 32;
const PHASES: usize = 1024;
pub struct SincResampler {
    channels: usize,
    input_rate: u32,
    output_rate: u32,
    drift: super::drift::DriftTracker,
    ratio: f64,
    nominal_ratio: f64,
    buffer: Vec<f32>,
    capacity: usize,
    head: usize,
    len: usize,
    position: f64,
    kernel: Vec<f32>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resampled {
    pub consumed_frames: usize,
    pub produced_frames: usize,
}
impl SincResampler {
    /// 16 input frames of lookahead; rates and channel counts may be changed only by
    /// constructing a replacement off the realtime thread. No allocations during process.
    pub fn new(
        input_rate: u32,
        output_rate: u32,
        channels: usize,
        capacity_frames: usize,
    ) -> Result<Self, MediaError> {
        if !(8000..=384000).contains(&input_rate)
            || !(8000..=384000).contains(&output_rate)
            || !(1..=64).contains(&channels)
            || !(64..=65536).contains(&capacity_frames)
        {
            return Err(MediaError::InvalidArgument("resampler configuration"));
        }
        let ratio = input_rate as f64 / output_rate as f64;
        let cutoff = (1.0 / (ratio * 1.001)).min(1.0) * 0.95;
        let mut kernel = vec![0.0; TAPS * PHASES];
        for phase in 0..PHASES {
            let frac = phase as f64 / PHASES as f64;
            let mut sum = 0.0;
            for tap in 0..TAPS {
                let x = tap as f64 - 15.0 - frac;
                let sinc = if x.abs() < 1e-10 {
                    cutoff
                } else {
                    (std::f64::consts::PI * x * cutoff).sin() / (std::f64::consts::PI * x)
                };
                let window = 0.5 + 0.5 * (std::f64::consts::PI * x / 16.0).cos();
                let weight = sinc * window;
                kernel[phase * TAPS + tap] = weight as f32;
                sum += weight;
            }
            for weight in &mut kernel[phase * TAPS..(phase + 1) * TAPS] {
                *weight /= sum as f32;
            }
        }
        Ok(Self {
            channels,
            input_rate,
            output_rate,
            drift: Default::default(),
            ratio,
            nominal_ratio: ratio,
            buffer: vec![0.0; capacity_frames * channels],
            capacity: capacity_frames,
            head: 0,
            len: 16,
            position: 16.0,
            kernel,
        })
    }
    /// Bounded clock-drift correction; allocation-free and safe on the processing thread.
    /// Positive ppm consumes input faster. Invalid values leave the current ratio unchanged.
    pub fn set_drift_ppm(&mut self, ppm: f64) -> Result<(), MediaError> {
        if !ppm.is_finite() || ppm.abs() > 1000.0 {
            return Err(MediaError::InvalidArgument(
                "resampler drift exceeds 1000 ppm",
            ));
        }
        self.ratio = self.nominal_ratio * (1.0 + ppm / 1_000_000.0);
        Ok(())
    }
    /// Track source/destination clock rates against the same host monotonic clock.
    /// Snapshots need not describe the same instant. Do not use packet arrival times or
    /// combine unrelated hosts' monotonic clocks. This never changes the nominal rates.
    /// Reset before replacing either stream: format generations are local to each owner.
    /// Call before processing each new observation; repeats are harmless. A discontinuity
    /// clears FIR history; also discard any caller-owned queued media. Errors reset all
    /// tracking/history and require fresh clocks before continuing synchronized playback.
    /// The method allocates nothing and locks nothing; obtain snapshots outside the RT
    /// callback, or construct them directly from the callback's timing on its owner thread.
    pub fn synchronize_clocks(
        &mut self,
        input: super::AudioClockSnapshot,
        output: super::AudioClockSnapshot,
        monotonic_now_ns: i64,
    ) -> Result<super::DriftAdjustment, MediaError> {
        if input.sample_rate != self.input_rate || output.sample_rate != self.output_rate {
            self.reset();
            return Err(MediaError::InvalidArgument("resampler clock sample rates"));
        }
        let adjustment = match self.drift.update(input, output, monotonic_now_ns) {
            Ok(adjustment) => adjustment,
            Err(error) => {
                self.reset();
                return Err(error);
            }
        };
        if adjustment.discontinuity {
            self.reset_history();
        }
        self.set_drift_ppm(adjustment.correction_ppm)?;
        Ok(adjustment)
    }
    pub fn latency_input_frames(&self) -> usize {
        16
    }
    pub fn reset(&mut self) {
        self.drift = Default::default();
        self.reset_history();
    }
    fn reset_history(&mut self) {
        self.ratio = self.nominal_ratio;
        self.buffer.fill(0.0);
        self.head = 0;
        self.len = 16;
        self.position = 16.0;
    }
    /// Partial consumption is explicit: retry unconsumed input with more output storage.
    /// To drain the finite FIR tail at EOF, feed at least 32 zero input frames.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<Resampled, MediaError> {
        if input.len() % self.channels != 0 || output.len() % self.channels != 0 {
            return Err(MediaError::InvalidArgument("resampler frame alignment"));
        }
        let consumed = (input.len() / self.channels).min(self.capacity - self.len);
        for frame in 0..consumed {
            let destination = (self.head + self.len + frame) % self.capacity;
            self.buffer[destination * self.channels..(destination + 1) * self.channels]
                .copy_from_slice(&input[frame * self.channels..(frame + 1) * self.channels]);
        }
        self.len += consumed;
        let mut produced = 0;
        while produced < output.len() / self.channels
            && self.position.floor() as usize + 16 < self.len
        {
            let integer = self.position.floor() as usize;
            let phase = ((self.position.fract() * PHASES as f64) as usize).min(PHASES - 1);
            for channel in 0..self.channels {
                let mut sample = 0.0;
                for tap in 0..TAPS {
                    let frame = (self.head + integer - 15 + tap) % self.capacity;
                    sample += self.buffer[frame * self.channels + channel]
                        * self.kernel[phase * TAPS + tap];
                }
                output[produced * self.channels + channel] = sample;
            }
            produced += 1;
            self.position += self.ratio;
        }
        let discard = (self.position.floor() as usize)
            .saturating_sub(15)
            .min(self.len.saturating_sub(16));
        self.head = (self.head + discard) % self.capacity;
        self.len -= discard;
        self.position -= discard as f64;
        Ok(Resampled {
            consumed_frames: consumed,
            produced_frames: produced,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dc_gain_and_stereo_isolation_survive_rate_conversion_and_reset() {
        let mut resampler = SincResampler::new(48000, 44100, 2, 1024).unwrap();
        let input: Vec<f32> = (0..512).flat_map(|_| [0.5, 0.0]).collect();
        let mut output = vec![0.0; 1024];
        let result = resampler.process(&input, &mut output).unwrap();
        assert_eq!(result.consumed_frames, 512);
        assert!(result.produced_frames > 440 && result.produced_frames < 480);
        for frame in output[64..result.produced_frames * 2].chunks_exact(2) {
            assert!((frame[0] - 0.5).abs() < 0.0001);
            assert_eq!(frame[1], 0.0);
        }
        resampler.reset();
        let result = resampler.process(&vec![0.0; 1024], &mut output).unwrap();
        assert!(
            output[..result.produced_frames * 2]
                .iter()
                .all(|v| *v == 0.0)
        );
    }
}
