//! Bounded sound assets decoded off the realtime thread. Initial codec set: RIFF/WAVE
//! PCM16/24/32 and IEEE float32, little endian, interleaved. No compressed codecs or encoding.
use super::*;
use crate::integrations::pipewire::{ConnectionHandle, MediaError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[derive(Clone)]
pub struct SoundAsset {
    samples: Arc<[f32]>,
    rate: u32,
    channels: u32,
}
impl SoundAsset {
    pub fn pcm(samples: Vec<f32>, rate: u32, channels: u32) -> Result<Self, MediaError> {
        AudioFormat {
            sample_format: SampleFormat::F32,
            rate,
            channels,
        }
        .validate()?;
        if samples.len() > 16 * 1024 * 1024
            || samples.len() % channels as usize != 0
            || samples.iter().any(|s| !s.is_finite())
        {
            return Err(MediaError::InvalidArgument("sound samples"));
        }
        Ok(Self {
            samples: samples.into(),
            rate,
            channels,
        })
    }
    /// The caller owns IO and must not call decoding from realtime callbacks. Encoded and
    /// decoded data are each limited to 64 MiB. Unknown RIFF chunks are skipped with padding.
    pub fn decode_wav(bytes: &[u8]) -> Result<Self, MediaError> {
        let invalid = || MediaError::InvalidArgument("WAVE file");
        if bytes.len() > 64 * 1024 * 1024
            || bytes.get(..4) != Some(b"RIFF")
            || bytes.get(8..12) != Some(b"WAVE")
        {
            return Err(invalid());
        }
        let u16_at = |b: &[u8], i| -> Result<u16, MediaError> {
            Ok(u16::from_le_bytes(
                b.get(i..i + 2)
                    .ok_or_else(invalid)?
                    .try_into()
                    .map_err(|_| invalid())?,
            ))
        };
        let u32_at = |b: &[u8], i| -> Result<u32, MediaError> {
            Ok(u32::from_le_bytes(
                b.get(i..i + 4)
                    .ok_or_else(invalid)?
                    .try_into()
                    .map_err(|_| invalid())?,
            ))
        };
        let end = (u32_at(bytes, 4)? as usize)
            .checked_add(8)
            .ok_or_else(invalid)?;
        if end > bytes.len() || end < 12 {
            return Err(invalid());
        }
        let mut format = None;
        let mut pcm = None;
        let mut offset = 12;
        while offset < end {
            let id = bytes.get(offset..offset + 4).ok_or_else(invalid)?;
            let length = u32_at(bytes, offset + 4)? as usize;
            let finish = (offset + 8).checked_add(length).ok_or_else(invalid)?;
            if finish > end {
                return Err(invalid());
            }
            let data = &bytes[offset + 8..finish];
            if id == b"fmt " {
                if format.is_some() {
                    return Err(invalid());
                }
                format = Some((
                    u16_at(data, 0)?,
                    u16_at(data, 2)? as u32,
                    u32_at(data, 4)?,
                    u16_at(data, 12)? as usize,
                    u16_at(data, 14)? as usize,
                ));
            }
            if id == b"data" {
                if pcm.is_some() {
                    return Err(invalid());
                }
                pcm = Some(data);
            }
            offset = finish.checked_add(length % 2).ok_or_else(invalid)?;
        }
        let (encoding, channels, rate, alignment, bits) = format.ok_or_else(invalid)?;
        let data = pcm.ok_or_else(invalid)?;
        AudioFormat {
            sample_format: SampleFormat::F32,
            rate,
            channels,
        }
        .validate()?;
        let bytes_per_sample = bits / 8;
        if !matches!((encoding, bits), (1, 16 | 24 | 32) | (3, 32)) {
            return Err(MediaError::Unsupported(
                "WAVE codec; supported PCM16/24/32 and float32",
            ));
        }
        if alignment != channels as usize * bytes_per_sample
            || data.len() % alignment != 0
            || data.len() / bytes_per_sample > 16 * 1024 * 1024
        {
            return Err(invalid());
        }
        let samples = data
            .chunks_exact(bytes_per_sample)
            .map(|b| match (encoding, bits) {
                (1, 16) => i16::from_le_bytes(b.try_into().unwrap()) as f32 / 32768.0,
                (1, 24) => {
                    ((i32::from_le_bytes([b[0], b[1], b[2], 0]) << 8) >> 8) as f32 / 8388608.0
                }
                (1, 32) => i32::from_le_bytes(b.try_into().unwrap()) as f32 / 2147483648.0,
                _ => f32::from_le_bytes(b.try_into().unwrap()),
            })
            .collect();
        Self::pcm(samples, rate, channels)
    }
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
    pub fn rate(&self) -> u32 {
        self.rate
    }
    pub fn channels(&self) -> u32 {
        self.channels
    }
    /// The asset's exact source rate is negotiated; PipeWire's adapter resamples to the
    /// device clock. A callback copies from this immutable asset without IO or allocation.
    pub fn play(
        &self,
        connection: ConnectionHandle,
        target: AudioTarget,
        repeat: bool,
    ) -> Result<SoundPlayback, MediaError> {
        self.start(connection, target, repeat, false)
    }
    /// Creates paused playback so gain/mute can be set before the first sample. Call
    /// stream().resume() and observe its Request to begin. Drop releases the stream.
    pub fn prepare(
        &self,
        connection: ConnectionHandle,
        target: AudioTarget,
        repeat: bool,
    ) -> Result<SoundPlayback, MediaError> {
        self.start(connection, target, repeat, true)
    }
    fn start(
        &self,
        connection: ConnectionHandle,
        target: AudioTarget,
        repeat: bool,
        start_paused: bool,
    ) -> Result<SoundPlayback, MediaError> {
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let samples = self.samples.clone();
        let mut offset = 0;
        let callback = move |cycle: AudioCycle<'_>| {
            for output in cycle.samples {
                if offset == samples.len() {
                    if repeat && !samples.is_empty() {
                        offset = 0;
                    } else {
                        *output = 0.0;
                        finished.store(true, Ordering::Release);
                        continue;
                    }
                }
                *output = samples[offset];
                offset += 1;
            }
        };
        let stream = AudioStream::realtime(
            connection,
            AudioConfig {
                name: "Telorgon sound".into(),
                format: AudioFormat {
                    sample_format: SampleFormat::F32,
                    rate: self.rate,
                    channels: self.channels,
                },
                target,
                start_paused,
                ..Default::default()
            },
            callback,
        )?;
        Ok(SoundPlayback { stream, done })
    }
}
/// Retains playback and asset until dropped. is_source_exhausted means the asset has been
/// submitted; queued server/device audio may still be audible for the reported latency.
pub struct SoundPlayback {
    stream: AudioStream,
    done: Arc<AtomicBool>,
}
impl SoundPlayback {
    pub fn stream(&self) -> &AudioStream {
        &self.stream
    }
    pub fn is_source_exhausted(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wav_decodes_signed_extremes_and_rejects_truncation() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend(40u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());
        bytes.extend(96000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(4u32.to_le_bytes());
        bytes.extend(i16::MIN.to_le_bytes());
        bytes.extend(i16::MAX.to_le_bytes());
        let asset = SoundAsset::decode_wav(&bytes).unwrap();
        assert_eq!(asset.samples()[0], -1.0);
        assert!(asset.samples()[1] > 0.999);
        for n in 0..bytes.len() {
            assert!(SoundAsset::decode_wav(&bytes[..n]).is_err());
        }
    }
}
