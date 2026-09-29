use crate::foundation::ColorRgba8;
use crate::ui::{
    Background, Border, BoxDecoration, BoxStyle, CornerRadii, LayoutStyle, Outline, Overflow,
    Shadow, ShadowList,
};

use crate::authoring::compose::{
    Alignment, Dimension, Element, ElementKind, Insets, Key, TextStyle, View,
};

#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct TextElement {
    pub content: String,
    pub style: TextStyle,
    pub box_style: BoxStyle,
    pub layout: LayoutStyle,
    pub effects: super::interaction::InteractionEffects,
}

#[derive(Clone, Debug)]
pub struct Text {
    key: Option<Key>,
    pointer_request: Option<crate::PointerRequest>,
    element: TextElement,
}

impl Text {
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

    pub fn cursor(mut self, icon: impl Into<crate::CursorIcon>) -> Self {
        self.pointer_request = Some(icon.into().into());
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Derives an automatic axis from the other using width divided by height.
    pub fn aspect_ratio(mut self, ratio: f32) -> Self {
        self.element.box_style.aspect_ratio = Some(ratio);
        self
    }

    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.element.box_style.width = width.into().into();
        self
    }

    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.element.box_style.height = height.into().into();
        self
    }

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.element.box_style.padding = padding.into().0;
        self
    }

    pub fn margin(mut self, margin: impl Into<Insets>) -> Self {
        self.element.box_style.margin = margin.into().0;
        self
    }

    pub fn box_style(mut self, style: BoxStyle) -> Self {
        self.element.box_style = style;
        self
    }

    pub fn layout_style(mut self, layout: LayoutStyle) -> Self {
        self.element.layout = layout;
        self
    }

    pub fn decoration(mut self, decoration: BoxDecoration) -> Self {
        self.element.box_style.decoration = decoration;
        self
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.element.box_style.decoration.background = background.into();
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.element.box_style.decoration.corner_radii = CornerRadii::all(radius);
        self
    }

    pub fn corner_radii(mut self, radii: CornerRadii) -> Self {
        self.element.box_style.decoration.corner_radii = radii;
        self
    }

    pub fn uniform_border(mut self, width: f32, color: ColorRgba8) -> Self {
        self.element.box_style.decoration.border = Border::all(width, color);
        self
    }

    pub fn border_sides(mut self, border: Border) -> Self {
        self.element.box_style.decoration.border = border;
        self
    }

    pub fn outline(mut self, outline: Outline) -> Self {
        self.element.box_style.decoration.outline = outline;
        self
    }

    pub fn shadow(mut self, shadow: Shadow) -> Self {
        self.element.box_style.decoration.shadows = ShadowList::one(shadow);
        self
    }

    pub fn shadows(mut self, shadows: ShadowList) -> Self {
        self.element.box_style.decoration.shadows = shadows;
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.element.box_style.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn overflow(mut self, overflow: Overflow) -> Self {
        self.element.box_style.overflow = overflow;
        self
    }

    /// Fits one line's height to the content box, independently of text width.
    /// Overrides font size and line spacing using a 1.25 line-height ratio.
    /// Shrink-to-content height uses the normal font size to avoid circular sizing.
    pub fn fit_height(mut self, enabled: bool) -> Self {
        self.element.style.fit_height = Some(enabled);
        self
    }

    pub fn vertical_align(mut self, alignment: Alignment) -> Self {
        self.element.style.vertical_align = Some(alignment);
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.element.style.size = Some(size);
        self
    }
    /// Select a registered or system font family by its embedded family name.
    pub fn font_family(mut self, family: &'static str) -> Self {
        self.element.style.font_family = Some(family);
        self
    }

    pub fn line_height(mut self, line_height: f32) -> Self {
        self.element.style.line_height = Some(line_height);
        self
    }

    pub fn color(mut self, color: ColorRgba8) -> Self {
        self.element.style.color = Some(color);
        self
    }

    pub fn weight(mut self, weight: u16) -> Self {
        self.element.style.weight = Some(weight);
        self
    }

    /// Aligns glyph lines within this text element's content box.
    pub fn text_align(mut self, alignment: Alignment) -> Self {
        self.element.style.text_align = Some(alignment);
        self
    }

    /// Replaces this text builder's complete local style override.
    ///
    /// Fluent setters called afterward override individual fields, matching container/`BoxStyle`
    /// ordering semantics.
    pub fn text_style(mut self, style: TextStyle) -> Self {
        self.element.style = style;
        self
    }

    #[deprecated(since = "0.1.12", note = "use `text_style` for normalized vocabulary")]
    pub fn style(self, style: TextStyle) -> Self {
        self.text_style(style)
    }
}

impl View for Text {
    fn into_element(self) -> Element {
        let element = Element::from_kind(self.key, ElementKind::Text(self.element));
        match self.pointer_request {
            Some(request) => element.with_pointer_request(request),
            None => element,
        }
    }
}

pub fn text(content: impl ToString) -> Text {
    Text {
        key: None,
        pointer_request: None,
        element: TextElement {
            content: content.to_string(),
            style: TextStyle::default(),
            box_style: BoxStyle::default(),
            layout: LayoutStyle::default(),
            effects: Default::default(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_style_and_fluent_setters_share_one_sparse_override() {
        let color = ColorRgba8::rgba(230, 232, 238, 255);
        let view = text("Heading")
            .text_style(TextStyle::new().size(24.0).weight(600).color(color))
            .cursor(crate::CursorIcon::Help)
            .size(28.0)
            .into_element();
        let ElementKind::Text(text) = view.kind() else {
            panic!("expected text")
        };

        assert_eq!(text.style.size, Some(28.0));
        assert_eq!(text.style.weight, Some(600));
        assert_eq!(text.style.color, Some(color));
        assert_eq!(text.style.line_height, None);

        let resolved = text.style.resolve();
        assert_eq!(resolved.size, 28.0);
        assert_eq!(resolved.line_height, 35.0);
        assert_eq!(resolved.weight, 600);
        assert_eq!(resolved.color, color);
        assert_eq!(view.into_parts().4, Some(crate::CursorIcon::Help.into()));
    }

    #[test]
    fn text_alignment_uses_the_shared_authoring_alignment() {
        let view = text("Centered")
            .text_style(TextStyle::new().text_align(Alignment::End))
            .text_align(Alignment::Center)
            .into_element();
        let ElementKind::Text(text) = view.kind() else {
            panic!("expected text")
        };

        assert_eq!(text.style.text_align, Some(Alignment::Center));
        assert_eq!(text.style.resolve().align, crate::ui::TextAlign::Center);
    }
}
