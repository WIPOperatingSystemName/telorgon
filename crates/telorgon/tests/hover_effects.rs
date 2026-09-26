use telorgon::app::*;
use telorgon::compose::ElementKind;
use telorgon::theme::{MotionPreference, ThemeRuntime};
use telorgon::ui::{InteractionFlags, StyleSlotId};
use telorgon::{MonotonicInstant, ViewRuntime};

fn flags(values: &[InteractionFlags]) -> InteractionFlags {
    InteractionFlags::from_bits(values.iter().fold(0, |bits, value| bits | value.bits()))
}

#[test]
fn effects_compose_relative_to_resting_transform_and_last_property_wins() {
    let mut base = BoxStyle::default();
    base.transform.translation.y = 10.0;
    base.transform.scale.x = 2.0;
    let first = ColorRgba8::rgba(10, 20, 30, 255);
    let last = ColorRgba8::rgba(40, 50, 60, 255);
    let view = button()
        .box_style(base)
        .hover_effects([
            HoverEffect::Background(first),
            HoverEffect::Lift(2.0),
            HoverEffect::Scale(1.5),
            HoverEffect::Background(last),
            HoverEffect::Lift(3.0),
        ])
        .into_element();
    let ElementKind::Button(props) = view.kind() else {
        panic!()
    };
    let style = props.inline_style.as_ref().unwrap();
    let root = StyleSlotId::named("root");
    let hovered = style
        .resolve_slot(&[], InteractionFlags::HOVERED, root)
        .unwrap()
        .patch;
    assert_eq!(hovered.background, Some(Background::Color(last)));
    let transform = hovered.transform.unwrap();
    assert_eq!(transform.translation.y, 7.0);
    assert_eq!(transform.scale.x, 3.0);
    assert_eq!(transform.scale.y, 1.5);
    let disabled = style
        .resolve_slot(
            &[],
            flags(&[InteractionFlags::HOVERED, InteractionFlags::DISABLED]),
            root,
        )
        .unwrap()
        .patch;
    assert_eq!(disabled.transform, Some(base.transform));
    assert_eq!(disabled.background, Some(base.decoration.background));
}

#[component]
struct Animated {
    #[state]
    changed: bool,
}
impl Component for Animated {
    fn view(&self) -> impl View {
        button()
            .background(if self.changed {
                ColorRgba8::rgba(90, 90, 90, 255)
            } else {
                ColorRgba8::rgba(20, 20, 20, 255)
            })
            .hover_effects([
                HoverEffect::Background(ColorRgba8::rgba(120, 120, 120, 255)),
                HoverEffect::Lift(4.0),
            ])
            .hover_transition(TransitionSpec {
                duration_ms: 100,
                easing: Easing::Linear,
                repeat: false,
            })
            .child(text("Hello"))
            .on_press(|this: &mut Self| this.changed = !this.changed)
    }
}
fn at(ms: u64) -> MonotonicInstant {
    MonotonicInstant::from_nanos(ms * 1_000_000)
}

#[test]
fn interrupted_hover_is_continuous_and_restores_updated_authored_background() {
    let mut runtime = ViewRuntime::from_composed(Animated::default()).unwrap();
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
    theme.update_styles(runtime.ui_mut(), at(50), MotionPreference::Full);
    let halfway = runtime
        .ui()
        .box_styles
        .get(node)
        .unwrap()
        .transform
        .translation
        .y;
    assert_eq!(halfway, -2.0);
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::HOVERED, false);
    theme.update_styles(runtime.ui_mut(), at(50), MotionPreference::Full);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(node)
            .unwrap()
            .transform
            .translation
            .y,
        halfway
    );
    let done = theme.update_styles(runtime.ui_mut(), at(190), MotionPreference::Full);
    assert!(!done.active_animations);
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
    assert!(runtime.dispatch_action(node));
    theme.update_styles(runtime.ui_mut(), at(200), MotionPreference::Full);
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::HOVERED, true);
    theme.update_styles(runtime.ui_mut(), at(200), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(300), MotionPreference::Full);
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::HOVERED, false);
    theme.update_styles(runtime.ui_mut(), at(300), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(400), MotionPreference::Full);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(node)
            .unwrap()
            .decoration
            .background,
        Background::Color(ColorRgba8::rgba(90, 90, 90, 255))
    );
}

#[test]
fn reduced_motion_snaps_lift_without_interpolation() {
    let mut runtime = ViewRuntime::from_composed(Animated::default()).unwrap();
    let node = runtime
        .ui()
        .kinds
        .iter()
        .find_map(|(node, kind)| (*kind == telorgon::NodeKind::Button).then_some(node))
        .unwrap();
    let mut theme = ThemeRuntime::default();
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Reduced);
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::HOVERED, true);
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Reduced);
    theme.update_styles(runtime.ui_mut(), at(100), MotionPreference::Reduced);
    // Existing reduced-motion policy snaps transforms instead of animating them.
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(node)
            .unwrap()
            .transform
            .translation
            .y,
        -4.0
    );
}

#[component]
struct InvalidEffect {}
impl Component for InvalidEffect {
    fn view(&self) -> impl View {
        button()
            .hover_effect(HoverEffect::Scale(f32::NAN))
            .child(text("Invalid"))
    }
}
#[test]
fn nonfinite_effect_is_rejected_before_mount() {
    assert!(ViewRuntime::from_composed(InvalidEffect::default()).is_err());
}

#[test]
fn hover_effects_preserve_theme_focus_and_disabled_states() {
    let mut runtime = ViewRuntime::from_composed(Animated::default()).unwrap();
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
        .route_interaction_flag(node, InteractionFlags::FOCUS_VISIBLE, true);
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(200), MotionPreference::Full);
    assert!(
        runtime
            .ui()
            .box_styles
            .get(node)
            .unwrap()
            .decoration
            .outline
            .width
            > 0.0
    );
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::HOVERED, true);
    runtime.ui_mut().set_disabled(node, true);
    theme.update_styles(runtime.ui_mut(), at(200), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(400), MotionPreference::Full);
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
    assert_ne!(
        runtime
            .ui()
            .box_styles
            .get(node)
            .unwrap()
            .decoration
            .background,
        Background::Color(ColorRgba8::rgba(120, 120, 120, 255))
    );
}

#[component]
struct RemovableEffect {
    #[state]
    removed: bool,
}
impl Component for RemovableEffect {
    fn view(&self) -> impl View {
        let view = button()
            .style_override(telorgon::ui::StylePropertyPatch {
                translation_y: Some(10.0),
                ..Default::default()
            })
            .child(text("Remove effect"))
            .on_press(|this: &mut Self| this.removed = true);
        if self.removed {
            view
        } else {
            view.hover_effect(HoverEffect::Lift(8.0))
                .hover_transition(TransitionSpec {
                    duration_ms: 100,
                    easing: Easing::Linear,
                    repeat: false,
                })
        }
    }
}

#[test]
fn removing_an_active_effect_restores_the_authored_style() {
    let mut runtime = ViewRuntime::from_composed(RemovableEffect::default()).unwrap();
    let node = runtime
        .ui()
        .kinds
        .iter()
        .find_map(|(node, kind)| (*kind == telorgon::NodeKind::Button).then_some(node))
        .unwrap();
    let mut theme = ThemeRuntime::default();
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(200), MotionPreference::Full);
    runtime
        .ui_mut()
        .route_interaction_flag(node, InteractionFlags::HOVERED, true);
    theme.update_styles(runtime.ui_mut(), at(200), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(250), MotionPreference::Full);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(node)
            .unwrap()
            .transform
            .translation
            .y,
        6.0
    );
    assert!(runtime.dispatch_action(node));
    theme.update_styles(runtime.ui_mut(), at(260), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(400), MotionPreference::Full);
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
}
