//! Non-realtime video transport, with bounded CPU frame leases and native buffer generations.
use super::{
    connection::{Completion, native},
    *,
};
use crate::media::video::*;
use pipewire as pw;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::Rc,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
pub(crate) enum VideoCommand {
    Active(bool),
    Formats(Vec<VideoFormat>),
}
struct Data {
    config: VideoConfig,
    shared: Arc<VideoShared>,
    format: Option<VideoFormat>,
    producer_period: Option<Duration>,
    modifier: Option<u64>,
    gpu_formats: Vec<VideoDmaBufFormat>,
    transfer: Option<Box<dyn VideoGpuTransfer>>,
    producer: Option<Box<dyn VideoGpuProducer>>,
    generation: u64,
    pool: Option<FramePool>,
    buffers: BTreeMap<usize, NativeBuffer>,
    transport: VideoState,
    sequence: u64,
    previous: Option<u64>,
    last_frame: Option<VideoFrame>,
    delivered_drop_count: Option<u64>,
}
struct NativeBuffer {
    generation: u64,
    output: Option<super::video_output::OutputBuffer>,
}
impl Data {
    fn update_retirement(&self) {
        let first_live = self
            .buffers
            .values()
            .map(|b| b.generation)
            .min()
            .unwrap_or(self.generation);
        self.shared
            .retired_generation
            .fetch_max(first_live.saturating_sub(1), Ordering::AcqRel);
    }

    fn fixate(&mut self, pod: &pw::spa::pod::Pod) -> Result<Option<Vec<Vec<u8>>>, MediaError> {
        let Some((format, candidates)) = super::video_format::fixation(pod)? else {
            return Ok(None);
        };
        self.format(None)?;
        let Some(producer) = &mut self.producer else {
            return Ok(Some(Vec::new()));
        };
        if !self
            .config
            .formats
            .iter()
            .any(|offer| super::video_format::accepts(*offer, format, self.config.negotiation_range(*offer)))
        {
            return Err(MediaError::Unsupported("unadvertised GPU output format"));
        }
        let mut chosen = None;
        for modifier in candidates {
            let Some(cap) = self
                .gpu_formats
                .iter()
                .find(|g| g.pixel == format.pixel && g.modifier == modifier)
            else {
                continue;
            };
            if let Ok(buffer) = producer.allocate(
                format,
                modifier,
                self.config.memory_budget / 2 / self.config.buffer_frames,
            ) {
                if buffer.planes().len() == cap.planes as usize {
                    chosen = Some(modifier);
                    break;
                }
            }
        }
        if chosen.is_none() {
            self.gpu_formats.clear();
        }
        let mut formats = super::video_format::formats(&self.config, &self.gpu_formats)?;
        if let Some(modifier) = chosen {
            formats.insert(0, super::video_format::fixed_format(format, modifier)?);
        }
        Ok(Some(formats))
    }

    fn publish_state(&self) {
        self.shared.state(
            if !matches!(self.transport, VideoState::Stopped | VideoState::Failed(_))
                && self.format.is_some()
                && self.config.direction == VideoDirection::Capture
                && self.pool.is_none()
                && self.modifier.is_none()
            {
                VideoState::Renegotiating
            } else {
                self.transport.clone()
            },
        );
    }
    fn pool(&mut self) -> Result<(), MediaError> {
        if self.config.direction == VideoDirection::Capture
            && self.pool.is_none()
            && self.modifier.is_none()
            && let Some(format) = self.format
        {
            match FramePool::new(
                format,
                self.generation,
                self.config.buffer_frames,
                self.shared.budget.clone(),
            ) {
                Ok(pool) => self.pool = Some(pool),
                Err(MediaError::ResourceLimit(_)) => {}
                Err(e) => return Err(e),
            }
        }
        self.publish_state();
        Ok(())
    }
    fn format(
        &mut self,
        pod: Option<&pw::spa::pod::Pod>,
    ) -> Result<Option<VideoFormat>, MediaError> {
        self.format = None;
        self.producer_period = None;
        self.modifier = None;
        self.pool = None;
        self.previous = None;
        self.last_frame = None;
        self.delivered_drop_count = None;
        self.shared
            .frames
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        *self
            .shared
            .negotiated
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        self.shared.notifications.notify(events::FORMAT);
        self.shared.state(VideoState::Renegotiating);
        let Some(pod) = pod else {
            return Ok(None);
        };
        let (format, modifier) = super::video_format::parse(pod)?;
        if modifier.is_none() && !self.config.permits_shared_memory(format.pixel) {
            return Err(MediaError::Unsupported(
                "unadvertised shared-memory video format",
            ));
        }
        if modifier.is_some_and(|modifier| {
            !self
                .gpu_formats
                .iter()
                .any(|g| g.pixel == format.pixel && g.modifier == modifier)
        }) {
            return Err(MediaError::Unsupported(
                "unadvertised video DMA-BUF modifier",
            ));
        }
        if !self
            .config
            .formats
            .iter()
            .any(|offer| super::video_format::accepts(*offer, format, self.config.negotiation_range(*offer)))
        {
            return Err(MediaError::Unsupported("unadvertised video format"));
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(MediaError::ResourceLimit("video generations"))?;
        self.update_retirement();
        self.producer_period = if self.config.direction == VideoDirection::Produce {
            Some(super::video_format::producer_period(pod, self.config.formats[0].rate)?)
        } else { None };
        self.format = Some(format);
        self.modifier = modifier;
        self.shared.format_changes.fetch_add(1, Ordering::Relaxed);
        *self
            .shared
            .negotiated
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(VideoNegotiation {
            format,
            generation: self.generation,
            transport: modifier.map_or(VideoTransport::SharedMemory, |m| {
                VideoTransport::DmaBuf(
                    *self
                        .gpu_formats
                        .iter()
                        .find(|g| g.pixel == format.pixel && g.modifier == m)
                        .unwrap(),
                )
            }),
        });
        self.shared.notifications.notify(events::FORMAT);
        self.pool()?;
        Ok(Some(format))
    }
    fn process(&mut self, stream: &pw::stream::Stream) {
        let Some(mut buffer) = super::video_buffer::Buffer::dequeue(stream) else {
            return;
        };
        let Some(format) = self.format else {
            return;
        };
        if self
            .buffers
            .get(&buffer.identity())
            .is_none_or(|b| b.generation != self.generation)
        {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if self.shared.stop.load(Ordering::Acquire) {
            return;
        }
        match self.config.direction {
            VideoDirection::Produce => {
                // Read drop count under the submission queue lock: a later overwrite must
                // invalidate the next delivery, never get acknowledged by this older frame.
                let (frame, dropped) = {
                    let mut queue = self.shared.frames.lock().unwrap_or_else(|e| e.into_inner());
                    (
                        queue.pop_front(),
                        self.shared.dropped.load(Ordering::Acquire),
                    )
                };
                let full_damage = self.delivered_drop_count != Some(dropped);
                let frame = if self.config.repeat_last_frame {
                    if frame.is_some() {
                        self.last_frame = frame;
                    }
                    self.last_frame.clone()
                } else {
                    frame
                };
                let result = if self.modifier.is_some() {
                    let output = self
                        .buffers
                        .get_mut(&buffer.identity())
                        .and_then(|b| b.output.as_mut());
                    let frame = frame.as_ref().and_then(|f| match f {
                        VideoFrame::Gpu(f) if f.format() == format => Some(f),
                        _ => None,
                    });
                    match output {
                        Some(output) => {
                            buffer.produce_gpu(output, format, frame, self.sequence, full_damage)
                        }
                        None => Err(MediaError::InvalidArgument("missing GPU output allocation")),
                    }
                } else {
                    let frame = frame.as_ref().and_then(|f| match f {
                        VideoFrame::Cpu(f) if f.format() == format => Some(f),
                        _ => None,
                    });
                    buffer.produce(format, frame, self.sequence, full_damage)
                };
                self.sequence = self.sequence.wrapping_add(1);
                match result {
                    Ok(true) => {
                        self.delivered_drop_count = Some(dropped);
                        self.shared.delivered.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(false) => {
                        self.delivered_drop_count = None;
                        self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(MediaError::ResourceLimit(_)) => {
                        self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        self.delivered_drop_count = None;
                        self.shared.malformed.fetch_add(1, Ordering::Relaxed);
                        if self.modifier.is_some() {
                            self.shared.state(VideoState::Failed(error));
                            self.shared.stop.store(true, Ordering::Release);
                        }
                    }
                }
            }
            VideoDirection::Capture => {
                let backlogged = {
                    let mut queue = self.shared.frames.lock().unwrap_or_else(|e| e.into_inner());
                    let had_pending = !queue.is_empty();
                    if queue.len() >= self.config.buffer_frames - 1 {
                        self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                        if self.config.queue_policy == VideoQueuePolicy::RejectNewest {
                            return;
                        }
                        queue.pop_front();
                    }
                    // Every frame behind a queued predecessor may later follow an eviction.
                    // Mark it conservatively discontinuous before publishing immutable pixels.
                    had_pending
                };
                if let Some(modifier) = self.modifier {
                    if self.shared.budget.gpu_frames() >= self.config.buffer_frames {
                        self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    let count = self
                        .gpu_formats
                        .iter()
                        .find(|g| g.pixel == format.pixel && g.modifier == modifier)
                        .unwrap()
                        .planes;
                    let limit = self
                        .config
                        .memory_budget
                        .saturating_sub(self.shared.budget.used());
                    let result = buffer.capture_gpu(
                        format,
                        modifier,
                        count,
                        self.transfer.as_mut().unwrap().as_mut(),
                        limit,
                        if backlogged { None } else { self.previous },
                    );
                    match result {
                        Ok(Some(mut frame)) => {
                            match frame.account(
                                self.shared.budget.clone(),
                                self.generation,
                                self.config.buffer_frames,
                            ) {
                                Ok(()) => {
                                    self.previous = frame.metadata().sequence;
                                    self.shared
                                        .frames
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .push_back(VideoFrame::Gpu(frame));
                                    self.shared.delivered.fetch_add(1, Ordering::Relaxed);
                                    self.shared.notifications.notify(events::FRAME);
                                }
                                Err(MediaError::ResourceLimit(_)) => {
                                    self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                                }
                                Err(error) => {
                                    self.shared.state(VideoState::Failed(error));
                                    self.shared.stop.store(true, Ordering::Release);
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(MediaError::ResourceLimit(_)) => {
                            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(error) => {
                            self.shared.state(VideoState::Failed(error));
                            self.shared.stop.store(true, Ordering::Release);
                        }
                    }
                    return;
                }
                let Some(pool) = &mut self.pool else {
                    self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let Some(slot) = pool.slots.iter_mut().find(|s| Arc::strong_count(s) == 1) else {
                    self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let Some(frame) = Arc::get_mut(slot) else {
                    self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                match buffer.capture(frame) {
                    Ok(true) => {
                        frame.metadata.discontinuity |=
                            backlogged || frame.metadata.sequence.is_none();
                        if let Some(sequence) = frame.metadata.sequence {
                            frame.metadata.discontinuity |= self
                                .previous
                                .is_none_or(|old| old.wrapping_add(1) != sequence);
                        }
                        self.previous = frame.metadata.sequence;
                        self.shared
                            .frames
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push_back(VideoFrame::Cpu(CpuVideoFrame { data: slot.clone() }));
                        self.shared.notifications.notify(events::FRAME);
                        self.shared.delivered.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(false) => {}
                    Err(_) => {
                        self.shared.malformed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }
}
pub(crate) struct NativeVideo {
    // Timer and listener retire before stream and callback state. All are loop-local.
    timer: Option<super::video_timer::VideoTimer>,
    listener: Option<pw::stream::StreamListener<()>>,
    stream: pw::stream::StreamRc,
    data: Rc<RefCell<Data>>,
    period: Option<Duration>,
    pending: Option<(VideoCommand, u64, Completion, Instant)>,
}
impl NativeVideo {
    pub fn create(
        core: pw::core::CoreRc,
        mainloop: pw::main_loop::MainLoopRc,
        config: VideoConfig,
        shared: Arc<VideoShared>,
        backend: Option<GpuBackend>,
        snapshot: &RegistrySnapshot,
    ) -> Result<Self, MediaError> {
        let output = config.direction == VideoDirection::Produce;
        let (transfer, producer) = match backend {
            Some(GpuBackend::Capture(transfer)) => (Some(transfer), None),
            Some(GpuBackend::Produce(producer)) => (None, Some(producer)),
            None => (None, None),
        };
        let allocating = producer.is_some();
        let gpu_formats = match (&transfer, &producer) {
            (Some(t), _) => t.formats(),
            (_, Some(p)) => p.formats(),
            _ => Vec::new(),
        };
        if gpu_formats.len() > 256
            || gpu_formats
                .iter()
                .any(|g| g.planes == 0 || g.planes > 4 || g.modifier == 0x00ff_ffff_ffff_ffff)
        {
            return Err(MediaError::InvalidArgument("GPU transfer capabilities"));
        }
        let formats = super::video_format::formats(&config, &gpu_formats)?;
        let mut props = pw::properties::properties! {"media.type"=>"Video","media.category"=>if output{"Capture"}else{"Playback"},"media.role"=>if config.virtual_camera{"Camera"}else{"Screen"},"node.name"=>config.name.clone(),"node.description"=>config.name.clone(),"media.class"=>if output{"Video/Source"}else{"Stream/Input/Video"},"node.want-driver"=>"true"};
        if output {
            props.insert("node.virtual", "true");
        }
        if let Some(target) = config.target {
            let object = snapshot.resolve(target)?;
            if object.kind != ObjectKind::Node {
                return Err(MediaError::InvalidArgument("video target node"));
            }
            props.insert(
                "target.object",
                object
                    .serial()
                    .ok_or(MediaError::Unsupported("video target serial"))?
                    .to_string(),
            );
            props.insert("node.dont-reconnect", "true");
        }
        let stream = pw::stream::StreamRc::new(core, &config.name, props).map_err(native)?;
        let data = Rc::new(RefCell::new(Data {
            config: config.clone(),
            shared,
            format: None,
            producer_period: None,
            modifier: None,
            gpu_formats,
            transfer,
            producer,
            generation: 0,
            pool: None,
            buffers: BTreeMap::new(),
            transport: VideoState::Connecting,
            sequence: 0,
            previous: None,
            last_frame: None,
            delivered_drop_count: None,
        }));
        let state = data.clone();
        let param = data.clone();
        let add = data.clone();
        let remove = data.clone();
        let process = data.clone();
        let listener = stream
            .add_local_listener_with_user_data(())
            .state_changed(move |_, _, _, value| {
                let mut data = state.borrow_mut();
                data.transport = match value {
                    pw::stream::StreamState::Paused => VideoState::Paused,
                    pw::stream::StreamState::Streaming => VideoState::Streaming,
                    pw::stream::StreamState::Error(error) => VideoState::Failed(native(error)),
                    pw::stream::StreamState::Unconnected => VideoState::Stopped,
                    _ => VideoState::Connecting,
                };
                data.publish_state();
            })
            .param_changed(move |stream, _, id, pod| {
                if id != pw::spa::sys::SPA_PARAM_Format {
                    return;
                }
                if let Some(pod) = pod {
                    let fixation = param.borrow_mut().fixate(pod);
                    match fixation {
                        Ok(Some(bytes)) => {
                            if !bytes.is_empty() {
                                let mut pods: Vec<_> = bytes
                                    .iter()
                                    .map(|b| pw::spa::pod::Pod::from_bytes(b).unwrap())
                                    .collect();
                                if let Err(error) = stream.update_params(&mut pods).map_err(native)
                                {
                                    let data = param.borrow();
                                    data.shared.state(VideoState::Failed(error));
                                    data.shared.stop.store(true, Ordering::Release);
                                }
                            }
                            return;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            let data = param.borrow();
                            data.shared.state(VideoState::Failed(error));
                            data.shared.stop.store(true, Ordering::Release);
                            return;
                        }
                    }
                }
                let result = param.borrow_mut().format(pod);
                let result = result.and_then(|format| {
                    if let Some(format) = format {
                        let data = param.borrow();
                        let planes = data.modifier.map(|m| {
                            data.gpu_formats
                                .iter()
                                .find(|g| g.pixel == format.pixel && g.modifier == m)
                                .unwrap()
                                .planes
                        });
                        let bytes = if data.config.direction == VideoDirection::Produce {
                            super::video_format::buffer_params_count(
                                format,
                                planes,
                                data.config.buffer_frames,
                            )?
                        } else {
                            super::video_format::buffer_params(format, planes)?
                        };
                        drop(data);
                        let mut pods = bytes
                            .iter()
                            .map(|bytes| {
                                pw::spa::pod::Pod::from_bytes(bytes)
                                    .ok_or(MediaError::InvalidArgument("video buffer POD"))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        stream.update_params(&mut pods).map_err(native)?;
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    let data = param.borrow();
                    data.shared.state(VideoState::Failed(error));
                    data.shared.stop.store(true, Ordering::Release);
                }
            })
            .add_buffer(move |stream, _, buffer| {
                let mut data = add.borrow_mut();
                if data.buffers.len() >= 64 {
                    data.shared
                        .state(VideoState::Failed(MediaError::ResourceLimit(
                            "native video buffers",
                        )));
                    data.shared.stop.store(true, Ordering::Release);
                    return;
                }
                let generation = data.generation;
                let format = data.format;
                let modifier = data.modifier;
                let budget = data.shared.budget.clone();
                let capture_budget = data.shared.capture_budget.clone();
                let limit = data.config.memory_budget / 2;
                let output =
                    if let (Some(format), Some(producer)) = (format, data.producer.as_mut()) {
                        // SAFETY: add_buffer owns live mutable native arrays; owner retained until remove_buffer.
                        match unsafe {
                            super::video_output::OutputBuffer::allocate(
                                buffer,
                                format,
                                modifier,
                                producer.as_mut(),
                                budget,
                                limit,
                                capture_budget,
                            )
                        } {
                            Ok(output) => Some(output),
                            Err(error) => {
                                data.shared.state(VideoState::Failed(error));
                                data.shared.stop.store(true, Ordering::Release);
                                drop(data);
                                // SAFETY: callback's live stream on its control worker. No
                                // RefCell borrow survives synchronous state/error callbacks.
                                unsafe {
                                    pw::sys::pw_stream_set_error(
                                        stream.as_raw_ptr(),
                                        -libc::EIO,
                                        c"video output allocation failed".as_ptr(),
                                    );
                                }
                                return;
                            }
                        }
                    } else {
                        None
                    };
                data.buffers
                    .insert(buffer as usize, NativeBuffer { generation, output });
            })
            .remove_buffer(move |_, _, buffer| {
                let mut data = remove.borrow_mut();
                data.buffers.remove(&(buffer as usize));
                data.update_retirement();
            })
            .process(move |stream, _| process.borrow_mut().process(stream))
            .register()
            .map_err(native)?;
        let weak = stream.downgrade();
        let timer = output.then(|| {
            super::video_timer::VideoTimer::new(mainloop, move |_| {
                if let Some(stream) = weak.upgrade()
                    && stream.state() == pw::stream::StreamState::Streaming
                {
                    let _ = stream.trigger_process();
                }
            })
        });
        let mut pods = formats
            .iter()
            .map(|b| {
                pw::spa::pod::Pod::from_bytes(b)
                    .ok_or(MediaError::InvalidArgument("video format POD"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut flags = pw::stream::StreamFlags::MAP_BUFFERS;
        if config.autoconnect {
            flags |= pw::stream::StreamFlags::AUTOCONNECT;
        }
        if config.start_paused {
            flags |= pw::stream::StreamFlags::INACTIVE;
        }
        if output {
            flags |= pw::stream::StreamFlags::DRIVER;
        }
        if allocating {
            flags |= pw::stream::StreamFlags::ALLOC_BUFFERS;
        }
        stream
            .connect(
                if output {
                    pw::spa::utils::Direction::Output
                } else {
                    pw::spa::utils::Direction::Input
                },
                None,
                flags,
                &mut pods,
            )
            .map_err(native)?;
        Ok(Self {
            timer,
            listener: Some(listener),
            stream,
            data,
            period: None,
            pending: None,
        })
    }
    pub fn command(&mut self, command: VideoCommand, completion: Completion) {
        if !completion.begin() {
            return;
        }
        let terminal = {
            let data = self.data.borrow();
            let state = data.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            match &*state {
                VideoState::Failed(error) => Some(error.clone()),
                VideoState::Stopped => Some(MediaError::NotReady),
                _ if data.shared.stop.load(Ordering::Acquire) => Some(MediaError::NotReady),
                _ => None,
            }
        };
        if let Some(error) = terminal {
            completion.finish(Err(error));
            return;
        }
        if self.pending.is_some() {
            completion.finish(Err(MediaError::NotReady));
            return;
        }
        let generation = self.data.borrow().generation;
        let result = match &command {
            VideoCommand::Active(active) => self.stream.set_active(*active).map_err(native),
            VideoCommand::Formats(formats) => {
                let bytes = {
                    let data = self.data.borrow();
                    let mut config = data.config.clone();
                    config.formats = formats.clone();
                    super::video_format::formats(&config, &data.gpu_formats)
                };
                bytes.and_then(|bytes| {
                    let mut pods = bytes
                        .iter()
                        .map(|b| {
                            pw::spa::pod::Pod::from_bytes(b)
                                .ok_or(MediaError::InvalidArgument("video format POD"))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    {
                        let mut data = self.data.borrow_mut();
                        data.config.formats = formats.clone();
                        let _ = data.format(None);
                    }
                    if let Err(error) = self.stream.update_params(&mut pods).map_err(native) {
                        let data = self.data.borrow();
                        data.shared.state(VideoState::Failed(error.clone()));
                        data.shared.stop.store(true, Ordering::Release);
                        return Err(error);
                    }
                    Ok(())
                })
            }
        };
        match result {
            Ok(()) => {
                self.pending = Some((
                    command,
                    generation,
                    completion,
                    Instant::now() + Duration::from_secs(5),
                ))
            }
            Err(e) => completion.finish(Err(e)),
        }
    }
    pub fn poll(&mut self, nodes: &BTreeMap<u32, ObjectHandle>) -> bool {
        let mut data = self.data.borrow_mut();
        let shared = data.shared.clone();
        if shared.stop.load(Ordering::Acquire) {
            return false;
        }
        if data
            .config
            .target
            .is_some_and(|h| nodes.get(&h.id()) != Some(&h))
        {
            shared.state(VideoState::Failed(MediaError::StaleHandle));
            return false;
        }
        {
            let mut node = shared.node.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(handle) = *node {
                if nodes.get(&handle.id()) != Some(&handle) {
                    shared.state(VideoState::Failed(MediaError::StaleHandle));
                    return false;
                }
            } else {
                *node = nodes.get(&self.stream.node_id()).copied();
            }
        }
        if let Err(error) = data.pool() {
            shared.state(VideoState::Failed(error));
            return false;
        }
        if matches!(
            *shared.state.lock().unwrap_or_else(|e| e.into_inner()),
            VideoState::Stopped | VideoState::Failed(_)
        ) {
            return false;
        }
        let period = if data.transport == VideoState::Streaming {
            data.producer_period
        } else {
            None
        };
        if self.period != period {
            if let Some(timer) = &self.timer {
                if let Err(error) = timer.set_period(period) {
                    shared.state(VideoState::Failed(error));
                    return false;
                }
            }
            self.period = period;
        }
        if let Some((command, old_generation, completion, deadline)) = &self.pending {
            let done = match command {
                VideoCommand::Active(active) => {
                    data.transport
                        == if *active {
                            VideoState::Streaming
                        } else {
                            VideoState::Paused
                        }
                }
                VideoCommand::Formats(_) => {
                    data.generation > *old_generation
                        && data.format.is_some()
                        && (data.config.direction == VideoDirection::Produce
                            || data.pool.is_some()
                            || data.modifier.is_some())
                }
            };
            if done {
                completion.finish(Ok(()));
                self.pending = None;
            } else if Instant::now() >= *deadline {
                completion.finish(Err(MediaError::Timeout));
                self.pending = None;
                // A late native reply must not resume or reconfigure a stream after
                // its caller has observed the transition failing.
                shared.state(VideoState::Failed(MediaError::Timeout));
                shared.stop.store(true, Ordering::Release);
                return false;
            }
        }
        true
    }
}
impl Drop for NativeVideo {
    fn drop(&mut self) {
        if let Some((_, _, completion, _)) = self.pending.take() {
            let data = self.data.borrow();
            let error = match &*data.shared.state.lock().unwrap_or_else(|e| e.into_inner()) {
                VideoState::Failed(error) => error.clone(),
                _ => MediaError::Disconnected,
            };
            completion.finish(Err(error));
        }
        self.timer.take();
        let _ = self.stream.set_active(false);
        self.listener.take();
        let _ = self.stream.disconnect();
        let data = self.data.borrow();
        *data.shared.node.lock().unwrap_or_else(|e| e.into_inner()) = None;
        data.shared.state(VideoState::Stopped);
        data.shared
            .retired_generation
            .store(u64::MAX, Ordering::Release);
        data.shared
            .frames
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}
