use crate::ui::SemanticCheckState;

use crate::authoring::compose::{
    Component, ComponentCallback, Element, ElementKind, EventContext, Key, ToggleElement,
    ToggleKind, View,
};

#[derive(Clone, Debug)]
pub struct Switch {
    key: Option<Key>,
    pointer_request: Option<crate::PointerRequest>,
    element: ToggleElement,
}

impl Switch {
    pub fn cursor(mut self, icon: impl Into<crate::CursorIcon>) -> Self {
        self.pointer_request = Some(icon.into().into());
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

    /// Replaces the outer control box without resizing its indicator, track, or thumb.
    pub fn box_style(mut self, style: crate::ui::BoxStyle) -> Self {
        self.element.style = style;
        self
    }

    pub fn padding(mut self, padding: impl Into<crate::authoring::compose::Insets>) -> Self {
        self.element.style.padding = padding.into().0;
        self
    }

    pub fn margin(mut self, margin: impl Into<crate::authoring::compose::Insets>) -> Self {
        self.element.style.margin = margin.into().0;
        self
    }

    pub fn aspect_ratio(mut self, ratio: f32) -> Self {
        self.element.style.aspect_ratio = Some(ratio);
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.element.style.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn overflow(mut self, overflow: crate::ui::Overflow) -> Self {
        self.element.style.overflow = overflow;
        self
    }

    pub fn decoration(mut self, decoration: crate::ui::BoxDecoration) -> Self {
        self.element.style.decoration = decoration;
        self
    }

    pub fn background(mut self, background: impl Into<crate::ui::Background>) -> Self {
        self.element.style.decoration.background = background.into();
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.element.style.decoration.corner_radii = crate::ui::CornerRadii::all(radius);
        self
    }

    pub fn corner_radii(mut self, radii: crate::ui::CornerRadii) -> Self {
        self.element.style.decoration.corner_radii = radii;
        self
    }

    pub fn uniform_border(mut self, width: f32, color: crate::foundation::ColorRgba8) -> Self {
        self.element.style.decoration.border = crate::ui::Border::all(width, color);
        self
    }

    pub fn border_sides(mut self, border: crate::ui::Border) -> Self {
        self.element.style.decoration.border = border;
        self
    }

    pub fn outline(mut self, outline: crate::ui::Outline) -> Self {
        self.element.style.decoration.outline = outline;
        self
    }

    pub fn shadow(mut self, shadow: crate::ui::Shadow) -> Self {
        self.element.style.decoration.shadows = crate::ui::ShadowList::one(shadow);
        self
    }

    pub fn shadows(mut self, shadows: crate::ui::ShadowList) -> Self {
        self.element.style.decoration.shadows = shadows;
        self
    }

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

    pub fn press_effect(mut self, effect: super::interaction::InteractionEffect) -> Self {
        self.element.effects.press.push(effect);
        self
    }

    pub fn press_effects(
        mut self,
        effects: impl IntoIterator<Item = super::interaction::InteractionEffect>,
    ) -> Self {
        self.element.effects.press.extend(effects);
        self
    }

    pub fn press_transition(mut self, transition: crate::theme::TransitionSpec) -> Self {
        self.element.effects.press_transition = Some(transition);
        self
    }

    /// Overrides the semantic name independently of the visible label.
    pub fn accessible_label(mut self, label: impl Into<String>) -> Self {
        self.element.accessible_label = Some(label.into());
        self
    }

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
        let element = Element::from_kind(self.key, ElementKind::Toggle(self.element));
        match self.pointer_request {
            Some(request) => element.with_pointer_request(request),
            None => element,
        }
    }
}

pub fn switch(label: impl Into<String>, value: bool) -> Switch {
    Switch {
        key: None,
        pointer_request: None,
        element: ToggleElement {
            kind: ToggleKind::Switch,
            label: label.into(),
            accessible_label: None,
            value: if value {
                SemanticCheckState::Checked
            } else {
                SemanticCheckState::Unchecked
            },
            enabled: true,
            style: super::toggle::default_box_style(),
            effects: Default::default(),
            on_change: None,
        },
    }
}
