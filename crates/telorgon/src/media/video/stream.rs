use super::*;
use crate::integrations::pipewire::{
    ConnectionHandle, ConnectionState, ObjectHandle, ObjectKind, Request,
    connection::{Command, Completion},
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoDirection {
    Capture,
    Produce,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoQueuePolicy {
    DropOldest,
    RejectNewest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoTransport {
    SharedMemory,
    DmaBuf(VideoDmaBufFormat),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoNegotiation {
    pub format: VideoFormat,
    pub generation: u64,
    pub transport: VideoTransport,
}
#[derive(Clone, Debug)]
pub struct VideoConfig {
    pub name: String,
    pub direction: VideoDirection,
    /// Ordered preferred alternatives, at most 16. Exact unless capture_range is set.
    /// Renegotiation may select any advertised
    /// alternative. Frame consumers always inspect the negotiated format and generation.
    pub formats: Vec<VideoFormat>,
    /// Pixel formats supported by the caller's CPU path. None permits all offered formats;
    /// Some(empty) requires GPU transport. GPU capability queries remain authoritative.
    /// This prevents a GPU-only format from silently negotiating an unusable CPU fallback.
    pub shared_memory_formats: Option<Vec<PixelFormat>>,
    pub capture_range: Option<VideoCaptureRange>,
    /// Keep the offered resolution fixed while accepting variable or lower rates up to its ceiling.
    pub allow_lower_frame_rate: bool,
    pub target: Option<ObjectHandle>,
    pub autoconnect: bool,
    /// Advertise a Video/Source node usable as a virtual camera by PipeWire clients.
    /// This does not create a V4L2 device or promise compatibility with V4L2-only clients.
    pub virtual_camera: bool,
    pub start_paused: bool,
    /// Producers may repeat the last delivered pixels when no new frame is queued. The
    /// retained image consumes one frame slot. Set its timestamp to None for fresh output
    /// clock timestamps on every repeat. Stop and reconfigure always discard retained pixels.
    pub repeat_last_frame: bool,
    pub queue_policy: VideoQueuePolicy,
    /// Capture pool slots (2..=16); queue capacity is one less, leaving a copy destination.
    /// GPU frames retained by callers or renderer leases also consume these slots, including
    /// across renegotiation; once all slots are held, incoming frames are dropped.
    pub buffer_frames: usize,
    /// Includes frames retained across renegotiation. Capture pauses allocation until old
    /// leases release enough bytes; it never grows an unbounded list of retired pools.
    pub memory_budget: usize,
}
impl VideoConfig {
    pub fn capture(target: ObjectHandle, formats: Vec<VideoFormat>) -> Self {
        Self {
            direction: VideoDirection::Capture,
            target: Some(target),
            autoconnect: true,
            formats,
            ..Self::producer(VideoFormat::rgba(640, 480, 30))
        }
    }
    pub fn producer(format: VideoFormat) -> Self {
        Self {
            name: "Telorgon video".into(),
            direction: VideoDirection::Produce,
            formats: vec![format],
            shared_memory_formats: None,
            capture_range: None,
            allow_lower_frame_rate: false,
            target: None,
            autoconnect: false,
            virtual_camera: false,
            start_paused: false,
            repeat_last_frame: false,
            queue_policy: VideoQueuePolicy::DropOldest,
            buffer_frames: 4,
            memory_budget: 128 * 1024 * 1024,
        }
    }
    pub(crate) fn negotiation_range(&self, format: VideoFormat) -> Option<VideoCaptureRange> {
        self.capture_range.or_else(|| self.allow_lower_frame_rate.then_some(VideoCaptureRange {
            min_size: [format.width, format.height], max_size: [format.width, format.height],
            min_rate: super::FrameRate { numerator: 0, denominator: 1 }, max_rate: format.rate,
        }))
    }
    pub(crate) fn validate(&self) -> Result<(), MediaError> {
        if self.allow_lower_frame_rate && self.capture_range.is_some() {
            return Err(MediaError::InvalidArgument("conflicting video negotiation ranges"));
        }
        if self.name.is_empty()
            || self.name.len() > 256
            || self.name.contains('\0')
            || !(2..=16).contains(&self.buffer_frames)
            || self.memory_budget > 512 * 1024 * 1024
            || self.memory_budget == 0
            || self.virtual_camera && self.direction != VideoDirection::Produce
        {
            return Err(MediaError::InvalidArgument("video stream configuration"));
        }
        if self
            .shared_memory_formats
            .as_ref()
            .is_some_and(|formats| formats.len() > 16)
        {
            return Err(MediaError::InvalidArgument(
                "shared-memory video format alternatives",
            ));
        }
        self.validate_formats(&self.formats)
    }
    pub(crate) fn permits_shared_memory(&self, pixel: PixelFormat) -> bool {
        self.shared_memory_formats
            .as_ref()
            .is_none_or(|formats| formats.contains(&pixel))
    }
    pub(crate) fn validate_formats(&self, formats: &[VideoFormat]) -> Result<(), MediaError> {
        if formats.is_empty() || formats.len() > 16 {
            return Err(MediaError::InvalidArgument("video format alternatives"));
        }
        for format in formats {
            if self.allow_lower_frame_rate { self.negotiation_range(*format).unwrap().validate(*format)?; }
            let mut maximum = *format;
            if let Some(range) = self.capture_range {
                if self.direction != VideoDirection::Capture {
                    return Err(MediaError::InvalidArgument("ranges are capture-only"));
                }
                range.validate(*format)?;
                maximum.width = range.max_size[0];
                maximum.height = range.max_size[1];
            }
            if self.direction == VideoDirection::Produce && format.rate.numerator == 0 {
                return Err(MediaError::InvalidArgument(
                    "producer needs fixed video cadence",
                ));
            }
            let required = (maximum.byte_len()?
                + MAX_CURSOR_BYTES
                + MAX_DAMAGE_RECTS * std::mem::size_of::<VideoRect>())
            .checked_mul(self.buffer_frames)
            .ok_or(MediaError::InvalidArgument("video memory budget overflow"))?;
            if required > self.memory_budget {
                return Err(MediaError::ResourceLimit("configured video frame pool"));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoState {
    Connecting,
    Paused,
    Streaming,
    Renegotiating,
    Stopped,
    Failed(MediaError),
}
#[derive(Clone, Copy, Debug, Default)]
pub struct VideoDiagnostics {
    pub frames: u64,
    pub dropped: u64,
    pub malformed: u64,
    pub underruns: u64,
    pub format_changes: u64,
    pub allocated_frame_bytes: usize,
}
pub(crate) struct VideoShared {
    pub notifications: events::Notifications,
    pub state: Mutex<VideoState>,
    pub negotiated: Mutex<Option<VideoNegotiation>>,
    pub node: Mutex<Option<ObjectHandle>>,
    pub frames: Mutex<VecDeque<VideoFrame>>,
    pub stop: AtomicBool,
    pub budget: Arc<MemoryBudget>,
    pub capture_budget: Option<Arc<MemoryBudget>>,
    pub delivered: AtomicU64,
    pub dropped: AtomicU64,
    pub malformed: AtomicU64,
    pub underruns: AtomicU64,
    pub format_changes: AtomicU64,
    pub retired_generation: AtomicU64,
}
impl VideoShared {
    pub fn state(&self, state: VideoState) {
        let mut current = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let changed = !matches!(*current, VideoState::Failed(_)) && *current != state;
        if changed {
            *current = state;
        }
        drop(current);
        if changed {
            self.notifications.notify(events::STATE);
        }
    }
}
static NEXT_STREAM: AtomicU64 = AtomicU64::new(1);
/// A control-thread owner; methods are not realtime-safe. Submit/receive do no waiting for
/// native I/O. Creation and controls are asynchronous; state and Request report observed
/// completion. Drop requests stop without joining. Recovery requires a new stream and fresh
/// handles. Camera/portal permission belongs to the connection supplied by the caller.
pub struct VideoStream {
    connection: ConnectionHandle,
    shared: Arc<VideoShared>,
    config: VideoConfig,
    gpu_enabled: bool,
    id: u64,
}
impl VideoStream {
    pub fn open(connection: ConnectionHandle, config: VideoConfig) -> Result<Self, MediaError> {
        Self::open_inner(connection, config, None)
    }
    /// Prefer the transfer backend's device-tested DMA-BUF tuples, with CPU shared-memory
    /// fallback. Use receive_frame to handle both results. The transfer owns completed copies,
    /// so application frame retention never pins a native PipeWire buffer across shutdown.
    pub fn open_gpu(
        connection: ConnectionHandle,
        config: VideoConfig,
        transfer: impl VideoGpuTransfer,
    ) -> Result<Self, MediaError> {
        if config.direction != VideoDirection::Capture {
            return Err(MediaError::Unsupported(
                "GPU capture entry point requires input direction",
            ));
        }
        Self::open_inner(
            connection,
            config,
            Some(GpuBackend::Capture(Box::new(transfer))),
        )
    }
    /// Prefer allocator-supported DMA-BUF output, with shared-memory fallback. Inspect
    /// negotiation() to choose submit_gpu or submit. Output buffers retain fixed allocations
    /// through native retirement; submitted frames are copied before publication. Half the
    /// byte budget bounds native allocations and half bounds queued submissions.
    pub fn open_gpu_producer(
        connection: ConnectionHandle,
        config: VideoConfig,
        producer: impl VideoGpuProducer,
    ) -> Result<Self, MediaError> {
        if config.direction != VideoDirection::Produce {
            return Err(MediaError::InvalidArgument(
                "GPU producer needs output direction",
            ));
        }
        let mut allocation_config = config.clone();
        allocation_config.memory_budget /= 2;
        allocation_config.validate()?;
        Self::open_inner(
            connection,
            config,
            Some(GpuBackend::Produce(Box::new(producer))),
        )
    }
    fn open_inner(
        connection: ConnectionHandle,
        config: VideoConfig,
        transfer: Option<GpuBackend>,
    ) -> Result<Self, MediaError> {
        Self::open_inner_budgeted(connection, config, transfer, None)
    }
    pub(crate) fn open_screen_producer(
        connection: ConnectionHandle,
        config: VideoConfig,
        producer: Option<Box<dyn VideoGpuProducer>>,
        capture_budget: Option<Arc<MemoryBudget>>,
    ) -> Result<Self, MediaError> {
        if config.direction != VideoDirection::Produce {
            return Err(MediaError::InvalidArgument("screen producer direction"));
        }
        let mut allocation_config = config.clone();
        allocation_config.memory_budget /= 2;
        allocation_config.validate()?;
        Self::open_inner_budgeted(
            connection,
            config,
            Some(GpuBackend::Produce(
                producer.unwrap_or_else(|| Box::new(CpuOutputAllocator)),
            )),
            capture_budget,
        )
    }
    fn open_inner_budgeted(
        connection: ConnectionHandle,
        config: VideoConfig,
        transfer: Option<GpuBackend>,
        capture_budget: Option<Arc<MemoryBudget>>,
    ) -> Result<Self, MediaError> {
        config.validate()?;
        connection.ensure_ready()?;
        let shared = Arc::new(VideoShared {
            notifications: events::Notifications::default(),
            state: Mutex::new(VideoState::Connecting),
            negotiated: Mutex::new(None),
            node: Mutex::new(None),
            frames: Mutex::new(VecDeque::with_capacity(config.buffer_frames - 1)),
            stop: AtomicBool::new(false),
            budget: MemoryBudget::new(config.memory_budget),
            capture_budget,
            delivered: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            malformed: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            format_changes: AtomicU64::new(0),
            retired_generation: AtomicU64::new(0),
        });
        let id = NEXT_STREAM.fetch_add(1, Ordering::Relaxed);
        let gpu_enabled = transfer.is_some();
        connection.send(Command::CreateVideo(
            id,
            config.clone(),
            shared.clone(),
            transfer,
        ))?;
        Ok(Self {
            connection,
            shared,
            config,
            gpu_enabled,
            id,
        })
    }
    pub fn gpu_enabled(&self) -> bool {
        self.gpu_enabled
    }
    pub fn direction(&self) -> VideoDirection {
        self.config.direction
    }
    pub fn state(&self) -> VideoState {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if matches!(state, VideoState::Failed(_)) {
            return state;
        }
        match self.connection.state() {
            ConnectionState::Ready => state,
            ConnectionState::Failed(error) => VideoState::Failed(error),
            _ if matches!(state, VideoState::Stopped) => state,
            _ => VideoState::Failed(MediaError::Disconnected),
        }
    }
    pub fn negotiated_format(&self) -> Option<(VideoFormat, u64)> {
        self.negotiation().map(|n| (n.format, n.generation))
    }
    /// One atomic snapshot of format, storage and generation. None during renegotiation.
    pub fn negotiation(&self) -> Option<VideoNegotiation> {
        if self.shared.stop.load(Ordering::Acquire)
            || matches!(self.state(), VideoState::Stopped | VideoState::Failed(_))
        {
            return None;
        }
        *self
            .shared
            .negotiated
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
    /// True after every native buffer up to this generation has retired. Independent
    /// application-owned frame copies can remain alive. Generation zero denotes no format.
    pub fn native_generation_retired(&self, generation: u64) -> bool {
        self.shared.retired_generation.load(Ordering::Acquire) >= generation
    }
    /// Coalesced state, format and frame notifications; at most eight subscriptions.
    pub fn subscribe(
        &self,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<VideoSubscription, MediaError> {
        self.shared.notifications.subscribe(wake)
    }
    pub fn node(&self) -> Option<ObjectHandle> {
        if self.shared.stop.load(Ordering::Acquire)
            || matches!(self.state(), VideoState::Stopped | VideoState::Failed(_))
        {
            return None;
        }
        let node = *self.shared.node.lock().unwrap_or_else(|e| e.into_inner());
        node.filter(|n| {
            self.connection
                .shared
                .registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .snapshot
                .resolve(*n)
                .is_ok()
        })
    }
    /// Captured frames are immutable pool leases. Retaining all slots intentionally causes
    /// new frames to be dropped; Drop frees a slot without calling native APIs. Concurrent
    /// callers share one FIFO and compete for frames; this is not a broadcast subscription.
    pub fn receive(&self) -> Result<Option<CpuVideoFrame>, MediaError> {
        if self.config.direction != VideoDirection::Capture {
            return Err(MediaError::Unsupported("video capture"));
        }
        let mut queue = self.shared.frames.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(queue.front(), Some(VideoFrame::Gpu(_))) {
            return Err(MediaError::Unsupported(
                "use receive_frame for GPU captures",
            ));
        }
        Ok(queue.pop_front().map(|f| match f {
            VideoFrame::Cpu(f) => f,
            _ => unreachable!(),
        }))
    }
    /// One bounded FIFO for CPU fallback and owned GPU images. Concurrent receivers compete.
    pub fn receive_frame(&self) -> Result<Option<VideoFrame>, MediaError> {
        if self.config.direction != VideoDirection::Capture {
            return Err(MediaError::Unsupported("video capture"));
        }
        Ok(self
            .shared
            .frames
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front())
    }
    /// Transfer an owned frame. RejectNewest returns the exact frame on overflow; DropOldest
    /// accepts it and counts the displaced frame. Acceptance does not promise delivery after
    /// disconnect, pause or reconfiguration. Submitted format must equal negotiated format.
    pub fn submit(&self, frame: CpuVideoFrame) -> Result<(), (MediaError, CpuVideoFrame)> {
        if self.config.direction != VideoDirection::Produce {
            return Err((MediaError::Unsupported("video production"), frame));
        }
        if self.shared.stop.load(Ordering::Acquire)
            || !matches!(self.state(), VideoState::Paused | VideoState::Streaming)
        {
            return Err((MediaError::NotReady, frame));
        }
        if self.negotiation().is_none_or(|n| {
            n.format != frame.format() || n.transport != VideoTransport::SharedMemory
        }) || frame.allocation_bytes() > self.submission_limit()
        {
            return Err((
                MediaError::InvalidArgument("submitted video format or memory size"),
                frame,
            ));
        }
        let mut queue = self.shared.frames.lock().unwrap_or_else(|e| e.into_inner());
        if queue.len() >= self.config.buffer_frames - 1 {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            if self.config.queue_policy == VideoQueuePolicy::RejectNewest {
                return Err((MediaError::QueueFull, frame));
            }
            queue.pop_front();
        }
        queue.push_back(VideoFrame::Cpu(frame));
        Ok(())
    }
    fn submission_limit(&self) -> usize {
        self.config.memory_budget / self.config.buffer_frames / if self.gpu_enabled { 2 } else { 1 }
    }
    /// Submit immutable GPU pixels for a negotiated DMA-BUF producer. Queue overload returns
    /// the exact frame for RejectNewest. Native copy completion is reported in diagnostics;
    /// acceptance alone does not promise delivery. Source modifier may differ from destination.
    pub fn submit_gpu(&self, frame: GpuVideoFrame) -> Result<(), (MediaError, GpuVideoFrame)> {
        if self.config.direction != VideoDirection::Produce || !self.gpu_enabled {
            return Err((MediaError::Unsupported("GPU video producer"), frame));
        }
        if self.shared.stop.load(Ordering::Acquire)
            || !matches!(self.state(), VideoState::Paused | VideoState::Streaming)
        {
            return Err((MediaError::NotReady, frame));
        }
        if self.negotiation().is_none_or(|n| {
            n.format != frame.format() || !matches!(n.transport, VideoTransport::DmaBuf(_))
        }) || frame.allocation_bytes() > self.submission_limit()
        {
            return Err((
                MediaError::InvalidArgument("submitted GPU format, transport or budget"),
                frame,
            ));
        }
        let mut queue = self.shared.frames.lock().unwrap_or_else(|e| e.into_inner());
        if queue.len() >= self.config.buffer_frames - 1 {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            if self.config.queue_policy == VideoQueuePolicy::RejectNewest {
                return Err((MediaError::QueueFull, frame));
            }
            queue.pop_front();
        }
        queue.push_back(VideoFrame::Gpu(frame));
        Ok(())
    }
    pub fn set_active(&self, active: bool) -> Result<Request, MediaError> {
        self.command(crate::integrations::pipewire::video::VideoCommand::Active(
            active,
        ))
    }
    /// Old frames already owned by the application remain readable with their old generation.
    /// Queued old frames are discarded. Completion requires a new observed Format event.
    pub fn reconfigure(&self, formats: Vec<VideoFormat>) -> Result<Request, MediaError> {
        if self.gpu_enabled && self.config.direction == VideoDirection::Produce {
            let mut config = self.config.clone();
            config.memory_budget /= 2;
            config.validate_formats(&formats)?;
        } else {
            self.config.validate_formats(&formats)?;
        }
        self.command(crate::integrations::pipewire::video::VideoCommand::Formats(
            formats,
        ))
    }
    fn command(
        &self,
        command: crate::integrations::pipewire::video::VideoCommand,
    ) -> Result<Request, MediaError> {
        match self.state() {
            VideoState::Failed(error) => return Err(error),
            VideoState::Stopped => return Err(MediaError::NotReady),
            _ => {}
        }
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(MediaError::NotReady);
        }
        self.connection.ensure_ready()?;
        let request = Request::new();
        self.connection.send(Command::Video(
            self.id,
            command,
            Completion(request.state.clone()),
        ))?;
        Ok(request)
    }
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
    }
    pub fn diagnostics(&self) -> VideoDiagnostics {
        let s = &self.shared;
        VideoDiagnostics {
            frames: s.delivered.load(Ordering::Relaxed),
            dropped: s.dropped.load(Ordering::Relaxed),
            malformed: s.malformed.load(Ordering::Relaxed),
            underruns: s.underruns.load(Ordering::Relaxed),
            format_changes: s.format_changes.load(Ordering::Relaxed),
            allocated_frame_bytes: s.budget.used(),
        }
    }
}
impl Drop for VideoStream {
    fn drop(&mut self) {
        self.stop();
    }
}
#[derive(Clone, Debug)]
pub struct VideoSource {
    pub handle: ObjectHandle,
    pub name: String,
    pub description: String,
    pub camera: bool,
    pub virtual_source: bool,
}
/// Registry discovery only; it does not acquire or start a camera. On portal connections
/// only authorized nodes are visible. Unknown sources remain available for explicit choice.
pub fn sources(connection: &ConnectionHandle) -> Vec<VideoSource> {
    let snapshot = connection.snapshot();
    snapshot
        .objects_of_kind(ObjectKind::Node)
        .filter(|o| o.media_class() == Some("Video/Source"))
        .map(|o| VideoSource {
            handle: o.handle,
            name: o.properties.get("node.name").cloned().unwrap_or_default(),
            description: o
                .properties
                .get("node.description")
                .cloned()
                .unwrap_or_default(),
            camera: o
                .properties
                .get("media.role")
                .is_some_and(|r| r == "Camera")
                || o.properties
                    .keys()
                    .any(|k| k.starts_with("api.v4l2.") || k.starts_with("api.libcamera.")),
            virtual_source: o
                .properties
                .get("node.virtual")
                .is_some_and(|v| v == "true"),
        })
        .collect()
}

// Native output owns its shared-memory fallback even when no GPU tuple is supported.
// SAFETY: this allocator advertises no DMA-BUF formats and never returns an allocation.
struct CpuOutputAllocator;
unsafe impl VideoGpuProducer for CpuOutputAllocator {
    fn formats(&self) -> Vec<VideoDmaBufFormat> {
        Vec::new()
    }
    fn allocate(
        &mut self,
        _: VideoFormat,
        _: u64,
        _: usize,
    ) -> Result<Box<dyn VideoGpuOutputBuffer>, MediaError> {
        Err(MediaError::Unsupported("CPU-only screen output"))
    }
}
