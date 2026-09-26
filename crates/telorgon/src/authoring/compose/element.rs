use std::any::TypeId;
use std::collections::HashSet;
use std::fmt;

use crate::authoring::compose::{
    ButtonElement, Component, ContainerElement, ErasedComponent, ImageElement, Key, SliderElement,
    TextElement, ToggleElement, ToggleKind,
};

/// One short-lived UI description that can be erased into an owned [`Element`].
pub trait View: 'static {
    fn into_element(self) -> Element;

    fn element(self) -> Element
    where
        Self: Sized,
    {
        self.into_element()
    }

    /// Assigns identity local to this view's immediate parent.
    fn keyed(self, key: impl Into<Key>) -> Element
    where
        Self: Sized,
    {
        self.into_element().key(key)
    }
}

/// Opaque owned view element. It is normally produced by a builder or component value.
pub struct Element {
    key: Option<Key>,
    kind: ElementKind,
    window_chrome_role: Option<crate::shell::window_chrome::WindowChromeRole>,
    window_chrome_hit_spec: Option<crate::shell::window_chrome::WindowChromeHitSpec>,
    pointer_request: Option<crate::assets::PointerRequest>,
}

impl fmt::Debug for Element {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Element")
            .field("key", &self.key)
            .field("kind", &self.kind)
            .field("window_chrome_role", &self.window_chrome_role)
            .field("window_chrome_hit_spec", &self.window_chrome_hit_spec)
            .field("pointer_request", &self.pointer_request)
            .finish()
    }
}

impl Element {
    pub(crate) const fn from_kind(key: Option<Key>, kind: ElementKind) -> Self {
        Self {
            key,
            kind,
            window_chrome_role: None,
            window_chrome_hit_spec: None,
            pointer_request: None,
        }
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn key_ref(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    #[doc(hidden)]
    pub fn into_parts(
        self,
    ) -> (
        Option<Key>,
        ElementKind,
        Option<crate::shell::window_chrome::WindowChromeRole>,
        Option<crate::shell::window_chrome::WindowChromeHitSpec>,
        Option<crate::assets::PointerRequest>,
    ) {
        (
            self.key,
            self.kind,
            self.window_chrome_role,
            self.window_chrome_hit_spec,
            self.pointer_request,
        )
    }

    #[doc(hidden)]
    pub fn with_window_chrome_role(mut self, role: crate::shell::window_chrome::WindowChromeRole) -> Self {
        self.window_chrome_role = Some(role);
        self
    }

    #[doc(hidden)]
    pub fn with_window_chrome_hit_slop(mut self, hit_slop: crate::foundation::EdgeInsets) -> Self {
        let default = self
            .window_chrome_role
            .map(crate::shell::window_chrome::WindowChromeHitSpec::for_role)
            .unwrap_or_default();
        self.window_chrome_hit_spec = Some(
            self.window_chrome_hit_spec
                .unwrap_or(default)
                .hit_slop(hit_slop),
        );
        self
    }

    #[doc(hidden)]
    pub fn with_window_chrome_border_hit(mut self, outset: crate::foundation::EdgeInsets) -> Self {
        let default = self
            .window_chrome_role
            .map(crate::shell::window_chrome::WindowChromeHitSpec::for_role)
            .unwrap_or_default();
        let spec = self.window_chrome_hit_spec.get_or_insert(default);
        spec.frame_border_outset = Some(outset);
        self
    }

    #[doc(hidden)]
    pub fn with_window_chrome_hit_priority(mut self, priority: u16) -> Self {
        let default = self
            .window_chrome_role
            .map(crate::shell::window_chrome::WindowChromeHitSpec::for_role)
            .unwrap_or_default();
        self.window_chrome_hit_spec = Some(
            self.window_chrome_hit_spec
                .unwrap_or(default)
                .priority(priority),
        );
        self
    }

    #[doc(hidden)]
    pub fn with_pointer_request(mut self, request: crate::assets::PointerRequest) -> Self {
        self.pointer_request = Some(request);
        self
    }

    #[doc(hidden)]
    pub fn kind(&self) -> &ElementKind {
        &self.kind
    }

    pub fn validate(&self) -> Result<(), ViewError> {
        validate_element(self, None)
    }

    #[doc(hidden)]
    pub fn validate_for_component(
        &self,
        component_type: TypeId,
        component_name: &'static str,
    ) -> Result<(), ViewError> {
        validate_element(self, Some((component_type, component_name)))
    }
}

impl View for Element {
    fn into_element(self) -> Element {
        self
    }
}

impl<C: Component> View for C {
    fn into_element(self) -> Element {
        Element::from_kind(None, ElementKind::Component(Box::new(self)))
    }
}

/// Runtime-facing element payloads. Application code should construct these through builders.
#[doc(hidden)]
pub enum ElementKind {
    Container(ContainerElement),
    Text(TextElement),
    Image(ImageElement),
    Button(ButtonElement),
    Toggle(ToggleElement),
    Slider(SliderElement),
    Component(Box<dyn ErasedComponent>),
}

impl fmt::Debug for ElementKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Container(value) => value.fmt(formatter),
            Self::Text(value) => value.fmt(formatter),
            Self::Image(value) => value.fmt(formatter),
            Self::Button(value) => value.fmt(formatter),
            Self::Toggle(value) => value.fmt(formatter),
            Self::Slider(value) => value.fmt(formatter),
            Self::Component(value) => formatter
                .debug_tuple("Component")
                .field(&value.component_type_name())
                .finish(),
        }
    }
}

impl ElementKind {
    #[doc(hidden)]
    pub fn identity(&self) -> ElementType {
        match self {
            Self::Container(_) => ElementType::Container,
            Self::Text(_) => ElementType::Text,
            Self::Image(_) => ElementType::Image,
            Self::Button(_) => ElementType::Button,
            Self::Toggle(toggle) => match toggle.kind {
                ToggleKind::Checkbox => ElementType::Checkbox,
                ToggleKind::Switch => ElementType::Switch,
            },
            Self::Slider(_) => ElementType::Slider,
            Self::Component(component) => ElementType::Component(component.component_type_id()),
        }
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElementType {
    Container,
    Text,
    Image,
    Button,
    Checkbox,
    Switch,
    Slider,
    Component(TypeId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewError {
    DuplicateKey(Key),
    InvalidNumber(&'static str),
    MissingButtonLabel,
    CallbackTypeMismatch {
        expected: &'static str,
        actual: &'static str,
    },
    ComponentTypeMismatch {
        expected: &'static str,
        actual: &'static str,
    },
    StaleParent,
}

impl fmt::Display for ViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateKey(key) => write!(formatter, "duplicate local element key: {key:?}"),
            Self::InvalidNumber(property) => {
                write!(formatter, "view property {property} must be finite")
            }
            Self::MissingButtonLabel => formatter.write_str("button accessible label is empty"),
            Self::CallbackTypeMismatch { expected, actual } => write!(
                formatter,
                "callback component type mismatch: expected {expected}, received {actual}"
            ),
            Self::ComponentTypeMismatch { expected, actual } => write!(
                formatter,
                "component candidate type mismatch: expected {expected}, received {actual}"
            ),
            Self::StaleParent => formatter.write_str("view parent node is stale"),
        }
    }
}

impl std::error::Error for ViewError {}

fn validate_element(
    element: &Element,
    component: Option<(TypeId, &'static str)>,
) -> Result<(), ViewError> {
    let style = match &element.kind {
        ElementKind::Container(value) => Some(&value.style),
        ElementKind::Button(value) => Some(&value.style),
        ElementKind::Image(value) => Some(&value.style),
        ElementKind::Text(value) => Some(&value.box_style),
        _ => None,
    };
    if style.and_then(|style| style.aspect_ratio).is_some_and(|ratio| !ratio.is_finite() || ratio <= 0.0) {
        return Err(ViewError::InvalidNumber("aspect ratio"));
    }
    match &element.kind {
        ElementKind::Container(container) => {
            if !container.layout.gap.is_finite() {
                return Err(ViewError::InvalidNumber("gap"));
            }
            let mut keys = HashSet::new();
            for child in &container.children {
                if let Some(key) = child.key_ref()
                    && !keys.insert(key)
                {
                    return Err(ViewError::DuplicateKey(key.clone()));
                }
                validate_element(child, component)?;
            }
        }
        ElementKind::Text(text) => {
            let resolved = text.style.resolve();
            if !resolved.size.is_finite() || !resolved.line_height.is_finite() {
                return Err(ViewError::InvalidNumber("text metrics"));
            }
        }
        ElementKind::Image(image) => {
            if image.content_version == 0 {
                return Err(ViewError::InvalidNumber("image content version"));
            }
            if image
                .accessible_label
                .as_ref()
                .is_some_and(|label| label.trim().is_empty())
            {
                return Err(ViewError::MissingButtonLabel);
            }
        }
        ElementKind::Button(button) => {
            if button.invalid_interaction_effect {
                return Err(ViewError::InvalidNumber("interaction effect or repeating interaction transition"));
            }
            if button.accessible_label.as_ref().is_some_and(|label| label.trim().is_empty()) {
                return Err(ViewError::MissingButtonLabel);
            }
            let mut keys = HashSet::new();
            for child in &button.children {
                if let Some(key) = child.key_ref() && !keys.insert(key) {
                    return Err(ViewError::DuplicateKey(key.clone()));
                }
                validate_element(child, component)?;
            }
            validate_callback(button.on_press.as_ref(), component)?;
        }
        ElementKind::Toggle(toggle) => {
            if toggle.label.trim().is_empty() {
                return Err(ViewError::MissingButtonLabel);
            }
            validate_callback(toggle.on_change.as_ref(), component)?;
        }
        ElementKind::Slider(slider) => {
            if slider.accessible_label.as_deref().unwrap_or(&slider.label).trim().is_empty() {
                return Err(ViewError::MissingButtonLabel);
            }
            if !slider.value.is_finite() {
                return Err(ViewError::InvalidNumber("slider value"));
            }
            validate_callback(slider.on_change.as_ref(), component)?;
        }
        ElementKind::Component(_) => {}
    }
    Ok(())
}

fn validate_callback(
    callback: Option<&crate::authoring::compose::ComponentCallback>,
    component: Option<(TypeId, &'static str)>,
) -> Result<(), ViewError> {
    let (Some(callback), Some((component_type, component_name))) = (callback, component) else {
        return Ok(());
    };
    if callback.component_type_id() != component_type {
        return Err(ViewError::CallbackTypeMismatch {
            expected: component_name,
            actual: callback.component_type_name(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::compose::{column, text};

    #[test]
    fn duplicate_keys_are_rejected_before_reconciliation() {
        let view = column()
            .child(text("one").key(7_u64))
            .child(text("two").key(7_u64))
            .into_element();
        assert_eq!(
            view.validate(),
            Err(ViewError::DuplicateKey(Key::Integer(7)))
        );
    }
}
