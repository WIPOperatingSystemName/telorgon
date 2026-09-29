use std::marker::PhantomData;

use crate::authoring::compose::{
    Alignment, Container, Dimension, Element, Insets, Key, View, stack,
};
use crate::foundation::ColorRgba8;
use crate::shell::window_chrome::{
    ShellActionId, WindowAction, WindowChromeRole, WindowResizeEdge,
};
use crate::ui::{
    Background, Border, BoxDecoration, BoxStyle, CornerRadii, LayoutStyle, Outline, Overflow,
    Shadow, ShadowList,
};

pub struct MissingContent;
pub struct HasContent;

/// Specialized overlay stack for server- or client-owned window chrome.
pub struct WindowFrame<State = MissingContent> {
    root: Container,
    _state: PhantomData<State>,
}

impl<State> WindowFrame<State> {
    pub fn cursor(mut self, icon: impl Into<crate::CursorIcon>) -> Self {
        self.root = self.root.cursor(icon);
        self
    }

    pub fn child(mut self, child: impl View) -> Self {
        self.root = self.root.child(child);
        self
    }

    pub fn children<I, V>(mut self, children: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: View,
    {
        self.root = self.root.children(children);
        self
    }

    pub fn maybe(mut self, condition: bool, child: impl View) -> Self {
        self.root = self.root.maybe(condition, child);
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.root = self.root.key(key);
        self
    }

    pub fn layout_style(mut self, layout: LayoutStyle) -> Self {
        self.root = self.root.layout_style(layout);
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.root = self.root.gap(gap);
        self
    }

    pub fn justify_content(mut self, alignment: Alignment) -> Self {
        self.root = self.root.justify_content(alignment);
        self
    }

    pub fn align_items(mut self, alignment: Alignment) -> Self {
        self.root = self.root.align_items(alignment);
        self
    }

    pub fn center_content(mut self) -> Self {
        self.root = self.root.center_content();
        self
    }

    pub fn decoration(mut self, decoration: BoxDecoration) -> Self {
        self.root = self.root.decoration(decoration);
        self
    }

    pub fn box_style(mut self, style: BoxStyle) -> Self {
        self.root = self.root.box_style(style);
        self
    }

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.root = self.root.padding(padding);
        self
    }

    pub fn margin(mut self, margin: impl Into<Insets>) -> Self {
        self.root = self.root.margin(margin);
        self
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.root = self.root.background(background);
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.root = self.root.corner_radius(radius);
        self
    }

    pub fn corner_radii(mut self, radii: CornerRadii) -> Self {
        self.root = self.root.corner_radii(radii);
        self
    }

    pub fn border_sides(mut self, border: Border) -> Self {
        self.root = self.root.border_sides(border);
        self
    }

    pub fn uniform_border(mut self, width: f32, color: ColorRgba8) -> Self {
        self.root = self.root.uniform_border(width, color);
        self
    }

    pub fn outline(mut self, outline: Outline) -> Self {
        self.root = self.root.outline(outline);
        self
    }

    pub fn shadow(mut self, shadow: Shadow) -> Self {
        self.root = self.root.shadow(shadow);
        self
    }

    pub fn shadows(mut self, shadows: ShadowList) -> Self {
        self.root = self.root.shadows(shadows);
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.root = self.root.opacity(opacity);
        self
    }

    pub fn overflow(mut self, overflow: Overflow) -> Self {
        self.root = self.root.overflow(overflow);
        self
    }

    pub fn aspect_ratio(mut self, ratio: f32) -> Self {
        self.root = self.root.aspect_ratio(ratio);
        self
    }

    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.root = self.root.width(width);
        self
    }

    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.root = self.root.height(height);
        self
    }
}

impl WindowFrame<MissingContent> {
    /// Installs the sole placeholder where the hosted client surface will be composed.
    pub fn content_slot(mut self, slot: WindowContentSlot) -> WindowFrame<HasContent> {
        self.root = self.root.child(slot);
        WindowFrame {
            root: self.root,
            _state: PhantomData,
        }
    }
}

impl View for WindowFrame<HasContent> {
    fn into_element(self) -> Element {
        self.root
            .into_element()
            .with_window_chrome_role(WindowChromeRole::Frame)
    }
}

pub fn window_frame() -> WindowFrame<MissingContent> {
    WindowFrame {
        root: stack(),
        _state: PhantomData,
    }
}

pub struct WindowContentSlot {
    content: Container,
}

impl WindowContentSlot {
    pub fn cursor(mut self, icon: impl Into<crate::CursorIcon>) -> Self {
        self.content = self.content.cursor(icon);
        self
    }

    /// Adds managed GUI content to this slot. Server-side compositor frames normally leave it
    /// empty because the hosted Wayland surface is composited into the same bounds externally.
    pub fn child(mut self, child: impl View) -> Self {
        self.content = self.content.child(child);
        self
    }

    pub fn children<I, V>(mut self, children: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: View,
    {
        self.content = self.content.children(children);
        self
    }

    pub fn maybe(mut self, condition: bool, child: impl View) -> Self {
        self.content = self.content.maybe(condition, child);
        self
    }

    pub fn box_style(mut self, style: BoxStyle) -> Self {
        self.content = self.content.box_style(style);
        self
    }

    pub fn layout_style(mut self, layout: LayoutStyle) -> Self {
        self.content = self.content.layout_style(layout);
        self
    }

    pub fn decoration(mut self, decoration: BoxDecoration) -> Self {
        self.content = self.content.decoration(decoration);
        self
    }

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.content = self.content.padding(padding);
        self
    }

    pub fn margin(mut self, margin: impl Into<Insets>) -> Self {
        self.content = self.content.margin(margin);
        self
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.content = self.content.background(background);
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.content = self.content.corner_radius(radius);
        self
    }

    pub fn corner_radii(mut self, radii: CornerRadii) -> Self {
        self.content = self.content.corner_radii(radii);
        self
    }

    pub fn border_sides(mut self, border: Border) -> Self {
        self.content = self.content.border_sides(border);
        self
    }

    pub fn uniform_border(mut self, width: f32, color: ColorRgba8) -> Self {
        self.content = self.content.uniform_border(width, color);
        self
    }

    pub fn outline(mut self, outline: Outline) -> Self {
        self.content = self.content.outline(outline);
        self
    }

    pub fn shadow(mut self, shadow: Shadow) -> Self {
        self.content = self.content.shadow(shadow);
        self
    }

    pub fn shadows(mut self, shadows: ShadowList) -> Self {
        self.content = self.content.shadows(shadows);
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.content = self.content.opacity(opacity);
        self
    }

    pub fn overflow(mut self, overflow: Overflow) -> Self {
        self.content = self.content.overflow(overflow);
        self
    }

    pub fn aspect_ratio(mut self, ratio: f32) -> Self {
        self.content = self.content.aspect_ratio(ratio);
        self
    }

    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.content = self.content.width(width);
        self
    }

    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.content = self.content.height(height);
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.content = self.content.key(key);
        self
    }
}

impl View for WindowContentSlot {
    fn into_element(self) -> Element {
        self.content
            .into_element()
            .with_window_chrome_role(WindowChromeRole::Content)
    }
}

pub fn window_content_slot() -> WindowContentSlot {
    WindowContentSlot { content: stack() }
}

/// Adds shell-understood meaning to an otherwise ordinary composed view.
pub trait WindowChromeViewExt: View + Sized {
    fn window_title(self) -> Element {
        self.into_element()
            .with_window_chrome_role(WindowChromeRole::Title)
    }

    fn window_app_icon(self) -> Element {
        self.into_element()
            .with_window_chrome_role(WindowChromeRole::AppIcon)
    }

    fn window_drag_region(self) -> Element {
        self.into_element()
            .with_window_chrome_role(WindowChromeRole::DragRegion)
    }

    fn window_action(self, action: WindowAction) -> Element {
        self.into_element()
            .with_window_chrome_role(WindowChromeRole::Action(action))
    }

    /// Marks a region as an action that runs only when the compositor registered the same ID.
    fn window_shell_action(self, action: ShellActionId) -> Element {
        self.into_element()
            .with_window_chrome_role(WindowChromeRole::ShellAction(action))
    }

    /// Expands this chrome region's hit target without changing its layout or paint bounds.
    fn window_hit_slop(self, hit_slop: impl Into<Insets>) -> Element {
        self.into_element()
            .with_window_chrome_hit_slop(hit_slop.into().0)
    }

    /// Overrides the default action > resize > drag hit precedence for this region.
    fn window_hit_priority(self, priority: u16) -> Element {
        self.into_element()
            .with_window_chrome_hit_priority(priority)
    }

    fn window_resize(self, edge: WindowResizeEdge) -> Element {
        self.window_action(WindowAction::BeginResize(edge))
    }

    fn window_system_menu(self) -> Element {
        self.window_action(WindowAction::ShowSystemMenu)
    }
}

impl<T: View> WindowChromeViewExt for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::compose::{ElementKind, text};

    #[test]
    fn common_modifiers_preserve_the_frame_content_slot_contract() {
        let frame = window_frame()
            .cursor(crate::CursorIcon::Move)
            .center_content()
            .content_slot(
                window_content_slot()
                    .cursor(crate::CursorIcon::Default)
                    .children([text("First"), text("Second")])
                    .maybe(false, text("Excluded"))
                    .maybe(true, text("Third")),
            )
            .cursor(crate::CursorIcon::Grab)
            .child(text("Title"))
            .into_element();
        let (_, ElementKind::Container(root), role, _, cursor) = frame.into_parts() else {
            panic!("expected a frame container")
        };
        assert_eq!(role, Some(WindowChromeRole::Frame));
        assert_eq!(cursor, Some(crate::CursorIcon::Grab.into()));
        assert_eq!(root.children.len(), 2);
        let ElementKind::Container(content) = root.children[0].kind() else {
            panic!("expected a content slot")
        };
        assert_eq!(content.children.len(), 3);
    }
}
