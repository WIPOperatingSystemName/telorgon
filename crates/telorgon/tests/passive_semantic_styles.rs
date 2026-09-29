use telorgon::app::*;
use telorgon::theme::{MotionPreference, ThemeRuntime};
use telorgon::ui::{InteractionFlags, LayoutStyle, MountWriter, MountedUi};
use telorgon::{MonotonicInstant, NodeKind, ViewRuntime};

const RESTING: ColorRgba8 = ColorRgba8::rgba(30, 70, 90, 255);

#[component]
struct SemanticChildren {
    #[state]
    updates: u32,
}

impl Component for SemanticChildren {
    fn view(&self) -> impl View {
        button()
            .child(
                text("Styled")
                    .color(RESTING)
                    .hover_effect(InteractionEffect::Lift(4.0))
                    .hover_transition(TransitionSpec {
                        duration_ms: 0,
                        easing: Easing::Linear,
                        repeat: false,
                    }),
            )
            .child(text("Plain").color(RESTING))
            .on_press(|this: &mut Self| this.updates += 1)
    }
}

fn settle(theme: &mut ThemeRuntime, ui: &mut MountedUi, ms: u64) {
    for ms in [ms, ms + 500] {
        theme.update_styles(
            ui,
            MonotonicInstant::from_nanos(ms * 1_000_000),
            MotionPreference::Full,
        );
    }
}

#[test]
fn passive_effects_inherit_semantic_state_without_parent_pointer_or_focus_state() {
    let mut runtime = ViewRuntime::from_composed(SemanticChildren::default()).unwrap();
    let button = runtime
        .ui()
        .kinds
        .iter()
        .find_map(|(node, kind)| (*kind == NodeKind::Button).then_some(node))
        .unwrap();
    let labels = runtime
        .ui()
        .kinds
        .iter()
        .filter_map(|(node, kind)| (*kind == NodeKind::Text).then_some(node))
        .collect::<Vec<_>>();
    let (styled, plain) = (labels[0], labels[1]);
    let binding = runtime
        .ui()
        .style_bindings()
        .iter()
        .find(|binding| binding.state_root == styled)
        .unwrap();
    assert_eq!(binding.inherited_state_root, Some(button));
    let mut theme = ThemeRuntime::default();
    settle(&mut theme, runtime.ui_mut(), 0);

    runtime.ui_mut().set_busy(button, true);
    settle(&mut theme, runtime.ui_mut(), 1_000);
    assert_eq!(
        runtime.ui().texts.get(styled).unwrap().style.color,
        runtime.ui().texts.get(plain).unwrap().style.color
    );
    assert_ne!(runtime.ui().texts.get(styled).unwrap().style.color, RESTING);
    runtime.ui_mut().set_busy(button, false);
    settle(&mut theme, runtime.ui_mut(), 2_000);
    assert_eq!(runtime.ui().texts.get(styled).unwrap().style.color, RESTING);

    for flag in [
        InteractionFlags::HOVERED,
        InteractionFlags::PRESSED,
        InteractionFlags::FOCUSED,
        InteractionFlags::FOCUS_VISIBLE,
    ] {
        runtime.ui_mut().route_interaction_flag(button, flag, true);
    }
    settle(&mut theme, runtime.ui_mut(), 3_000);
    assert_eq!(runtime.ui().texts.get(styled).unwrap().style.color, RESTING);
    assert_ne!(runtime.ui().texts.get(plain).unwrap().style.color, RESTING);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(styled)
            .copied()
            .unwrap_or_default()
            .transform
            .translation
            .y,
        0.0
    );
    runtime
        .ui_mut()
        .route_interaction_flag(styled, InteractionFlags::HOVERED, true);
    settle(&mut theme, runtime.ui_mut(), 4_000);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(styled)
            .unwrap()
            .transform
            .translation
            .y,
        -4.0
    );

    runtime.ui_mut().set_disabled(button, true);
    settle(&mut theme, runtime.ui_mut(), 5_000);
    assert_eq!(
        runtime.ui().texts.get(styled).unwrap().style.color,
        runtime.ui().texts.get(plain).unwrap().style.color
    );
    assert_ne!(runtime.ui().texts.get(styled).unwrap().style.color, RESTING);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(styled)
            .unwrap()
            .transform
            .translation
            .y,
        0.0
    );
    runtime.ui_mut().set_disabled(button, false);
    runtime
        .ui_mut()
        .route_interaction_flag(styled, InteractionFlags::HOVERED, false);
    settle(&mut theme, runtime.ui_mut(), 6_000);
    assert_eq!(runtime.ui().texts.get(styled).unwrap().style.color, RESTING);
}

#[test]
fn semantic_property_transactions_notify_inherited_style_bindings() {
    let mut ui = MountedUi::default();
    let mut control = None;
    let mut label = None;
    MountWriter::<()>::new(&mut ui).root(BoxStyle::default(), LayoutStyle::default(), |writer| {
        control = Some(writer.button_node(BoxStyle::default(), |_| {}));
        label = Some(writer.text("Owned label", RESTING, 14.0).node);
    });
    let control = control.unwrap();
    let label = label.unwrap();
    // Retain the label's own foundation binding when placing it under the control.
    assert!(ui.nodes.reparent_before(label, control.node, None));
    let mut bindings = ui.take_style_bindings_for_processing();
    bindings
        .iter_mut()
        .find(|binding| binding.state_root == label)
        .unwrap()
        .inherited_state_root = Some(control.node);
    ui.restore_style_bindings_after_processing(bindings);
    let mut theme = ThemeRuntime::default();
    settle(&mut theme, &mut ui, 0);
    ui.transaction(|tx| tx.set(control.enabled, false));
    settle(&mut theme, &mut ui, 1_000);
    assert_ne!(ui.texts.get(label).unwrap().style.color, RESTING);
    ui.transaction(|tx| tx.set(control.enabled, true));
    settle(&mut theme, &mut ui, 2_000);
    assert_eq!(ui.texts.get(label).unwrap().style.color, RESTING);
    ui.transaction(|tx| tx.set(control.busy, true));
    settle(&mut theme, &mut ui, 3_000);
    assert_ne!(ui.texts.get(label).unwrap().style.color, RESTING);
}
