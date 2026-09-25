use crate::ui::SemanticCheckState;

use crate::authoring::compose::{
    Component, ComponentCallback, Element, ElementKind, EventContext, Key, ToggleElement,
    ToggleKind, View,
};

#[derive(Clone, Debug)]
pub struct Checkbox {
    key: Option<Key>,
    element: ToggleElement,
}

impl Checkbox {
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.element.enabled = enabled;
        self
    }

    pub fn mixed(mut self) -> Self {
        self.element.value = SemanticCheckState::Mixed;
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

impl View for Checkbox {
    fn into_element(self) -> Element {
        Element::from_kind(self.key, ElementKind::Toggle(self.element))
    }
}

pub fn checkbox(label: impl Into<String>, checked: bool) -> Checkbox {
    Checkbox {
        key: None,
        element: ToggleElement {
            kind: ToggleKind::Checkbox,
            label: label.into(),
            value: if checked {
                SemanticCheckState::Checked
            } else {
                SemanticCheckState::Unchecked
            },
            enabled: true,
            width: None,
            on_change: None,
        },
    }
}
