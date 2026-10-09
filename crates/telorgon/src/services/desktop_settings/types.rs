use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub display: DisplayConfiguration,
    pub sound: SoundSettings,
    pub personalization: PersonalizationSettings,
}
impl Preferences {
    pub fn validate(&self) -> Result<()> {
        self.display.validate()?;
        self.sound.validate()?;
        self.personalization.validate()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SoundSettings {
    pub output: SoundChannel,
    pub input: SoundChannel,
}
impl SoundSettings {
    pub fn validate(&self) -> Result<()> {
        self.output.validate()?;
        self.input.validate()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SoundChannel {
    /// PipeWire node name. Empty follows the current default; never persist a live object ID.
    pub device: String,
    /// Normalized UI volume; None leaves the observed volume unchanged.
    pub volume: Option<f32>,
    pub muted: Option<bool>,
}
impl SoundChannel {
    pub fn validate(&self) -> Result<()> {
        if self.device.len() > 1024 || self.device.chars().any(char::is_control) {
            return Err("Invalid audio device name".into());
        }
        if self
            .volume
            .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err("Sound volume must be between 0% and 100%".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonalizationSettings {
    /// A content-addressed filename in this store's background library. None uses the shell default.
    pub background: Option<String>,
}
impl PersonalizationSettings {
    pub fn validate(&self) -> Result<()> {
        if let Some(name) = &self.background {
            let digest = name
                .strip_prefix("background-")
                .and_then(|s| s.strip_suffix(".png"));
            if !digest.is_some_and(|d| {
                d.len() == 64
                    && d.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }) {
                return Err("Invalid managed background filename".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonalizationSnapshot {
    pub current: PersonalizationSettings,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShellSnapshot {
    pub display: DisplaySnapshot,
    pub audio_ready: bool,
    pub audio: Option<AudioSnapshot>,
    pub devices: Vec<AudioDevice>,
    pub audio_error: Option<String>,
    pub personalization: Option<PersonalizationSnapshot>,
}
