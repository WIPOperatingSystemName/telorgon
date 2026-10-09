//! Versioned JSON inside D-Bus strings. Validate requests before invoking hardware owners.
use super::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub trait Message: Serialize + DeserializeOwned {
    fn validate_message(&self) -> Result<()>;
}
macro_rules! message {
    ($($ty:ty),* $(,)?) => { $(impl Message for $ty {
        fn validate_message(&self) -> Result<()> { self.validate() }
    })* };
}
message!(
    DisplayConfiguration,
    SoundSettings,
    PersonalizationSettings,
    Preferences
);

impl Message for ShellSnapshot {
    fn validate_message(&self) -> Result<()> {
        self.display.current.validate()?;
        if !self.display.resolved_scale.is_finite()
            || self.display.resolved_scale < 0.0
            || (self.display.ready && !(1.0..=4.0).contains(&self.display.resolved_scale))
        {
            return Err("Invalid observed display scale".into());
        }
        for mode in self
            .display
            .modes
            .iter()
            .chain(self.display.preferred_mode.iter())
        {
            DisplayConfiguration {
                mode: Some(*mode),
                ..Default::default()
            }
            .validate()?;
        }
        if let Some(personalization) = &self.personalization {
            personalization.current.validate()?;
        }
        let gain = |volume: Option<f32>| -> Result<()> {
            if volume.is_some_and(|v| !v.is_finite() || v < 0.0) {
                return Err("Invalid observed audio volume".into());
            }
            Ok(())
        };
        for device in &self.devices {
            gain(device.volume)?;
        }
        if let Some(audio) = &self.audio {
            for node in &audio.nodes {
                gain(node.volume)?;
                if node
                    .balance
                    .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
                {
                    return Err("Invalid observed audio balance".into());
                }
                for channel in &node.channels {
                    gain(channel.volume)?;
                    gain(channel.linear_gain)?;
                }
            }
            for app in &audio.applications {
                gain(app.playback_volume)?;
                gain(app.recording_volume)?;
            }
            for operation in &audio.operations {
                gain(operation.preview_volume)?;
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<T> {
    version: u32,
    payload: T,
}

pub fn encode<T: Message>(message: &T) -> Result<String> {
    message.validate_message()?;
    let text = serde_json::to_string(&Envelope {
        version: 1,
        payload: message,
    })
    .map_err(|e| e.to_string())?;
    if text.len() > MAX_MESSAGE_BYTES {
        return Err("Desktop settings message is too large".into());
    }
    Ok(text)
}
pub fn decode<T: Message>(text: &str) -> Result<T> {
    if text.len() > MAX_MESSAGE_BYTES {
        return Err("Desktop settings message is too large".into());
    }
    let envelope: Envelope<T> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if envelope.version != 1 {
        return Err("Unsupported desktop settings protocol version".into());
    }
    envelope.payload.validate_message()?;
    Ok(envelope.payload)
}
pub fn encode_snapshot(snapshot: &ShellSnapshot) -> Result<String> {
    encode(snapshot)
}
