use super::{AudioConfig, AudioDirection, AudioShared, SampleFormat};
use std::sync::{Arc, atomic::Ordering};
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioTiming {
    pub monotonic_ns: i64,
    pub graph_ticks: u64,
    pub rate_num: u32,
    pub rate_denom: u32,
    pub delay_ticks: i64,
    pub frame_position: u64,
    /// Native format generation; a new generation always begins with a discontinuity.
    pub format_generation: u64,
    /// The graph clock jumped, the stream was flushed, or its format was republished.
    pub discontinuity: bool,
}
impl AudioTiming {
    /// Graph-clock observation paired with PipeWire's monotonic clock snapshot.
    /// The owner supplies its incarnation and discontinuity state; device delay is not a
    /// clock offset and remains available separately in `delay_ticks`.
    pub fn clock_observation(
        self,
        epoch: u64,
        discontinuity: bool,
    ) -> Option<crate::media::timing::ClockObservation> {
        if self.rate_num == 0 || self.rate_denom == 0 {
            return None;
        }
        let source = i128::from(self.graph_ticks)
            .checked_mul(i128::from(self.rate_num))?
            .checked_mul(1_000_000_000)?
            / i128::from(self.rate_denom);
        Some(crate::media::timing::ClockObservation {
            epoch,
            source_ns: i64::try_from(source).ok()?,
            reference_ns: self.monotonic_ns,
            discontinuity: discontinuity || self.discontinuity,
        })
    }
}
pub struct AudioCycle<'a> {
    pub samples: &'a mut [f32],
    pub channels: u32,
    pub rate: u32,
    pub direction: AudioDirection,
    pub timing: AudioTiming,
}
/// Runs on PipeWire's realtime data thread. Never allocate, block, lock, log, perform IO,
/// call UI/control methods, or retain cycle references. Playback starts as silence; capture
/// contains input. State and buffers must be prepared before opening the stream.
pub trait AudioCallback: Send + 'static {
    fn process(&mut self, cycle: AudioCycle<'_>);
}
impl<F> AudioCallback for F
where
    F: for<'a> FnMut(AudioCycle<'a>) + Send + 'static,
{
    fn process(&mut self, cycle: AudioCycle<'_>) {
        self(cycle)
    }
}
pub(crate) enum Endpoint {
    Playback(rtrb::Consumer<f32>),
    Capture(rtrb::Producer<f32>),
    CaptureTimed {
        samples: rtrb::Producer<f32>,
        blocks: rtrb::Producer<super::capture_timing::CaptureBlock>,
    },
    Callback(Box<dyn AudioCallback>),
}
pub(crate) struct RealtimeData {
    config: AudioConfig,
    shared: Arc<AudioShared>,
    endpoint: Endpoint,
    scratch: Vec<f32>,
    last_ticks: Option<u64>,
    last_format_epoch: u64,
    last_quantum: u64,
    last_clock_rate: Option<(u32, u32)>,
    last_capture_errors: (u64, u64),
    meter: super::meter::MeterAccumulator,
}
impl RealtimeData {
    pub(crate) fn new(config: AudioConfig, shared: Arc<AudioShared>, endpoint: Endpoint) -> Self {
        let scratch = vec![0.0; config.max_quantum * config.format.channels as usize];
        Self {
            config,
            shared,
            endpoint,
            scratch,
            last_ticks: None,
            last_format_epoch: 0,
            last_quantum: 0,
            last_clock_rate: None,
            last_capture_errors: (0, 0),
            meter: Default::default(),
        }
    }
    pub(crate) fn process(&mut self, stream: &pipewire::stream::Stream) {
        let Some(mut buffer) = stream.dequeue_buffer() else {
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let requested = buffer.requested() as usize;
        let timing = stream.time().ok();
        let data = buffer.datas_mut();
        let Some(data) = data.first_mut() else {
            return;
        };
        let format_epoch = self.shared.format_epoch.load(Ordering::Acquire);
        if format_epoch & 1 == 0 || self.shared.stop.load(Ordering::Acquire) {
            *data.chunk_mut().size_mut() = 0;
            return;
        }
        let channels = self.config.format.channels as usize;
        let sample_bytes = self.config.format.sample_format.bytes();
        let stride = sample_bytes * channels;
        let offset = data.chunk().offset() as usize;
        let size = data.chunk().size() as usize;
        let Some(bytes) = data.data() else {
            self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let (start, frames) = match self.config.direction {
            AudioDirection::Playback => (
                0,
                (bytes.len() / stride)
                    .min(self.config.max_quantum)
                    .min(if requested == 0 {
                        self.config.max_quantum
                    } else {
                        requested
                    }),
            ),
            AudioDirection::Capture => {
                if offset > bytes.len() || size > bytes.len() - offset || size % stride != 0 {
                    self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                if size / stride > self.config.max_quantum {
                    self.shared.overruns.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                (offset, size / stride)
            }
        };
        let drain_epoch = self.shared.drain_epoch.load(Ordering::Acquire);
        let draining = self.shared.drain.load(Ordering::Acquire);
        let mut queued_empty = false;
        let mut delivered_frames = frames;
        let count = frames * channels;
        let samples = &mut self.scratch[..count];
        samples.fill(0.0);
        let flush = self.shared.flush.load(Ordering::Acquire);
        let flushing = flush != self.shared.flushed.load(Ordering::Relaxed);
        if flushing {
            if let Endpoint::Playback(consumer) = &mut self.endpoint {
                let n = consumer.slots();
                if let Ok(chunk) = consumer.read_chunk(n) {
                    chunk.commit_all();
                }
            }
            self.shared.flushed.store(flush, Ordering::Release);
            self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
        }
        if self.config.direction == AudioDirection::Capture {
            for (sample, bytes) in samples
                .iter_mut()
                .zip(bytes[start..start + count * sample_bytes].chunks_exact(sample_bytes))
            {
                *sample = match self.config.format.sample_format {
                    SampleFormat::F32 => f32::from_le_bytes(bytes.try_into().unwrap()),
                    SampleFormat::S16 => {
                        i16::from_le_bytes(bytes.try_into().unwrap()) as f32 / 32768.0
                    }
                };
                if !sample.is_finite() {
                    *sample = 0.0;
                }
            }
        }
        let format_changed = self.last_format_epoch != format_epoch;
        if format_changed {
            self.last_ticks = None;
            self.last_quantum = 0;
            self.last_clock_rate = None;
            self.last_format_epoch = format_epoch;
        }
        let mut clock = AudioTiming {
            frame_position: self.shared.frames.load(Ordering::Relaxed),
            format_generation: format_epoch >> 1,
            discontinuity: flushing || format_changed,
            ..Default::default()
        };
        if let Some(t) = timing {
            clock.monotonic_ns = t.now();
            clock.graph_ticks = t.ticks();
            clock.rate_num = t.rate().num;
            clock.rate_denom = t.rate().denom;
            clock.delay_ticks = t.delay();
            let clock_rate = (t.rate().num, t.rate().denom);
            // Graph ticks need not use the application's sample rate. Round the previous
            // application's quantum to graph ticks, allowing one tick of rounding.
            if self
                .last_clock_rate
                .is_some_and(|previous| previous != clock_rate)
                || self.last_ticks.is_some_and(|last| {
                    t.ticks() < last
                        || t.ticks().abs_diff(last.saturating_add(self.last_quantum)) > 1
                })
            {
                clock.discontinuity = true;
                self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
            }
            self.last_ticks = Some(t.ticks());
            self.last_clock_rate = Some(clock_rate);
            let divisor = u128::from(self.config.format.rate) * u128::from(t.rate().num);
            self.last_quantum = if divisor != 0 {
                ((frames as u128 * u128::from(t.rate().denom) + divisor / 2) / divisor)
                    .min(u128::from(u64::MAX)) as u64
            } else {
                0
            };
            self.shared.ticks.store(t.ticks(), Ordering::Relaxed);
            self.shared.now.store(t.now(), Ordering::Relaxed);
            let delay = (t.delay() as i128 * t.rate().num as i128 * self.config.format.rate as i128
                / t.rate().denom.max(1) as i128)
                .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
            self.shared.delay.store(delay, Ordering::Relaxed);
        }
        match &mut self.endpoint {
            Endpoint::Playback(consumer) => {
                let available = count.min(consumer.slots()) / channels * channels;
                if let Ok(chunk) = consumer.read_chunk(available) {
                    let (a, b) = chunk.as_slices();
                    samples[..a.len()].copy_from_slice(a);
                    samples[a.len()..available].copy_from_slice(b);
                    chunk.commit_all();
                }
                if available < count && !draining {
                    self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                }
                queued_empty = consumer.slots() == 0;
                if draining {
                    delivered_frames = available / channels;
                }
            }
            Endpoint::Callback(callback) => {
                if !self.shared.processor_failed.load(Ordering::Relaxed) {
                    // Normal execution is allocation-free. A panic is a terminal processor
                    // failure; contain it before C, silence output, and retire on control thread.
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        callback.process(AudioCycle {
                            samples,
                            channels: channels as u32,
                            rate: self.config.format.rate,
                            direction: self.config.direction,
                            timing: clock,
                        })
                    }))
                    .is_err()
                    {
                        self.shared.processor_failed.store(true, Ordering::Release);
                        samples.fill(0.0);
                    }
                }
            }
            Endpoint::Capture(_) | Endpoint::CaptureTimed { .. } => {}
        }
        let gain = if self.shared.mute.load(Ordering::Acquire) {
            0.0
        } else {
            f32::from_bits(self.shared.gain.load(Ordering::Acquire))
        };
        for (i, sample) in samples.iter_mut().enumerate() {
            *sample *=
                gain * f32::from_bits(self.shared.channels[i % channels].load(Ordering::Acquire));
            if !sample.is_finite() {
                *sample = 0.0;
            }
        }
        self.meter.process(
            &self.shared.meter,
            &samples[..delivered_frames * channels],
            channels,
            self.config.format.rate,
            clock,
        );
        if let Endpoint::Capture(producer) = &mut self.endpoint {
            if let Ok(mut chunk) = producer.write_chunk(count) {
                let (a, b) = chunk.as_mut_slices();
                let split = a.len();
                a.copy_from_slice(&samples[..split]);
                b.copy_from_slice(&samples[split..]);
                chunk.commit_all();
            } else {
                self.shared.overruns.fetch_add(1, Ordering::Relaxed);
            }
        }
        if let Endpoint::CaptureTimed {
            samples: producer,
            blocks,
        } = &mut self.endpoint
        {
            if count > 0 {
                if blocks.slots() == 0 || producer.slots() < count {
                    self.shared.overruns.fetch_add(1, Ordering::Relaxed);
                } else if let Ok(mut chunk) = producer.write_chunk(count) {
                    let (a, b) = chunk.as_mut_slices();
                    let split = a.len();
                    a.copy_from_slice(&samples[..split]);
                    b.copy_from_slice(&samples[split..]);
                    chunk.commit_all();
                    let errors = (
                        self.shared.discontinuities.load(Ordering::Relaxed),
                        self.shared.overruns.load(Ordering::Relaxed),
                    );
                    let block = super::capture_timing::CaptureBlock {
                        timing: clock,
                        frames,
                        discontinuity: clock.discontinuity || errors != self.last_capture_errors,
                    };
                    // Only this producer writes descriptors, and capacity was reserved above.
                    let _ = blocks.push(block);
                    self.last_capture_errors = errors;
                }
            }
        }
        if self.config.direction == AudioDirection::Playback {
            for (sample, bytes) in samples
                .iter()
                .zip(bytes[..count * sample_bytes].chunks_exact_mut(sample_bytes))
            {
                match self.config.format.sample_format {
                    SampleFormat::F32 => bytes.copy_from_slice(&sample.to_le_bytes()),
                    SampleFormat::S16 => bytes.copy_from_slice(
                        &((*sample * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
                            .to_le_bytes(),
                    ),
                }
            }
            *data.chunk_mut().offset_mut() = 0;
            *data.chunk_mut().stride_mut() = stride as i32;
            *data.chunk_mut().size_mut() = (delivered_frames * stride) as u32;
        }
        drop(buffer);
        if draining && queued_empty {
            self.shared
                .drained_epoch
                .store(drain_epoch, Ordering::Release);
        }
        self.shared
            .frames
            .fetch_add(delivered_frames as u64, Ordering::Relaxed);
        self.shared.quantum.store(frames as u32, Ordering::Relaxed);
        if clock.rate_num != 0 && clock.rate_denom != 0 {
            self.shared.clock.publish(super::AudioClockSnapshot {
                timing: clock,
                sample_rate: self.config.format.rate,
                quantum_frames: frames as u32,
                discontinuities: self.shared.discontinuities.load(Ordering::Relaxed),
            });
        }
    }
}
