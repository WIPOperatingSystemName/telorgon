//! Owned video frames and explicit Linux PipeWire streams. Copies run on the native control
//! worker, never the audio realtime callback. CPU and GPU frames retain independent storage;
//! neither keeps a native buffer dequeued while the application holds a frame. GPU backends
//! must complete their copy before native buffers can be returned.
pub(crate) mod events;
mod format;
mod frame;
mod stream;
use crate::integrations::pipewire::MediaError;
pub use events::{VideoSubscription, VideoUpdate};
pub use format::*;
pub(crate) use frame::{CapturePixels, FrameData, FramePool, MemoryBudget, MemoryReservation};
pub use frame::{
    CpuPlane, CpuVideoFrame, CursorBitmap, FrameMetadata, MAX_CURSOR_BYTES, MAX_DAMAGE_RECTS,
    PlaneView, VideoCursor,
};
pub(crate) use stream::VideoShared;
pub use stream::{
    VideoConfig, VideoDiagnostics, VideoDirection, VideoNegotiation, VideoQueuePolicy, VideoSource,
    VideoState, VideoStream, VideoTransport, sources,
};

pub(crate) mod controls;
pub use controls::{CameraControl, CameraControlId, CameraControls, CameraDomain, CameraValue};

pub(crate) mod gpu;
pub use gpu::{
    BorrowedGpuVideoFrame, BorrowedVideoDmaBufPlane, GpuVideoFrame, VideoDmaBufFormat,
    VideoDmaBufPlane, VideoFrame, VideoGpuTransfer,
};

mod gpu_output;
pub(crate) use gpu_output::GpuBackend;
pub use gpu_output::{VideoGpuOutputBuffer, VideoGpuProducer};

mod playout;
pub use playout::{VideoPlayout, VideoPlayoutConfig, VideoPlayoutPoll};
