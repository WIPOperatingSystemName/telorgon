//! Reusable allocation-free processing after construction. These utilities process
//! interleaved f32 in caller-owned slices; they do not own threads or native resources.
//! Plugin hosting, general encoding and a broad effects library are separate extensions.
use crate::integrations::pipewire::MediaError;
/// Custom DSP contract. Prepare storage before processing; reset handles discontinuities.
/// Implementations must not allocate/block in process and must honor their latency report.
pub trait Processor: Send + 'static {
    fn latency_frames(&self) -> u32;
    fn reset(&mut self);
    fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        channels: usize,
    ) -> Result<(), MediaError>;
}
/// Weighted channel matrix, output-major. Mixing occurs in float with no implicit clipping.
pub struct ChannelMixer {
    inputs: usize,
    outputs: usize,
    weights: Vec<f32>,
}
impl ChannelMixer {
    pub fn new(inputs: usize, outputs: usize, weights: Vec<f32>) -> Result<Self, MediaError> {
        if !(1..=64).contains(&inputs)
            || !(1..=64).contains(&outputs)
            || weights.len() != inputs * outputs
            || weights.iter().any(|x| !x.is_finite())
        {
            return Err(MediaError::InvalidArgument("channel matrix"));
        }
        Ok(Self {
            inputs,
            outputs,
            weights,
        })
    }
    pub fn process(&self, input: &[f32], output: &mut [f32]) -> Result<usize, MediaError> {
        if input.len() % self.inputs != 0 || output.len() < input.len() / self.inputs * self.outputs
        {
            return Err(MediaError::InvalidArgument("channel matrix buffers"));
        }
        let frames = input.len() / self.inputs;
        for (source, dest) in input
            .chunks_exact(self.inputs)
            .zip(output.chunks_exact_mut(self.outputs))
        {
            for (channel, sample) in dest.iter_mut().enumerate() {
                *sample = source
                    .iter()
                    .zip(&self.weights[channel * self.inputs..(channel + 1) * self.inputs])
                    .map(|(s, w)| s * w)
                    .sum();
            }
        }
        Ok(frames)
    }
}
/// Adds one source to a caller-initialized mix buffer. Caller controls headroom/clipping.
pub fn mix_add(source: &[f32], destination: &mut [f32], gain: f32) -> Result<(), MediaError> {
    if source.len() != destination.len() || !gain.is_finite() {
        return Err(MediaError::InvalidArgument("mix buffers or gain"));
    }
    for (input, output) in source.iter().zip(destination) {
        *output += input * gain;
    }
    Ok(())
}
#[derive(Clone, Copy, Debug)]
pub struct GainUpdate {
    pub frame: u64,
    pub gain: f32,
}
/// Control-thread producer. Updates are FIFO and must be submitted in nondecreasing frame
/// order; full queues reject the new update. Dropping this sender does not stop processing.
pub struct GainAutomation {
    producer: rtrb::Producer<GainUpdate>,
    last: Option<u64>,
}
pub struct TimedGain {
    consumer: rtrb::Consumer<GainUpdate>,
    next: Option<GainUpdate>,
    gain: f32,
    frame: u64,
}
impl TimedGain {
    pub fn new(initial: f32, capacity: usize) -> Result<(GainAutomation, Self), MediaError> {
        if !initial.is_finite()
            || !(0.0..=16.0).contains(&initial)
            || !(1..=4096).contains(&capacity)
        {
            return Err(MediaError::InvalidArgument("gain automation"));
        }
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);
        Ok((
            GainAutomation {
                producer,
                last: None,
            },
            Self {
                consumer,
                next: None,
                gain: initial,
                frame: 0,
            },
        ))
    }
    /// Updates arriving after their frame take effect at the first subsequently processed
    /// frame. reset_clock clears queued automation so old-timeline events cannot leak.
    pub fn process(&mut self, samples: &mut [f32], channels: usize) -> Result<(), MediaError> {
        if channels == 0 || samples.len() % channels != 0 {
            return Err(MediaError::InvalidArgument("automation channels"));
        }
        for frame in samples.chunks_exact_mut(channels) {
            loop {
                if self.next.is_none() {
                    self.next = self.consumer.pop().ok();
                }
                if let Some(update) = self.next.filter(|u| u.frame <= self.frame) {
                    self.gain = update.gain;
                    self.next = None;
                } else {
                    break;
                }
            }
            for sample in frame {
                *sample *= self.gain;
            }
            self.frame = self.frame.saturating_add(1);
        }
        Ok(())
    }
    pub fn reset_clock(&mut self, frame: u64) {
        self.frame = frame;
        self.next = None;
        while self.consumer.pop().is_ok() {}
    }
}
impl GainAutomation {
    pub fn schedule(&mut self, update: GainUpdate) -> Result<(), MediaError> {
        if !update.gain.is_finite()
            || !(0.0..=16.0).contains(&update.gain)
            || self.last.is_some_and(|last| update.frame < last)
        {
            return Err(MediaError::InvalidArgument("ordered gain update"));
        }
        self.producer
            .push(update)
            .map_err(|_| MediaError::QueueFull)?;
        self.last = Some(update.frame);
        Ok(())
    }
}
/// Explicit delay for feedback paths. Construction preallocates; zero-frame delay is
/// rejected so opting into feedback cannot accidentally create an instantaneous loop.
pub struct Delay {
    channels: usize,
    storage: Vec<f32>,
    cursor: usize,
}
impl Delay {
    pub fn new(channels: usize, frames: usize) -> Result<Self, MediaError> {
        if !(1..=64).contains(&channels) || !(1..=384000).contains(&frames) {
            return Err(MediaError::InvalidArgument("delay size"));
        }
        Ok(Self {
            channels,
            storage: vec![0.0; frames * channels],
            cursor: 0,
        })
    }
}
impl Processor for Delay {
    fn latency_frames(&self) -> u32 {
        (self.storage.len() / self.channels) as u32
    }
    fn reset(&mut self) {
        self.storage.fill(0.0);
        self.cursor = 0;
    }
    fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        channels: usize,
    ) -> Result<(), MediaError> {
        if channels != self.channels || input.len() != output.len() || input.len() % channels != 0 {
            return Err(MediaError::InvalidArgument("delay buffers"));
        }
        for (sample, out) in input.iter().zip(output) {
            *out = self.storage[self.cursor];
            self.storage[self.cursor] = *sample;
            self.cursor = (self.cursor + 1) % self.storage.len();
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automation_is_sample_accurate_across_quanta_and_bounded() {
        let (mut control, mut gain) = TimedGain::new(1.0, 2).unwrap();
        control
            .schedule(GainUpdate {
                frame: 2,
                gain: 0.5,
            })
            .unwrap();
        control
            .schedule(GainUpdate {
                frame: 4,
                gain: 0.0,
            })
            .unwrap();
        assert_eq!(
            control.schedule(GainUpdate {
                frame: 6,
                gain: 1.0
            }),
            Err(MediaError::QueueFull)
        );
        let mut a = [1.0; 6];
        gain.process(&mut a, 2).unwrap();
        assert_eq!(a, [1.0, 1.0, 1.0, 1.0, 0.5, 0.5]);
        let mut b = [1.0; 4];
        gain.process(&mut b, 2).unwrap();
        assert_eq!(b, [0.5, 0.5, 0.0, 0.0]);
    }
    #[test]
    fn mapping_mixing_and_delay_preserve_channel_order_and_latency() {
        let mixer = ChannelMixer::new(2, 1, vec![0.5, 0.5]).unwrap();
        let mut mono = [0.0; 3];
        mixer
            .process(&[1.0, 0.0, 0.0, 1.0, 1.0, 1.0], &mut mono)
            .unwrap();
        assert_eq!(mono, [0.5, 0.5, 1.0]);
        mix_add(&[0.25; 3], &mut mono, 2.0).unwrap();
        assert_eq!(mono, [1.0, 1.0, 1.5]);
        let mut delay = Delay::new(1, 2).unwrap();
        let mut out = [0.0; 3];
        delay.process(&mono, &mut out, 1).unwrap();
        assert_eq!(out, [0.0, 0.0, 1.0]);
        assert_eq!(delay.latency_frames(), 2);
        delay.reset();
        delay.process(&[0.0; 3], &mut out, 1).unwrap();
        assert_eq!(out, [0.0; 3]);
    }
}
