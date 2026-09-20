use crate::core::{ColorRgba8, EdgeInsets};
use crate::ui::{
    Background, Border, BoxDecoration, BoxStyle, CornerRadii, Flow, LayoutStyle, Outline, Shadow,
    ShadowList, SizeRule,
};

use crate::compose::{Alignment, Dimension, Element, ElementKind, Insets, Key, View};

#[doc(hidden)]
#[derive(Debug)]
pub struct ContainerElement {
    pub inline_style: Option<std::sync::Arc<crate::theme::CompiledComponentStyle>>,
    pub hover_within: bool,
    pub style: BoxStyle,
    pub layout: LayoutStyle,
    pub children: Vec<Element>,
}

/// A non-generic container builder. Children are erased as they are appended.
#[derive(Debug)]
pub struct Container {
    key: Option<Key>,
    element: ContainerElement,
}

impl Container {
    /// Installs a code-defined state style without registering it in the application theme.
    /// Styling alone does not enable descendant hover tracking.
    pub fn inline_style(mut self, style: std::sync::Arc<crate::theme::CompiledComponentStyle>) -> Self {
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

    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.element.style.width = width.into().into();
        self
    }

    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.element.style.height = height.into().into();
        self
    }
}

impl View for Container {
    fn into_element(self) -> Element {
        Element::from_kind(self.key, ElementKind::Container(self.element))
    }
}

fn container(flow: Flow) -> Container {
    Container {
        key: None,
        element: ContainerElement {
            inline_style: None,
            hover_within: false,
            style: BoxStyle {
                width: SizeRule::Fill(1.0),
                height: SizeRule::Fill(1.0),
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
    column().width(Dimension::FILL).height(Dimension::FILL)
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
