use telorgon::app::*;
use telorgon::theme::{MotionPreference, ThemeRuntime};
use telorgon::ui::{InteractionFlags, StyleSlotId, UiNodeId};
use telorgon::{
    ChangeSource, CompositionDriver, MonotonicInstant, NodeKind, ValueChangePhase, ViewRuntime,
};

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

fn node(runtime: &ViewRuntime<CompositionDriver>, kind: NodeKind, index: usize) -> UiNodeId {
    runtime
        .ui()
        .kinds
        .iter()
        .filter_map(|(node, current)| (*current == kind).then_some(node))
        .nth(index)
        .unwrap()
}

fn slot(runtime: &ViewRuntime<CompositionDriver>, root: UiNodeId, name: &str) -> UiNodeId {
    runtime
        .ui()
        .style_bindings()
        .iter()
        .find(|binding| {
            binding.state_root == root
                && binding
                    .slots
                    .iter()
                    .any(|s| s.slot == StyleSlotId::named(name))
        })
        .unwrap()
        .slots
        .iter()
        .find(|slot| slot.slot == StyleSlotId::named(name))
        .unwrap()
        .node
}

#[component]
struct StyledControls {
    #[state]
    phase: u32,
    #[state]
    value: f32,
}

impl Component for StyledControls {
    fn view(&self) -> impl View {
        let visible_label = if self.phase == 1 { "Label" } else { "" };
        let checkbox = checkbox(visible_label, false)
            .accessible_label("Notifications")
            .width(Dimension::FILL)
            .height(48.0)
            .padding(8.0)
            .margin(3.0)
            .cursor(CursorIcon::Pointer)
            .opacity(0.9);
        let switch = switch(visible_label, true)
            .accessible_label("Wireless")
            .width(Dimension::FILL)
            .height(48.0)
            .padding(8.0)
            .margin(3.0)
            .cursor(CursorIcon::Pointer)
            .opacity(0.9);
        let slider = slider(visible_label, self.value)
            .accessible_label("Volume")
            .width(Dimension::FILL)
            .height(48.0)
            .padding(8.0)
            .margin(3.0)
            .cursor(CursorIcon::Pointer)
            .opacity(0.9)
            .on_change(|this: &mut Self, value| this.value = value);
        let (checkbox, switch, slider) = if self.phase < 2 {
            (
                checkbox
                    .hover_effect(InteractionEffect::Lift(4.0))
                    .hover_transition(transition(100)),
                switch
                    .hover_effect(InteractionEffect::Lift(4.0))
                    .hover_transition(transition(100)),
                slider
                    .hover_effect(InteractionEffect::Lift(4.0))
                    .hover_transition(transition(100))
                    .thumb_press_effect(InteractionEffect::Scale(1.5))
                    .press_transition(transition(100)),
            )
        } else {
            (checkbox, switch, slider)
        };
        column()
            .child(checkbox)
            .child(switch)
            .child(slider)
            .child(button().on_press(|this: &mut Self| this.phase += 1))
    }
}

#[test]
fn outer_styles_and_accessible_names_preserve_internal_control_geometry() {
    let runtime = ViewRuntime::from_composed(StyledControls::default()).unwrap();
    let checkbox = node(&runtime, NodeKind::Toggle, 0);
    let switch = node(&runtime, NodeKind::Toggle, 1);
    let slider = node(&runtime, NodeKind::Slider, 0);
    for (control, name) in [
        (checkbox, "Notifications"),
        (switch, "Wireless"),
        (slider, "Volume"),
    ] {
        let style = runtime.ui().box_styles.get(control).unwrap();
        assert_eq!(style.width, SizeRule::Fill(1.0));
        assert_eq!(style.height, SizeRule::Logical(48.0));
        assert_eq!(style.padding, telorgon::EdgeInsets::all(8.0));
        assert_eq!(style.margin, telorgon::EdgeInsets::all(3.0));
        let telorgon::SemanticName::Text(label) = runtime.ui().semantics.get(control).unwrap().name
        else {
            panic!()
        };
        assert_eq!(runtime.ui().string(label), Some(name));
    }
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(slot(&runtime, checkbox, "indicator"))
            .unwrap()
            .width,
        SizeRule::Logical(18.0)
    );
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(slot(&runtime, switch, "track"))
            .unwrap()
            .width,
        SizeRule::Logical(38.0)
    );
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(slot(&runtime, slider, "track"))
            .unwrap()
            .width,
        SizeRule::Fill(1.0)
    );
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(slot(&runtime, slider, "thumb"))
            .unwrap()
            .width,
        SizeRule::Logical(18.0)
    );
}

#[test]
fn control_effects_survive_rerender_and_removed_effects_restore_all_slots() {
    let mut runtime = ViewRuntime::from_composed(StyledControls::default()).unwrap();
    let checkbox = node(&runtime, NodeKind::Toggle, 0);
    let switch = node(&runtime, NodeKind::Toggle, 1);
    let slider = node(&runtime, NodeKind::Slider, 0);
    let update = node(&runtime, NodeKind::Button, 0);
    let thumb = slot(&runtime, slider, "thumb");
    let mut theme = ThemeRuntime::default();
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    for control in [checkbox, switch, slider] {
        runtime
            .ui_mut()
            .route_interaction_flag(control, InteractionFlags::HOVERED, true);
    }
    runtime
        .ui_mut()
        .route_interaction_flag(slider, InteractionFlags::PRESSED, true);
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(50), MotionPreference::Full);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(thumb)
            .unwrap()
            .transform
            .scale
            .x,
        1.25
    );
    assert!(runtime.dispatch_action(update));
    assert!(runtime.dispatch_value(
        slider,
        0.75,
        ValueChangePhase::Commit,
        ChangeSource::Pointer
    ));
    theme.update_styles(runtime.ui_mut(), at(50), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(100), MotionPreference::Full);
    for control in [checkbox, switch, slider] {
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(control)
                .unwrap()
                .transform
                .translation
                .y,
            -4.0
        );
    }
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(thumb)
            .unwrap()
            .transform
            .scale
            .x,
        1.5
    );
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(slider)
            .unwrap()
            .transform
            .scale
            .x,
        1.0
    );
    assert_eq!(runtime.ui().interactions.get(slider).unwrap().value, 0.75);

    assert!(runtime.dispatch_action(update));
    theme.update_styles(runtime.ui_mut(), at(100), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(250), MotionPreference::Full);
    for control in [checkbox, switch, slider] {
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(control)
                .unwrap()
                .transform
                .translation
                .y,
            0.0
        );
    }
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(thumb)
            .unwrap()
            .transform
            .scale
            .x,
        1.0
    );
}

#[test]
fn disabled_controls_suppress_both_root_and_thumb_effects() {
    let mut runtime = ViewRuntime::from_composed(StyledControls::default()).unwrap();
    let controls = [
        node(&runtime, NodeKind::Toggle, 0),
        node(&runtime, NodeKind::Toggle, 1),
        node(&runtime, NodeKind::Slider, 0),
    ];
    let thumb = slot(&runtime, controls[2], "thumb");
    let mut theme = ThemeRuntime::default();
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    for control in controls {
        runtime
            .ui_mut()
            .route_interaction_flag(control, InteractionFlags::HOVERED, true);
        runtime
            .ui_mut()
            .route_interaction_flag(control, InteractionFlags::PRESSED, true);
        runtime.ui_mut().set_disabled(control, true);
    }
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(200), MotionPreference::Full);
    for control in controls {
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(control)
                .unwrap()
                .transform
                .translation
                .y,
            0.0
        );
    }
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(thumb)
            .unwrap()
            .transform
            .scale
            .x,
        1.0
    );
}

#[test]
fn removing_a_thumb_effect_cancels_its_in_progress_animation() {
    let mut runtime = ViewRuntime::from_composed(StyledControls::default()).unwrap();
    let slider = node(&runtime, NodeKind::Slider, 0);
    let thumb = slot(&runtime, slider, "thumb");
    let update = node(&runtime, NodeKind::Button, 0);
    let mut theme = ThemeRuntime::default();
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    runtime
        .ui_mut()
        .route_interaction_flag(slider, InteractionFlags::PRESSED, true);
    theme.update_styles(runtime.ui_mut(), at(0), MotionPreference::Full);
    theme.update_styles(runtime.ui_mut(), at(50), MotionPreference::Full);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(thumb)
            .unwrap()
            .transform
            .scale
            .x,
        1.25
    );
    assert!(runtime.dispatch_action(update));
    assert!(runtime.dispatch_action(update));
    theme.update_styles(runtime.ui_mut(), at(60), MotionPreference::Full);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(thumb)
            .unwrap()
            .transform
            .scale
            .x,
        1.0
    );
    theme.update_styles(runtime.ui_mut(), at(250), MotionPreference::Full);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(thumb)
            .unwrap()
            .transform
            .scale
            .x,
        1.0
    );
}

#[test]
fn label_spacing_updates_computed_geometry_when_visible_labels_change() {
    let mut runtime = ViewRuntime::from_composed(StyledControls::default()).unwrap();
    let checkbox = node(&runtime, NodeKind::Toggle, 0);
    let switch = node(&runtime, NodeKind::Toggle, 1);
    let slider = node(&runtime, NodeKind::Slider, 0);
    let update = node(&runtime, NodeKind::Button, 0);
    let mut layout = telorgon::layout::LayoutEngine::default();
    let mut text = telorgon::text::RetainedTextSystem::new(4096).unwrap();
    for expected_gap in [0.0, 8.0, 0.0] {
        layout.update(
            runtime.ui_mut(),
            &mut text,
            telorgon::SizeF {
                width: 400.0,
                height: 300.0,
            },
            1.0,
        );
        for (control, visual, label_first) in [
            (checkbox, "indicator", false),
            (switch, "track", false),
            (slider, "track", true),
        ] {
            let label = layout
                .computed(slot(&runtime, control, "label"))
                .unwrap()
                .border_rect;
            let visual = layout
                .computed(slot(&runtime, control, visual))
                .unwrap()
                .border_rect;
            let actual_gap = if label_first {
                visual.x - (label.x + label.width)
            } else {
                label.x - (visual.x + visual.width)
            };
            assert!(
                (actual_gap - expected_gap).abs() < 0.001,
                "label spacing {actual_gap} should be {expected_gap}"
            );
            let root = layout.computed(control).unwrap().border_rect;
            assert!(visual.x >= root.x && visual.x + visual.width <= root.x + root.width);
            assert!(visual.y >= root.y && visual.y + visual.height <= root.y + root.height);
        }
        assert!(runtime.dispatch_action(update));
    }
}
