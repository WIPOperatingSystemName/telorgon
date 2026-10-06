use std::sync::{Arc, Mutex};
use telorgon::{
    ComposedAppRuntime, InputEvent, MonotonicInstant, NodeKind,
    app::*,
    input::{
        ButtonState, KeyEvent, KeyText, LogicalKey, Modifiers, NamedKey, PhysicalKey, PointerButton,
    },
    ui::{
        ControlBehavior, InteractionFlags, SemanticName, SemanticRole, UiEvent, UiEventKind,
        UiNodeId,
    },
};

#[derive(Clone)]
struct EventLog(Arc<Mutex<Vec<UiEvent>>>);
impl PartialEq for EventLog {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl std::ops::Deref for EventLog {
    type Target = Mutex<Vec<UiEvent>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[component(no_default)]
struct Editors {
    #[input]
    events: EventLog,
    #[state]
    modal: bool,
}

impl Component for Editors {
    fn view(&self) -> impl View {
        let surface = column()
            .width(Dimension::FILL)
            .height(Dimension::FILL)
            .padding(20.0)
            .child(
                text_input()
                    .key("main-editor")
                    .accessible_label("Main editor")
                    .value("original")
                    .height(34.0)
                    .padding(8.0)
                    .on_input(|this: &mut Self, event| {
                        this.events.lock().unwrap().push(event.clone());
                        false
                    }),
            )
            .child(
                button()
                    .accessible_label("Show dialog")
                    .height(34.0)
                    .child(text("Show dialog"))
                    .on_press(|this: &mut Self| this.modal = true),
            );
        let dialog = self.modal.then(|| {
            column()
                .key("modal")
                .width(200.0)
                .height(120.0)
                .focus_scope(true)
                .child(
                    text_input()
                        .key("dialog-editor")
                        .accessible_label("Dialog editor")
                        .autofocus(true)
                        .on_input(|this: &mut Self, event| {
                            this.events.lock().unwrap().push(event.clone());
                            false
                        }),
                )
                .child(
                    button()
                        .accessible_label("Close dialog")
                        .height(34.0)
                        .child(text("Close dialog"))
                        .on_press(|this: &mut Self| this.modal = false),
                )
        });
        stack().child(surface).children(dialog)
    }
}

fn mounted() -> (ComposedAppRuntime, Arc<Mutex<Vec<UiEvent>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut runtime = ComposedAppRuntime::from_composed_with_extent(
        Editors {
            events: EventLog(events.clone()),
            modal: false,
        },
        SizeI {
            width: 480,
            height: 300,
        },
    )
    .unwrap();
    runtime
        .prepare_frame(MonotonicInstant::ZERO, false)
        .unwrap();
    (runtime, events)
}

fn named(runtime: &ComposedAppRuntime, name: &str) -> UiNodeId {
    runtime
        .ui()
        .semantics
        .iter()
        .find_map(|(node, semantic)| match semantic.name {
            SemanticName::Text(value)
                if runtime.ui().string(value) == Some(name)
                    && matches!(
                        semantic.role,
                        SemanticRole::Button | SemanticRole::TextInput
                    ) =>
            {
                Some(node)
            }
            _ => None,
        })
        .unwrap()
}

fn input(runtime: &mut ComposedAppRuntime, event: InputEvent, tick: &mut u64) {
    *tick += 1;
    let now = MonotonicInstant::from_nanos(*tick * 1_000_000);
    runtime.queue_input(event);
    runtime.flush_input(now);
    runtime.prepare_frame(now, false).unwrap();
}

fn key(
    runtime: &mut ComposedAppRuntime,
    key: NamedKey,
    state: ButtonState,
    text: Option<&str>,
    tick: &mut u64,
) {
    input(
        runtime,
        InputEvent::Key(
            KeyEvent::new(PhysicalKey::UNIDENTIFIED, state)
                .with_logical_key(LogicalKey::Named(key))
                .with_text(text.map(|text| KeyText::new(text).unwrap())),
        ),
        tick,
    );
}

fn click(runtime: &mut ComposedAppRuntime, label: &str, tick: &mut u64) -> UiNodeId {
    let node = named(runtime, label);
    let rect = runtime.layout().computed(node).unwrap().border_rect;
    input(
        runtime,
        InputEvent::mouse_moved(PointF {
            x: rect.x + rect.width * 0.5,
            y: rect.y + rect.height * 0.5,
        }),
        tick,
    );
    input(
        runtime,
        InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Pressed),
        tick,
    );
    input(
        runtime,
        InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Released),
        tick,
    );
    node
}

fn focused(runtime: &ComposedAppRuntime, node: UiNodeId) -> bool {
    runtime
        .ui()
        .interactions
        .get(node)
        .unwrap()
        .flags
        .contains(InteractionFlags::FOCUSED)
}

#[test]
fn editors_receive_real_pointer_geometry_capture_and_modifier_snapshots_without_button_activation()
{
    let (mut runtime, events) = mounted();
    let mut tick = 0;
    let editor = click(&mut runtime, "Main editor", &mut tick);
    assert_eq!(runtime.ui().kinds.get(editor), Some(&NodeKind::TextInput));
    assert_eq!(
        runtime.ui().interactions.get(editor).unwrap().behavior,
        ControlBehavior::TextInput
    );
    assert_eq!(
        runtime.ui().semantics.get(editor).unwrap().role,
        SemanticRole::TextInput
    );
    assert!(focused(&runtime, editor));
    events.lock().unwrap().clear();
    key(
        &mut runtime,
        NamedKey::Space,
        ButtonState::Pressed,
        Some(" "),
        &mut tick,
    );
    assert!(
        !runtime
            .ui()
            .interactions
            .get(editor)
            .unwrap()
            .flags
            .contains(InteractionFlags::PRESSED)
    );
    key(
        &mut runtime,
        NamedKey::Space,
        ButtonState::Released,
        None,
        &mut tick,
    );
    assert!(focused(&runtime, editor));
    input(
        &mut runtime,
        InputEvent::ModifiersChanged(Modifiers::SHIFT),
        &mut tick,
    );
    let rect = runtime.layout().computed(editor).unwrap().border_rect;
    input(
        &mut runtime,
        InputEvent::mouse_moved(PointF {
            x: rect.x + 15.0,
            y: rect.y + 10.0,
        }),
        &mut tick,
    );
    input(
        &mut runtime,
        InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Pressed),
        &mut tick,
    );
    input(
        &mut runtime,
        InputEvent::mouse_moved(PointF {
            x: rect.x + rect.width + 70.0,
            y: rect.y + 10.0,
        }),
        &mut tick,
    );
    input(
        &mut runtime,
        InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Released),
        &mut tick,
    );
    let events = events.lock().unwrap();
    let press = events
        .iter()
        .find(|event| {
            matches!(
                event.kind,
                UiEventKind::Input(InputEvent::PointerButton {
                    state: ButtonState::Pressed,
                    ..
                })
            )
        })
        .unwrap();
    let geometry = press.geometry.unwrap();
    assert_eq!(geometry.position.unwrap(), PointF { x: 15.0, y: 10.0 });
    assert_eq!(geometry.content_rect.x, 8.0);
    assert_eq!(press.modifiers, Modifiers::SHIFT);
    assert!(events.iter().any(|event| event.target == editor
        && event.geometry.is_some_and(|geometry| {
            geometry
                .position
                .is_some_and(|position| position.x > geometry.border_rect.width)
        })));
}

#[test]
fn modal_autofocus_traps_tab_and_restores_the_trigger_after_close() {
    let (mut runtime, _) = mounted();
    let mut tick = 0;
    let trigger = click(&mut runtime, "Show dialog", &mut tick);
    let editor = named(&runtime, "Dialog editor");
    let close = named(&runtime, "Close dialog");
    assert!(focused(&runtime, editor));
    key(
        &mut runtime,
        NamedKey::Tab,
        ButtonState::Pressed,
        None,
        &mut tick,
    );
    assert!(
        runtime.ui().nodes.contains(close),
        "Close node was replaced; active named node {:?}",
        named(&runtime, "Close dialog")
    );
    assert!(focused(&runtime, close));
    key(
        &mut runtime,
        NamedKey::Tab,
        ButtonState::Pressed,
        None,
        &mut tick,
    );
    assert!(focused(&runtime, editor));
    click(&mut runtime, "Close dialog", &mut tick);
    assert!(!runtime.ui().nodes.contains(editor));
    assert!(focused(&runtime, trigger));
}

#[test]
fn window_reactivation_restores_editor_and_resize_publishes_current_content_bounds() {
    let (mut runtime, events) = mounted();
    let mut tick = 0;
    let editor = click(&mut runtime, "Main editor", &mut tick);
    runtime.deactivate_view(MonotonicInstant::from_nanos(10_000_000));
    assert!(!focused(&runtime, editor));
    runtime.activate_view(MonotonicInstant::from_nanos(20_000_000));
    assert!(focused(&runtime, editor));
    events.lock().unwrap().clear();
    runtime.queue_input(telorgon::PlatformInput::Resize(SizeF {
        width: 600.0,
        height: 300.0,
    }));
    runtime.flush_input(MonotonicInstant::from_nanos(30_000_000));
    runtime
        .prepare_frame(MonotonicInstant::from_nanos(30_000_000), false)
        .unwrap();
    assert!(events.lock().unwrap().iter().any(|event| {
        matches!(event.kind, UiEventKind::Layout(geometry) if geometry.content_rect.width > 500.0)
    }));
}

#[test]
fn native_text_composition_is_delivered_only_to_its_still_focused_generation() {
    use telorgon::input::TextInputEvent;
    let (mut runtime, events) = mounted();
    let mut tick = 0;
    let previous = click(&mut runtime, "Main editor", &mut tick);
    click(&mut runtime, "Show dialog", &mut tick);
    let current = named(&runtime, "Dialog editor");
    events.lock().unwrap().clear();
    runtime.queue_input(telorgon::PlatformInput::TextInput {
        target: previous,
        event: TextInputEvent::Commit("stale".into()),
    });
    runtime.queue_input(telorgon::PlatformInput::TextInput {
        target: current,
        event: TextInputEvent::Preedit {
            text: "輸入".into(),
            selection: Some((6, 6)),
        },
    });
    runtime.queue_input(telorgon::PlatformInput::TextInput {
        target: current,
        event: TextInputEvent::Commit("輸入".into()),
    });
    runtime.flush_input(MonotonicInstant::from_nanos(50_000_000));
    let events = events.lock().unwrap();
    assert!(!events.iter().any(|event| event.target == previous));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.kind,
                UiEventKind::Input(InputEvent::TextInput(TextInputEvent::Commit(_)))
            ))
            .count(),
        1
    );
    assert!(events.iter().any(|event| event.target == current
        && matches!(
            event.kind,
            UiEventKind::Input(InputEvent::TextInput(TextInputEvent::Preedit { .. }))
        )));
}
