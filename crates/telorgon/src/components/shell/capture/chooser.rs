//! Default visual chooser. Layout, source previews and consent controls share one geometry model.
use super::selection::{Selection, Tab};
use super::*;
use crate::input::{ButtonState, InputEvent, LogicalKey, NamedKey};
use crate::{Background, ColorRgba8, RectF, SizeF};

const PANEL: ColorRgba8 = ColorRgba8::rgba(27, 30, 39, 255);
const CARD: ColorRgba8 = ColorRgba8::rgba(39, 44, 57, 255);
const ACCENT: ColorRgba8 = ColorRgba8::rgba(122, 162, 255, 255);
const WHITE: ColorRgba8 = ColorRgba8::rgba(241, 244, 252, 255);
const MUTED: ColorRgba8 = ColorRgba8::rgba(177, 186, 205, 255);
use super::layout::{GAP, PAD, PickerLayout};

/// A visual screen/window chooser for `Compositor::capture_chooser`. Selection does not start
/// sharing; only the Share button submits consent for the exact current source and epoch.
#[crate::component]
pub struct CapturePicker {
    #[input]
    ui: CaptureUi,
    #[state]
    selection: Selection,
}
impl CapturePicker {
    pub fn new(ui: CaptureUi) -> Self {
        Self {
            ui,
            selection: Selection::default(),
        }
    }
    fn layout(&self) -> PickerLayout {
        let snapshot = self.ui.snapshot.snapshot();
        let tab = self.selection.current(&snapshot, 6).tab;
        let count = snapshot
            .sources
            .iter()
            .filter(|s| tab.contains(s.0))
            .count();
        PickerLayout::new(
            self.try_context::<ShellContext>().map_or(
                SizeF {
                    width: 1024.0,
                    height: 768.0,
                },
                |c| c.output_size(),
            ),
            count,
        )
    }
    fn choose(&mut self, request: u64, source: CaptureSource, epoch: u64) {
        let snapshot = self.ui.snapshot.snapshot();
        if snapshot.pending.as_ref().is_some_and(|p| p.0 == request)
            && snapshot
                .sources
                .iter()
                .any(|s| (s.0, s.1) == (source, epoch))
        {
            self.selection = self.selection.current(&snapshot, self.layout().capacity());
            self.selection.selected = Some((source, epoch));
        }
    }
    fn switch(&mut self, tab: Tab) {
        self.selection = self
            .selection
            .current(&self.ui.snapshot.snapshot(), self.layout().capacity());
        self.selection.switch(tab);
    }
    fn page(&mut self, forward: bool) {
        let snapshot = self.ui.snapshot.snapshot();
        let capacity = self.layout().capacity();
        self.selection = self.selection.current(&snapshot, capacity);
        let count = snapshot
            .sources
            .iter()
            .filter(|s| self.selection.tab.contains(s.0))
            .count();
        self.selection.page = if forward {
            (self.selection.page + 1).min(count.saturating_sub(1) / capacity)
        } else {
            self.selection.page.saturating_sub(1)
        };
    }
}
impl Component for CapturePicker {
    fn view(&self) -> impl View {
        let snapshot = self.watch(&self.ui.snapshot);
        let layout = self.layout();
        let state = self.selection.current(&snapshot, layout.capacity());
        let (id, app) = snapshot.pending.clone().unwrap_or_default();
        let sources: Vec<_> = snapshot
            .sources
            .iter()
            .filter(|s| state.tab.contains(s.0))
            .collect();
        let mut tabs = row().height(36.0).gap(8.0);
        for (tab, title) in [(Tab::Screens, "Screens"), (Tab::Windows, "Windows")] {
            tabs = tabs.child(
                button(title)
                    .width(112.0)
                    .height(36.0)
                    .background(if state.tab == tab { ACCENT } else { CARD })
                    .enabled(snapshot.sources.iter().any(|s| tab.contains(s.0)))
                    .on_press(move |this: &mut Self| this.switch(tab)),
            );
        }
        let mut grid = column()
            .height(layout.card_height * layout.rows as f32 + GAP * (layout.rows - 1) as f32)
            .gap(GAP);
        for row_index in 0..layout.rows {
            let mut cards = row().height(layout.card_height).gap(GAP);
            for column_index in 0..layout.columns {
                let index = row_index * layout.columns + column_index;
                let Some((source, epoch, label)) =
                    sources.get(state.page * layout.capacity() + index).copied()
                else {
                    continue;
                };
                let (source, epoch) = (*source, *epoch);
                let selected = state.selected == Some((source, epoch));
                let label = application_label(label);
                let short = short_title(&label, ((layout.card_width - 20.0) / 8.0) as usize);
                cards = cards.child(
                    stack()
                        .key(format!("{id}:{source:?}:{epoch}"))
                        .width(layout.card_width)
                        .height(layout.card_height)
                        .corner_radius(8.0)
                        .background(CARD)
                        .uniform_border(2.0, if selected { ACCENT } else { CARD })
                        .child(
                            button(label)
                                .width(Dimension::FILL)
                                .height(Dimension::FILL)
                                .background(Background::Color(ColorRgba8::rgba(0, 0, 0, 0)))
                                .on_press(move |this: &mut Self| this.choose(id, source, epoch)),
                        )
                        // The preview is composed over this blank well; the title remains below it.
                        .child(
                            column()
                                .padding(6.0)
                                .child(
                                    stack()
                                        .height((layout.card_height - 42.0).max(1.0))
                                        .background(ColorRgba8::rgba(15, 17, 24, 255)),
                                )
                                .child(
                                    row()
                                        .height(30.0)
                                        .align_items(Alignment::Center)
                                        .child(text(short).size(13.0).color(WHITE)),
                                ),
                        ),
                );
            }
            grid = grid.child(cards);
        }
        let count = sources.len();
        let status = if count == 0 {
            "No available sources".into()
        } else if let Some(selected) = state.selected {
            snapshot
                .sources
                .iter()
                .find(|s| (s.0, s.1) == selected)
                .map(|s| format!("Selected: {}", short_title(&s.2, 64)))
                .unwrap_or_default()
        } else {
            "Select a source, then choose Share.".into()
        };
        let selected = state.selected;
        column()
            .key(id)
            .width(layout.width)
            .height(layout.height)
            .padding(PAD)
            .gap(12.0)
            .background(PANEL)
            .corner_radius(12.0)
            .child(
                column()
                    .height(46.0)
                    .gap(4.0)
                    .child(text("Choose what to share").size(22.0).color(WHITE))
                    .child(
                        text(format!(
                            "Share with {}",
                            short_title(&application_label(&app), 70)
                        ))
                        .size(14.0)
                        .color(MUTED),
                    ),
            )
            .child(tabs)
            .child(grid)
            .child(
                row().height(24.0).gap(8.0).child(
                    text(short_title(
                        &status,
                        ((layout.width - 2.0 * PAD) / 8.0) as usize,
                    ))
                    .size(13.0)
                    .color(MUTED),
                ),
            )
            .child(
                row()
                    .height(36.0)
                    .gap(8.0)
                    .child(
                        button("‹")
                            .width(36.0)
                            .height(36.0)
                            .enabled(state.page > 0)
                            .on_press(|this: &mut Self| this.page(false)),
                    )
                    .child(
                        button("›")
                            .width(36.0)
                            .height(36.0)
                            .enabled((state.page + 1) * layout.capacity() < count)
                            .on_press(|this: &mut Self| this.page(true)),
                    )
                    .child(spacer())
                    .child(button("Cancel").width(76.0).height(36.0).on_press(
                        move |this: &mut Self| {
                            let _ = this.ui.deny(id);
                        },
                    ))
                    .child(
                        button("Share")
                            .key(format!("share:{id}:{selected:?}"))
                            .width(76.0)
                            .height(36.0)
                            .background(if selected.is_some() && !state.submitted {
                                ACCENT
                            } else {
                                CARD
                            })
                            .enabled(selected.is_some() && !state.submitted)
                            .on_press(move |this: &mut Self| {
                                let current = this.selection.current(
                                    &this.ui.snapshot.snapshot(),
                                    this.layout().capacity(),
                                );
                                if current.request == id
                                    && current.selected == selected
                                    && !current.submitted
                                {
                                    if let Some((source, epoch)) = selected {
                                        if this.ui.approve(id, source, epoch) {
                                            this.selection.submitted = true;
                                        }
                                    }
                                }
                            }),
                    ),
            )
    }
}
impl ShellWidget for CapturePicker {
    fn surface(&self) -> ShellSurfaceSpec {
        let layout = self.layout();
        ShellSurfaceSpec::new()
            .layer(ShellSurfaceLayer::Overlay)
            .order(i32::MAX)
            .placement(
                WidgetPlacement::center()
                    .width(layout.width)
                    .height(layout.height)
                    .margin(8.0),
            )
            .visible(self.ui.snapshot.snapshot().pending.is_some())
            .pointer(ShellPointer::Modal)
            .focus(ShellFocus::OnOpen)
            .dismiss_on_escape(true)
    }
    fn window_previews(&self) -> Vec<ShellWindowPreview> {
        self.previews()
            .into_iter()
            .filter_map(|(source, rect)| match source {
                CaptureSource::Window(id) => Some(ShellWindowPreview::new(id, rect)),
                _ => None,
            })
            .collect()
    }
    fn output_previews(&self) -> Vec<ShellOutputPreview> {
        self.previews()
            .into_iter()
            .filter_map(|(source, rect)| match source {
                CaptureSource::Output(id) => Some(ShellOutputPreview::new(id, rect)),
                _ => None,
            })
            .collect()
    }
    fn dismissed(&mut self, _: ShellDismissReason) {
        if let Some((id, _)) = self.ui.snapshot.snapshot().pending {
            let _ = self.ui.deny(id);
        }
    }
    fn input(&mut self, event: InputEvent) -> bool {
        match event {
            InputEvent::Scroll { delta, .. } if delta.y != 0.0 => {
                self.page(delta.y > 0.0);
                true
            }
            InputEvent::Key(event) if event.state == ButtonState::Pressed => {
                let offset = match event.logical_key {
                    LogicalKey::Named(NamedKey::ArrowLeft) => -1,
                    LogicalKey::Named(NamedKey::ArrowRight) => 1,
                    LogicalKey::Named(NamedKey::ArrowUp) => -(self.layout().columns as isize),
                    LogicalKey::Named(NamedKey::ArrowDown) => self.layout().columns as isize,
                    _ => return false,
                };
                let snapshot = self.ui.snapshot.snapshot();
                let capacity = self.layout().capacity();
                self.selection = self.selection.current(&snapshot, capacity);
                let sources: Vec<_> = snapshot
                    .sources
                    .iter()
                    .filter(|s| self.selection.tab.contains(s.0))
                    .collect();
                if sources.is_empty() {
                    return false;
                }
                let index = self
                    .selection
                    .selected
                    .and_then(|selected| sources.iter().position(|s| (s.0, s.1) == selected));
                let index = index.map_or(0, |index| {
                    index.saturating_add_signed(offset).min(sources.len() - 1)
                });
                self.selection.selected = Some((sources[index].0, sources[index].1));
                self.selection.page = index / capacity;
                true
            }
            _ => false,
        }
    }
}
impl CapturePicker {
    fn previews(&self) -> Vec<(CaptureSource, RectF)> {
        let snapshot = self.watch(&self.ui.snapshot);
        if snapshot.pending.is_none() {
            return Vec::new();
        }
        let layout = self.layout();
        let state = self.selection.current(&snapshot, layout.capacity());
        snapshot
            .sources
            .iter()
            .filter(|s| state.tab.contains(s.0))
            .skip(state.page * layout.capacity())
            .take(layout.capacity())
            .enumerate()
            .map(|(index, s)| (s.0, layout.preview(index)))
            .collect()
    }
}
fn short_title(label: &str, limit: usize) -> String {
    let mut chars = label.chars();
    let mut text: String = chars.by_ref().take(limit.saturating_sub(1)).collect();
    if chars.next().is_some() {
        text.push('…');
    }
    text
}
