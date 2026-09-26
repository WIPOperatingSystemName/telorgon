use std::sync::Arc;

use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::ui::{
    Background, Border, BoxDecoration, BoxStyle, ComponentStyleId, CornerRadii, SizeRule,
    SizeRule2D, StylePropertyPatch, ThemeDomainId,
};

use crate::authoring::compose::{
    Component, ComponentCallback, Element, ElementKind, Insets, Key, View,
};

#[doc(hidden)]
#[derive(Debug)]
pub struct ButtonElement {
    pub accessible_label: Option<String>,
    pub children: Vec<Element>,
    pub(crate) content_style_slot: Option<crate::ui::StyleSlotId>,
    pub enabled: bool,
    pub busy: bool,
    pub style: BoxStyle,
    pub style_id: ComponentStyleId,
    pub style_override: StylePropertyPatch,
    pub inline_style: Option<Arc<crate::theme::CompiledComponentStyle>>,
    pub on_press: Option<ComponentCallback>,
    pub(crate) invalid_interaction_effect: bool,
    pub(crate) effect_properties: u8,
    pub(crate) effect_overlay: bool,
}

#[derive(Debug)]
pub struct Button {
    key: Option<Key>,
    pointer_request: Option<crate::PointerRequest>,
    element: ButtonElement,
    hover_effects: Vec<super::interaction::InteractionEffect>,
    hover_transition: Option<crate::theme::TransitionSpec>,
    press_effects: Vec<super::interaction::InteractionEffect>,
    press_transition: Option<crate::theme::TransitionSpec>,
}

impl Button {
    pub fn cursor(mut self, icon: crate::CursorIcon) -> Self {
        self.pointer_request = Some(crate::PointerRequest::Semantic(icon));
        self
    }

    pub fn hide_pointer(mut self) -> Self {
        self.pointer_request = Some(crate::PointerRequest::Hidden);
        self
    }

    pub fn hover_effect(mut self, effect: super::interaction::InteractionEffect) -> Self {
        self.hover_effects.push(effect);
        self
    }

    pub fn hover_effects(
        mut self,
        effects: impl IntoIterator<Item = super::interaction::InteractionEffect>,
    ) -> Self {
        self.hover_effects.extend(effects);
        self
    }

    pub fn hover_transition(mut self, transition: crate::theme::TransitionSpec) -> Self {
        self.hover_transition = Some(transition);
        self
    }

    pub fn press_effect(mut self, effect: super::interaction::InteractionEffect) -> Self {
        self.press_effects.push(effect);
        self
    }

    pub fn press_effects(
        mut self,
        effects: impl IntoIterator<Item = super::interaction::InteractionEffect>,
    ) -> Self {
        self.press_effects.extend(effects);
        self
    }

    pub fn press_transition(mut self, transition: crate::theme::TransitionSpec) -> Self {
        self.press_transition = Some(transition);
        self
    }

    pub(crate) fn content_style_slot(mut self, slot: crate::ui::StyleSlotId) -> Self {
        self.element.content_style_slot = Some(slot);
        self
    }

    pub fn child(mut self, child: impl View) -> Self {
        self.element.children.push(child.into_element());
        self
    }

    pub fn children<I, V>(mut self, children: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: View,
    {
        self.element
            .children
            .extend(children.into_iter().map(View::into_element));
        self
    }

    /// Overrides the accessible name normally derived from child content.
    pub fn accessible_label(mut self, label: impl Into<String>) -> Self {
        self.element.accessible_label = Some(label.into());
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

    pub fn overflow(mut self, overflow: crate::ui::Overflow) -> Self {
        self.element.style.overflow = overflow;
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
    fn into_element(mut self) -> Element {
        if !self.hover_effects.is_empty()
            || !self.press_effects.is_empty()
            || self.hover_transition.is_some()
            || self.press_transition.is_some()
        {
            self.element.effect_properties = self
                .hover_effects
                .iter()
                .chain(&self.press_effects)
                .fold(0, |mask, effect| mask | effect.property());
            self.element.effect_overlay = self.element.inline_style.is_none();
            self.element.invalid_interaction_effect = self
                .hover_effects
                .iter()
                .chain(&self.press_effects)
                .any(|effect| !effect.valid())
                || self.hover_transition.is_some_and(|spec| spec.repeat)
                || self.press_transition.is_some_and(|spec| spec.repeat);
            self.element.inline_style = Some(super::interaction::compile(
                &self.element,
                &self.hover_effects,
                self.hover_transition,
                &self.press_effects,
                self.press_transition,
            ));
            self.element.style_override = StylePropertyPatch::default();
        }
        let element = Element::from_kind(self.key, ElementKind::Button(self.element));
        match self.pointer_request {
            Some(request) => element.with_pointer_request(request),
            None => element,
        }
    }
}

pub fn button() -> Button {
    let style = BoxStyle {
        overflow: crate::ui::Overflow::Clip,
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
        pointer_request: None,
        hover_effects: Vec::new(),
        hover_transition: None,
        press_effects: Vec::new(),
        press_transition: None,
        element: ButtonElement {
            accessible_label: None,
            children: Vec::new(),
            content_style_slot: None,
            enabled: true,
            busy: false,
            style,
            style_id: ComponentStyleId::named(ThemeDomainId::APPLICATION, "button", "default"),
            style_override: StylePropertyPatch::default(),
            inline_style: None,
            on_press: None,
            invalid_interaction_effect: false,
            effect_properties: 0,
            effect_overlay: false,
        },
    }
}
