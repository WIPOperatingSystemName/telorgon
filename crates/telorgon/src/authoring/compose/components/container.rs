use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::ui::{
    Background, Border, BoxDecoration, BoxStyle, CornerRadii, Flow, LayoutStyle, Outline, Shadow,
    ShadowList, SizeRule,
};

use crate::authoring::compose::{Alignment, Dimension, Element, ElementKind, Insets, Key, View};

#[doc(hidden)]
#[derive(Debug)]
pub struct ContainerElement {
    pub inline_style: Option<std::sync::Arc<crate::theme::CompiledComponentStyle>>,
    pub hover_within: bool,
    pub effects: super::interaction::InteractionEffects,
    pub scrollable: bool,
    pub style: BoxStyle,
    pub layout: LayoutStyle,
    pub children: Vec<Element>,
}

/// A non-generic container builder. Children are erased as they are appended.
#[derive(Debug)]
pub struct Container {
    key: Option<Key>,
    pointer_request: Option<crate::PointerRequest>,
    element: ContainerElement,
    explicit_width: bool,
    explicit_height: bool,
}

impl Container {
    pub fn cursor(mut self, icon: impl Into<crate::CursorIcon>) -> Self {
        self.pointer_request = Some(icon.into().into());
        self
    }

    pub fn hover_effect(mut self, effect: super::interaction::InteractionEffect) -> Self {
        self.element.effects.hover.push(effect);
        self
    }
    pub fn hover_effects(
        mut self,
        effects: impl IntoIterator<Item = super::interaction::InteractionEffect>,
    ) -> Self {
        self.element.effects.hover.extend(effects);
        self
    }
    pub fn hover_transition(mut self, transition: crate::theme::TransitionSpec) -> Self {
        self.element.effects.hover_transition = Some(transition);
        self
    }

    pub fn press_effect(mut self, effect: super::interaction::InteractionEffect) -> Self {
        self.element.effects.press.push(effect);
        self
    }
    pub fn press_effects(
        mut self,
        effects: impl IntoIterator<Item = super::interaction::InteractionEffect>,
    ) -> Self {
        self.element.effects.press.extend(effects);
        self
    }
    pub fn press_transition(mut self, transition: crate::theme::TransitionSpec) -> Self {
        self.element.effects.press_transition = Some(transition);
        self
    }

    /// Installs a code-defined state style without registering it in the application theme.
    /// Styling alone does not enable descendant hover tracking.
    pub fn inline_style(
        mut self,
        style: std::sync::Arc<crate::theme::CompiledComponentStyle>,
    ) -> Self {
        self.element.inline_style = Some(style);
        self
    }

    /// Tracks hover over this container and its descendants. Defaults to false.
    /// Child controls retain their own hover, focus, and activation behavior.
    /// Works independently of whether an inline style is installed.
    pub fn hover_within(mut self, enabled: bool) -> Self {
        self.element.hover_within = enabled;
        self
    }

    /// Clips overflowing content and accepts wheel/trackpad scrolling inside this viewport.
    pub fn scrollable(mut self) -> Self {
        self.element.scrollable = true;
        self.element.style.overflow = crate::ui::Overflow::Scroll;
        self
    }

    /// Wraps equal-size cells into as many columns as the available width permits.
    pub fn grid(mut self, cell_width: u16, cell_height: u16) -> Self {
        self.element.layout.flow = Flow::Grid {
            cell_width: cell_width.max(1),
            cell_height: cell_height.max(1),
        };
        self.element.style.height = SizeRule::Shrink;
        self.element.style.max_size.height = SizeRule::Logical(f32::MAX);
        self
    }

    pub fn child(mut self, child: impl View) -> Self {
        self.element.children.push(child.into_element());
        self
    }

    pub fn children<I, V>(mut self, children: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: View,
    {
        self.element
            .children
            .extend(children.into_iter().map(View::into_element));
        self
    }

    pub fn maybe(self, condition: bool, child: impl View) -> Self {
        if condition { self.child(child) } else { self }
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.element.layout.gap = gap;
        self
    }

    /// Replaces the complete layout contract while retaining this container's children and style.
    pub fn layout_style(mut self, layout: LayoutStyle) -> Self {
        self.element.layout = layout;
        self
    }

    /// Positions children along this container's flow direction.
    ///
    /// For a row this is the horizontal axis; for a column this is the vertical axis.
    pub fn justify_content(mut self, alignment: Alignment) -> Self {
        self.element.layout.main_axis_alignment = alignment.into();
        self
    }

    /// Positions children across this container's flow direction.
    ///
    /// For a row this is the vertical axis; for a column this is the horizontal axis.
    pub fn align_items(mut self, alignment: Alignment) -> Self {
        self.element.layout.cross_axis_alignment = alignment.into();
        self
    }

    /// Centers children along and across this container's flow direction.
    pub fn center_content(self) -> Self {
        self.justify_content(Alignment::Center)
            .align_items(Alignment::Center)
    }

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.element.style.padding = padding.into().0;
        self
    }

    pub fn padding_edges(mut self, padding: EdgeInsets) -> Self {
        self.element.style.padding = padding;
        self
    }

    pub fn box_style(mut self, style: BoxStyle) -> Self {
        self.element.style = style;
        self.explicit_width = true;
        self.explicit_height = true;
        self
    }

    #[deprecated(since = "0.1.12", note = "use `box_style` for normalized vocabulary")]
    pub fn style(self, style: BoxStyle) -> Self {
        self.box_style(style)
    }

    pub fn decoration(mut self, decoration: BoxDecoration) -> Self {
        self.element.style.decoration = decoration;
        self
    }

    pub fn margin(mut self, margin: impl Into<Insets>) -> Self {
        self.element.style.margin = margin.into().0;
        self
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.element.style.decoration.background = background.into();
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.element.style.decoration.corner_radii = CornerRadii::all(radius);
        self
    }

    pub fn corner_radii(mut self, radii: CornerRadii) -> Self {
        self.element.style.decoration.corner_radii = radii;
        self
    }

    #[deprecated(since = "0.1.12", note = "use `corner_radius`")]
    pub fn radius(self, radius: f32) -> Self {
        self.corner_radius(radius)
    }

    #[deprecated(since = "0.1.12", note = "use `uniform_border`")]
    pub fn border(mut self, width: f32, color: ColorRgba8) -> Self {
        self.element.style.decoration.border = Border::all(width, color);
        self
    }

    pub fn border_sides(mut self, border: Border) -> Self {
        self.element.style.decoration.border = border;
        self
    }

    pub fn uniform_border(mut self, width: f32, color: ColorRgba8) -> Self {
        self.element.style.decoration.border = Border::all(width, color);
        self
    }

    pub fn outline(mut self, outline: Outline) -> Self {
        self.element.style.decoration.outline = outline;
        self
    }

    pub fn shadow(mut self, shadow: Shadow) -> Self {
        self.element.style.decoration.shadows = ShadowList::one(shadow);
        self
    }

    pub fn shadows(mut self, shadows: ShadowList) -> Self {
        self.element.style.decoration.shadows = shadows;
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.element.style.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn overflow(mut self, overflow: crate::ui::Overflow) -> Self {
        self.element.style.overflow = overflow;
        self
    }

    /// Derives an automatic axis from the other using width divided by height.
    pub fn aspect_ratio(mut self, ratio: f32) -> Self {
        self.element.style.aspect_ratio = Some(ratio);
        self
    }

    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.element.style.width = width.into().into();
        self.explicit_width = true;
        self
    }

    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.element.style.height = height.into().into();
        self.explicit_height = true;
        self
    }
}

impl View for Container {
    fn into_element(mut self) -> Element {
        if self.element.style.aspect_ratio.is_some() {
            if self.explicit_width && !self.explicit_height {
                self.element.style.height = SizeRule::Shrink;
            } else if self.explicit_height && !self.explicit_width {
                self.element.style.width = SizeRule::Shrink;
            }
        }
        let element = Element::from_kind(self.key, ElementKind::Container(self.element));
        match self.pointer_request {
            Some(request) => element.with_pointer_request(request),
            None => element,
        }
    }
}

fn container(flow: Flow) -> Container {
    Container {
        key: None,
        pointer_request: None,
        explicit_width: false,
        explicit_height: false,
        element: ContainerElement {
            inline_style: None,
            hover_within: false,
            effects: Default::default(),
            scrollable: false,
            style: BoxStyle {
                width: SizeRule::Fill(1.0),
                height: SizeRule::Fill(1.0),
                overflow: crate::ui::Overflow::Clip,
                ..BoxStyle::default()
            },
            layout: LayoutStyle {
                flow,
                ..LayoutStyle::default()
            },
            children: Vec::new(),
        },
    }
}

pub fn column() -> Container {
    container(Flow::Vertical)
}

pub fn row() -> Container {
    container(Flow::Horizontal)
}

pub fn stack() -> Container {
    container(Flow::Overlay)
}

/// Flexible empty space for rows and columns.
pub fn spacer() -> Container {
    column()
}

/// A neutral content surface. This is a convenience, not a requirement for grouping content.
pub fn card() -> Container {
    column()
        .padding(16.0)
        .background(ColorRgba8::rgba(31, 37, 50, 255))
        .corner_radius(6.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_containers_fill_their_available_box_by_default() {
        for view in [column(), row(), stack(), card()] {
            let element = view.into_element();
            let ElementKind::Container(container) = element.kind() else {
                panic!("expected container")
            };
            assert_eq!(container.style.width, SizeRule::Fill(1.0));
            assert_eq!(container.style.height, SizeRule::Fill(1.0));
        }

        let element = column()
            .width(Dimension::Shrink)
            .height(Dimension::Shrink)
            .into_element();
        let ElementKind::Container(container) = element.kind() else {
            panic!("expected container")
        };
        assert_eq!(container.style.width, SizeRule::Shrink);
        assert_eq!(container.style.height, SizeRule::Shrink);
    }

    #[test]
    fn flex_alignment_builders_set_the_matching_layout_axes() {
        let element = row()
            .justify_content(Alignment::End)
            .align_items(Alignment::Center)
            .into_element();
        let ElementKind::Container(container) = element.kind() else {
            panic!("expected container")
        };

        assert_eq!(
            container.layout.main_axis_alignment,
            crate::ui::MainAxisAlignment::End
        );
        assert_eq!(
            container.layout.cross_axis_alignment,
            crate::ui::CrossAxisAlignment::Center
        );
    }

    #[test]
    fn center_content_centers_both_layout_axes() {
        let element = column().center_content().into_element();
        let ElementKind::Container(container) = element.kind() else {
            panic!("expected container")
        };

        assert_eq!(
            container.layout.main_axis_alignment,
            crate::ui::MainAxisAlignment::Center
        );
        assert_eq!(
            container.layout.cross_axis_alignment,
            crate::ui::CrossAxisAlignment::Center
        );
    }
}
