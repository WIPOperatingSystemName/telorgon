//! Bounded optional settings view over the explicitly supplied desktop-control owner.
use crate::{
    authoring::compose::*,
    host::application::desktop_audio::DesktopAudioHandle,
    integrations::{pipewire::ObjectHandle, wireplumber::DefaultKind},
    services::audio::{Amplification, AudioAction, AudioControlTarget, AudioSystemAction},
};
const PAGE_SIZE: usize = 6;

#[crate::component(no_default)]
pub struct AudioSettings {
    #[input]
    audio: DesktopAudioHandle,
    #[state]
    page: usize,
    #[state]
    show_devices: bool,
    #[state]
    selected_stream: Option<ObjectHandle>,
    #[state]
    admission_error: Option<String>,
}
impl AudioSettings {
    pub fn new(audio: DesktopAudioHandle) -> Self {
        Self {
            audio,
            page: 0,
            show_devices: false,
            selected_stream: None,
            admission_error: None,
        }
    }
    fn execute(&mut self, action: AudioSystemAction) {
        self.admission_error = self
            .audio
            .execute(action)
            .err()
            .map(|error| error.to_string());
    }
    fn adjust(&mut self, target: ObjectHandle, delta_ui: f32) {
        self.execute(AudioSystemAction::AdjustVolume {
            target: AudioControlTarget::Node(target),
            delta_ui,
            amplification: Amplification::Forbid,
        });
    }
}
impl Component for AudioSettings {
    fn view(&self) -> impl View {
        if self.show_devices {
            return column()
                .padding(12.0)
                .gap(10.0)
                .child(
                    button("Back to streams and defaults")
                        .on_press(|this: &mut Self| this.show_devices = false),
                )
                .child(super::audio_devices::AudioDeviceSettings::new(
                    self.audio.clone(),
                ));
        }
        let signal = self.audio.signal();
        let snapshot = self.watch(&signal);
        let last_page = snapshot.nodes.len().saturating_sub(1) / PAGE_SIZE;
        let page = self.page.min(last_page);
        let selected = self
            .selected_stream
            .and_then(|handle| snapshot.nodes.iter().find(|node| node.handle == handle));
        let mut content = column()
            .gap(10.0)
            .padding(12.0)
            .child(text("Devices and application streams"))
            .child(
                button("Device profiles and routes")
                    .on_press(|this: &mut Self| this.show_devices = true),
            )
            .child(
                text(format!(
                    "{:?} · {} pending",
                    snapshot.state, snapshot.pending
                ))
                .size(12.0),
            );
        if let Some(stream) = selected {
            content = content.child(
                row()
                    .gap(8.0)
                    .child(text(format!("Move: {}", stream.description)))
                    .child(
                        button("Cancel selection")
                            .on_press(|this: &mut Self| this.selected_stream = None),
                    ),
            );
        } else {
            content = content
                .child(text("Select an application stream to choose its destination.").size(12.0));
        }
        if snapshot.nodes.is_empty() {
            content = content.child(text("No audio devices or streams available."));
        }
        for node in snapshot.nodes.iter().skip(page * PAGE_SIZE).take(PAGE_SIZE) {
            let target = node.handle;
            let kind = match node.media_class.as_str() {
                "Audio/Sink" => Some(DefaultKind::Output),
                "Audio/Source" => Some(DefaultKind::Input),
                _ => None,
            };
            let is_default =
                snapshot.default_output == Some(target) || snapshot.default_input == Some(target);
            let gain = node
                .channel_volumes
                .iter()
                .copied()
                .max_by(|a, b| a.value().total_cmp(&b.value()))
                .or(node.volume);
            let level = if node.mute == Some(true) {
                "Muted".to_owned()
            } else {
                gain.map_or("Volume unavailable".into(), |gain| {
                    format!("{:.0}%", gain.as_ui() * 100.0)
                })
            };
            let description = if node.description.is_empty() {
                &node.name
            } else {
                &node.description
            };
            let mut controls = row()
                .height(32.0)
                .gap(8.0)
                .child(text(level).size(13.0))
                .child(
                    button("−")
                        .width(32.0)
                        .enabled(node.can_set_volume && gain.is_some())
                        .on_press(move |this: &mut Self| this.adjust(target, -0.05)),
                )
                .child(
                    button("+")
                        .width(32.0)
                        .enabled(node.can_set_volume && gain.is_some())
                        .on_press(move |this: &mut Self| this.adjust(target, 0.05)),
                )
                .child(
                    button(if node.mute == Some(true) {
                        "Unmute"
                    } else {
                        "Mute"
                    })
                    .enabled(node.can_set_volume && node.mute.is_some())
                    .on_press(move |this: &mut Self| {
                        this.execute(AudioSystemAction::ToggleMute {
                            target: AudioControlTarget::Node(target),
                        })
                    }),
                );
            if let Some(kind) = kind {
                controls = controls.child(
                    button(if is_default {
                        "Default"
                    } else {
                        "Use by default"
                    })
                    .enabled(!is_default && snapshot.pending == 0)
                    .on_press(move |this: &mut Self| {
                        this.execute(AudioSystemAction::Direct(AudioAction::Default {
                            target,
                            kind,
                        }))
                    }),
                );
                if let Some(stream) = selected.filter(|stream| {
                    matches!(
                        (stream.media_class.as_str(), node.media_class.as_str()),
                        ("Stream/Output/Audio", "Audio/Sink")
                            | ("Stream/Input/Audio", "Audio/Source")
                    )
                }) {
                    let stream = stream.handle;
                    controls = controls.child(
                        button("Move here").enabled(snapshot.pending == 0).on_press(
                            move |this: &mut Self| {
                                this.execute(AudioSystemAction::Direct(AudioAction::MoveStream {
                                    stream,
                                    target,
                                }))
                            },
                        ),
                    );
                }
            } else if matches!(
                node.media_class.as_str(),
                "Stream/Output/Audio" | "Stream/Input/Audio"
            ) {
                controls = controls.child(
                    button(if self.selected_stream == Some(target) {
                        "Selected"
                    } else {
                        "Choose destination"
                    })
                    .on_press(move |this: &mut Self| this.selected_stream = Some(target)),
                );
            }
            content = content.child(
                column()
                    .gap(3.0)
                    .child(text(format!("{} · {}", description, node.media_class)).size(14.0))
                    .child(controls),
            );
        }
        if last_page > 0 {
            content =
                content.child(
                    row()
                        .gap(10.0)
                        .height(32.0)
                        .child(
                            button("Previous").enabled(page > 0).on_press(
                                move |this: &mut Self| this.page = page.saturating_sub(1),
                            ),
                        )
                        .child(text(format!("{} / {}", page + 1, last_page + 1)))
                        .child(button("Next").enabled(page < last_page).on_press(
                            move |this: &mut Self| this.page = (page + 1).min(last_page),
                        )),
                );
        }
        content
            .child(
                text("Move requests confirm policy acceptance; routing may still be changing.")
                    .size(12.0),
            )
            .child(
                text(
                    self.admission_error
                        .clone()
                        .or_else(|| snapshot.last_error.as_ref().map(ToString::to_string))
                        .unwrap_or_default(),
                )
                .size(12.0),
            )
    }
}
