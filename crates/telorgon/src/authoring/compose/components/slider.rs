use crate::authoring::compose::{Component, ComponentCallback, Element, ElementKind, EventContext, Key, View};

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct SliderElement {
    pub label: String,
    pub value: f32,
    pub enabled: bool,
    pub on_change: Option<ComponentCallback>,
}

#[derive(Clone, Debug)]
pub struct Slider {
    key: Option<Key>,
    element: SliderElement,
}

impl Slider {
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.element.enabled = enabled;
        self
    }

    pub fn on_change<C, F>(self, callback: F) -> Self
    where
        C: Component,
        F: Fn(&mut C, f32) + 'static,
    {
        self.on_change_event(move |component, event| {
            if let Some(value) = event.value() {
                callback(component, value);
            }
        })
    }

    pub fn on_change_event<C, F>(mut self, callback: F) -> Self
    where
        C: Component,
        F: Fn(&mut C, &mut EventContext) + 'static,
    {
        self.element.on_change = Some(ComponentCallback::for_component(callback));
        self
    }
}

impl View for Slider {
    fn into_element(self) -> Element {
        Element::from_kind(self.key, ElementKind::Slider(self.element))
    }
}

pub fn slider(label: impl Into<String>, value: f32) -> Slider {
    Slider {
        key: None,
        element: SliderElement {
            label: label.into(),
            value: value.clamp(0.0, 1.0),
            enabled: true,
            on_change: None,
        },
    }
}
