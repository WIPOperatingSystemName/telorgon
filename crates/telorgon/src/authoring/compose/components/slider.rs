use crate::authoring::compose::{
    Component, ComponentCallback, Element, ElementKind, EventContext, Key, View,
};

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct SliderElement {
    pub label: String,
    pub accessible_label: Option<String>,
    pub value: f32,
    pub enabled: bool,
    pub style: crate::ui::BoxStyle,
    pub(crate) effects: super::interaction::InteractionEffects,
    pub(crate) thumb_press_effects: Vec<super::interaction::InteractionEffect>,
    pub on_change: Option<ComponentCallback>,
}

#[derive(Clone, Debug)]
pub struct Slider {
    key: Option<Key>,
    pointer_request: Option<crate::PointerRequest>,
    element: SliderElement,
}

impl Slider {
    /// Styles the thumb while pressed, including a captured pointer drag.
    pub fn thumb_press_effect(mut self, effect: super::interaction::InteractionEffect) -> Self {
        self.element.thumb_press_effects.push(effect);
        self
    }

    pub fn thumb_press_effects(
        mut self,
        effects: impl IntoIterator<Item = super::interaction::InteractionEffect>,
    ) -> Self {
        self.element.thumb_press_effects.extend(effects);
        self
    }

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
        let element = Element::from_kind(self.key, ElementKind::Slider(self.element));
        match self.pointer_request {
            Some(request) => element.with_pointer_request(request),
            None => element,
        }
    }
}

pub fn slider(label: impl Into<String>, value: f32) -> Slider {
    Slider {
        key: None,
        pointer_request: None,
        element: SliderElement {
            label: label.into(),
            accessible_label: None,
            value: value.clamp(0.0, 1.0),
            enabled: true,
            style: crate::ui::BoxStyle {
                height: crate::ui::SizeRule::Logical(32.0),
                padding: crate::foundation::EdgeInsets {
                    top: 5.0,
                    bottom: 5.0,
                    left: 0.0,
                    right: 0.0,
                },
                min_size: crate::ui::SizeRule2D {
                    width: crate::ui::SizeRule::Logical(32.0),
                    height: crate::ui::SizeRule::Logical(32.0),
                },
                ..Default::default()
            },
            effects: Default::default(),
            thumb_press_effects: Vec::new(),
            on_change: None,
        },
    }
}

impl SliderElement {
    pub(crate) fn style_id(&self) -> crate::ui::ComponentStyleId {
        crate::ui::ComponentStyleId::named(
            crate::ui::ThemeDomainId::APPLICATION,
            "slider",
            "default",
        )
    }

    pub(crate) fn local_style(
        &self,
        thumb: &crate::ui::BoxStyle,
    ) -> Option<std::sync::Arc<crate::theme::CompiledComponentStyle>> {
        let root = self
            .effects
            .compile_for(self.style_id(), &self.style, Default::default(), None);
        if self.thumb_press_effects.is_empty() {
            return root;
        }
        let mut compiled = crate::authoring::compose::compile_slot_effects(
            self.style_id(),
            thumb,
            Default::default(),
            root.as_deref(),
            crate::ui::StyleSlotId::named("thumb"),
            crate::theme::InteractionState::Pressed,
            &self.thumb_press_effects,
            self.effects
                .press_transition
                .unwrap_or(crate::theme::TransitionSpec {
                    duration_ms: 120,
                    ..Default::default()
                }),
        );
        if let Some(root) = root {
            std::sync::Arc::make_mut(&mut compiled).transition = root.transition;
        }
        Some(compiled)
    }

    pub(crate) fn thumb_effect_mask(&self) -> u8 {
        self.thumb_press_effects
            .iter()
            .fold(0, |mask, effect| mask | effect.property())
    }
}
