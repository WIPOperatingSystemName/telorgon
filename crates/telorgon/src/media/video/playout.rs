//! Bounded deadline scheduling for owned CPU/GPU capture frames.
use super::*;
use crate::media::timing::{VideoDeadline, video_deadline};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct VideoPlayoutConfig {
    /// Added to source CLOCK_MONOTONIC timestamps, for intentional pipeline latency.
    pub delay_ns: u64,
    pub maximum_lateness_ns: u64,
    /// Reject implausible future timestamps instead of pinning a pool slot indefinitely.
    pub maximum_future_ns: u64,
}
impl Default for VideoPlayoutConfig {
    fn default() -> Self {
        Self {
            delay_ns: 0,
            maximum_lateness_ns: 50_000_000,
            maximum_future_ns: 1_000_000_000,
        }
    }
}
impl VideoPlayoutConfig {
    pub(crate) fn validate(self) -> Result<(), MediaError> {
        if self.maximum_future_ns == 0
            || self.maximum_future_ns > 10_000_000_000
            || self.delay_ns > self.maximum_future_ns
            || self.maximum_lateness_ns > 10_000_000_000
        {
            return Err(MediaError::InvalidArgument("video playout timing bounds"));
        }
        Ok(())
    }
}
#[derive(Debug)]
pub enum VideoPlayoutPoll {
    Empty,
    /// Wake on the next stream event or after this duration, whichever occurs first.
    Pending {
        wait_ns: u64,
    },
    Ready {
        frame: VideoFrame,
        discontinuity: bool,
    },
}
/// One control-thread consumer holding at most one early frame, charged to the stream's
/// existing pool budget. Poll does not sleep; it examines at most 16 frames. Other receivers
/// compete with this owner and should not be used for the same playback pipeline.
/// Dropping/resetting releases its lease; it does not stop the shared stream.
pub struct VideoPlayout {
    stream: Arc<VideoStream>,
    config: VideoPlayoutConfig,
    pending: Option<VideoFrame>,
    generation: Option<u64>,
    last_now: Option<i64>,
    discontinuity: bool,
    dropped: u64,
}
impl VideoPlayout {
    pub fn new(stream: Arc<VideoStream>, config: VideoPlayoutConfig) -> Result<Self, MediaError> {
        config.validate()?;
        if stream.direction() != VideoDirection::Capture {
            return Err(MediaError::Unsupported("video playout requires capture"));
        }
        Ok(Self {
            stream,
            config,
            pending: None,
            generation: None,
            last_now: None,
            discontinuity: true,
            dropped: 0,
        })
    }
    pub fn reset(&mut self) {
        self.pending = None;
        self.generation = None;
        self.last_now = None;
        self.discontinuity = true;
    }
    /// Adjust presentation latency without dropping an early frame. The next poll applies
    /// the new delay to its original source timestamp. Use reset for a clock discontinuity.
    pub fn set_delay_ns(&mut self, delay_ns: u64) -> Result<(), MediaError> {
        let config = VideoPlayoutConfig {
            delay_ns,
            ..self.config
        };
        config.validate()?;
        self.config = config;
        Ok(())
    }
    pub fn dropped_frames(&self) -> u64 {
        self.dropped
    }
    fn discard(&mut self) {
        self.dropped = self.dropped.saturating_add(1);
        self.discontinuity = true;
    }
    pub fn poll(&mut self, monotonic_now_ns: i64) -> Result<VideoPlayoutPoll, MediaError> {
        if self.last_now.is_some_and(|last| monotonic_now_ns < last) {
            self.reset();
        }
        self.last_now = Some(monotonic_now_ns);
        if self.stream.state() != VideoState::Streaming {
            self.reset();
            return Ok(VideoPlayoutPoll::Empty);
        }
        let Some(negotiated) = self.stream.negotiation() else {
            self.reset();
            return Ok(VideoPlayoutPoll::Empty);
        };
        if self.generation != Some(negotiated.generation) {
            self.pending = None;
            self.generation = Some(negotiated.generation);
            self.discontinuity = true;
        }
        for _ in 0..16 {
            let Some(frame) = (match self.pending.take() {
                Some(frame) => Some(frame),
                None => self.stream.receive_frame()?,
            }) else {
                return Ok(VideoPlayoutPoll::Empty);
            };
            if frame.generation() != negotiated.generation || frame.format() != negotiated.format {
                self.discard();
                continue;
            }
            let metadata = frame.metadata();
            if let Some(timestamp) = metadata.timestamp_ns {
                let Some(timestamp) = timestamp.checked_add(self.config.delay_ns as i64) else {
                    self.discard();
                    continue;
                };
                match video_deadline(timestamp, monotonic_now_ns, self.config.maximum_lateness_ns) {
                    VideoDeadline::Drop => {
                        self.discard();
                        continue;
                    }
                    VideoDeadline::Wait(wait_ns) if wait_ns > self.config.maximum_future_ns => {
                        self.discard();
                        continue;
                    }
                    VideoDeadline::Wait(wait_ns) => {
                        self.pending = Some(frame);
                        return Ok(VideoPlayoutPoll::Pending { wait_ns });
                    }
                    VideoDeadline::Present => {}
                }
            }
            // Missing source timestamps are presented immediately, with a discontinuity so
            // callers cannot mistake unscheduled arrival-time playback for clock alignment.
            let discontinuity =
                self.discontinuity || metadata.discontinuity || metadata.timestamp_ns.is_none();
            self.discontinuity = false;
            return Ok(VideoPlayoutPoll::Ready {
                frame,
                discontinuity,
            });
        }
        Ok(VideoPlayoutPoll::Empty)
    }
}
