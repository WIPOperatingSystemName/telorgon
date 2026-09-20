use crate::ui::SemanticCheckState;

use crate::authoring::compose::{
    Component, ComponentCallback, Element, ElementKind, EventContext, Key, ToggleElement,
    ToggleKind, View,
};

#[derive(Clone, Debug)]
pub struct Switch {
    key: Option<Key>,
    element: ToggleElement,
}

impl Switch {
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
        F: Fn(&mut C, bool) + 'static,
    {
        self.on_change_event(move |component, event| {
            if let Some(checked) = event.checked() {
                callback(component, checked == SemanticCheckState::Checked);
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

impl View for Switch {
    fn into_element(self) -> Element {
        Element::from_kind(self.key, ElementKind::Toggle(self.element))
    }
}

pub fn switch(label: impl Into<String>, value: bool) -> Switch {
    Switch {
        key: None,
        element: ToggleElement {
            kind: ToggleKind::Switch,
            label: label.into(),
            value: if value {
                SemanticCheckState::Checked
            } else {
                SemanticCheckState::Unchecked
            },
            enabled: true,
            on_change: None,
        },
    }
}
