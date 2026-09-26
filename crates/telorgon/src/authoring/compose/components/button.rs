use std::sync::Arc;

use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::ui::{
    Background, Border, BoxDecoration, BoxStyle, ComponentStyleId, CornerRadii, SizeRule,
    SizeRule2D, StylePropertyPatch, ThemeDomainId,
};

use crate::authoring::compose::{
    Component, ComponentCallback, Element, ElementKind, Insets, Key, View,
};

#[doc(hidden)]
#[derive(Debug)]
pub struct ButtonElement {
    pub accessible_label: Option<String>,
    pub children: Vec<Element>,
    pub(crate) content_style_slot: Option<crate::ui::StyleSlotId>,
    pub enabled: bool,
    pub busy: bool,
    pub style: BoxStyle,
    pub style_id: ComponentStyleId,
    pub style_override: StylePropertyPatch,
    pub inline_style: Option<Arc<crate::theme::CompiledComponentStyle>>,
    pub on_press: Option<ComponentCallback>,
}

#[derive(Debug)]
pub struct Button {
    key: Option<Key>,
    element: ButtonElement,
}

impl Button {
    pub(crate) fn content_style_slot(mut self, slot: crate::ui::StyleSlotId) -> Self {
        self.element.content_style_slot = Some(slot);
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

    /// Overrides the accessible name normally derived from child content.
    pub fn accessible_label(mut self, label: impl Into<String>) -> Self {
        self.element.accessible_label = Some(label.into());
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn on_press<C, F>(mut self, callback: F) -> Self
    where
        C: Component,
        F: Fn(&mut C) + 'static,
    {
        self.element.on_press = Some(ComponentCallback::for_component(
            move |component, _event| callback(component),
        ));
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.element.enabled = enabled;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.element.busy = busy;
        self
    }

    pub fn primary(self) -> Self {
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

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.element.style.decoration.background = background.into();
        self
    }

    pub fn uniform_border(mut self, width: f32, color: ColorRgba8) -> Self {
        self.element.style.decoration.border = Border::all(width, color);
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.element.style.decoration.corner_radii = CornerRadii::all(radius);
        self
    }

    pub fn width(mut self, width: impl Into<crate::authoring::compose::Dimension>) -> Self {
        self.element.style.width = width.into().into();
        self
    }

    pub fn height(mut self, height: impl Into<crate::authoring::compose::Dimension>) -> Self {
        self.element.style.height = height.into().into();
        self
    }

    pub fn overflow(mut self, overflow: crate::ui::Overflow) -> Self {
        self.element.style.overflow = overflow;
        self
    }

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.element.style.padding = padding.into().0;
        self
    }

    #[deprecated(since = "0.1.12", note = "use `corner_radius`")]
    pub fn radius(self, radius: f32) -> Self {
        self.corner_radius(radius)
    }

    pub fn style_id(mut self, style: ComponentStyleId) -> Self {
        self.element.style_id = style;
        self
    }

    pub fn style_override(mut self, style: StylePropertyPatch) -> Self {
        self.element.style_override = style;
        self
    }

    /// Installs one code-defined state style without registering it in the application theme.
    #[doc(hidden)]
    pub fn inline_style(mut self, style: Arc<crate::theme::CompiledComponentStyle>) -> Self {
        self.element.inline_style = Some(style);
        self
    }
}

impl View for Button {
    fn into_element(self) -> Element {
        Element::from_kind(self.key, ElementKind::Button(self.element))
    }
}

pub fn button() -> Button {
    let style = BoxStyle {
        overflow: crate::ui::Overflow::Clip,
        min_size: SizeRule2D {
            width: SizeRule::Logical(32.0),
            height: SizeRule::Logical(32.0),
        },
        padding: EdgeInsets {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        },
        decoration: crate::ui::BoxDecoration {
            background: Background::Color(ColorRgba8::rgba(54, 60, 74, 255)),
            corner_radii: CornerRadii::all(0.0),
            ..crate::ui::BoxDecoration::default()
        },
        ..BoxStyle::default()
    };
    Button {
        key: None,
        element: ButtonElement {
            accessible_label: None,
            children: Vec::new(),
            content_style_slot: None,
            enabled: true,
            busy: false,
            style,
            style_id: ComponentStyleId::named(ThemeDomainId::APPLICATION, "button", "default"),
            style_override: StylePropertyPatch::default(),
            inline_style: None,
            on_press: None,
        },
    }
}
