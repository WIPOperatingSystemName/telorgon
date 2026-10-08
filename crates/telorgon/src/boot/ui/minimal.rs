use super::{BootInterface, assets, contained_artwork, label_text};
use crate::authoring::compose::{
    Alignment, Button, Component, Container, Dimension, InteractionEffect, button, column, row,
    spacer,
};
use crate::boot::{BootCommand, BootPhase, BootSnapshot, BootTarget};
use crate::foundation::ColorRgba8;

pub(super) const BACKGROUND: ColorRgba8 = ColorRgba8::rgba(245, 245, 247, 255);
const TEXT: ColorRgba8 = ColorRgba8::rgba(29, 29, 31, 255);
const MUTED: ColorRgba8 = ColorRgba8::rgba(119, 119, 127, 255);
const SELECTED: ColorRgba8 = ColorRgba8::rgba(229, 229, 234, 255);
const LINE: ColorRgba8 = ColorRgba8::rgba(207, 207, 214, 255);
const WHITE: ColorRgba8 = ColorRgba8::rgba(246, 246, 246, 255);
const CLEAR: ColorRgba8 = ColorRgba8::rgba(0, 0, 0, 0);

impl BootInterface {
    pub(super) fn disk_picker(&self, snapshot: &BootSnapshot) -> Container {
        let viewport = self.viewport_size();
        let available = viewport.width - 48.0;
        let limit = if viewport.width >= 1050.0 { 4 } else { 3 };
        let mut capacity = (((available + 20.0) / 200.0) as usize).clamp(1, limit);
        if snapshot.targets.len() > capacity {
            capacity = (((available - 128.0 + 20.0) / 200.0) as usize).clamp(1, limit);
        }
        let visible = snapshot.targets.len().min(capacity);
        let start = snapshot
            .selected
            .saturating_sub(capacity / 2)
            .min(snapshot.targets.len().saturating_sub(capacity));
        let navigation = snapshot.targets.len() > capacity;
        let reserved = if navigation { 128.0 } else { 0.0 };
        let width = ((available - reserved - 20.0 * visible.saturating_sub(1) as f32)
            / visible.max(1) as f32)
            .clamp(160.0, 180.0);
        let group_width =
            visible as f32 * width + visible.saturating_sub(1) as f32 * 20.0 + reserved;
        let mut options = row()
            .key(format!("carousel:{start}"))
            .focus_scope(true)
            .width(group_width)
            .height(196.0)
            .gap(20.0)
            .center_content();
        if navigation {
            options = options.child(
                Self::minimal_action("‹", BootCommand::MoveSelection(-1), false)
                    .width(44.0)
                    .accessible_label("Previous OS"),
            );
        }
        for (index, target) in snapshot
            .targets
            .iter()
            .enumerate()
            .skip(start)
            .take(capacity)
        {
            options = options.child(Self::disk_option(target, index == snapshot.selected, width));
        }
        if navigation {
            options = options.child(
                Self::minimal_action("›", BootCommand::MoveSelection(1), false)
                    .width(44.0)
                    .accessible_label("Next OS"),
            );
        }
        column()
            .height(Dimension::FILL)
            .padding(24.0)
            .gap(16.0)
            .center_content()
            .child(label_text("Choose an operating system", 20.0, TEXT).weight(500))
            .child(options)
            .child(label_text(
                if snapshot.simulation {
                    "← → choose · Enter or click to boot"
                } else {
                    "← → to choose · Enter to boot"
                },
                13.0,
                MUTED,
            ))
    }

    fn disk_option(target: &BootTarget, selected: bool, width: f32) -> Container {
        let target_id = target.id.clone();
        let splash_thumbnail = target.icon.is_none() && target.splash.is_some();
        let artwork_size = if splash_thumbnail { 120.0 } else { 136.0 };
        let option = button()
            .key(format!("disk:{}", target.id.as_str()))
            .width(width)
            .height(196.0)
            .padding(12.0)
            .corner_radius(10.0)
            .background(if selected { SELECTED } else { CLEAR })
            .hover_effect(InteractionEffect::Background(SELECTED))
            .accessible_label(format!("Boot {}", target.name))
            .child(
                column()
                    .gap(8.0)
                    .center_content()
                    .child(
                        contained_artwork(
                            target
                                .icon
                                .as_ref()
                                .or(target.splash.as_ref())
                                .cloned()
                                .unwrap_or_else(|| assets::disk_icon(target.kind)),
                            artwork_size,
                            artwork_size,
                            &target.name,
                        )
                        .width(136.0)
                        .height(136.0)
                        // Boot splashes commonly contain light artwork designed
                        // for a dark screen, even when their pixels are transparent.
                        .background(if splash_thumbnail {
                            ColorRgba8::rgba(0, 0, 0, 255)
                        } else {
                            CLEAR
                        })
                        .corner_radius(8.0),
                    )
                    .child(
                        label_text(&target.name, 16.0, TEXT)
                            .weight(500)
                            .text_align(Alignment::Center),
                    ),
            )
            .on_press(move |this: &mut Self| this.act(BootCommand::Boot(target_id.clone())))
            .on_input(Self::keyboard);
        column()
            .key(format!("option:{}", target.id.as_str()))
            .width(width)
            .height(196.0)
            .focus_scope(selected)
            .child(option)
    }

    fn minimal_action(label: &str, command: BootCommand, dark: bool) -> Button {
        button()
            .width((label.chars().count() as f32 * 8.0 + 40.0).max(100.0))
            .height(44.0)
            .padding((10.0, 16.0))
            .corner_radius(8.0)
            .background(if dark {
                ColorRgba8::rgba(37, 37, 40, 255)
            } else {
                WHITE
            })
            .uniform_border(
                1.0,
                if dark {
                    ColorRgba8::rgba(65, 65, 70, 255)
                } else {
                    LINE
                },
            )
            .hover_effect(InteractionEffect::Background(if dark {
                ColorRgba8::rgba(57, 57, 61, 255)
            } else {
                SELECTED
            }))
            .child(label_text(label, 14.0, if dark { WHITE } else { TEXT }).weight(500))
            .on_press(move |this: &mut Self| this.act(command.clone()))
            .on_input(Self::keyboard)
    }

    pub(super) fn minimal_splash(&self, snapshot: &BootSnapshot) -> Container {
        let target = snapshot.targets.get(snapshot.selected);
        let failed = snapshot.phase == BootPhase::Failed;
        let mut content = column()
            .focus_scope(failed)
            .height(Dimension::FILL)
            .padding(28.0)
            .center_content();
        if let Some(target) = target {
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
                    .cloned()
                    .unwrap_or_else(|| assets::boot_mark(target.kind)),
                width,
                height,
                &target.name,
            ));
        }
        if failed {
            content = content
                .child(spacer().height(16.0))
                .child(label_text("Unable to start", 23.0, WHITE).weight(500))
                .child(spacer().height(12.0))
                .child(
                    label_text(&snapshot.status, 14.0, ColorRgba8::rgba(170, 170, 178, 255))
                        .width((self.viewport_size().width - 120.0).clamp(200.0, 650.0))
                        .text_align(Alignment::Center),
                )
                .child(spacer().height(24.0));
            let retry = snapshot.simulation || target.is_some_and(|target| target.source.is_some());
            let mut actions = row()
                .width(if retry { 332.0 } else { 176.0 })
                .height(44.0)
                .gap(12.0);
            if retry {
                actions = actions.child(Self::minimal_action(
                    "Retry startup",
                    BootCommand::Retry,
                    true,
                ));
            }
            return content.child(actions.child(Self::minimal_action(
                "Choose another OS",
                BootCommand::Reset,
                true,
            )));
        }
        content = content
            .child(spacer().height(28.0))
            .child(minimal_progress(snapshot))
            .child(spacer().height(24.0));
        if let Some(target) = target {
            content = content.child(label_text(
                &target.name,
                14.0,
                ColorRgba8::rgba(150, 150, 158, 255),
            ));
        }
        content
    }
}

fn minimal_progress(snapshot: &BootSnapshot) -> Container {
    const WIDTH: f32 = 180.0;
    let mut track = row()
        .width(WIDTH)
        .height(4.0)
        .background(ColorRgba8::rgba(49, 49, 53, 255))
        .corner_radius(2.0);
    if snapshot.phase == BootPhase::Complete {
        track = track.child(column().width(WIDTH).background(WHITE));
    } else if let Some(progress) = snapshot.progress_kind.fraction() {
        track = track.child(
            column()
                .width(WIDTH * progress.clamp(0.0, 1.0))
                .background(WHITE),
        );
    } else {
        // Activity remains indeterminate until the boot host reports real progress.
        let offset = (snapshot.frame / 2 % 33) as f32 * 4.0;
        track = track
            .child(column().width(offset))
            .child(column().width(48.0).background(WHITE));
    }
    track.child(spacer())
}
