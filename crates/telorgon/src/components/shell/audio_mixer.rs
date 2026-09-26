//! Reusable mixer presentation over the shared, explicitly created audio owner.
use crate::{
    Background,
    authoring::compose::*,
    host::application::audio_mixer::{AudioMixerHandle, MixerAction, MixerSnapshot},
    integrations::pipewire::{ConnectionState, ObjectHandle, Request, RequestState},
    services::audio::{AudioAction, AudioNode, mixer::*},
};

mod picker;

fn column() -> Container {
    // Expanded routing groups must retain their full height inside the viewport.
    crate::authoring::compose::column().box_style(crate::ui::BoxStyle {
        width: crate::ui::SizeRule::Fill(1.0),
        height: crate::ui::SizeRule::Shrink,
        max_size: crate::ui::SizeRule2D {
            width: crate::ui::SizeRule::Fill(1.0),
            height: crate::ui::SizeRule::Logical(f32::MAX),
        },
        ..Default::default()
    })
}
fn text(value: impl ToString) -> Text {
    crate::authoring::compose::text(value).color(crate::ColorRgba8::rgba(232, 235, 243, 255))
}
fn button(label: impl Into<String>) -> Button {
    crate::authoring::compose::button().child(text(label.into()))
        .width(Dimension::FILL)
        .height(36.0)
        .corner_radius(5.0)
}

#[crate::component(no_default)]
pub struct AudioMixerControl {
    #[input]
    mixer: AudioMixerHandle,
    #[input]
    target: MixerTarget,
    #[input]
    label: String,
    #[state]
    error: Option<String>,
    #[state]
    preview: Option<f32>,
    #[state]
    request: Option<Request>,
}
impl AudioMixerControl {
    pub fn new(mixer: AudioMixerHandle, target: MixerTarget, label: impl Into<String>) -> Self {
        Self {
            mixer,
            target,
            label: label.into(),
            error: None,
            preview: None,
            request: None,
        }
    }
    fn execute(&mut self, action: MixerAction) {
        match self.mixer.execute(action) {
            Ok(request) => {
                self.error = None;
                self.request = Some(request);
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.preview = None;
            }
        }
    }
}
impl Component for AudioMixerControl {
    fn view(&self) -> impl View {
        let signal = self.mixer.signal();
        let snapshot = self.watch(&signal);
        let nodes = snapshot.targets(&self.target);
        let mute = group_mute(&nodes);
        let live_preview = self
            .request
            .as_ref()
            .is_some_and(|r| !matches!(r.state(), RequestState::Complete(_)));
        let volume = if live_preview { self.preview } else { None }
            .or_else(|| snapshot.volume(&self.target));
        let enabled = snapshot.state == ConnectionState::Ready
            && !nodes.is_empty()
            && nodes.iter().all(|n| n.can_set_volume);
        let mute_label = match mute {
            MuteState::Muted => "Unmute",
            MuteState::Mixed => "Mute all",
            _ => "Mute",
        };
        let operation = snapshot.operations.get(&self.target);
        let error = self.error.clone().or_else(|| {
            operation.and_then(|o| {
                o.error.as_ref().map(|error| {
                    if o.applied > 0 {
                        format!("{} applied, {} failed: {error}", o.applied, o.failed)
                    } else {
                        error.to_string()
                    }
                })
            })
        });
        let mut content = column()
            .height(
                84.0 + if error.is_some() { 40.0 } else { 0.0 }
                    + if mute == MuteState::Muted { 20.0 } else { 0.0 },
            )
            .gap(4.0)
            .child(
                row()
                    .height(32.0)
                    .gap(8.0)
                    .child(
                        text(if operation.is_some_and(|o| o.pending) {
                            format!("{} …", self.label)
                        } else {
                            self.label.clone()
                        })
                        .size(14.0),
                    )
                    .child(spacer())
                    .child(
                        text(
                            volume
                                .map(|v| format!("{:.0}%", v * 100.0))
                                .unwrap_or_else(|| "—".into()),
                        )
                        .size(13.0),
                    )
                    .child(
                        button(mute_label)
                            .height(32.0)
                            .width(72.0)
                            .enabled(enabled && mute != MuteState::Unknown)
                            .on_press(|this: &mut Self| {
                                this.preview = None;
                                this.execute(MixerAction::ToggleMute(this.target.clone()));
                            }),
                    ),
            )
            .child(
                slider(self.label.clone(), volume.unwrap_or(0.0))
                    .width(Dimension::FILL)
                    .enabled(enabled && volume.is_some())
                    .on_change(|this: &mut Self, value| {
                        this.preview = Some(value);
                        this.execute(MixerAction::SetVolume {
                            target: this.target.clone(),
                            volume: value,
                        });
                    }),
            );
        if mute == MuteState::Muted {
            content = content.child(text("Muted").size(12.0));
        }
        if let Some(error) = error {
            content = content.child(text(error).size(12.0));
        }
        content
    }
}

#[crate::component(no_default)]
pub struct AudioMixerPanel {
    #[input]
    mixer: AudioMixerHandle,
    #[state]
    expanded: Vec<(ApplicationGroupId, StreamDirection)>,
    #[state]
    output_choices: bool,
    #[state]
    advanced: bool,
    #[state]
    input_choices: bool,
    #[state]
    routing: Option<ObjectHandle>,
    #[state]
    error: Option<String>,
}
impl AudioMixerPanel {
    pub fn new(mixer: AudioMixerHandle) -> Self {
        Self {
            mixer,
            expanded: vec![],
            advanced: false,
            output_choices: false,
            input_choices: false,
            routing: None,
            error: None,
        }
    }
    fn execute(&mut self, action: MixerAction) {
        self.error = self.mixer.execute(action).err().map(|e| e.to_string());
    }
    fn devices(&self, snapshot: &MixerSnapshot, input: bool) -> Container {
        let selected = if input {
            snapshot.default_input
        } else {
            snapshot.default_output
        };
        let count = snapshot
            .nodes
            .iter()
            .filter(|n| n.media_class == if input { "Audio/Source" } else { "Audio/Sink" })
            .count();
        let mut height = (count as f32 * 76.0 + 24.0).max(48.0);
        let mut choices = column()
            .padding(12.0)
            .gap(8.0)
            .background(Background::Color(crate::ColorRgba8::rgba(30, 36, 48, 255)))
            .corner_radius(8.0);
        for node in snapshot
            .nodes
            .iter()
            .filter(|n| n.media_class == if input { "Audio/Source" } else { "Audio/Sink" })
        {
            let device = node.handle;
            let label = format!(
                "{}{}",
                if selected == Some(device) { "✓ " } else { "" },
                node_label(node)
            );
            choices = choices.child(
                picker::choice(label, selected == Some(device), true)
                    .enabled(snapshot.state == ConnectionState::Ready)
                    .on_press(move |this: &mut Self| {
                        this.execute(MixerAction::SetDefault { device, input });
                        if input {
                            this.input_choices = false;
                        } else {
                            this.output_choices = false;
                        }
                    }),
            );
            if let Some(error) = snapshot
                .operations
                .get(&MixerTarget::Node(device))
                .and_then(|s| s.error.as_ref())
            {
                height += 64.0;
                choices = choices.child(column().height(56.0).child(text(error.to_string()).size(12.0)));
            }
        }
        if count == 0 {
            choices = choices.child(text("No devices available.").size(12.0));
        }
        choices.height(height)
    }
    fn stream_height(
        &self,
        snapshot: &MixerSnapshot,
        stream: &AudioNode,
        direction: StreamDirection,
    ) -> f32 {
        let devices = snapshot
            .nodes
            .iter()
            .filter(|n| {
                n.media_class
                    == if direction == StreamDirection::Playback {
                        "Audio/Sink"
                    } else {
                        "Audio/Source"
                    }
            })
            .count();
        16.0 + control_height(snapshot, &MixerTarget::Node(stream.handle))
            + 40.0
            + if self.routing == Some(stream.handle) {
                devices as f32 * 40.0 + 24.0
            } else {
                0.0
            }
    }
    fn stream(
        &self,
        snapshot: &MixerSnapshot,
        stream: &AudioNode,
        direction: StreamDirection,
    ) -> Container {
        let id = stream.handle;
        let current = snapshot
            .destinations
            .get(&id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| snapshot.nodes.iter().find(|n| n.handle == *id))
                    .map(node_label)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Routing unknown".into());
        let mut row = column()
            .height(self.stream_height(snapshot, stream, direction))
            .key(format!("stream-{id:?}"))
            .gap(4.0)
            .padding(8.0)
            .child(AudioMixerControl::new(
                self.mixer.clone(),
                MixerTarget::Node(id),
                node_label(stream),
            ))
            .child(
                button(format!("Destination: {current}")).on_press(move |this: &mut Self| {
                    this.routing = if this.routing == Some(id) {
                        None
                    } else {
                        Some(id)
                    };
                }),
            );
        if self.routing == Some(id) {
            for device in snapshot.nodes.iter().filter(|n| {
                n.media_class
                    == if direction == StreamDirection::Playback {
                        "Audio/Sink"
                    } else {
                        "Audio/Source"
                    }
            }) {
                let destination = device.handle;
                row = row.child(button(node_label(device)).on_press(move |this: &mut Self| {
                    this.execute(MixerAction::MoveStream {
                        stream: id,
                        destination,
                    });
                    this.routing = None;
                }));
            }
            row = row.child(text("Destination updates after routing is observed.").size(11.0));
        }
        row
    }
}
fn control_height(snapshot: &MixerSnapshot, target: &MixerTarget) -> f32 {
    84.0 + if snapshot
        .operations
        .get(target)
        .is_some_and(|s| s.error.is_some())
    {
        40.0
    } else {
        0.0
    } + if group_mute(&snapshot.targets(target)) == MuteState::Muted {
        20.0
    } else {
        0.0
    }
}
fn node_label(node: &AudioNode) -> String {
    if node.description.is_empty() {
        node.name.clone()
    } else {
        node.description.clone()
    }
}
impl Component for AudioMixerPanel {
    fn view(&self) -> impl View {
        let signal = self.mixer.signal();
        let snapshot = self.watch(&signal);
        let mut content = column()
            .gap(12.0)
            .padding(16.0)
            .width(Dimension::FILL)
            .height(Dimension::FILL)
            .scrollable()
            .child(text("Sound mixer").size(20.0));
        if snapshot.state != ConnectionState::Ready {
            return content.child(
                text(match &snapshot.state {
                    ConnectionState::Failed(error) => {
                        format!("Audio unavailable: {error}. Reconnecting…")
                    }
                    _ => "Connecting to audio…".into(),
                })
                .size(13.0),
            );
        }
        for (input, title, selected, show) in [
            (
                false,
                "Output",
                snapshot.default_output,
                self.output_choices,
            ),
            (
                true,
                "Microphone",
                snapshot.default_input,
                self.input_choices,
            ),
        ] {
            let name = selected
                .and_then(|id| snapshot.nodes.iter().find(|n| n.handle == id))
                .map(node_label)
                .unwrap_or_else(|| "No device".into());
            content = content.child(text(title).size(15.0).weight(600));
            let toggle = if show { "▴ Hide choices" } else { "▾ Change device" };
            content = content.child(picker::choice(format!("{name}\n{toggle}"), false, true).on_press(
                move |this: &mut Self| {
                    if input {
                        this.input_choices = !this.input_choices;
                    } else {
                        this.output_choices = !this.output_choices;
                    }
                },
            ));
            if show {
                content = content.child(self.devices(&snapshot, input));
            }
            content = content.child(AudioMixerControl::new(
                self.mixer.clone(),
                if input {
                    MixerTarget::DefaultInput
                } else {
                    MixerTarget::DefaultOutput
                },
                title,
            ));
        }
        content = content.child(text("Applications").size(17.0));
        if snapshot.applications.is_empty() {
            content = content.child(text("No applications using audio.").size(13.0));
        }
        let catalog = self
            .try_context::<ShellContext>()
            .map(|shell| shell.applications());
        for app in &snapshot.applications {
            let app_id = match &app.id {
                ApplicationGroupId::Application(id) => Some(ApplicationId::new(id)),
                _ => None,
            };
            let metadata = catalog
                .as_ref()
                .zip(app_id.as_ref())
                .and_then(|(catalog, id)| catalog.get(id));
            let name = metadata
                .as_ref()
                .map(|m| m.name.clone())
                .unwrap_or_else(|| app.name.clone());
            for direction in [StreamDirection::Playback, StreamDirection::Recording] {
                let streams = app.streams(direction);
                if streams.is_empty() {
                    continue;
                }
                let key = (app.id.clone(), direction);
                let expanded = self.expanded.contains(&key);
                let target = MixerTarget::Application(app.id.clone(), direction);
                let label = if direction == StreamDirection::Recording {
                    format!("{name} · recording")
                } else {
                    name.clone()
                };
                let mut header = row().height(28.0).gap(6.0);
                if let Some(catalog) = &catalog {
                    let icon = app
                        .icon_name
                        .as_ref()
                        .map(|name| {
                            catalog.resolve_named_icon(name, IconRequest::new().logical_size(24))
                        })
                        .or_else(|| {
                            app_id.as_ref().map(|id| {
                                catalog.resolve_icon(id, IconRequest::new().logical_size(24))
                            })
                        });
                    if let Some(icon) = icon {
                        header = header.child(image(icon).width(24.0).height(24.0));
                    }
                }
                header = header.child(text(label.clone()).size(14.0));
                let group_height = 28.0
                    + 8.0
                    + control_height(&snapshot, &target)
                    + 36.0
                    + if expanded {
                        streams
                            .iter()
                            .map(|stream| self.stream_height(&snapshot, stream, direction) + 4.0)
                            .sum::<f32>()
                    } else {
                        0.0
                    };
                let mut group = column()
                    .height(group_height)
                    .key(format!("app-{key:?}"))
                    .gap(4.0)
                    .child(header)
                    .child(AudioMixerControl::new(self.mixer.clone(), target, "Volume"))
                    .child(
                        button(format!(
                            "{} {} stream{} / routing",
                            if expanded { "▾" } else { "▸" },
                            streams.len(),
                            if streams.len() == 1 { "" } else { "s" }
                        ))
                        .on_press(move |this: &mut Self| {
                            if let Some(index) =
                                this.expanded.iter().position(|value| value == &key)
                            {
                                this.expanded.remove(index);
                            } else {
                                this.expanded.push(key.clone());
                            }
                        }),
                    );
                if expanded {
                    for stream in streams {
                        group = group.child(self.stream(&snapshot, stream, direction));
                    }
                }
                content = content.child(group);
            }
        }
        content = content.child(
            button(if self.advanced {
                "▾ Hardware settings"
            } else {
                "▸ Hardware settings"
            })
            .on_press(|this: &mut Self| this.advanced = !this.advanced),
        );
        if self.advanced {
            if snapshot.devices.is_empty() {
                content = content.child(text("No configurable audio devices.").size(13.0));
            }
            for device in &snapshot.devices {
                content = content.child(self.device_card(&snapshot, device));
            }
        }
        if let Some(error) = &self.error {
            content = content.child(text(error.clone()).size(12.0));
        }
        content
    }
}
