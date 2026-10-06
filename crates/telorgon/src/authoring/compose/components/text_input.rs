use crate::authoring::compose::{
    Component, Container, ContainerElement, Element, ElementKind, InputCallback, Key, View, column,
};

/// A focusable editor surface. Editing and presentation are owned by its input callback.
#[derive(Debug)]
pub struct TextInput {
    content: Container,
    label: Option<String>,
    value: String,
    enabled: bool,
    read_only: bool,
    secure: bool,
    autofocus: bool,
    ime_cursor_rect: Option<crate::foundation::RectF>,
    on_input: Option<InputCallback>,
}

#[doc(hidden)]
#[derive(Debug)]
pub struct TextInputElement {
    pub content: ContainerElement,
    pub accessible_label: Option<String>,
    pub value: String,
    pub enabled: bool,
    pub read_only: bool,
    pub secure: bool,
    pub autofocus: bool,
    pub ime_cursor_rect: Option<crate::foundation::RectF>,
    pub on_input: Option<InputCallback>,
}

macro_rules! content_method {
    ($name:ident, $arg:ident: $ty:ty) => {
        pub fn $name(mut self, $arg: $ty) -> Self {
            self.content = self.content.$name($arg);
            self
        }
    };
}

impl TextInput {
    content_method!(key, key: impl Into<Key>);
    content_method!(cursor, icon: impl Into<crate::CursorIcon>);
    content_method!(width, width: impl Into<crate::authoring::compose::Dimension>);
    content_method!(height, height: impl Into<crate::authoring::compose::Dimension>);
    content_method!(padding, padding: impl Into<crate::authoring::compose::Insets>);
    content_method!(margin, margin: impl Into<crate::authoring::compose::Insets>);
    content_method!(background, background: impl Into<crate::ui::Background>);
    content_method!(corner_radius, radius: f32);
    content_method!(overflow, overflow: crate::ui::Overflow);
    content_method!(box_style, style: crate::ui::BoxStyle);
    content_method!(layout_style, layout: crate::ui::LayoutStyle);
    content_method!(child, child: impl View);
    content_method!(align_items, alignment: crate::authoring::compose::Alignment);
    content_method!(justify_content, alignment: crate::authoring::compose::Alignment);

    pub fn uniform_border(mut self, width: f32, color: crate::foundation::ColorRgba8) -> Self {
        self.content = self.content.uniform_border(width, color);
        self
    }
    pub fn accessible_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }
    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }
    /// Requests focus when this editor is mounted, or when this flag becomes true.
    pub fn autofocus(mut self, autofocus: bool) -> Self {
        self.autofocus = autofocus;
        self
    }
    /// Candidate-window caret rectangle in logical coordinates relative to this control's border.
    pub fn ime_cursor_rect(mut self, rect: crate::foundation::RectF) -> Self {
        self.ime_cursor_rect = Some(rect);
        self
    }
    pub fn on_input<C, F>(mut self, callback: F) -> Self
    where
        C: Component,
        F: Fn(&mut C, &crate::ui::UiEvent) -> bool + 'static,
    {
        self.on_input = Some(InputCallback::new(callback));
        self
    }
}

impl View for TextInput {
    fn into_element(self) -> Element {
        let (key, kind, _, _, pointer) = self.content.into_element().into_parts();
        let ElementKind::Container(content) = kind else {
            unreachable!()
        };
        let element = Element::from_kind(
            key,
            ElementKind::TextInput(TextInputElement {
                content,
                accessible_label: self.label,
                value: self.value,
                enabled: self.enabled,
                read_only: self.read_only,
                secure: self.secure,
                autofocus: self.autofocus,
                ime_cursor_rect: self.ime_cursor_rect,
                on_input: self.on_input,
            }),
        );
        match pointer {
            Some(pointer) => element.with_pointer_request(pointer),
            None => element,
        }
    }
}

pub fn text_input() -> TextInput {
    TextInput {
        content: column()
            .height(34.0)
            .padding(8.0)
            .cursor(crate::CursorIcon::Text),
        label: None,
        value: String::new(),
        enabled: true,
        read_only: false,
        secure: false,
        autofocus: false,
        ime_cursor_rect: None,
        on_input: None,
    }
}
