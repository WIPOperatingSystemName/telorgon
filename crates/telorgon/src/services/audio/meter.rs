//! Desktop-node metering is explicit capture, separate from controls and stream-local taps.
use super::*;
use crate::media::audio::{
    AudioConfig, AudioDirection, AudioFormat, AudioLevels, AudioMeter, AudioState, AudioStream,
    AudioTarget,
};

/// Owns a monitor capture stream; Drop requests its stop. It retains no recorded samples.
/// The supplied connection must stay alive. No target replacement is followed implicitly.
pub struct AudioNodeMeter {
    stream: AudioStream,
    meter: AudioMeter,
    target: ObjectHandle,
}
impl AudioNodeMeter {
    pub fn target(&self) -> ObjectHandle {
        self.target
    }
    pub fn state(&self) -> AudioState {
        self.stream.state()
    }
    pub fn read(&mut self) -> Option<AudioLevels> {
        self.meter.read()
    }
    pub fn stop(&self) {
        self.stream.stop();
    }
}
impl AudioControls {
    /// Start explicit metering capture for this exact node. Input devices use their source,
    /// outputs use monitor capture, and application playback nodes use stream capture.
    /// Requires both desktop-audio-linux and audio-linux. Samples are discarded after peak/
    /// RMS accumulation; permissions and active capture indicators still apply normally.
    /// Format negotiation/readiness is asynchronous and observed through AudioNodeMeter::state.
    pub fn open_meter(
        &self,
        target: ObjectHandle,
        format: AudioFormat,
    ) -> Result<AudioNodeMeter, MediaError> {
        let snapshot = self.connection.snapshot();
        let node = snapshot.resolve(target)?;
        let selection = match node.media_class() {
            Some("Audio/Source") => AudioTarget::Node(target),
            Some("Audio/Sink") => AudioTarget::SystemOutput(target),
            Some("Stream/Output/Audio") => AudioTarget::Application(target),
            _ => {
                return Err(MediaError::Unsupported(
                    "node does not expose a meter capture target",
                ));
            }
        };
        let stream = AudioStream::realtime(
            self.connection.clone(),
            AudioConfig {
                name: "Telorgon level meter".into(),
                direction: AudioDirection::Capture,
                target: selection,
                format,
                ..Default::default()
            },
            |_: crate::media::audio::AudioCycle<'_>| {},
        )?;
        let meter = stream.meter();
        Ok(AudioNodeMeter {
            stream,
            meter,
            target,
        })
    }
}
