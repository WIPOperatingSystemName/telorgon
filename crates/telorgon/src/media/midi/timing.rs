//! Checked graph/monotonic conversion; no allocation, locks, or native operations.
use super::MidiTime;
use crate::integrations::pipewire::MediaError;
use std::time::Duration;

impl MidiTime {
    /// Schedule at an audio stream's application sample-frame position using two
    /// current snapshots from the same host. Keeps this MIDI stream's generation.
    /// Callers must refresh snapshots after either stream changes clock/format and
    /// check freshness before scheduling. Physical device latency is not inferred.
    #[cfg(feature = "audio-linux")]
    pub fn at_audio_frame(
        self,
        audio: crate::media::audio::AudioClockSnapshot,
        frame_position: u64,
    ) -> Result<Self, MediaError> {
        let timestamp = audio
            .frame_time_ns(frame_position)
            .and_then(|ns| u64::try_from(ns).ok())
            .ok_or(MediaError::InvalidArgument(
                "audio frame has no valid host timestamp",
            ))?;
        self.at_monotonic(timestamp)
    }

    /// Freshness check for a clock snapshot (before scheduling it into the future).
    /// A clock from the future or an elapsed monotonic epoch is not considered recent.
    pub fn is_recent(self, monotonic_now_ns: u64, maximum_age: Duration) -> bool {
        self.validate_clock().is_ok()
            && monotonic_now_ns
                .checked_sub(self.monotonic_ns)
                .is_some_and(|age| u128::from(age) <= maximum_age.as_nanos())
    }

    fn validate_clock(self) -> Result<(), MediaError> {
        if self.generation == 0 || self.rate_num == 0 || self.rate_denom == 0 {
            return Err(MediaError::NotReady);
        }
        Ok(())
    }

    /// Schedule relative to this snapshot, rounding up to the next graph tick.
    /// Updates both graph position and monotonic timestamp. Admission/delivery still
    /// rejects this generation if the destination clock resets before processing.
    pub fn after(self, delay: Duration) -> Result<Self, MediaError> {
        let delay = u64::try_from(delay.as_nanos())
            .map_err(|_| MediaError::InvalidArgument("MIDI delay overflow"))?;
        let target = self
            .monotonic_ns
            .checked_add(delay)
            .ok_or(MediaError::InvalidArgument("MIDI timestamp overflow"))?;
        self.at_monotonic(target)
    }

    /// Map a host CLOCK_MONOTONIC timestamp into this stream's clock generation.
    /// Uses the snapshot's rate; refresh the snapshot after a discontinuity or drift.
    /// Rounds toward the next tick so quantization does not schedule an event early.
    /// Past timestamps are allowed: native delivery counts them as late. This does
    /// not infer transport/device latency or synchronize clocks on different hosts.
    pub fn at_monotonic(self, timestamp_ns: u64) -> Result<Self, MediaError> {
        self.validate_clock()?;
        let delta = i128::from(timestamp_ns) - i128::from(self.monotonic_ns);
        let numerator = delta * i128::from(self.rate_denom);
        let denominator = i128::from(self.rate_num) * 1_000_000_000;
        let ticks =
            numerator.div_euclid(denominator) + i128::from(numerator.rem_euclid(denominator) != 0);
        let position = u64::try_from(i128::from(self.position) + ticks)
            .map_err(|_| MediaError::InvalidArgument("MIDI graph position overflow"))?;
        // A representable graph position does not necessarily imply a representable
        // monotonic time (large tick periods near either endpoint).
        let ns = (ticks * denominator).div_euclid(i128::from(self.rate_denom));
        let monotonic_ns = u64::try_from(i128::from(self.monotonic_ns) + ns)
            .map_err(|_| MediaError::InvalidArgument("MIDI timestamp overflow"))?;
        Ok(Self {
            position,
            monotonic_ns,
            ..self
        })
    }

    /// Preserve this event's host time, add an explicit forwarding delay, and stamp
    /// it with the destination snapshot's clock ID, rate and generation. Callers
    /// decide whether an old input epoch remains relevant before forwarding it.
    pub fn rebase(self, destination: Self, delay: Duration) -> Result<Self, MediaError> {
        self.validate_clock()?;
        let delay = u64::try_from(delay.as_nanos())
            .map_err(|_| MediaError::InvalidArgument("MIDI forwarding delay overflow"))?;
        let target = self
            .monotonic_ns
            .checked_add(delay)
            .ok_or(MediaError::InvalidArgument("MIDI timestamp overflow"))?;
        destination.at_monotonic(target)
    }
}
