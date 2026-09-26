mod navigation;
pub use navigation::*;
// Composed tray views. The shell owns popup surfaces and the service owns remote state.
use crate::{
    authoring::compose::*,
    components::application::MenuStyle,
    foundation::ColorRgba8,
    tray::*,
    ui::{Background, BoxStyle},
};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrayAreaStyle {
    pub menu: MenuStyle,
    pub icon_size: f32,
    pub cell_size: f32,
    pub columns: usize,
    pub gap: f32,
}
impl Default for TrayAreaStyle {
    fn default() -> Self {
        Self {
            menu: MenuStyle::default(),
            icon_size: 24.0,
            cell_size: 44.0,
            columns: 5,
            gap: 4.0,
        }
    }
}
#[crate::component(no_default)]
pub struct TrayArea {
    #[input]
    host: TrayHandle,
    #[input]
    style: TrayAreaStyle,
    #[input]
    primary_opens_menu: bool,
}
impl TrayArea {
    pub fn new(host: TrayHandle) -> Self {
        Self {
            host,
            style: TrayAreaStyle::default(),
            primary_opens_menu: false,
        }
    }
    pub fn style(mut self, style: TrayAreaStyle) -> Self {
        self.style = style;
        self
    }
    pub fn primary_opens_menu(mut self, enabled: bool) -> Self {
        self.primary_opens_menu = enabled;
        self
    }
}
fn grid_container(style: TrayAreaStyle, rows: usize) -> Container {
    let height = rows.max(1) as f32 * style.cell_size + rows.saturating_sub(1) as f32 * style.gap;
    // Intrinsic container sizing estimates 32px per child, which compresses square cells.
    // The content must also be allowed to extend beyond the clipped three-row viewport.
    column()
        .box_style(BoxStyle {
            width: crate::ui::SizeRule::Fill(1.0),
            height: crate::ui::SizeRule::Logical(height),
            max_size: crate::ui::SizeRule2D {
                width: crate::ui::SizeRule::Fill(1.0),
                height: crate::ui::SizeRule::Logical(height),
            },
            ..Default::default()
        })
        .gap(style.gap)
}

impl Component for TrayArea {
    fn view(&self) -> impl View {
        let signal = self.host.signal();
        let snapshot = self.watch(&signal);
        let rows = snapshot
            .items
            .len()
            .div_ceil(self.style.columns.clamp(1, 16));
        let mut grid = grid_container(self.style, rows);
        for chunk in snapshot.items.chunks(self.style.columns.clamp(1, 16)) {
            let mut line = row().height(self.style.cell_size).gap(self.style.gap);
            for item in chunk {
                let id = item.id.clone();
                let has_menu = item.has_menu;
                let source = if let Some(resource) = item.image_resource() {
                    Some(crate::authoring::compose::context::bind_image(resource))
                } else {
                    self.try_context::<ShellContext>().map(|shell| {
                        shell.applications().resolve_named_icon(
                            &item.icon.name,
                            IconRequest::new().logical_size(self.style.icon_size as u32),
                        )
                    })
                };
                let mut control = button().accessible_label(&item.title)
                    .key(item.id.as_str())
                    .width(self.style.cell_size)
                    .height(self.style.cell_size)
                    .padding(Insets::all(6.0))
                    .corner_radius(5.0)
                    .on_press(move |this: &mut Self| {
                        if this.primary_opens_menu && has_menu {
                            let _ = this.host.request_menu(id.clone());
                        } else {
                            let _ = this.host.activate(id.clone(), 0, 0);
                        }
                    });
                if let Some(source) = source {
                    control = control.child(image(source)
                        .width(self.style.icon_size).height(self.style.icon_size).without_tint());
                } else {
                    control = control.child(text(&item.title).color(ColorRgba8::rgba(248, 249, 252, 255)));
                }
                line = line.child(control);
            }
            grid = grid.child(line);
        }
        if snapshot.items.is_empty() {
            grid = grid.child(
                text(if snapshot.connected {
                    "No background apps"
                } else {
                    "Connecting to tray…"
                })
                .size(12.0),
            );
        }
        grid
    }
}
/// Remote menu columns with shared keyboard and pointer navigation.
#[crate::component(no_default)]
pub struct TrayMenuView {
    #[input]
    host: TrayHandle,
    #[input]
    style: MenuStyle,
    #[input]
    navigation: TrayMenuNavigation,
}
impl TrayMenuView {
    pub fn new(host: TrayHandle) -> Self {
        Self {
            host,
            style: MenuStyle::default(),
            navigation: TrayMenuNavigation::default(),
        }
    }
    pub fn style(mut self, style: MenuStyle) -> Self {
        self.style = style;
        self
    }
    pub fn navigation(mut self, navigation: TrayMenuNavigation) -> Self {
        self.navigation = navigation;
        self
    }
}
impl Component for TrayMenuView {
    fn view(&self) -> impl View {
        let signal = self.host.signal();
        let snapshot = self.watch(&signal);
        let nav_signal = self.navigation.signal();
        let nav = self.watch(&nav_signal);
        let mut columns = row().gap(4.0).scrollable();
        let Some(menu) = &snapshot.menu else {
            return columns.child(text("Loading menu…"));
        };
        for (level, items) in self.navigation.levels(&menu.menu).into_iter().enumerate() {
            let mut content = column()
                .box_style(self.style.container)
                .width(280.0)
                .gap(self.style.gap)
                .scrollable();
            for item in items.iter().filter(|i| i.visible) {
                if item.separator {
                    content =
                        content.child(column().height(1.0).background(Background::Color(
                            self.style.indicator_color.with_alpha(70),
                        )));
                    continue;
                }
                let id = item.id;
                let submenu = item.is_submenu || !item.children.is_empty();
                let check = match item.check {
                    TrayCheck::Check(true) => "✓ ",
                    TrayCheck::Radio(true) => "● ",
                    _ => "",
                };
                let label = format!(
                    "{check}{}{}{}",
                    menu_label(&item.label),
                    if item.shortcut.is_empty() {
                        String::new()
                    } else {
                        format!("    {}", item.shortcut)
                    },
                    if submenu { "    ›" } else { "" }
                );
                let highlighted = (nav.level == level && nav.highlighted == Some(id))
                    || nav.path.get(level) == Some(&id);
                let style = if highlighted {
                    self.style.highlighted_item
                } else if matches!(item.check, TrayCheck::Check(true) | TrayCheck::Radio(true)) {
                    self.style.checked_item
                } else {
                    self.style.item
                };
                let control = button().child(text(label)
                        .color(if item.enabled { self.style.label_color } else { self.style.disabled_label_color })
                        .size(self.style.label_size).text_align(Alignment::Start))
                    .key(format!("menu-{id}"))
                    .box_style(BoxStyle {
                        min_size: crate::ui::SizeRule2D {
                            width: crate::ui::SizeRule::Logical(1.0),
                            height: crate::ui::SizeRule::Logical(30.0),
                        },
                        ..style
                    })
                    .inline_style(menu_row_style(
                        style,
                        self.style.highlighted_item,
                        if item.enabled {
                            self.style.label_color
                        } else {
                            self.style.disabled_label_color
                        },
                        self.style.label_size,
                    ))
                    .width(Dimension::FILL)
                    .enabled(item.enabled)
                    .on_press(move |this: &mut Self| {
                        let _ = this.navigation.activate(&this.host, id, level);
                    });
                if !item.icon.is_empty() {
                    if let Some(shell) = self.try_context::<ShellContext>() {
                        let icon = shell
                            .applications()
                            .resolve_named_icon(&item.icon, IconRequest::new().logical_size(16));
                        content = content.child(
                            row()
                                .gap(4.0)
                                .child(image(icon).width(16.0).height(16.0))
                                .child(control),
                        );
                    } else {
                        content = content.child(control);
                    }
                } else {
                    content = content.child(control);
                }
            }
            if items.is_empty() {
                content = content.child(text("No actions").size(self.style.label_size));
            }
            columns = columns.child(content);
        }
        columns
    }
}
fn menu_label(label: &str) -> String {
    let mut result = String::new();
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                result.push('_');
                chars.next();
            }
        } else {
            result.push(c);
        }
    }
    result
}
fn menu_row_style(
    normal: BoxStyle,
    hover: BoxStyle,
    color: ColorRgba8,
    size: f32,
) -> std::sync::Arc<crate::theme::CompiledComponentStyle> {
    use crate::{theme::*, ui::*};
    use std::{collections::BTreeMap, sync::Arc};
    let slot = StyleSlotId::named("root");
    let patch = StylePropertyPatch {
        background: Some(normal.decoration.background),
        text_color: Some(color),
        text_size: Some(size),
        ..Default::default()
    };
    Arc::new(CompiledComponentStyle {
        id: ComponentStyleId::named(ThemeDomainId::SHELL, "tray", "menu-row"),
        slots: BTreeMap::from([(
            slot,
            CompiledSlotStyle {
                patch,
                font_family: None,
            },
        )]),
        variants: Default::default(),
        states: BTreeMap::from([(
            InteractionState::Hovered,
            CompiledStateStyle {
                slots: BTreeMap::from([(
                    slot,
                    CompiledSlotStyle {
                        patch: StylePropertyPatch {
                            background: Some(hover.decoration.background),
                            ..Default::default()
                        },
                        font_family: None,
                    },
                )]),
                transition: None,
            },
        )]),
        state_precedence: vec![InteractionState::Hovered],
        relevant_states: InteractionFlags::HOVERED,
        transition: crate::TransitionSpec {
            duration_ms: 0,
            ..Default::default()
        },
        controlled_slots: BTreeMap::from([(slot, patch)]),
        controlled_font_families: Default::default(),
    })
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    use crate::{ComposedAppRuntime, SizeI, foundation::MonotonicInstant};

    #[crate::component]
    struct GridFixture {
        #[input]
        rows: usize,
    }
    impl Component for GridFixture {
        fn view(&self) -> impl View {
            let style = TrayAreaStyle::default();
            column()
                .width(236.0)
                .height(140.0)
                .overflow(crate::ui::Overflow::Clip)
                .child(
                    grid_container(style, self.rows).children((0..self.rows).map(|_| {
                        row()
                            .height(style.cell_size)
                            .gap(style.gap)
                            .children((0..5).map(|_| {
                                button().accessible_label("tray")
                                    .width(style.cell_size)
                                    .height(style.cell_size)
                                    .child(image(crate::ui::ImageId(0)).width(style.icon_size).height(style.icon_size))
                            }))
                    })),
                )
        }
    }

    #[test]
    fn tray_cells_remain_square_in_short_and_overflowing_grids() {
        for rows in [1, 2, 3, 5] {
            let mut runtime = ComposedAppRuntime::from_composed_with_extent(
                GridFixture { rows },
                SizeI {
                    width: 236,
                    height: 140,
                },
            )
            .unwrap();
            runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
            let mut count = 0;
            for (node, kind) in runtime.ui().kinds.iter() {
                if *kind == crate::ui::NodeKind::Button {
                    let rect = runtime.layout().computed(node).unwrap().local_border_rect;
                    assert_eq!((rect.width, rect.height), (44.0, 44.0), "rows={rows}");
                    count += 1;
                } else if *kind == crate::ui::NodeKind::Image {
                    let rect = runtime.layout().computed(node).unwrap().local_border_rect;
                    assert_eq!((rect.width, rect.height), (24.0, 24.0), "icon rows={rows}");
                }
            }
            assert_eq!(count, rows * 5);
        }
    }
}
