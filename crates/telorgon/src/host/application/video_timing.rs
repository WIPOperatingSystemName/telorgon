//! Preview scheduling policy stays off the native media callbacks.
use crate::{
    integrations::pipewire::MediaError,
    media::video::{VideoPlayout, VideoPlayoutConfig},
};

pub(super) struct PreviewTiming {
    pub video: VideoPlayoutConfig,
    #[cfg(feature = "audio-linux")]
    pub audio: Option<crate::media::audio::AudioClock>,
    #[cfg(feature = "audio-linux")]
    last_audio_discontinuities: Option<u64>,
    #[cfg(feature = "audio-linux")]
    last_audio_time: Option<i64>,
    #[cfg(feature = "audio-linux")]
    audio_waiting: bool,
}
impl PreviewTiming {
    pub fn monotonic(video: VideoPlayoutConfig) -> Self {
        Self {
            video,
            #[cfg(feature = "audio-linux")]
            audio: None,
            #[cfg(feature = "audio-linux")]
            last_audio_discontinuities: None,
            #[cfg(feature = "audio-linux")]
            last_audio_time: None,
            #[cfg(feature = "audio-linux")]
            audio_waiting: false,
        }
    }
    #[cfg(feature = "audio-linux")]
    pub fn audio(video: VideoPlayoutConfig, audio: crate::media::audio::AudioClock) -> Self {
        Self {
            audio: Some(audio),
            ..Self::monotonic(video)
        }
    }
    pub fn follows_audio(&self) -> bool {
        #[cfg(feature = "audio-linux")]
        {
            self.audio.is_some()
        }
        #[cfg(not(feature = "audio-linux"))]
        {
            false
        }
    }
    /// False means retain no scheduled frame until a fresh audio clock is available.
    pub fn update(&mut self, playout: &mut VideoPlayout, now: i64) -> Result<bool, MediaError> {
        #[cfg(feature = "audio-linux")]
        if let Some(clock) = &self.audio {
            let Some(snapshot) = clock.snapshot().filter(|sample| {
                let quantum_ns = u64::from(sample.quantum_frames) * 1_000_000_000
                    / u64::from(sample.sample_rate.max(1));
                sample.is_recent(now, quantum_ns.saturating_mul(3).max(250_000_000))
            }) else {
                if !self.audio_waiting {
                    playout.reset();
                }
                self.audio_waiting = true;
                return Ok(false);
            };
            if self.audio_waiting
                || self
                    .last_audio_discontinuities
                    .is_some_and(|previous| previous != snapshot.discontinuities)
                || self
                    .last_audio_time
                    .is_some_and(|previous| snapshot.timing.monotonic_ns < previous)
            {
                playout.reset();
            }
            self.audio_waiting = false;
            self.last_audio_discontinuities = Some(snapshot.discontinuities);
            self.last_audio_time = Some(snapshot.timing.monotonic_ns);
            let delay = snapshot
                .presentation_delay_ns(0)
                .and_then(|delay| delay.checked_add(self.video.delay_ns))
                .ok_or(MediaError::InvalidArgument(
                    "audio presentation delay overflow",
                ))?;
            playout.set_delay_ns(delay)?;
        }
        let _ = (playout, now);
        Ok(true)
    }
}
