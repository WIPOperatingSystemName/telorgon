use super::{AudioCallback, Endpoint, RealtimeData};
use crate::integrations::pipewire::{self as transport, *};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    F32,
    S16,
}
impl SampleFormat {
    pub(crate) fn bytes(self) -> usize {
        match self {
            Self::F32 => 4,
            Self::S16 => 2,
        }
    }
}
#[derive(serde::Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_format: SampleFormat,
    pub rate: u32,
    pub channels: u32,
}
impl AudioFormat {
    pub fn validate(self) -> Result<(), MediaError> {
        if !(8000..=384000).contains(&self.rate) || !(1..=64).contains(&self.channels) {
            return Err(MediaError::InvalidArgument("audio format"));
        }
        Ok(())
    }
}
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioDirection {
    Playback,
    Capture,
}
/// Target selection is explicit. Application nodes must be selected from a current registry
/// snapshot; no window identity is assumed. Each stream captures one target node.
#[derive(Clone, Copy, Debug)]
pub enum AudioTarget {
    Default,
    Node(ObjectHandle),
    SystemOutput(ObjectHandle),
    Application(ObjectHandle),
}
impl AudioTarget {
    pub(crate) fn handle(self) -> Option<ObjectHandle> {
        match self {
            Self::Default => None,
            Self::Node(h) | Self::SystemOutput(h) | Self::Application(h) => Some(h),
        }
    }
}
#[derive(Clone, Debug)]
pub enum AudioRole {
    Application,
    /// An application produces samples exposed as a virtual source.
    VirtualSource,
    /// An application consumes samples delivered to its virtual sink.
    VirtualSink,
}
#[derive(Clone, Debug)]
pub struct AudioConfig {
    pub name: String,
    pub role: AudioRole,
    pub format: AudioFormat,
    pub direction: AudioDirection,
    pub target: AudioTarget,
    pub buffer_frames: usize,
    pub max_quantum: usize,
    pub start_paused: bool,
}
impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            name: "Telorgon audio".into(),
            role: AudioRole::Application,
            format: AudioFormat {
                sample_format: SampleFormat::F32,
                rate: 48000,
                channels: 2,
            },
            direction: AudioDirection::Playback,
            target: AudioTarget::Default,
            buffer_frames: 8192,
            max_quantum: 8192,
            start_paused: false,
        }
    }
}
impl AudioConfig {
    pub(crate) fn validate(&self) -> Result<(), MediaError> {
        self.format.validate()?;
        if (matches!(self.role,AudioRole::VirtualSource) && self.direction!=AudioDirection::Playback) || (matches!(self.role,AudioRole::VirtualSink) && self.direction!=AudioDirection::Capture) || (!matches!(self.role,AudioRole::Application) && !matches!(self.target,AudioTarget::Default)) {return Err(MediaError::InvalidArgument("virtual audio role and direction/target"));}
        if self.name.is_empty()
            || self.name.len() > 256
            || self.name.contains('\0')
            || !(1..=65536).contains(&self.buffer_frames)
            || !(1..=32768).contains(&self.max_quantum)
        {
            return Err(MediaError::InvalidArgument("audio stream configuration"));
        }
        if self.direction == AudioDirection::Playback
            && matches!(
                self.target,
                AudioTarget::SystemOutput(_) | AudioTarget::Application(_)
            )
        {
            return Err(MediaError::InvalidArgument("capture target on playback"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioState {
    Connecting,
    Paused,
    Streaming,
    Draining,
    Stopped,
    Failed(MediaError),
}
#[derive(Clone, Debug)]
pub struct AudioDiagnostics {
    pub frames: u64,
    pub underruns: u64,
    pub overruns: u64,
    pub discontinuities: u64,
    pub graph_ticks: u64,
    pub monotonic_ns: i64,
    pub latency_frames: i64,
    pub quantum: u32,
}
pub(crate) struct AudioShared {
    pub clock: super::timing::ClockPublication,
    pub meter: super::meter::MeterPublication,
    pub state: Mutex<AudioState>,
    pub stop: AtomicBool,
    // Low bit: negotiated/usable. Remaining bits: monotonically increasing format generation.
    pub format_epoch: AtomicU64,
    pub gain: AtomicU32,
    pub mute: AtomicBool,
    pub channels: [AtomicU32; 64],
    pub frames: AtomicU64,
    pub underruns: AtomicU64,
    pub overruns: AtomicU64,
    pub discontinuities: AtomicU64,
    pub ticks: AtomicU64,
    pub now: std::sync::atomic::AtomicI64,
    pub delay: std::sync::atomic::AtomicI64,
    pub quantum: AtomicU32,
    pub flush: AtomicU64,
    pub flushed: AtomicU64,
    pub drain: AtomicBool,
    pub drain_epoch: AtomicU64,
    pub drained_epoch: AtomicU64,
    pub empty: AtomicBool,
    pub processor_failed: AtomicBool,
}
impl AudioShared {
    fn new() -> Self {
        Self {
            state: Mutex::new(AudioState::Connecting),
            clock: Default::default(),
            meter: Default::default(),
            stop: AtomicBool::new(false),
            format_epoch: AtomicU64::new(0),
            gain: AtomicU32::new(1.0_f32.to_bits()),
            mute: AtomicBool::new(false),
            channels: std::array::from_fn(|_| AtomicU32::new(1.0_f32.to_bits())),
            frames: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            overruns: AtomicU64::new(0),
            discontinuities: AtomicU64::new(0),
            ticks: AtomicU64::new(0),
            now: std::sync::atomic::AtomicI64::new(0),
            delay: std::sync::atomic::AtomicI64::new(0),
            quantum: AtomicU32::new(0),
            flush: AtomicU64::new(0),
            flushed: AtomicU64::new(0),
            drain: AtomicBool::new(false),
            drain_epoch: AtomicU64::new(0),
            drained_epoch: AtomicU64::new(0),
            empty: AtomicBool::new(false),
            processor_failed: AtomicBool::new(false),
        }
    }
    pub(crate) fn negotiated(&self, ready: bool) {
        if self.format_epoch.fetch_update(Ordering::AcqRel, Ordering::Acquire, |epoch| {
            (epoch & !1).checked_add(2).map(|next| next | u64::from(ready))
        }).is_err() {
            self.format_epoch.fetch_and(!1, Ordering::Release);
            self.state(AudioState::Failed(MediaError::ResourceLimit("audio format generations")));
            self.stop.store(true, Ordering::Release);
        }
    }
    pub(crate) fn state(&self, state: AudioState) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }
}
/// Exact application-side format negotiation. Device-side resampling/channel conversion
/// belongs to PipeWire's adapter. A changed application format fails this stream; open a
/// replacement explicitly so existing buffers and realtime processors cannot be reinterpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioNegotiation {
    pub requested: AudioFormat,
    /// None before negotiation, while the format is withdrawn, or after stop/disconnect.
    pub negotiated: Option<AudioFormat>,
    /// Changes on every native format event, including withdrawal/republication of the same
    /// format. Compare with AudioTiming::format_generation to identify retained capture blocks.
    pub generation: u64,
}
/// Owner of a buffered or realtime stream. It is movable but buffered read/write require
/// exclusive access. Drop requests stop without blocking; resources retire on the worker.
/// Local gain/mute update atomics for the next quantum; native state is separately observed.
pub struct AudioStream {
    pub(crate) id: u64,
    pub(crate) connection: ConnectionHandle,
    pub(crate) shared: Arc<AudioShared>,
    pub(super) config: AudioConfig,
    playback: Option<rtrb::Producer<f32>>,
    capture: Option<rtrb::Consumer<f32>>,
    timed_capture: Option<super::capture_timing::CaptureReader>,
}
static NEXT_STREAM: AtomicU64 = AtomicU64::new(1);
impl AudioStream {
    pub fn buffered(connection: ConnectionHandle, config: AudioConfig) -> Result<Self, MediaError> {
        Self::create(connection, config, None, false)
    }
    pub fn realtime(
        connection: ConnectionHandle,
        config: AudioConfig,
        callback: impl AudioCallback,
    ) -> Result<Self, MediaError> {
        Self::create(connection, config, Some(Box::new(callback)), false)
    }
    /// Buffered capture preserving each quantum's clock and discontinuity boundary.
    /// `buffer_frames` bounds both samples and descriptors; overflow drops the whole incoming
    /// quantum. Use read_timestamped; ordinary read is deliberately unavailable for this mode.
    pub fn timestamped_capture(connection: ConnectionHandle, config: AudioConfig) -> Result<Self, MediaError> {
        if config.direction != AudioDirection::Capture { return Err(MediaError::InvalidArgument("timestamped capture direction")); }
        Self::create(connection, config, None, true)
    }
    fn create(
        connection: ConnectionHandle,
        config: AudioConfig,
        callback: Option<Box<dyn AudioCallback>>,
        timestamped: bool,
    ) -> Result<Self, MediaError> {
        config.validate()?;
        connection.ensure_ready()?;
        if let Some(target) = config.target.handle() {
            connection.snapshot().resolve(target)?;
        }
        let id = NEXT_STREAM
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| MediaError::ResourceLimit("stream IDs"))?;
        let shared = Arc::new(AudioShared::new());
        let mut timed_capture = None;
        let (playback, capture, endpoint) = if timestamped {
            let (samples, consumer) = rtrb::RingBuffer::new(config.buffer_frames * config.format.channels as usize);
            let (blocks, descriptors) = rtrb::RingBuffer::new(config.buffer_frames);
            timed_capture = Some(super::capture_timing::CaptureReader { samples: consumer, blocks: descriptors, pending: None, offset: 0, next_discontinuity: true });
            (None, None, Endpoint::CaptureTimed { samples, blocks })
        } else if let Some(callback) = callback {
            (None, None, Endpoint::Callback(callback))
        } else {
            let (producer, consumer) =
                rtrb::RingBuffer::new(config.buffer_frames * config.format.channels as usize);
            match config.direction {
                AudioDirection::Playback => (Some(producer), None, Endpoint::Playback(consumer)),
                AudioDirection::Capture => (None, Some(consumer), Endpoint::Capture(producer)),
            }
        };
        let data = RealtimeData::new(config.clone(), shared.clone(), endpoint);
        connection.create_audio(id, config.clone(), shared.clone(), data)?;
        Ok(Self {
            id,
            connection,
            shared,
            config,
            playback,
            capture,
            timed_capture,
        })
    }
    pub fn state(&self) -> AudioState {
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if matches!(state, AudioState::Failed(_)) { return state; }
        match self.connection.state() {
            ConnectionState::Ready => state,
            ConnectionState::Failed(error) => AudioState::Failed(error),
            _ if matches!(state, AudioState::Stopped) => state,
            _ => AudioState::Failed(MediaError::Disconnected),
        }
    }
    pub fn negotiation(&self) -> AudioNegotiation {
        let epoch = self.shared.format_epoch.load(Ordering::Acquire);
        let usable = epoch & 1 != 0 && !self.shared.stop.load(Ordering::Acquire)
            && !matches!(self.state(), AudioState::Stopped | AudioState::Failed(_));
        AudioNegotiation {
            requested: self.config.format,
            negotiated: usable.then_some(self.config.format),
            generation: epoch >> 1,
        }
    }
    /// Requested application format. Use negotiation() to observe whether it is usable.
    pub fn format(&self) -> AudioFormat {
        self.config.format
    }
    /// All-or-nothing complete-frame enqueue. QueueFull preserves caller ownership.
    /// Writes are rejected during drain/flush so their completion has a finite boundary.
    pub fn write(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        if self.shared.stop.load(Ordering::Acquire)
            || matches!(self.state(), AudioState::Stopped | AudioState::Failed(_))
        {
            return Err(MediaError::Disconnected);
        }
        if self.shared.drain.load(Ordering::Acquire)
            || self.shared.flush.load(Ordering::Acquire)
                != self.shared.flushed.load(Ordering::Acquire)
        {
            return Err(MediaError::NotReady);
        }
        if samples.len() % self.config.format.channels as usize != 0
            || samples.iter().any(|v| !v.is_finite())
        {
            return Err(MediaError::InvalidArgument("interleaved finite samples"));
        }
        let producer = self
            .playback
            .as_mut()
            .ok_or(MediaError::Unsupported("buffered playback"))?;
        let mut chunk = producer
            .write_chunk(samples.len())
            .map_err(|_| MediaError::QueueFull)?;
        let (a, b) = chunk.as_mut_slices();
        let split = a.len();
        a.copy_from_slice(&samples[..split]);
        b.copy_from_slice(&samples[split..]);
        chunk.commit_all();
        Ok(())
    }
    /// Returns complete frames copied into caller storage. A slow reader drops incoming
    /// whole frames and increments overruns. No allocation or blocking occurs here.
    pub fn read(&mut self, samples: &mut [f32]) -> Result<usize, MediaError> {
        let consumer = self
            .capture
            .as_mut()
            .ok_or(MediaError::Unsupported("buffered capture"))?;
        let channels = self.config.format.channels as usize;
        let count = samples.len().min(consumer.slots()) / channels * channels;
        let chunk = consumer
            .read_chunk(count)
            .map_err(|_| MediaError::NotReady)?;
        let (a, b) = chunk.as_slices();
        samples[..a.len()].copy_from_slice(a);
        samples[a.len()..count].copy_from_slice(b);
        chunk.commit_all();
        Ok(count / channels)
    }
    /// Read at most one quantum (or a fragment), retaining the remaining samples and exact
    /// offset for the next call. None means no published block; no allocation/wait occurs.
    pub fn read_timestamped(&mut self, samples: &mut [f32]) -> Result<Option<super::CapturedAudio>, MediaError> {
        self.timed_capture.as_mut().ok_or(MediaError::Unsupported("timestamped capture"))?
            .read(samples, self.config.format.channels as usize)
    }
    pub fn pause(&self) -> Result<Request, MediaError> {
        self.control(transport::audio::AudioCommand::Active(false))
    }
    pub fn resume(&self) -> Result<Request, MediaError> {
        self.control(transport::audio::AudioCommand::Active(true))
    }
    /// Playback only. Completes on PipeWire's drained event, including queued application
    /// frames. Paused streams must be resumed first; writes remain closed until resume.
    pub fn drain(&self) -> Result<Request, MediaError> {
        if self.config.direction != AudioDirection::Playback || self.playback.is_none() {
            return Err(MediaError::Unsupported("buffered playback drain"));
        }
        self.control(transport::audio::AudioCommand::Drain)
    }
    /// Discards queued media and PipeWire buffers. Completes when the process callback
    /// acknowledges discard; use resume first if the graph is not processing.
    pub fn flush(&mut self) -> Result<Request, MediaError> {
        let request = self.control(transport::audio::AudioCommand::Flush)?;
        if let Some(reader) = &mut self.timed_capture { reader.discard_queued(self.config.format.channels as usize); }
        if let Some(consumer) = self.capture.as_mut() {
            while consumer.pop().is_ok() {}
        }
        Ok(request)
    }
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
    }
    fn control(&self, command: transport::audio::AudioCommand) -> Result<Request, MediaError> {
        match self.state() {
            AudioState::Failed(error) => return Err(error),
            AudioState::Stopped => return Err(MediaError::NotReady),
            _ => {}
        }
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(MediaError::NotReady);
        }
        self.connection.audio_command(self.id, command)
    }
    pub fn set_gain(&self, linear: f32) -> Result<(), MediaError> {
        if !linear.is_finite() || !(0.0..=16.0).contains(&linear) {
            return Err(MediaError::InvalidArgument("stream gain"));
        }
        self.shared.gain.store(linear.to_bits(), Ordering::Release);
        Ok(())
    }
    pub fn set_mute(&self, mute: bool) {
        self.shared.mute.store(mute, Ordering::Release);
    }
    pub fn set_channel_gain(&self, channel: usize, linear: f32) -> Result<(), MediaError> {
        if channel >= self.config.format.channels as usize
            || !linear.is_finite()
            || !(0.0..=16.0).contains(&linear)
        {
            return Err(MediaError::InvalidArgument("channel gain"));
        }
        self.shared.channels[channel].store(linear.to_bits(), Ordering::Release);
        Ok(())
    }
    /// Enable metering of this stream's post-gain samples while a returned handle exists.
    /// No capture stream or desktop authority is created by this operation.
    pub fn meter(&self) -> super::AudioMeter { super::AudioMeter::new(self.shared.clone()) }
    pub fn clock(&self) -> super::AudioClock {
        super::AudioClock { shared: self.shared.clone() }
    }
    pub fn diagnostics(&self) -> AudioDiagnostics {
        let s = &self.shared;
        AudioDiagnostics {
            frames: s.frames.load(Ordering::Relaxed),
            underruns: s.underruns.load(Ordering::Relaxed),
            overruns: s.overruns.load(Ordering::Relaxed),
            discontinuities: s.discontinuities.load(Ordering::Relaxed),
            graph_ticks: s.ticks.load(Ordering::Relaxed),
            monotonic_ns: s.now.load(Ordering::Relaxed),
            latency_frames: s.delay.load(Ordering::Relaxed),
            quantum: s.quantum.load(Ordering::Relaxed),
        }
    }
}
impl Drop for AudioStream {
    fn drop(&mut self) {
        self.stop();
    }
}
