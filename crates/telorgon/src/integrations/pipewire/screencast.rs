//! Screen-host assembly over the shared media transport. Receives approved pixels only;
//! authorization and GPU jobs remain owned by the shell. No duplicate PipeWire loop or PODs.
use super::{
    Connection, ConnectionConfig, ConnectionState, Remote, Request, RequestState, Subscription,
};
use crate::{
    media::video::{
        self as video, CpuVideoFrame, FrameMetadata, VideoConfig, VideoState, VideoSubscription,
    },
    shell::capture::CaptureLayout,
};
use std::{cell::RefCell, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StreamStatus {
    Connecting,
    Ready { node_id: u32 },
    Failed(String),
    Stopped,
}
struct State {
    video: Option<video::VideoStream>,
    subscription: Option<VideoSubscription>,
    current: Option<video::VideoFrame>,
    producer: Option<Box<dyn video::VideoGpuProducer>>,
    retired: Vec<CpuVideoFrame>,
    spare: Option<video::CapturePixels>,
    submitted_generation: Option<u64>,
    resize: Option<(Request, u64)>,
    stopped: bool,
    failure: Option<String>,
}
impl State {
    fn reclaim(&mut self) {
        let mut held = Vec::with_capacity(2);
        for frame in self.retired.drain(..) {
            match frame.try_into_capture() {
                Ok(bytes) => self.spare = Some(bytes),
                Err(frame) if held.len() < 2 => held.push(frame),
                Err(_) => {}
            }
        }
        self.retired = held;
    }
    fn replace(&mut self, frame: video::VideoFrame) {
        if let Some(video::VideoFrame::Cpu(old)) = self.current.replace(frame) {
            self.retired.push(old);
        }
        self.submitted_generation = None;
        self.reclaim();
    }
    fn clear(&mut self) {
        self.current = None;
        self.retired.clear();
        self.spare = None;
        self.submitted_generation = None;
    }
}

pub(crate) struct VideoStream {
    connection: Connection,
    events: Subscription,
    state: RefCell<State>,
    layout: CaptureLayout,
    fps: u32,
    stream_id: u64,
    float16: bool,
    capture_budget: Option<Arc<video::MemoryBudget>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl VideoStream {
    /// Asynchronous readiness: publishing the node does not require a portal consumer to
    /// connect. Pixels can be retained before format negotiation and then submitted once.
    pub(crate) fn start(
        stream_id: u64,
        layout: CaptureLayout,
        fps: u32,
        wake: Arc<dyn Fn() + Send + Sync>,
        producer: Option<Box<dyn video::VideoGpuProducer>>,
    ) -> Result<Self, String> {
        Self::start_budgeted(stream_id, layout, fps, wake, producer, None)
    }
    pub(crate) fn start_budgeted(
        stream_id: u64,
        layout: CaptureLayout,
        fps: u32,
        wake: Arc<dyn Fn() + Send + Sync>,
        producer: Option<Box<dyn video::VideoGpuProducer>>,
        capture_budget: Option<Arc<video::MemoryBudget>>,
    ) -> Result<Self, String> {
        validate_layout(layout, fps)?;
        let connection = Connection::connect(
            ConnectionConfig {
                application_name: "Telorgon screen sharing".into(),
                ..Default::default()
            },
            Remote::Default,
        )
        .map_err(|e| e.to_string())?;
        let notify = wake.clone();
        let events = connection
            .handle()
            .subscribe(32, move || notify())
            .map_err(|e| e.to_string())?;
        let float16 = producer.as_ref().is_some_and(|producer| {
            producer
                .formats()
                .iter()
                .any(|format| format.pixel == video::PixelFormat::RgbaF16)
        });
        Ok(Self {
            float16,
            capture_budget,
            connection,
            events,
            layout,
            fps,
            stream_id,
            wake,
            state: RefCell::new(State {
                video: None,
                subscription: None,
                current: None,
                producer,
                retired: Vec::with_capacity(2),
                spare: None,
                submitted_generation: None,
                resize: None,
                stopped: false,
                failure: None,
            }),
        })
    }
    fn format(&self) -> video::VideoFormat {
        video::VideoFormat::rgba(self.layout.width(), self.layout.height(), self.fps)
    }
    fn config(&self) -> VideoConfig {
        let mut config = VideoConfig::producer(self.format());
        config.formats = screen_formats(self.layout, self.fps, self.float16);
        config.allow_lower_frame_rate = true;
        config.shared_memory_formats = Some(vec![
            video::PixelFormat::Rgba8,
            video::PixelFormat::Bgra8,
            video::PixelFormat::Rgbx8,
            video::PixelFormat::Bgrx8,
        ]);
        config.name = format!("telorgon.capture.{}", self.stream_id);
        config.buffer_frames = 2;
        config.repeat_last_frame = true;
        // Reserve a fixed ceiling so growing a negotiated source does not inherit the
        // initial small frame's budget. Allocation remains lazy; the shell admission
        // service separately accounts all active renderer and delivery allocations.
        config.memory_budget = 512 * 1024 * 1024;
        config
    }
    fn accepts_format(&self, format: video::VideoFormat) -> bool {
        let config = self.config();
        config.formats.iter().any(|offer|
            super::video_format::accepts(*offer, format, config.negotiation_range(*offer)))
    }
    fn poll(&self) {
        while self.events.try_recv().is_some() {}
        let mut state = self.state.borrow_mut();
        if let Some(subscription) = &state.subscription {
            let _ = subscription.take_update();
        }
        if state.stopped || state.failure.is_some() {
            return;
        }
        match self.connection.handle().state() {
            ConnectionState::Ready => {}
            ConnectionState::Failed(error) => {
                state.failure = Some(error.to_string());
                return;
            }
            ConnectionState::Stopped => {
                state.stopped = true;
                return;
            }
            _ => return,
        }
        if state.video.is_none() {
            let opened = video::VideoStream::open_screen_producer(
                self.connection.handle(),
                self.config(),
                state.producer.take(),
                self.capture_budget.clone(),
            )
            .and_then(|stream| {
                let wake = self.wake.clone();
                let subscription = stream.subscribe(move || wake())?;
                Ok((stream, subscription))
            });
            match opened {
                Ok((stream, subscription)) => {
                    state.video = Some(stream);
                    state.subscription = Some(subscription);
                }
                Err(error) => {
                    state.failure = Some(error.to_string());
                    return;
                }
            }
        }
        let stream = state.video.as_ref().unwrap();
        match stream.state() {
            VideoState::Failed(error) => {
                state.failure = Some(error.to_string());
                return;
            }
            VideoState::Stopped => {
                state.stopped = true;
                return;
            }
            _ => {}
        }
        if let Some((request, old_generation)) = &state.resize {
            match request.state() {
                RequestState::Complete(Ok(()))
                    if stream.native_generation_retired(*old_generation) =>
                {
                    state.resize = None
                }
                RequestState::Complete(Err(error)) => {
                    state.failure = Some(error.to_string());
                    return;
                }
                _ => return,
            }
        }
        let stream = state.video.as_ref().unwrap();
        if let Some(negotiated) = stream.negotiation()
            && self.accepts_format(negotiated.format)
            && state.submitted_generation != Some(negotiated.generation)
            && let Some(frame) = &state.current
        {
            let submitted_format = match frame {
                video::VideoFrame::Cpu(f) => f.format(),
                video::VideoFrame::Gpu(f) => f.format(),
            };
            if submitted_format != negotiated.format {
                state.clear();
                return;
            }
            let result = match (frame, negotiated.transport) {
                (video::VideoFrame::Cpu(frame), video::VideoTransport::SharedMemory) => {
                    stream.submit(frame.clone()).map_err(|(error, _)| error)
                }
                (video::VideoFrame::Gpu(frame), video::VideoTransport::DmaBuf(_)) => {
                    stream.submit_gpu(frame.clone()).map_err(|(error, _)| error)
                }
                _ => {
                    state.clear();
                    return;
                }
            };
            match result {
                Ok(()) => state.submitted_generation = Some(negotiated.generation),
                Err(super::MediaError::NotReady | super::MediaError::QueueFull) => {}
                Err(error) => {
                    state.failure = Some(error.to_string());
                    return;
                }
            }
        }
        state.reclaim();
    }
    pub(crate) fn status(&self) -> StreamStatus {
        self.poll();
        let state = self.state.borrow();
        if let Some(error) = &state.failure {
            return StreamStatus::Failed(error.clone());
        }
        if state.stopped {
            return StreamStatus::Stopped;
        }
        if state.resize.is_none()
            && let Some(node) = state.video.as_ref().and_then(|s| s.node())
        {
            return StreamStatus::Ready { node_id: node.id() };
        }
        StreamStatus::Connecting
    }
    pub(crate) fn resize(&mut self, layout: CaptureLayout, fps: u32) -> Result<(), String> {
        validate_layout(layout, fps)?;
        if !matches!(self.status(), StreamStatus::Ready { .. }) {
            return Err("video stream is not ready for resize".into());
        }
        let mut state = self.state.borrow_mut();
        let stream = state.video.as_ref().unwrap();
        let old = stream
            .negotiated_format()
            .map_or(0, |(_, generation)| generation);
        let request = stream
            .reconfigure(screen_formats(layout, fps, self.float16))
            .map_err(|e| e.to_string())?;
        state.clear();
        state.resize = Some((request, old));
        self.layout = layout;
        self.fps = fps;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn publish(
        &self,
        pixels: Vec<u8>,
        format: video::VideoFormat,
        metadata: FrameMetadata,
    ) -> Result<Option<Vec<u8>>, Vec<u8>> {
        self.publish_owned(video::CapturePixels::unaccounted(pixels), format, metadata)
            .map(|p| p.map(|p| p.bytes))
            .map_err(|p| p.bytes)
    }
    pub(crate) fn publish_owned(
        &self,
        pixels: video::CapturePixels,
        format: video::VideoFormat,
        metadata: FrameMetadata,
    ) -> Result<Option<video::CapturePixels>, video::CapturePixels> {
        if metadata.validate(format).is_err()
            || !self.accepts_format(format)
            || pixels.len() != self.layout.byte_len()
            || format
                .byte_len()
                .map_or(true, |bytes| bytes != pixels.len())
            || pixels
                .capacity()
                .saturating_add(metadata.allocation_bytes())
                > self.config().memory_budget / 4
            || !matches!(self.status(), StreamStatus::Ready { .. })
        {
            return Err(pixels);
        }
        let metadata_charge = match self
            .capture_budget
            .as_ref()
            .map(|budget| {
                video::MemoryReservation::new(budget.clone(), metadata.allocation_bytes())
            })
            .transpose()
        {
            Ok(charge) => charge,
            Err(_) => return Err(pixels),
        };
        let frame = match CpuVideoFrame::packed_capture(format, pixels, metadata, metadata_charge) {
            Ok(frame) => frame,
            Err((_, pixels)) => return Err(pixels),
        };
        self.state
            .borrow_mut()
            .replace(video::VideoFrame::Cpu(frame));
        self.poll();
        Ok(self.state.borrow_mut().spare.take())
    }
    pub(crate) fn delivery_diagnostics(&self) -> Option<(VideoState, video::VideoDiagnostics)> {
        self.poll();
        let state = self.state.borrow();
        let stream = state.video.as_ref()?;
        Some((stream.state(), stream.diagnostics()))
    }
    pub(crate) fn negotiation(&self) -> Option<video::VideoNegotiation> {
        self.poll();
        self.state.borrow().video.as_ref()?.negotiation()
    }
    /// A replacement must include full damage if its predecessor never reached transport.
    pub(crate) fn has_unsubmitted_frame(&self) -> bool {
        let state = self.state.borrow();
        state.current.is_some() && state.submitted_generation.is_none()
    }
    pub(crate) fn publish_gpu(
        &self,
        frame: video::GpuVideoFrame,
    ) -> Result<(), video::GpuVideoFrame> {
        if !self.accepts_format(frame.format())
            || !matches!(self.status(), StreamStatus::Ready { .. })
        {
            return Err(frame);
        }
        self.state
            .borrow_mut()
            .replace(video::VideoFrame::Gpu(frame));
        self.poll();
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn take_spare(&self) -> Option<Vec<u8>> {
        self.take_spare_owned().map(|pixels| pixels.bytes)
    }
    pub(crate) fn take_spare_owned(&self) -> Option<video::CapturePixels> {
        self.poll();
        self.state.borrow_mut().spare.take()
    }
    pub(crate) fn is_stopping(&self) -> bool {
        self.state.borrow().stopped
    }
    pub(crate) fn stop(&self) {
        let mut state = self.state.borrow_mut();
        state.stopped = true;
        if let Some((request, _)) = state.resize.take() {
            request.cancel();
        }
        state.clear();
        if let Some(stream) = &state.video {
            stream.stop();
        }
        self.connection.request_shutdown();
    }
    /// Join only after the native worker has ended. Never wait on the shell owner thread.
    pub(crate) fn retired(&mut self) -> bool {
        if !self.connection.is_finished() {
            return false;
        }
        let _ = self.connection.shutdown();
        true
    }
}
impl Drop for VideoStream {
    fn drop(&mut self) {
        self.stop();
    }
}
fn screen_formats(layout: CaptureLayout, fps: u32, float16: bool) -> Vec<video::VideoFormat> {
    let mut formats = [
        video::PixelFormat::Rgba8,
        video::PixelFormat::Bgra8,
        video::PixelFormat::Rgbx8,
        video::PixelFormat::Bgrx8,
    ]
    .map(|pixel| video::VideoFormat {
        pixel,
        ..video::VideoFormat::rgba(layout.width(), layout.height(), fps)
    })
    .to_vec();
    if float16 {
        let format = video::VideoFormat {
            pixel: video::PixelFormat::RgbaF16,
            color: video::Colorimetry::LINEAR_BT709,
            ..video::VideoFormat::rgba(layout.width(), layout.height(), fps)
        };
        let mut config = VideoConfig::producer(format);
        config.buffer_frames = 2;
        config.memory_budget = 512 * 1024 * 1024;
        if config.validate().is_ok() {
            formats.push(format);
        }
    }
    formats
}
fn validate_layout(layout: CaptureLayout, fps: u32) -> Result<(), String> {
    if fps == 0
        || fps > 240
        || layout.width() > 8192
        || layout.height() > 8192
        || layout.stride()
            != layout
                .width()
                .checked_mul(4)
                .ok_or("video stride overflow")?
        || (layout.byte_len()
            + video::MAX_CURSOR_BYTES
            + video::MAX_DAMAGE_RECTS * std::mem::size_of::<video::VideoRect>())
            * 2
            > 512 * 1024 * 1024
    {
        return Err("unsupported screen stream layout or frame rate".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
