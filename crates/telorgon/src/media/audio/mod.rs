//! Application audio. Explicit streams share a transport connection, never instantiate
//! desktop controls, and stop when their owner is dropped or the connection disappears.
//! Buffered IO is nonblocking, interleaved f32; negotiation can use f32 or signed 16-bit PCM.
//! File decoding and UI work belong outside realtime callbacks.
mod realtime;
mod stream;
pub use realtime::{AudioCallback, AudioCycle, AudioTiming};
pub(crate) use realtime::{Endpoint, RealtimeData};
pub use stream::*;

pub(crate) use stream::AudioShared;
mod assets;
pub use assets::{SoundAsset, SoundPlayback};
pub mod dsp;
mod resample;
pub use resample::{Resampled, SincResampler};
mod capture;
pub use capture::{ApplicationCapture, ApplicationSelector, DuplexAudio};

mod filter;
pub use filter::{AudioFilter,FilterConfig,FilterCycle,FilterClock,FilterInputs,FilterOutputs,FilterProcessor,FilterState};
pub(crate) use filter::FilterShared;

mod timing;
pub use timing::{AudioClock, AudioClockSnapshot};

mod capture_timing;
pub use capture_timing::CapturedAudio;

mod meter;
pub use meter::{AudioLevels, AudioMeter, ChannelLevel};

mod loopback;
pub use loopback::{AudioLoopback, LoopbackConfig, LoopbackState};

mod drift;
pub use drift::DriftAdjustment;

mod share;
pub use share::{AudioShareMix, AudioShareResolver, AudioShareEndpoints};

mod share_delivery;
pub use share_delivery::{PipeWireAudioDelivery, AudioShareHandoff};

mod preferences;
pub use preferences::{AudioDeviceId, AudioDeviceFallback, AudioDevicePreference, AudioStreamPreference, RestoredAudioStream};
