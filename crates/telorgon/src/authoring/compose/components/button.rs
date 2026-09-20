use std::sync::Arc;

use crate::assets::ImageSource;
use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::ui::{
    Background, Border, BoxDecoration, BoxStyle, ComponentStyleId, CornerRadii, ImageId, SizeRule,
    SizeRule2D, StylePropertyPatch, TextAlign, TextStyle as RetainedTextStyle, ThemeDomainId,
};

use crate::authoring::compose::{Component, ComponentCallback, Element, ElementKind, Insets, Key, View};

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct ButtonElement {
    pub label: String,
    pub font_family: Option<&'static str>,
    pub enabled: bool,
    pub busy: bool,
    pub style: BoxStyle,
    pub label_style: RetainedTextStyle,
    pub style_id: ComponentStyleId,
    pub style_override: StylePropertyPatch,
    pub inline_style: Option<Arc<crate::theme::CompiledComponentStyle>>,
    pub icon: Option<ImageId>,
    pub icon_tint: Option<ColorRgba8>,
    pub icon_size: f32,
    pub on_press: Option<ComponentCallback>,
}

#[derive(Clone, Debug)]
pub struct Button {
    key: Option<Key>,
    element: ButtonElement,
}

impl Button {
    pub fn font_family(mut self, family: &'static str) -> Self {
        self.element.font_family = Some(family);
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

    /// Replaces the visible text with registered icon artwork while retaining `label` as the
    /// button's accessible name.
    pub fn icon(mut self, icon: impl Into<ImageSource>) -> Self {
        let source = icon.into();
        self.element.icon = Some(source.image_id());
        self.element.icon_tint = source.tint_color();
        self
    }

    /// Recolors icon artwork from its alpha mask without affecting the accessible label.
    pub fn icon_tint(mut self, color: ColorRgba8) -> Self {
        self.element.icon_tint = Some(color);
        self
    }

    pub fn without_icon_tint(mut self) -> Self {
        self.element.icon_tint = None;
        self
    }

    pub fn icon_size(mut self, size: f32) -> Self {
        self.element.icon_size = size.max(1.0);
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

pub fn button(label: impl Into<String>) -> Button {
    let style = BoxStyle {
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
            label: label.into(),
            font_family: None,
            enabled: true,
            busy: false,
            style,
            label_style: RetainedTextStyle {
                color: ColorRgba8::rgba(248, 249, 252, 255),
                size: 14.0,
                line_height: 17.5,
                family: crate::ui::StringId(1),
                weight: 400,
                align: TextAlign::Center,
            },
            style_id: ComponentStyleId::named(ThemeDomainId::APPLICATION, "button", "default"),
            style_override: StylePropertyPatch::default(),
            inline_style: None,
            icon: None,
            icon_tint: None,
            icon_size: 18.0,
            on_press: None,
        },
    }
}
