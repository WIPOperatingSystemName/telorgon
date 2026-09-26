use telorgon::app::*;
use telorgon::theme::{MotionPreference, ThemeRuntime};
use telorgon::ui::InteractionFlags;
use telorgon::{MonotonicInstant, ViewRuntime};

fn at(ms: u64) -> MonotonicInstant {
    MonotonicInstant::from_nanos(ms * 1_000_000)
}
fn transition(duration_ms: u32) -> TransitionSpec {
    TransitionSpec {
        duration_ms,
        easing: Easing::Linear,
        repeat: false,
    }
}

#[component]
struct Interactive {
    #[state]
    removed: bool,
}
impl Component for Interactive {
    fn view(&self) -> impl View {
        let mut base = BoxStyle::default();
        base.transform.translation.y = 10.0;
        base.transform.scale.x = 2.0;
        let view = button()
            .box_style(base)
            .hover_effects([
                InteractionEffect::Lift(4.0),
                InteractionEffect::Scale(1.1),
                InteractionEffect::Background(ColorRgba8::rgba(80, 80, 80, 255)),
            ])
            .hover_transition(transition(100))
            .child(text("Press me"))
            .on_press(|this: &mut Self| this.removed = true);
        if self.removed {
            view
        } else {
            view.press_effect(InteractionEffect::Scale(0.9))
                .press_effects([InteractionEffect::Background(ColorRgba8::rgba(
                    40, 40, 40, 255,
                ))])
                .press_transition(transition(40))
        }
    }
}

#[test]
fn press_combines_with_hover_and_release_retargets_continuously() {
    let mut runtime = ViewRuntime::from_composed(Interactive::default()).unwrap();
    let node = runtime
        .ui()
        .kinds
        .iter()
        .find_map(|(node, kind)| (*kind == telorgon::NodeKind::Button).then_some(node))
        .unwrap();
    let mut theme = ThemeRuntime::default();
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::HOVERED, true);
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(100), MotionPreference::Full);
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::PRESSED, true);
    theme.update_styles(runtime.ui_mut(), at(100), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(120), MotionPreference::Full);
    let halfway = *runtime.ui().box_styles.get(node).unwrap();
    assert!((halfway.transform.scale.x - 2.0).abs() < 0.0001);
    assert_eq!(halfway.transform.translation.y, 6.0);
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::PRESSED, false);
    theme.update_styles(runtime.ui_mut(), at(120), MotionPreference::Full);
    assert_eq!(
        runtime.ui().box_styles.get(node).unwrap().transform,
        halfway.transform
    );
    theme.update_styles(runtime.ui_mut(), at(220), MotionPreference::Full);
    assert_eq!(
        runtime.ui().box_styles.get(node).unwrap().transform.scale.x,
        2.2
    );
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::PRESSED, true);
    theme.update_styles(runtime.ui_mut(), at(220), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(260), MotionPreference::Full);
    let pressed = runtime.ui().box_styles.get(node).unwrap();
    assert_eq!(pressed.transform.scale.x, 1.8);
    assert_eq!(pressed.transform.translation.y, 6.0);
    assert_eq!(
        pressed.decoration.background,
        Background::Color(ColorRgba8::rgba(40, 40, 40, 255))
    );
    // A press without hover (keyboard, or pointer dragged outside) keeps only press effects.
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::HOVERED, false);
    theme.update_styles(runtime.ui_mut(), at(260), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(300), MotionPreference::Full);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(node)
            .unwrap()
            .transform
            .translation
            .y,
        10.0
    );
    runtime.ui_mut().set_disabled(node, true);
    theme.update_styles(runtime.ui_mut(), at(300), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(500), MotionPreference::Full);
    assert_eq!(
        runtime.ui().box_styles.get(node).unwrap().transform.scale.x,
        2.0
    );
}

#[test]
fn removing_press_effects_while_pressed_restores_hover() {
    let mut runtime = ViewRuntime::from_composed(Interactive::default()).unwrap();
    let node = runtime
        .ui()
        .kinds
        .iter()
        .find_map(|(node, kind)| (*kind == telorgon::NodeKind::Button).then_some(node))
        .unwrap();
    let mut theme = ThemeRuntime::default();
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::HOVERED, true);
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::PRESSED, true);
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(20), MotionPreference::Full);
    assert!(runtime.dispatch_action(node));
    theme.update_styles(runtime.ui_mut(), at(20), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(200), MotionPreference::Full);
    let transform = runtime.ui().box_styles.get(node).unwrap().transform;
    assert_eq!(transform.translation.y, 6.0);
    assert_eq!(transform.scale.x, 2.2);
}

#[component]
struct PressOnly {}
impl Component for PressOnly {
    fn view(&self) -> impl View {
        button()
            .press_effect(InteractionEffect::Lift(3.0))
            .press_transition(transition(0))
    }
}
#[test]
fn press_only_effect_is_instant_and_restores_on_release() {
    let mut runtime = ViewRuntime::from_composed(PressOnly::default()).unwrap();
    let node = runtime
        .ui()
        .kinds
        .iter()
        .find_map(|(node, kind)| (*kind == telorgon::NodeKind::Button).then_some(node))
        .unwrap();
    let mut theme = ThemeRuntime::default();
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::PRESSED, true);
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(node)
            .unwrap()
            .transform
            .translation
            .y,
        -3.0
    );
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::PRESSED, false);
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(node)
            .unwrap()
            .transform
            .translation
            .y,
        0.0
    );
}

#[component]
struct InvalidPress {}
impl Component for InvalidPress {
    fn view(&self) -> impl View {
        button().press_effect(InteractionEffect::Scale(f32::NAN))
    }
}
#[component]
struct RepeatingPress {}
impl Component for RepeatingPress {
    fn view(&self) -> impl View {
        button().press_transition(TransitionSpec {
            repeat: true,
            ..transition(100)
        })
    }
}
#[test]
fn invalid_press_effects_and_repeating_transitions_are_rejected() {
    assert!(ViewRuntime::from_composed(InvalidPress::default()).is_err());
    assert!(ViewRuntime::from_composed(RepeatingPress::default()).is_err());
}

#[test]
fn transition_builders_work_without_explicit_effects() {
    use telorgon::compose::ElementKind;
    use telorgon::ui::StyleSlotId;
    let element = button()
        .hover_transition(transition(80))
        .press_transition(transition(0))
        .into_element();
    let ElementKind::Button(props) = element.kind() else {
        panic!()
    };
    let style = props.inline_style.as_ref().unwrap();
    let root = StyleSlotId::named("root");
    assert_eq!(
        style
            .resolve_slot(&[], InteractionFlags::HOVERED, root)
            .unwrap()
            .transition
            .duration_ms,
        80
    );
    assert_eq!(
        style
            .resolve_slot(&[], InteractionFlags::PRESSED, root)
            .unwrap()
            .transition
            .duration_ms,
        0
    );
}
