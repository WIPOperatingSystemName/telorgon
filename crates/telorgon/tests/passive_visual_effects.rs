use telorgon::app::*;
use telorgon::theme::{
    MotionPreference, ThemeDomain, ThemeRuntime, ThemeSource, application_catalog,
};
use telorgon::ui::{InteractionFlags, UiNodeId};
use telorgon::{CompositionDriver, ImageId, MonotonicInstant, NodeKind, ViewRuntime};

const RESTING: ColorRgba8 = ColorRgba8::rgba(20, 40, 60, 255);
const HOVERED: ColorRgba8 = ColorRgba8::rgba(90, 120, 160, 255);
const SIBLING: ColorRgba8 = ColorRgba8::rgba(60, 50, 40, 255);

#[component]
struct PassiveChildren {
    #[state]
    updates: u32,
}

impl Component for PassiveChildren {
    fn view(&self) -> impl View {
        let mut base = BoxStyle::default();
        base.decoration.background = Background::Color(RESTING);
        base.transform.translation.y = 10.0;
        let mut label = text("Effect text").key("text").box_style(base);
        let mut picture = image(ImageId(1)).key("image").box_style(base);
        if self.updates < 2 {
            let effects = [
                InteractionEffect::Background(HOVERED),
                InteractionEffect::Lift(8.0),
            ];
            let transition = TransitionSpec {
                duration_ms: 100,
                easing: Easing::Linear,
                repeat: false,
            };
            label = label.hover_effects(effects).hover_transition(transition);
            picture = picture.hover_effects(effects).hover_transition(transition);
        }
        column().scrollable().child(
            button()
                .child(label)
                .child(picture)
                .child(text(format!("Sibling {}", self.updates)).background(SIBLING))
                .on_press(|this: &mut Self| this.updates += 1),
        )
    }
}

fn at(ms: u64) -> MonotonicInstant {
    MonotonicInstant::from_nanos(ms * 1_000_000)
}

fn background(runtime: &ViewRuntime<CompositionDriver>, node: UiNodeId) -> Background {
    runtime
        .ui()
        .box_styles
        .get(node)
        .copied()
        .unwrap_or_default()
        .decoration
        .background
}

fn offset(runtime: &ViewRuntime<CompositionDriver>, node: UiNodeId) -> f32 {
    runtime
        .ui()
        .box_styles
        .get(node)
        .copied()
        .unwrap_or_default()
        .transform
        .translation
        .y
}

fn exercise_leaf(kind: NodeKind) {
    let mut runtime = ViewRuntime::from_composed(PassiveChildren::default()).unwrap();
    let find_kind = |kind| {
        runtime
            .ui()
            .kinds
            .iter()
            .filter_map(move |(node, found)| (*found == kind).then_some(node))
            .collect::<Vec<_>>()
    };
    let texts = find_kind(NodeKind::Text);
    let picture = find_kind(NodeKind::Image)[0];
    let button = find_kind(NodeKind::Button)[0];
    let scroll = find_kind(NodeKind::Scroll)[0];
    let target = if kind == NodeKind::Text {
        texts[0]
    } else {
        picture
    };
    let untouched_effect_leaf = if kind == NodeKind::Text {
        picture
    } else {
        texts[0]
    };
    let sibling = texts[1];
    let mut theme = ThemeRuntime::default();
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);

    // The leaf owns its optional effect state even when normal labels inherit their button's
    // state and that button is itself nested in a scrolling viewport.
    let binding = runtime
        .ui()
        .style_bindings()
        .iter()
        .find(|binding| binding.slots.iter().any(|slot| slot.node == target))
        .unwrap();
    assert_eq!(binding.state_root, target);
    runtime
        .ui_mut()
        .route_interaction_flag(button, InteractionFlags::HOVERED, true);
    runtime
        .ui_mut()
        .route_interaction_flag(scroll, InteractionFlags::HOVERED, true);
    runtime
        .ui_mut()
        .route_interaction_flag(target, InteractionFlags::HOVERED, true);
    theme.update_styles(runtime.ui_mut(), at(10), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(60), MotionPreference::Full);
    assert_eq!(offset(&runtime, target), 6.0);
    assert_eq!(offset(&runtime, untouched_effect_leaf), 10.0);
    assert_eq!(
        background(&runtime, untouched_effect_leaf),
        Background::Color(RESTING)
    );
    assert_eq!(offset(&runtime, sibling), 0.0);
    assert_eq!(background(&runtime, sibling), Background::Color(SIBLING));

    // A parent update retains the current hover sample and continues the same transition.
    assert!(runtime.dispatch_action(button));
    theme.update_styles(runtime.ui_mut(), at(60), MotionPreference::Full);
    assert_eq!(offset(&runtime, target), 6.0);
    theme.update_styles(runtime.ui_mut(), at(85), MotionPreference::Full);
    assert_eq!(offset(&runtime, target), 4.0);

    // Remove effects midway through their transition. Advancing time must never resurrect an
    // old state-root track over the newly authored box after it resumes inherited styling.
    assert!(runtime.dispatch_action(button));
    theme.update_styles(runtime.ui_mut(), at(85), MotionPreference::Full);
    assert_eq!(offset(&runtime, target), 10.0);
    assert_eq!(background(&runtime, target), Background::Color(RESTING));
    let done = theme.update_styles(runtime.ui_mut(), at(500), MotionPreference::Full);
    assert!(!done.active_animations);
    assert_eq!(offset(&runtime, target), 10.0);
    assert_eq!(background(&runtime, target), Background::Color(RESTING));
    assert_eq!(background(&runtime, sibling), Background::Color(SIBLING));
    assert_eq!(offset(&runtime, untouched_effect_leaf), 10.0);
}

#[test]
fn text_hover_stays_local_survives_rerender_and_cancels_on_removal() {
    exercise_leaf(NodeKind::Text);
}

#[test]
fn image_hover_stays_local_survives_rerender_and_cancels_on_removal() {
    exercise_leaf(NodeKind::Image);
}

#[test]
fn optional_text_hover_effect_preserves_base_theme_typography_and_color() {
    let mut runtime = ViewRuntime::from_composed(PassiveChildren::default()).unwrap();
    let label = runtime
        .ui()
        .kinds
        .iter()
        .find_map(|(node, kind)| (*kind == NodeKind::Text).then_some(node))
        .unwrap();
    let mut theme = ThemeRuntime::default();
    let source = ThemeSource::parse(
        r##"
format = "v4"
domain = "application"
[components.text.default.slots.root]
foreground = "#3579adff"
typography = { size = 23, line_height = 29, weight = 600 }
"##,
    )
    .unwrap();
    theme
        .compile_and_replace(
            ThemeRuntime::root_scope(ThemeDomain::Application),
            &source,
            &application_catalog(),
        )
        .unwrap();
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(200), MotionPreference::Full);
    let check_theme = |runtime: &ViewRuntime<CompositionDriver>| {
        let style = runtime.ui().texts.get(label).unwrap().style;
        assert_eq!(style.color, ColorRgba8::rgba(53, 121, 173, 255));
        assert_eq!(style.size, 23.0);
        assert_eq!(style.line_height, 29.0);
        assert_eq!(style.weight, 600);
    };
    check_theme(&runtime);
    runtime
        .ui_mut()
        .route_interaction_flag(label, InteractionFlags::HOVERED, true);
    theme.update_styles(runtime.ui_mut(), at(200), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(300), MotionPreference::Full);
    check_theme(&runtime);
    assert_eq!(offset(&runtime, label), 2.0);
    assert_eq!(background(&runtime, label), Background::Color(HOVERED));
    let button = runtime
        .ui()
        .kinds
        .iter()
        .find_map(|(node, kind)| (*kind == NodeKind::Button).then_some(node))
        .unwrap();
    assert!(runtime.dispatch_action(button));
    theme.update_styles(runtime.ui_mut(), at(300), MotionPreference::Full);
    check_theme(&runtime);
    assert_eq!(offset(&runtime, label), 2.0);
    theme.update_styles(runtime.ui_mut(), at(500), MotionPreference::Full);
    check_theme(&runtime);
}
