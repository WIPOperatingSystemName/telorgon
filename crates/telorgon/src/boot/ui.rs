mod minimal;

use super::ui_assets as assets;

use crate::authoring::compose::{
    Alignment, Button, Component, Container, Dimension, Image, InteractionEffect, Signal, Text,
    View, button, column, row, spacer, stack, text,
};
use crate::boot::{
    BootCommand, BootController, BootPhase, BootSnapshot, BootTargetKind, BootTheme,
};
use crate::component;
use crate::foundation::ColorRgba8;
use crate::graphics::render::ImageResource;
use crate::input::{ButtonState, InputEvent, LogicalKey, NamedKey};
use crate::ui::{Border, UiEvent, UiEventKind};

pub(super) const BG: ColorRgba8 = ColorRgba8::rgba(12, 17, 26, 255);
pub(super) const PANEL: ColorRgba8 = ColorRgba8::rgba(22, 29, 41, 255);
pub(super) const LINE: ColorRgba8 = ColorRgba8::rgba(48, 60, 77, 255);
pub(super) const TEXT: ColorRgba8 = ColorRgba8::rgba(237, 242, 250, 255);
pub(super) const MUTED: ColorRgba8 = ColorRgba8::rgba(151, 169, 191, 255);
pub(super) const ACCENT: ColorRgba8 = ColorRgba8::rgba(152, 225, 199, 255);
pub(super) const INK: ColorRgba8 = ColorRgba8::rgba(19, 46, 42, 255);
pub(super) const RED: ColorRgba8 = ColorRgba8::rgba(245, 157, 156, 255);

/// Shared selector and loading views. A native host supplies execution and frame advancement.
#[component(no_default)]
pub struct BootInterface {
    #[input(always)]
    pub(super) controller: BootController,
    #[input]
    signal: Signal<BootSnapshot>,
}

impl BootInterface {
    pub fn new(controller: BootController) -> Self {
        Self {
            signal: controller.signal(),
            controller,
        }
    }

    fn act(&mut self, command: BootCommand) {
        self.controller.dispatch(command);
    }

    pub(super) fn keyboard(&mut self, event: &UiEvent) -> bool {
        let UiEventKind::Input(InputEvent::Key(key)) = &event.kind else {
            return false;
        };
        if key.state != ButtonState::Pressed || key.repeat {
            return false;
        }
        #[cfg(feature = "boot-preview")]
        if self.controller.snapshot().simulation
            && super::preview::ui::keyboard(self, &key.logical_key)
        {
            return true;
        }
        let command = match &key.logical_key {
            LogicalKey::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => {
                BootCommand::MoveSelection(-1)
            }
            LogicalKey::Named(NamedKey::ArrowRight | NamedKey::ArrowDown) => {
                BootCommand::MoveSelection(1)
            }
            LogicalKey::Named(NamedKey::Enter) => {
                if self.controller.snapshot().phase == BootPhase::Failed {
                    BootCommand::Retry
                } else {
                    BootCommand::Launch
                }
            }
            LogicalKey::Named(NamedKey::Escape) => BootCommand::Reset,
            LogicalKey::Character(value) => match value.as_str().to_ascii_lowercase().as_str() {
                "r" => BootCommand::Reset,
                "b" => BootCommand::Launch,
                "t" => BootCommand::SetTheme(match self.controller.snapshot().theme {
                    BootTheme::Disks => BootTheme::Voxel,
                    BootTheme::Voxel => BootTheme::Disks,
                }),
                "1" => BootCommand::Select(0),
                "2" => BootCommand::Select(1),
                "3" => BootCommand::Select(2),
                _ => return false,
            },
            _ => return false,
        };
        self.act(command);
        true
    }

    pub(super) fn action(label: impl ToString, command: BootCommand, active: bool) -> Button {
        let label = label.to_string();
        let width = (label.chars().count() as f32 * 8.0 + 40.0).max(72.0);
        button()
            .width(width)
            .padding((10.0, 15.0))
            .height(38.0)
            .corner_radius(7.0)
            .background(if active {
                ColorRgba8::rgba(42, 64, 65, 255)
            } else {
                PANEL
            })
            .uniform_border(1.0, if active { ACCENT } else { LINE })
            .hover_effect(InteractionEffect::Background(ColorRgba8::rgba(
                49, 65, 77, 255,
            )))
            .child(label_text(label, 13.0, if active { ACCENT } else { TEXT }))
            .on_press(move |this: &mut Self| this.act(command.clone()))
            .on_input(Self::keyboard)
    }

    fn primary(label: impl ToString, command: BootCommand) -> Button {
        let label = label.to_string();
        let width = (label.chars().count() as f32 * 8.0 + 48.0).clamp(160.0, 650.0);
        button()
            .width(width)
            .height(44.0)
            .padding((10.0, 24.0))
            .corner_radius(8.0)
            .background(ACCENT)
            .hover_effect(InteractionEffect::Background(ColorRgba8::rgba(
                184, 244, 221, 255,
            )))
            .child(label_text(label, 15.0, INK).weight(600))
            .on_press(move |this: &mut Self| this.act(command.clone()))
            .on_input(Self::keyboard)
    }

    fn voxel_picker(&self, snapshot: &BootSnapshot) -> Container {
        let compact = self.viewport_size().height < 730.0;
        let capacity = if snapshot.targets.len() > 3 { 2 } else { 3 };
        let start = snapshot
            .selected
            .saturating_sub(capacity / 2)
            .min(snapshot.targets.len().saturating_sub(capacity));
        let visible = snapshot.targets.len().min(capacity);
        let has_navigation = snapshot.targets.len() > capacity;
        let menu_height = visible as f32 * 40.0
            + 44.0
            + if has_navigation { 34.0 } else { 0.0 }
            + (visible + usize::from(has_navigation)) as f32 * 8.0;
        let mut menu = column()
            .key(format!("voxel:{start}"))
            .focus_scope(true)
            .width(370.0)
            .height(menu_height)
            .gap(8.0);
        for (index, target) in snapshot
            .targets
            .iter()
            .enumerate()
            .skip(start)
            .take(capacity)
        {
            let selected = index == snapshot.selected;
            let target_id = target.id.clone();
            let mut label = row()
                .height(26.0)
                .width(Dimension::FILL)
                .gap(8.0)
                .align_items(Alignment::Center);
            if let Some(artwork) = target.icon.as_ref().or(target.splash.as_ref()) {
                label = label.child(contained_artwork(artwork.clone(), 24.0, 24.0, &target.name));
            }
            label = label.child(
                label_text(
                    format!("{}Boot {}", if selected { ">  " } else { "" }, target.name),
                    16.0,
                    ColorRgba8::rgba(252, 250, 236, 255),
                )
                .weight(650),
            );
            menu = menu.child(
                button()
                    .key(format!("world:{}", target.id.as_str()))
                    .height(40.0)
                    .width(Dimension::FILL)
                    .padding((7.0, 12.0))
                    .background(if selected {
                        ColorRgba8::rgba(166, 178, 141, 255)
                    } else {
                        ColorRgba8::rgba(100, 111, 103, 255)
                    })
                    .border_sides(pixel_border(selected))
                    .hover_effect(InteractionEffect::Background(ColorRgba8::rgba(
                        159, 173, 145, 255,
                    )))
                    .child(label)
                    .on_press(move |this: &mut Self| this.act(BootCommand::Boot(target_id.clone())))
                    .on_input(Self::keyboard),
            );
        }
        if snapshot.targets.len() > capacity {
            menu = menu.child(
                row()
                    .height(34.0)
                    .gap(8.0)
                    .child(
                        Self::action("Previous", BootCommand::MoveSelection(-1), false)
                            .width(Dimension::FILL)
                            .height(34.0),
                    )
                    .child(
                        Self::action("Next", BootCommand::MoveSelection(1), false)
                            .width(Dimension::FILL)
                            .height(34.0),
                    ),
            );
        }
        menu = menu.child(
            button()
                .height(44.0)
                .width(Dimension::FILL)
                .padding((8.0, 16.0))
                .background(ColorRgba8::rgba(212, 224, 164, 255))
                .border_sides(pixel_border(true))
                .hover_effect(InteractionEffect::Background(ColorRgba8::rgba(
                    237, 244, 193, 255,
                )))
                .child(
                    label_text(
                        "Enter selected world",
                        16.0,
                        ColorRgba8::rgba(43, 63, 41, 255),
                    )
                    .weight(700),
                )
                .on_press(|this: &mut Self| this.act(BootCommand::Launch))
                .on_input(Self::keyboard),
        );
        column()
            .padding((if compact { 16.0 } else { 20.0 }, 28.0))
            .align_items(Alignment::Center)
            .gap(6.0)
            .child(
                Image::resource(assets::voxel_logo())
                    .width(if compact { 330.0 } else { 410.0 })
                    .height(if compact { 58.0 } else { 72.0 })
                    .accessible_label("Blockboot"),
            )
            .child(
                label_text(
                    "A new world starts here.",
                    14.0,
                    ColorRgba8::rgba(241, 234, 191, 255),
                )
                .weight(600),
            )
            .child(spacer().height(if compact { 0.0 } else { 6.0 }))
            .child(menu)
            .child(spacer())
            .child(label_text(
                "BLOCKBOOT  /  ORIGINAL VOXEL THEME",
                10.0,
                ColorRgba8::rgba(225, 235, 212, 255),
            ))
    }

    fn splash(&self, snapshot: &BootSnapshot) -> Container {
        if snapshot.theme == BootTheme::Disks {
            return self.minimal_splash(snapshot);
        }
        let target = snapshot.targets.get(snapshot.selected);
        let compact = self.viewport_size().height < 730.0;
        let failed = snapshot.phase == BootPhase::Failed;
        let complete = snapshot.phase == BootPhase::Complete;
        let accent = if failed {
            RED
        } else {
            target.map_or(ACCENT, |target| target_color(target.kind))
        };
        let title = match snapshot.phase {
            BootPhase::Loading => target.map_or("Starting up".into(), |target| {
                format!("Starting {}", target.name)
            }),
            BootPhase::OsStarting => "Building your world".into(),
            BootPhase::Complete => "Your world is ready".into(),
            BootPhase::Failed => "Startup interrupted".into(),
            BootPhase::Selecting => "Choose a destination".into(),
        };
        let mut content = column()
            .focus_scope(failed)
            .padding((if compact { 18.0 } else { 24.0 }, 30.0))
            .gap(if compact { 8.0 } else { 10.0 })
            .align_items(Alignment::Center)
            .justify_content(Alignment::Center);
        if let Some(target) =
            target.filter(|target| target.splash.is_some() || target.icon.is_some())
        {
            let (width, height) = if target.splash.is_some() {
                (320.0, 200.0)
            } else {
                (160.0, 160.0)
            };
            content = content.child(contained_artwork(
                target
                    .splash
                    .as_ref()
                    .or(target.icon.as_ref())
                    .unwrap()
                    .clone(),
                width,
                height,
                &target.name,
            ));
        } else {
            content = content.child(
                Image::resource(assets::voxel_logo())
                    .width(350.0)
                    .height(62.0),
            );
        }
        content = content
            .child(
                label_text(
                    title,
                    if compact { 25.0 } else { 28.0 },
                    if failed { RED } else { TEXT },
                )
                .weight(600),
            )
            .child(
                label_text(&snapshot.status, 14.0, TEXT)
                    .text_align(Alignment::Center)
                    .width(650.0),
            )
            .child(spacer().height(if compact { 4.0 } else { 8.0 }));
        if !failed && snapshot.progress_kind.fraction().is_some() {
            content = content
                .child(progress_track(snapshot.progress, accent))
                .child(
                    label_text(format!("{:3.0}%", snapshot.progress * 100.0), 14.0, accent)
                        .weight(600),
                );
        }
        if matches!(snapshot.phase, BootPhase::Loading | BootPhase::OsStarting) {
            let mut activity = row()
                .height(20.0)
                .width(84.0)
                .gap(7.0)
                .align_items(Alignment::Center);
            for index in 0..7 {
                activity = activity.child(
                    column()
                        .width(6.0)
                        .height(6.0)
                        .corner_radius(0.0)
                        .background(if index == (snapshot.frame / 3 % 7) as usize {
                            accent
                        } else {
                            ColorRgba8::rgba(94, 117, 127, 170)
                        }),
                );
            }
            content = content.child(activity).child(phase_steps(snapshot.phase));
        } else if snapshot.simulation
            || (failed && target.is_some_and(|target| target.source.is_some()))
        {
            content = content.child(spacer().height(5.0)).child(
                row()
                    .width(if failed { 380.0 } else { 170.0 })
                    .height(44.0)
                    .gap(10.0)
                    .child(Self::primary(
                        if complete {
                            "Try again"
                        } else {
                            "Retry startup"
                        },
                        if complete {
                            BootCommand::Reset
                        } else {
                            BootCommand::Retry
                        },
                    ))
                    .maybe(
                        failed,
                        Self::action("Choose another OS", BootCommand::Reset, false),
                    ),
            );
        }
        if target.is_some_and(|target| target.kind == BootTargetKind::Windows)
            && !failed
            && !complete
        {
            content = content.child(label_text(
                "Windows supplies its own screen after the loader hands off.",
                12.0,
                MUTED,
            ));
        }
        content
    }

    fn stage(&self, snapshot: &BootSnapshot) -> Container {
        if snapshot.simulation
            && snapshot.phase == BootPhase::OsStarting
            && snapshot
                .targets
                .get(snapshot.selected)
                .is_some_and(|target| target.kind == BootTargetKind::Windows)
        {
            return self.windows_splash(snapshot);
        }
        let screens = if snapshot.phase == BootPhase::Selecting {
            match snapshot.theme {
                BootTheme::Disks => self.disk_picker(snapshot),
                BootTheme::Voxel => self.voxel_picker(snapshot),
            }
        } else {
            self.splash(snapshot)
        };
        if snapshot.theme == BootTheme::Disks {
            return stack()
                .background(if snapshot.phase == BootPhase::Selecting {
                    minimal::BACKGROUND
                } else {
                    ColorRgba8::rgba(0, 0, 0, 255)
                })
                .child(screens);
        }
        stack()
            .corner_radius(18.0)
            .uniform_border(1.0, LINE)
            .child(
                Image::resource(assets::voxel_background())
                    .width(Dimension::FILL)
                    .height(Dimension::FILL),
            )
            .maybe(
                snapshot.theme == BootTheme::Voxel && snapshot.phase != BootPhase::Selecting,
                column().background(ColorRgba8::rgba(10, 23, 22, 190)),
            )
            .child(screens)
    }

    fn windows_splash(&self, snapshot: &BootSnapshot) -> Container {
        let blue = ColorRgba8::rgba(40, 155, 238, 255);
        let mut mark = column().width(104.0).height(104.0).gap(8.0);
        for _ in 0..2 {
            mark = mark.child(
                row()
                    .height(48.0)
                    .gap(8.0)
                    .child(column().width(48.0).background(blue))
                    .child(column().width(48.0).background(blue)),
            );
        }
        let mut dots = row().height(12.0).width(89.0).gap(9.0);
        for index in 0..7 {
            dots = dots.child(
                column()
                    .width(5.0)
                    .height(5.0)
                    .corner_radius(2.5)
                    .background(if index == (snapshot.frame / 3 % 7) as usize {
                        TEXT
                    } else {
                        ColorRgba8::rgba(61, 67, 76, 255)
                    }),
            );
        }
        column()
            .background(ColorRgba8::rgba(5, 7, 12, 255))
            .corner_radius(18.0)
            .center_content()
            .gap(24.0)
            .child(mark)
            .child(label_text("Windows", 22.0, TEXT))
            .child(dots)
            .child(label_text(
                "Windows system startup (simulated)",
                12.0,
                MUTED,
            ))
    }
}

impl Component for BootInterface {
    fn view(&self) -> impl View {
        let snapshot = self.watch(&self.signal);
        column().background(BG).child(self.stage(&snapshot))
    }
}

pub(super) fn label_text(content: impl ToString, size: f32, color: ColorRgba8) -> Text {
    text(content).font_family("Inter").size(size).color(color)
}

fn contained_artwork(
    resource: ImageResource,
    slot_width: f32,
    slot_height: f32,
    label: &str,
) -> Container {
    let width = resource.extent.width.max(1) as f32;
    let height = resource.extent.height.max(1) as f32;
    let scale = (slot_width / width).min(slot_height / height);
    column()
        .width(slot_width)
        .height(slot_height)
        .center_content()
        .child(
            Image::resource(resource)
                .width(width * scale)
                .height(height * scale)
                .accessible_label(label),
        )
}

fn target_color(kind: BootTargetKind) -> ColorRgba8 {
    match kind {
        BootTargetKind::Linux => ColorRgba8::rgba(145, 229, 179, 255),
        BootTargetKind::Windows => ColorRgba8::rgba(125, 188, 248, 255),
        BootTargetKind::Custom => ColorRgba8::rgba(211, 173, 244, 255),
        BootTargetKind::Unknown => ColorRgba8::rgba(174, 184, 196, 255),
    }
}

fn pixel_border(selected: bool) -> Border {
    let top = ColorRgba8::rgba(
        if selected { 232 } else { 175 },
        if selected { 236 } else { 186 },
        if selected { 197 } else { 174 },
        255,
    );
    let bottom = ColorRgba8::rgba(44, 56, 45, 255);
    Border {
        top: crate::ui::BorderSide {
            width: 3.0,
            color: top,
        },
        left: crate::ui::BorderSide {
            width: 3.0,
            color: top,
        },
        right: crate::ui::BorderSide {
            width: 3.0,
            color: bottom,
        },
        bottom: crate::ui::BorderSide {
            width: 3.0,
            color: bottom,
        },
    }
}

fn progress_track(progress: f32, accent: ColorRgba8) -> Container {
    let width = 370.0;
    let progress = progress.clamp(0.0, 1.0);
    row()
        .width(width)
        .height(14.0)
        .background(ColorRgba8::rgba(45, 63, 71, 255))
        .corner_radius(0.0)
        .child(column().width(width * progress).background(accent))
        .child(spacer())
}

fn phase_steps(phase: BootPhase) -> Container {
    let muted = ColorRgba8::rgba(181, 199, 181, 255);
    row()
        .width(320.0)
        .height(20.0)
        .gap(13.0)
        .child(label_text(
            "01  Loader",
            11.0,
            if phase == BootPhase::Loading {
                ACCENT
            } else {
                muted
            },
        ))
        .child(label_text("→", 11.0, muted))
        .child(label_text(
            "02  OS startup",
            11.0,
            if phase == BootPhase::OsStarting {
                ACCENT
            } else {
                muted
            },
        ))
        .child(label_text("→", 11.0, muted))
        .child(label_text("03  Ready", 11.0, muted))
}
