//! Optional volume presentation over the host's explicitly created desktop-audio owner.
use crate::authoring::compose::*;
use crate::host::application::desktop_audio::DesktopAudioHandle;
use crate::services::audio::{Amplification, AudioControlTarget, AudioSystemAction};

#[crate::component(no_default)]
pub struct AudioVolume {
    #[input]
    audio: DesktopAudioHandle,
    #[input]
    microphone: bool,
    #[state]
    admission_error: Option<String>,
}
impl AudioVolume {
    pub fn new(audio: DesktopAudioHandle) -> Self {
        Self {
            audio,
            microphone: false,
            admission_error: None,
        }
    }
    pub fn for_microphone(audio: DesktopAudioHandle) -> Self {
        Self {
            audio,
            microphone: true,
            admission_error: None,
        }
    }
    fn target(&self) -> AudioControlTarget {
        if self.microphone {
            AudioControlTarget::DefaultInput
        } else {
            AudioControlTarget::DefaultOutput
        }
    }
    fn adjust(&mut self, delta_ui: f32) {
        self.admission_error = self
            .audio
            .execute(AudioSystemAction::AdjustVolume {
                target: self.target(),
                delta_ui,
                amplification: Amplification::Forbid,
            })
            .err()
            .map(|error| error.to_string());
    }
    fn toggle(&mut self) {
        self.admission_error = self
            .audio
            .execute(AudioSystemAction::ToggleMute {
                target: self.target(),
            })
            .err()
            .map(|error| error.to_string());
    }
}
impl Component for AudioVolume {
    fn view(&self) -> impl View {
        let signal = self.audio.signal();
        let snapshot = self.watch(&signal);
        let target = if self.microphone {
            snapshot.default_input
        } else {
            snapshot.default_output
        };
        let node =
            target.and_then(|target| snapshot.nodes.iter().find(|node| node.handle == target));
        let gain = node.and_then(|node| {
            node.channel_volumes
                .iter()
                .copied()
                .max_by(|a, b| a.value().total_cmp(&b.value()))
                .or(node.volume)
        });
        let label = match node {
            Some(node) if node.mute == Some(true) => "Muted".to_owned(),
            Some(_) => gain.map_or("Volume unavailable".into(), |gain| {
                format!("{:.0}%", gain.as_ui() * 100.0)
            }),
            None => "No default device".into(),
        };
        let enabled = node.is_some_and(|node| node.can_set_volume);
        let status = self
            .admission_error
            .clone()
            .or_else(|| snapshot.last_error.as_ref().map(ToString::to_string))
            .unwrap_or_else(|| {
                if snapshot.pending > 0 {
                    format!("{} pending", snapshot.pending)
                } else {
                    String::new()
                }
            });
        column()
            .gap(4.0)
            .child(
                text(if self.microphone {
                    "Microphone"
                } else {
                    "Sound"
                })
                .size(13.0),
            )
            .child(
                row()
                    .height(32.0)
                    .gap(6.0)
                    .child(
                        button("−")
                            .width(32.0)
                            .enabled(enabled && gain.is_some())
                            .on_press(|this: &mut Self| this.adjust(-0.05)),
                    )
                    .child(text(label).size(14.0))
                    .child(
                        button("+")
                            .width(32.0)
                            .enabled(enabled && gain.is_some())
                            .on_press(|this: &mut Self| this.adjust(0.05)),
                    )
                    .child(
                        button(if node.is_some_and(|node| node.mute == Some(true)) {
                            "Unmute"
                        } else {
                            "Mute"
                        })
                        .enabled(enabled && node.is_some_and(|node| node.mute.is_some()))
                        .on_press(|this: &mut Self| this.toggle()),
                    ),
            )
            .child(text(status).size(12.0))
    }
}
