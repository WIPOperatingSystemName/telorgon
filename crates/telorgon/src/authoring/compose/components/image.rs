use crate::assets::ImageSource;
use crate::foundation::ColorRgba8;
use crate::ui::{BoxStyle, ImageId, LayoutStyle};

use crate::authoring::compose::{Dimension, Element, ElementKind, Key, View};

#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct ImageElement {
    pub image: ImageId,
    pub tint: Option<ColorRgba8>,
    pub content_version: u64,
    pub accessible_label: Option<String>,
    pub style: BoxStyle,
    pub layout: LayoutStyle,
}

#[derive(Clone, Debug)]
pub struct Image {
    key: Option<Key>,
    element: ImageElement,
}

impl Image {
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

    pub fn layout(mut self, layout: LayoutStyle) -> Self {
        self.element.layout = layout;
        self
    }
}

impl View for Image {
    fn into_element(self) -> Element {
        Element::from_kind(self.key, ElementKind::Image(self.element))
    }
}

pub fn image(image: impl Into<ImageSource>) -> Image {
    let source = image.into();
    Image {
        key: None,
        element: ImageElement {
            image: source.image_id(),
            tint: source.tint_color(),
            content_version: 1,
            accessible_label: None,
            style: BoxStyle::default(),
            layout: LayoutStyle::default(),
        },
    }
}
