//! Device profile/route settings; controls never invent unadvertised device indexes.
use crate::{
    authoring::compose::*,
    host::application::desktop_audio::DesktopAudioHandle,
    services::audio::{AudioAction, AudioSystemAction},
};
const PAGE: usize = 3;
#[crate::component(no_default)]
pub struct AudioDeviceSettings {
    #[input]
    audio: DesktopAudioHandle,
    #[state]
    device_index: usize,
    #[state]
    profile_page: usize,
    #[state]
    route_page: usize,
    #[state]
    error: Option<String>,
}
impl AudioDeviceSettings {
    pub fn new(audio: DesktopAudioHandle) -> Self {
        Self {
            audio,
            device_index: 0,
            profile_page: 0,
            route_page: 0,
            error: None,
        }
    }
    fn apply(&mut self, action: AudioAction) {
        self.error = self
            .audio
            .execute(AudioSystemAction::Direct(action))
            .err()
            .map(|error| error.to_string());
    }
    fn device(&mut self, index: usize) {
        self.device_index = index;
        self.profile_page = 0;
        self.route_page = 0;
        self.error = None;
    }
}
impl Component for AudioDeviceSettings {
    fn view(&self) -> impl View {
        let signal = self.audio.signal();
        let snapshot = self.watch(&signal);
        let index = self
            .device_index
            .min(snapshot.devices.len().saturating_sub(1));
        let mut content = column().gap(8.0);
        let Some(device) = snapshot.devices.get(index) else {
            return content.child(text("No audio devices advertising profiles or routes."));
        };
        let target = device.handle;
        let label = if device.description.is_empty() {
            &device.name
        } else {
            &device.description
        };
        let next = (index + 1) % snapshot.devices.len();
        content = content
            .child(
                row()
                    .height(32.0)
                    .gap(8.0)
                    .child(
                        button().child(text("Previous device").color(crate::ColorRgba8::rgba(248, 249, 252, 255)))
                            .enabled(index > 0)
                            .on_press(move |this: &mut Self| this.device(index.saturating_sub(1))),
                    )
                    .child(text(format!(
                        "{} ({}/{})",
                        label,
                        index + 1,
                        snapshot.devices.len()
                    )))
                    .child(
                        button().child(text("Next device").color(crate::ColorRgba8::rgba(248, 249, 252, 255)))
                            .enabled(snapshot.devices.len() > 1)
                            .on_press(move |this: &mut Self| this.device(next)),
                    ),
            )
            .child(text("Profiles").size(15.0));
        let profile_last = device.profiles.len().saturating_sub(1) / PAGE;
        let profile_page = self.profile_page.min(profile_last);
        for profile in device.profiles.iter().skip(profile_page * PAGE).take(PAGE) {
            let choice = profile.index;
            let active = device.active_profile == Some(choice);
            let label = if profile.description.is_empty() {
                &profile.name
            } else {
                &profile.description
            };
            content = content.child(
                row()
                    .height(32.0)
                    .gap(8.0)
                    .child(text(format!(
                        "{}{}",
                        label,
                        if profile.available == Some(false) {
                            " (unavailable)"
                        } else {
                            ""
                        }
                    )))
                    .child(
                        button().child(text(if active { "Active" } else { "Select" }).color(crate::ColorRgba8::rgba(248, 249, 252, 255)))
                            .enabled(
                                !active
                                    && profile.available != Some(false)
                                    && device.can_set_profile
                                    && snapshot.pending == 0,
                            )
                            .on_press(move |this: &mut Self| {
                                this.apply(AudioAction::Profile {
                                    target,
                                    index: choice,
                                })
                            }),
                    ),
            );
        }
        if profile_last > 0 {
            content = content.child(
                row()
                    .height(28.0)
                    .gap(8.0)
                    .child(
                        button().child(text("Previous profiles").color(crate::ColorRgba8::rgba(248, 249, 252, 255)))
                            .enabled(profile_page > 0)
                            .on_press(move |this: &mut Self| {
                                this.profile_page = profile_page.saturating_sub(1)
                            }),
                    )
                    .child(
                        button().child(text("More profiles").color(crate::ColorRgba8::rgba(248, 249, 252, 255)))
                            .enabled(profile_page < profile_last)
                            .on_press(move |this: &mut Self| this.profile_page = profile_page + 1),
                    ),
            );
        }
        content = content.child(text("Routes").size(15.0));
        // At most 64 parameter objects × 64 device indexes from the bounded decoder.
        // Collect only indexes/references; the UI renders at most three choices per page.
        let routes: Vec<_> = device
            .routes
            .iter()
            .filter(|route| {
                device.active_profile.is_none_or(|active| {
                    route.profiles.is_empty() || route.profiles.contains(&active)
                })
            })
            .flat_map(|route| {
                let mut ids = route.devices.clone();
                if let Some(id) = route.device {
                    ids.push(id);
                }
                ids.sort_unstable();
                ids.dedup();
                ids.into_iter().map(move |id| (route, id))
            })
            .collect();
        let route_last = routes.len().saturating_sub(1) / PAGE;
        let route_page = self.route_page.min(route_last);
        for (route, device_id) in routes.iter().skip(route_page * PAGE).take(PAGE) {
            let choice = route.index;
            let device_id = *device_id;
            let active = device
                .active_routes
                .iter()
                .any(|active| active.index == choice && active.device == device_id);
            let label = if route.description.is_empty() {
                &route.name
            } else {
                &route.description
            };
            content = content.child(
                row()
                    .height(32.0)
                    .gap(8.0)
                    .child(text(format!(
                        "{} · port {}{}",
                        label,
                        device_id,
                        if route.available == Some(false) {
                            " (unavailable)"
                        } else {
                            ""
                        }
                    )))
                    .child(
                        button().child(text(if active { "Active" } else { "Select" }).color(crate::ColorRgba8::rgba(248, 249, 252, 255)))
                            .enabled(
                                !active
                                    && route.available != Some(false)
                                    && device.can_set_route
                                    && snapshot.pending == 0,
                            )
                            .on_press(move |this: &mut Self| {
                                this.apply(AudioAction::Route {
                                    target,
                                    index: choice,
                                    device: device_id,
                                })
                            }),
                    ),
            );
        }
        if routes.is_empty() {
            content = content.child(text("No selectable route/device pairs advertised."));
        }
        if route_last > 0 {
            content = content.child(
                row()
                    .height(28.0)
                    .gap(8.0)
                    .child(button().child(text("Previous routes").color(crate::ColorRgba8::rgba(248, 249, 252, 255))).enabled(route_page > 0).on_press(
                        move |this: &mut Self| this.route_page = route_page.saturating_sub(1),
                    ))
                    .child(
                        button().child(text("More routes").color(crate::ColorRgba8::rgba(248, 249, 252, 255)))
                            .enabled(route_page < route_last)
                            .on_press(move |this: &mut Self| this.route_page = route_page + 1),
                    ),
            );
        }
        content
            .child(text(format!("{} pending", snapshot.pending)).size(12.0))
            .child(
                text(
                    self.error
                        .clone()
                        .or_else(|| snapshot.last_error.as_ref().map(ToString::to_string))
                        .unwrap_or_default(),
                )
                .size(12.0),
            )
    }
}
