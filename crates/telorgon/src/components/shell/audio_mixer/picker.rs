use super::*;
use crate::services::audio::{AudioDevice, DeviceChoice};

pub(super) fn choice(label: String, active: bool, available: bool) -> Button {
    button(label)
        .height(68.0)
        .padding(8.0)
        .corner_radius(7.0)
        .background(Background::Color(if active {
            crate::ColorRgba8::rgba(40, 70, 88, 255)
        } else {
            crate::ColorRgba8::rgba(39, 46, 59, 255)
        }))
        .uniform_border(
            1.0,
            if active {
                crate::ColorRgba8::rgba(104, 190, 180, 255)
            } else {
                crate::ColorRgba8::rgba(63, 73, 89, 255)
            },
        )
        .enabled(available)
}

fn label(choice: &DeviceChoice) -> &str {
    if choice.description.is_empty() {
        &choice.name
    } else {
        &choice.description
    }
}

impl AudioMixerPanel {
    pub(super) fn device_card(&self, snapshot: &MixerSnapshot, device: &AudioDevice) -> Container {
        let target = device.handle;
        let pending = snapshot
            .operations
            .get(&MixerTarget::Node(target))
            .is_some_and(|o| o.pending);
        let mut height = 100.0;
        let mut card = column()
            .key(format!("hardware-{target:?}"))
            .padding(12.0)
            .gap(8.0)
            .background(Background::Color(crate::ColorRgba8::rgba(30, 36, 48, 255)))
            .uniform_border(1.0, crate::ColorRgba8::rgba(63, 73, 89, 255))
            .corner_radius(10.0)
            .child(
                column().height(48.0).child(
                    text(if device.description.is_empty() {
                        &device.name
                    } else {
                        &device.description
                    })
                    .size(15.0)
                    .weight(600),
                ),
            )
            .child(
                text(if pending {
                    "Applying changes…"
                } else {
                    "Device settings"
                })
                .size(12.0),
            );

        if !device.profiles.is_empty() {
            height += 32.0;
            card = card.child(text("Sound mode · profiles").size(13.0).weight(600));
            for profile in &device.profiles {
                let index = profile.index;
                let active = device.active_profile == Some(index);
                let available = profile.available != Some(false);
                let status = if active {
                    "Selected"
                } else if !available {
                    "Unavailable"
                } else {
                    "Select mode"
                };
                height += 76.0;
                card = card.child(
                    choice(format!("{}\n{status}", label(profile)), active, available)
                        .enabled(available && device.can_set_profile && !pending && !active)
                        .on_press(move |this: &mut Self| {
                            this.execute(MixerAction::Device(AudioAction::Profile {
                                target,
                                index,
                            }));
                        }),
                );
            }
        }

        height += 32.0;
        card = card.child(
            text("Connections · ports and routes")
                .size(13.0)
                .weight(600),
        );
        let mut route_count = 0;
        for route in device.routes.iter().filter(|route| {
            device.active_profile.is_none_or(|profile| {
                route.profiles.is_empty() || route.profiles.contains(&profile)
            })
        }) {
            let ids: std::collections::BTreeSet<_> = route
                .device
                .into_iter()
                .chain(route.devices.iter().copied())
                .collect();
            for route_device in &ids {
                let route_device = *route_device;
                let index = route.index;
                let active = device
                    .active_routes
                    .iter()
                    .any(|r| r.index == index && r.device == route_device);
                let available = route.available != Some(false);
                let status = if active {
                    "Selected"
                } else if !available {
                    "Unavailable"
                } else {
                    "Select connection"
                };
                let endpoint = if ids.len() > 1 {
                    format!(" · endpoint {route_device}")
                } else {
                    String::new()
                };
                height += 76.0;
                route_count += 1;
                card = card.child(
                    choice(
                        format!("{}\n{status}{endpoint}", label(route)),
                        active,
                        available,
                    )
                    .enabled(available && device.can_set_route && !pending && !active)
                    .on_press(move |this: &mut Self| {
                        this.execute(MixerAction::Device(AudioAction::Route {
                            target,
                            index,
                            device: route_device,
                        }));
                    }),
                );
            }
        }
        if route_count == 0 {
            height += 48.0;
            card = card.child(
                column()
                    .height(40.0)
                    .child(text("No connections listed for the current mode.").size(12.0)),
            );
        }
        if let Some(error) = snapshot
            .operations
            .get(&MixerTarget::Node(target))
            .and_then(|o| o.error.as_ref())
        {
            height += 64.0;
            card = card.child(
                column()
                    .height(56.0)
                    .child(text(error.to_string()).size(12.0)),
            );
        }
        card.height(height)
    }
}
