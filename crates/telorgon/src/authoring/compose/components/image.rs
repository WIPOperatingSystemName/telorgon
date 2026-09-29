use crate::assets::ImageSource;
use crate::foundation::ColorRgba8;
use crate::ui::{
    Background, Border, BoxDecoration, BoxStyle, CornerRadii, ImageId, LayoutStyle, Outline,
    Overflow, Shadow, ShadowList,
};

use crate::authoring::compose::{Dimension, Element, ElementKind, Insets, Key, View};

#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct ImageElement {
    pub image: ImageId,
    pub tint: Option<ColorRgba8>,
    pub content_version: u64,
    pub accessible_label: Option<String>,
    pub style: BoxStyle,
    pub layout: LayoutStyle,
    pub effects: super::interaction::InteractionEffects,
}

#[derive(Clone, Debug)]
pub struct Image {
    key: Option<Key>,
    pointer_request: Option<crate::PointerRequest>,
    element: ImageElement,
}

impl Image {
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

    /// Bind owned pixels while evaluating a composed view. The host admits the exact
    /// resource revision with this image's draw and releases it when no longer referenced.
    pub fn resource(resource: crate::graphics::render::ImageResource) -> Self {
        let revision = resource.content_version;
        image(crate::authoring::compose::context::bind_image(resource)).content_version(revision)
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn content_version(mut self, content_version: u64) -> Self {
        self.element.content_version = content_version.max(1);
        self
    }

    pub fn accessible_label(mut self, label: impl Into<String>) -> Self {
        self.element.accessible_label = Some(label.into());
        self
    }

    /// Recolors the image from its alpha mask while preserving transparent edges.
    pub fn tint(mut self, color: ColorRgba8) -> Self {
        self.element.tint = Some(color);
        self
    }

    pub fn without_tint(mut self) -> Self {
        self.element.tint = None;
        self
    }

    pub fn box_style(mut self, style: BoxStyle) -> Self {
        self.element.style = style;
        self
    }

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.element.style.padding = padding.into().0;
        self
    }

    pub fn margin(mut self, margin: impl Into<Insets>) -> Self {
        self.element.style.margin = margin.into().0;
        self
    }

    pub fn decoration(mut self, decoration: BoxDecoration) -> Self {
        self.element.style.decoration = decoration;
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

    pub fn uniform_border(mut self, width: f32, color: ColorRgba8) -> Self {
        self.element.style.decoration.border = Border::all(width, color);
        self
    }

    pub fn border_sides(mut self, border: Border) -> Self {
        self.element.style.decoration.border = border;
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

    pub fn overflow(mut self, overflow: Overflow) -> Self {
        self.element.style.overflow = overflow;
        self
    }

    #[deprecated(since = "0.1.12", note = "use `box_style` for normalized vocabulary")]
    pub fn style(self, style: BoxStyle) -> Self {
        self.box_style(style)
    }

    /// Derives an automatic axis from the other using width divided by height.
    pub fn aspect_ratio(mut self, ratio: f32) -> Self {
        self.element.style.aspect_ratio = Some(ratio);
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

    pub fn layout_style(mut self, layout: LayoutStyle) -> Self {
        self.element.layout = layout;
        self
    }

    #[deprecated(
        since = "0.1.12",
        note = "use `layout_style` for normalized vocabulary"
    )]
    pub fn layout(self, layout: LayoutStyle) -> Self {
        self.layout_style(layout)
    }
}

impl View for Image {
    fn into_element(self) -> Element {
        let element = Element::from_kind(self.key, ElementKind::Image(self.element));
        match self.pointer_request {
            Some(request) => element.with_pointer_request(request),
            None => element,
        }
    }
}

pub fn image(image: impl Into<ImageSource>) -> Image {
    let source = image.into();
    Image {
        key: None,
        pointer_request: None,
        element: ImageElement {
            image: source.image_id(),
            tint: source.tint_color(),
            content_version: 1,
            accessible_label: None,
            style: BoxStyle::default(),
            layout: LayoutStyle::default(),
            effects: Default::default(),
        },
    }
}
