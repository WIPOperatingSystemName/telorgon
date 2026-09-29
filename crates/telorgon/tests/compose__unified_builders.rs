use telorgon::compose::{
    Button, Checkbox, Container, ElementKind, EventDispatch, HasContent, Image, MissingContent,
    Slider, Switch, Text, WindowContentSlot, WindowFrame,
};
use telorgon::theme::TransitionSpec;
use telorgon::{
    Alignment, Background, Border, BoxDecoration, BoxStyle, ChangeSource, ColorRgba8, Component,
    ComponentInstanceId, CornerRadii, CrossAxisAlignment, CursorIcon, Dimension, Flow, ImageId,
    Insets, InteractionEffect, LayoutStyle, MainAxisAlignment, Outline, Overflow, Shadow,
    ShadowList, SizeRule, TextStyle, View, button, card, checkbox, column, image, row, slider,
    spacer, stack, switch, text, window_content_slot, window_frame,
};

#[telorgon::component]
struct Fixture {
    #[state]
    last_source: Option<ChangeSource>,
}

impl Component for Fixture {
    fn view(&self) -> impl View {
        text("Fixture")
    }
}

// Each invocation type-checks the same fluent vocabulary against a concrete builder. Keeping
// cursor in the middle catches accidental erasure to Element before widget-specific modifiers.
macro_rules! box_modifiers {
    ($builder:expr) => {
        $builder
            .cursor(CursorIcon::Default)
            .box_style(BoxStyle {
                width: Dimension::FILL.into(),
                height: SizeRule::Logical(32.0),
                ..BoxStyle::default()
            })
            .width(160.0)
            .height(Dimension::Shrink)
            .padding(Insets::symmetric(2.0, 4.0))
            .margin((1.0, 2.0))
            .aspect_ratio(2.0)
            .decoration(BoxDecoration::default())
            .background(Background::Color(ColorRgba8::rgba(10, 20, 30, 255)))
            .corner_radius(3.0)
            .corner_radii(CornerRadii::all(4.0))
            .uniform_border(1.0, ColorRgba8::rgba(40, 50, 60, 255))
            .border_sides(Border::default())
            .outline(Outline::default())
            .shadow(Shadow::default())
            .shadows(ShadowList::default())
            .opacity(0.9)
            .cursor(CursorIcon::Help)
            .overflow(Overflow::Clip)
    };
}

macro_rules! control_effects {
    ($builder:expr) => {
        $builder
            .hover_effect(InteractionEffect::Lift(1.0))
            .hover_effects([InteractionEffect::Scale(1.05)])
            .hover_transition(TransitionSpec::default())
            .press_effect(InteractionEffect::Scale(0.98))
            .press_effects([InteractionEffect::Lift(0.5)])
            .press_transition(TransitionSpec::default())
    };
}

#[test]
fn common_modifiers_preserve_each_concrete_builder() {
    let label: Text = box_modifiers!(text("Label"))
        .text_style(TextStyle::new().size(18.0))
        .color(ColorRgba8::rgba(240, 240, 240, 255))
        .layout_style(LayoutStyle::default())
        .hover_effect(InteractionEffect::Lift(1.0))
        .hover_effects([InteractionEffect::Scale(1.05)])
        .hover_transition(TransitionSpec::default());
    let icon: Image = box_modifiers!(image(ImageId(1)))
        .layout_style(LayoutStyle::default())
        .accessible_label("Settings")
        .tint(ColorRgba8::rgba(240, 240, 240, 255))
        .hover_effect(InteractionEffect::Lift(1.0))
        .hover_effects([InteractionEffect::Scale(1.05)])
        .hover_transition(TransitionSpec::default());
    let action: Button = control_effects!(box_modifiers!(button()))
        .layout_style(LayoutStyle {
            flow: Flow::Horizontal,
            ..LayoutStyle::default()
        })
        .gap(4.0)
        .align_items(Alignment::Start)
        .justify_content(Alignment::End)
        .center_content()
        .child(text("Action"))
        .children([text("Detail")])
        .maybe(false, text("Hidden"))
        .accessible_label("Action details")
        .on_press(|_: &mut Fixture| {})
        .on_press_event(|this: &mut Fixture, event| this.last_source = Some(event.source()));
    let check: Checkbox = control_effects!(box_modifiers!(checkbox("Check", false)))
        .width(Dimension::FILL)
        .height(32.0)
        .accessible_label("Check option")
        .mixed()
        .enabled(true)
        .on_change(|_: &mut Fixture, _: bool| {})
        .on_change_event(|this: &mut Fixture, event| this.last_source = Some(event.source()));
    let toggle: Switch = control_effects!(box_modifiers!(switch("Switch", false)))
        .width(Dimension::FILL)
        .height(32.0)
        .accessible_label("Switch option")
        .enabled(true)
        .on_change(|_: &mut Fixture, _: bool| {})
        .on_change_event(|this: &mut Fixture, event| this.last_source = Some(event.source()));
    let range: Slider = control_effects!(box_modifiers!(slider("Range", 0.5)))
        .accessible_label("Range option")
        .enabled(true)
        .thumb_press_effect(InteractionEffect::Scale(1.1))
        .thumb_press_effects([InteractionEffect::Lift(1.0)])
        .on_change(|_: &mut Fixture, _: f32| {})
        .on_change_event(|this: &mut Fixture, event| this.last_source = Some(event.source()));

    let root: Container = control_effects!(box_modifiers!(column()))
        .hover_within(true)
        .gap(8.0)
        .center_content()
        .children([
            label.into_element(),
            icon.into_element(),
            action.into_element(),
            check.into_element(),
            toggle.into_element(),
            range.into_element(),
        ]);
    root.into_element().validate().unwrap();

    for builder in [row(), column(), stack(), card(), spacer()] {
        box_modifiers!(builder)
            .maybe(true, text("Content"))
            .into_element()
            .validate()
            .unwrap();
    }
}

#[test]
fn window_modifiers_preserve_content_slot_typestate() {
    let frame: WindowFrame<MissingContent> = box_modifiers!(window_frame()).center_content();
    let content: WindowContentSlot = box_modifiers!(window_content_slot())
        .children([text("First"), text("Second")])
        .maybe(true, text("Third"));
    let frame: WindowFrame<HasContent> = frame
        .content_slot(content)
        .cursor(CursorIcon::Move)
        .child(text("Title"))
        .center_content();
    frame.into_element().validate().unwrap();
}

#[test]
fn detailed_press_callback_receives_activation_source() {
    let element = button()
        .child(text("Activate"))
        .on_press_event(|this: &mut Fixture, event| this.last_source = Some(event.source()))
        .into_element();
    let ElementKind::Button(button) = element.kind() else {
        panic!("expected button")
    };
    let handler = button
        .on_press
        .as_ref()
        .unwrap()
        .bind(ComponentInstanceId::new(0, 1));
    let mut fixture = Fixture { last_source: None };
    for source in [ChangeSource::Pointer, ChangeSource::Keyboard] {
        assert_eq!(
            handler.dispatch(&mut fixture, source),
            EventDispatch::Delivered {
                input_mutated: false
            }
        );
        assert_eq!(fixture.last_source, Some(source));
    }
}

#[test]
#[allow(deprecated)]
fn previous_style_names_remain_usable() {
    let elements = [
        text("Legacy label")
            .style(TextStyle::new().size(16.0))
            .into_element(),
        image(ImageId(1))
            .style(BoxStyle::default())
            .layout(LayoutStyle::default())
            .into_element(),
        button()
            .style(BoxStyle::default())
            .child(text("Legacy action"))
            .into_element(),
        column()
            .style(BoxStyle::default())
            .child(text("Legacy layout"))
            .into_element(),
    ];
    for element in elements {
        element.validate().unwrap();
    }
}

#[test]
fn button_default_layout_preserves_centered_vertical_content() {
    let element = button().children([text("One"), text("Two")]).into_element();
    let ElementKind::Button(button) = element.kind() else {
        panic!("expected button")
    };
    assert_eq!(button.layout.flow, Flow::Vertical);
    assert_eq!(button.layout.main_axis_alignment, MainAxisAlignment::Center);
    assert_eq!(
        button.layout.cross_axis_alignment,
        CrossAxisAlignment::Center
    );
    assert_eq!(button.layout.gap, 0.0);
}
