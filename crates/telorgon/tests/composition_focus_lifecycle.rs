use std::sync::{Arc, Mutex};
use telorgon::{
    ComposedAppRuntime, DirtyFlags, InputEvent, MonotonicInstant, PlatformInput,
    app::*,
    input::{
        ButtonState, KeyEvent, KeyText, LogicalKey, Modifiers, NamedKey, PhysicalKey,
        PointerButton, TextInputEvent,
    },
    ui::{
        ControlBehavior, InteractionFlags, SemanticName, SemanticRole, UiEvent, UiEventKind,
        UiNodeId,
    },
};

#[derive(Default)]
struct Observations {
    events: Vec<UiEvent>,
    activations: usize,
}

#[derive(Clone)]
struct EventLog(Arc<Mutex<Observations>>);
impl PartialEq for EventLog {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl EventLog {
    fn record(&self, event: &UiEvent) -> bool {
        self.0.lock().unwrap().events.push(event.clone());
        false
    }
}

#[component(no_default)]
struct FocusFixture {
    #[input]
    observations: EventLog,
}

impl Component for FocusFixture {
    fn view(&self) -> impl View {
        column()
            .width(Dimension::FILL)
            .height(Dimension::FILL)
            .scrollable()
            .child(
                button()
                    .key("action")
                    .accessible_label("Action")
                    .height(40.0)
                    .child(text("Action"))
                    .on_press(|this: &mut Self| {
                        this.observations.0.lock().unwrap().activations += 1;
                    })
                    .on_input(|this: &mut Self, event| this.observations.record(event)),
            )
            .child(
                column().key("editor-parent").height(60.0).child(
                    text_input()
                        .key("editor")
                        .accessible_label("Editor")
                        .value("original")
                        .height(40.0)
                        .on_input(|this: &mut Self, event| this.observations.record(event)),
                ),
            )
            .child(
                button()
                    .key("other")
                    .accessible_label("Other")
                    .height(40.0)
                    .child(text("Other"))
                    .on_press(|this: &mut Self| {
                        this.observations.0.lock().unwrap().activations += 1;
                    })
                    .on_input(|this: &mut Self, event| this.observations.record(event)),
            )
            .child(column().height(700.0).child(text("Scrollable content")))
    }
}

struct Harness {
    runtime: ComposedAppRuntime,
    observations: Arc<Mutex<Observations>>,
    tick: u64,
}

impl Harness {
    fn new() -> Self {
        let observations = Arc::new(Mutex::new(Observations::default()));
        let runtime = ComposedAppRuntime::from_composed_with_extent(
            FocusFixture {
                observations: EventLog(observations.clone()),
            },
            SizeI {
                width: 480,
                height: 300,
            },
        )
        .unwrap();
        let mut harness = Self {
            runtime,
            observations,
            tick: 0,
        };
        harness.flush();
        harness
    }

    fn named(&self, name: &str) -> UiNodeId {
        self.runtime
            .ui()
            .semantics
            .iter()
            .find_map(|(node, semantic)| match semantic.name {
                SemanticName::Text(value)
                    if self.runtime.ui().string(value) == Some(name)
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

    fn point(&self, node: UiNodeId) -> PointF {
        let rect = self.runtime.layout().computed(node).unwrap().border_rect;
        PointF {
            x: rect.x + rect.width * 0.5,
            y: rect.y + rect.height * 0.5,
        }
    }

    fn now(&mut self) -> MonotonicInstant {
        self.tick += 1;
        MonotonicInstant::from_nanos(self.tick * 1_000_000)
    }

    fn flush(&mut self) {
        let now = self.now();
        self.runtime.flush_input(now);
        self.runtime.prepare_frame(now, false).unwrap();
    }

    fn input(&mut self, event: InputEvent) {
        self.runtime.queue_input(event);
        self.flush();
    }

    fn click(&mut self, label: &str) -> UiNodeId {
        let node = self.named(label);
        self.runtime
            .queue_input(InputEvent::mouse_moved(self.point(node)));
        self.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Pressed,
        ));
        self.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Released,
        ));
        self.flush();
        node
    }

    fn deactivate(&mut self) {
        let now = self.now();
        self.runtime.deactivate_view(now);
    }

    fn activate(&mut self) {
        let now = self.now();
        self.runtime.activate_view(now);
    }

    fn clear_events(&mut self) {
        self.observations.lock().unwrap().events.clear();
    }

    fn has_flag(&self, node: UiNodeId, flag: InteractionFlags) -> bool {
        self.runtime
            .ui()
            .interactions
            .get(node)
            .unwrap()
            .flags
            .contains(flag)
    }

    fn focus_events(&self, node: UiNodeId) -> Vec<bool> {
        self.observations
            .lock()
            .unwrap()
            .events
            .iter()
            .filter_map(|event| match event.kind {
                UiEventKind::Focus(value) if event.target == node => Some(value),
                _ => None,
            })
            .collect()
    }

    fn input_events(&self) -> Vec<UiEvent> {
        self.observations
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| matches!(event.kind, UiEventKind::Input(_)))
            .cloned()
            .collect()
    }

    fn activations(&self) -> usize {
        self.observations.lock().unwrap().activations
    }

    fn editor_parent(&self, editor: UiNodeId) -> UiNodeId {
        self.runtime
            .ui()
            .nodes
            .core(editor)
            .unwrap()
            .parent
            .unwrap()
    }

    fn set_visible(&mut self, node: UiNodeId, visible: bool) {
        let ui = self.runtime.ui_mut();
        ui.set_focus_scope(node, false);
        ui.interactions.get_mut(node).unwrap().visible = visible;
        ui.nodes.mark_dirty(
            node,
            DirtyFlags::VISIBILITY | DirtyFlags::SPATIAL | DirtyFlags::CLIP | DirtyFlags::PAINT,
        );
        self.runtime.scheduler_mut().request();
        self.flush();
    }

    fn scroll_offset(&self) -> PointF {
        let node = self
            .runtime
            .ui()
            .interactions
            .iter()
            .find_map(|(node, state)| (state.behavior == ControlBehavior::Scroll).then_some(node))
            .unwrap();
        self.runtime
            .ui()
            .layouts
            .get(node)
            .map_or(PointF::default(), |layout| layout.scroll_offset)
    }
}

fn named_key(key: NamedKey) -> InputEvent {
    InputEvent::Key(
        KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Pressed)
            .with_logical_key(LogicalKey::Named(key)),
    )
}

fn typed_key(text: &str) -> InputEvent {
    InputEvent::Key(
        KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Pressed)
            .with_logical_key(LogicalKey::Character(KeyText::new(text).unwrap()))
            .with_text(Some(KeyText::new(text).unwrap())),
    )
}

#[test]
fn deactivation_discards_pending_input_before_a_quick_reactivation_but_keeps_resize() {
    let mut harness = Harness::new();
    let editor = harness.click("Editor");
    let action = harness.named("Action");
    harness.clear_events();
    harness.runtime.queue_input(PlatformInput::TextInput {
        target: editor,
        event: TextInputEvent::Commit("stale composition".into()),
    });
    harness.runtime.queue_input(typed_key("stale key"));
    harness
        .runtime
        .queue_input(InputEvent::mouse_moved(harness.point(action)));
    harness.runtime.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Pressed,
    ));
    harness.runtime.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Released,
    ));
    harness
        .runtime
        .queue_input(InputEvent::ModifiersChanged(Modifiers::SHIFT));
    harness
        .runtime
        .queue_input(InputEvent::mouse_scroll(PointF { x: 0.0, y: -40.0 }));
    harness.runtime.queue_input(PlatformInput::Resize(SizeF {
        width: 640.0,
        height: 400.0,
    }));
    harness.deactivate();
    harness.activate();
    harness.flush();

    assert!(
        harness.input_events().is_empty(),
        "queued input crossed the activation boundary"
    );
    assert_eq!(harness.activations(), 0);
    assert_eq!(harness.runtime.interaction_diagnostics().activations, 0);
    assert!(harness.has_flag(editor, InteractionFlags::FOCUSED));
    assert!(!harness.has_flag(action, InteractionFlags::PRESSED));
    assert_eq!(
        harness.runtime.extent(),
        SizeF {
            width: 640.0,
            height: 400.0
        }
    );

    harness.click("Action");
    assert_eq!(
        harness.activations(),
        1,
        "a fresh click must still activate once"
    );
}

#[test]
fn late_inactive_pointer_keyboard_composition_and_scroll_are_ignored() {
    let mut harness = Harness::new();
    let action = harness.click("Action");
    let editor = harness.named("Editor");
    let activations = harness.activations();
    let offset = harness.scroll_offset();
    harness.deactivate();
    harness.clear_events();
    harness
        .runtime
        .queue_input(InputEvent::mouse_moved(harness.point(action)));
    harness.runtime.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Pressed,
    ));
    harness.runtime.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Released,
    ));
    harness.runtime.queue_input(named_key(NamedKey::Tab));
    harness.runtime.queue_input(named_key(NamedKey::Enter));
    harness.runtime.queue_input(typed_key("late key"));
    harness
        .runtime
        .queue_input(InputEvent::ModifiersChanged(Modifiers::SHIFT));
    harness
        .runtime
        .queue_input(InputEvent::TextInput(TextInputEvent::Commit(
            "late legacy composition".into(),
        )));
    harness.runtime.queue_input(PlatformInput::TextInput {
        target: editor,
        event: TextInputEvent::Commit("late native composition".into()),
    });
    harness
        .runtime
        .queue_input(InputEvent::mouse_scroll(PointF { x: 0.0, y: -50.0 }));
    harness.runtime.queue_input(PlatformInput::Resize(SizeF {
        width: 700.0,
        height: 450.0,
    }));
    harness.flush();

    assert!(harness.input_events().is_empty());
    assert_eq!(harness.activations(), activations);
    assert!(harness.runtime.ui().interactions.iter().all(|(_, state)| {
        !state.flags.contains(InteractionFlags::FOCUSED)
            && !state.flags.contains(InteractionFlags::PRESSED)
            && !state.flags.contains(InteractionFlags::HOVERED)
    }));
    assert_eq!(harness.scroll_offset(), offset);
    assert_eq!(
        harness.runtime.extent(),
        SizeF {
            width: 700.0,
            height: 450.0
        }
    );

    harness.activate();
    assert!(harness.has_flag(action, InteractionFlags::FOCUSED));
    harness.click("Action");
    assert_eq!(harness.activations(), activations + 1);
    harness.input(named_key(NamedKey::Enter));
    assert_eq!(harness.activations(), activations + 2);
    harness.input(InputEvent::mouse_scroll(PointF { x: 0.0, y: -25.0 }));
    assert!(
        harness.scroll_offset().y > offset.y,
        "fresh active scrolling must still move content"
    );
}

#[test]
fn hiding_an_editor_ancestor_blurs_once_and_blocks_key_and_composition_delivery() {
    let mut harness = Harness::new();
    let editor = harness.click("Editor");
    let parent = harness.editor_parent(editor);
    harness.clear_events();
    harness.set_visible(parent, false);

    assert!(
        harness
            .runtime
            .ui()
            .interactions
            .get(editor)
            .unwrap()
            .visible,
        "the editor itself stays visible; ancestor visibility must decide eligibility"
    );
    assert!(!harness.has_flag(editor, InteractionFlags::FOCUSED));
    assert_eq!(harness.focus_events(editor), vec![false]);
    harness.clear_events();
    harness.runtime.queue_input(typed_key("hidden key"));
    harness.runtime.queue_input(PlatformInput::TextInput {
        target: editor,
        event: TextInputEvent::Preedit {
            text: "hidden preedit".into(),
            selection: Some((0, 0)),
        },
    });
    harness.runtime.queue_input(PlatformInput::TextInput {
        target: editor,
        event: TextInputEvent::Commit("hidden commit".into()),
    });
    harness
        .runtime
        .queue_input(InputEvent::TextInput(TextInputEvent::Commit(
            "hidden legacy commit".into(),
        )));
    harness.flush();
    assert!(harness.input_events().is_empty());
    assert_eq!(
        harness.named("Editor"),
        editor,
        "hiding must not replace the control generation"
    );

    harness.set_visible(parent, true);
    assert_eq!(harness.click("Editor"), editor);
    harness.clear_events();
    harness.runtime.queue_input(typed_key("fresh key"));
    harness.runtime.queue_input(PlatformInput::TextInput {
        target: editor,
        event: TextInputEvent::Commit("fresh commit".into()),
    });
    harness.flush();
    let events = harness.input_events();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event.target == editor));
    assert!(matches!(
        events[0].kind,
        UiEventKind::Input(InputEvent::Key(_))
    ));
    assert!(matches!(&events[1].kind,
        UiEventKind::Input(InputEvent::TextInput(TextInputEvent::Commit(text))) if text == "fresh commit"));
}

#[test]
fn a_saved_editor_hidden_while_inactive_is_not_restored_or_sent_text() {
    let mut harness = Harness::new();
    let editor = harness.click("Editor");
    let parent = harness.editor_parent(editor);
    harness.deactivate();
    harness.clear_events();
    harness.set_visible(parent, false);
    harness.activate();
    harness.flush();

    assert!(!harness.has_flag(editor, InteractionFlags::FOCUSED));
    assert!(
        harness.focus_events(editor).is_empty(),
        "hidden saved owner was restored"
    );
    harness.runtime.queue_input(typed_key("blocked key"));
    harness.runtime.queue_input(PlatformInput::TextInput {
        target: editor,
        event: TextInputEvent::Commit("blocked commit".into()),
    });
    harness.flush();
    assert!(harness.input_events().is_empty());
    harness.set_visible(parent, true);
    assert!(!harness.has_flag(editor, InteractionFlags::FOCUSED));
    harness.click("Editor");
    assert!(harness.has_flag(editor, InteractionFlags::FOCUSED));
}

#[test]
fn window_focus_notifications_are_idempotent_and_preserve_keyboard_focus_visibility() {
    let mut harness = Harness::new();
    let action = harness.named("Action");
    harness.clear_events();
    harness.input(named_key(NamedKey::Tab));
    assert!(harness.has_flag(action, InteractionFlags::FOCUSED));
    assert!(harness.has_flag(action, InteractionFlags::FOCUS_VISIBLE));
    harness.activate();
    assert!(harness.has_flag(action, InteractionFlags::FOCUS_VISIBLE));
    harness.deactivate();
    harness.deactivate();
    assert!(!harness.has_flag(action, InteractionFlags::FOCUSED));
    harness.activate();
    harness.activate();
    harness.flush();

    assert!(harness.has_flag(action, InteractionFlags::FOCUSED));
    assert!(harness.has_flag(action, InteractionFlags::FOCUS_VISIBLE));
    assert_eq!(harness.focus_events(action), vec![true, false, true]);
    assert_eq!(harness.activations(), 0);
    harness.input(named_key(NamedKey::Enter));
    assert_eq!(harness.activations(), 1);
}

#[test]
fn queued_active_motion_preserves_pointer_button_order_and_capture_owner() {
    let mut harness = Harness::new();
    let action = harness.click("Action");
    let other = harness.named("Other");
    let activations = harness.activations();
    let routed_activations = harness.runtime.interaction_diagnostics().activations;
    harness.clear_events();
    harness.runtime.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Pressed,
    ));
    harness
        .runtime
        .queue_input(InputEvent::mouse_moved(harness.point(other)));
    harness.runtime.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Released,
    ));
    harness.flush();

    assert_eq!(
        harness.activations(),
        activations,
        "dragging out must not click either button"
    );
    assert_eq!(
        harness.runtime.interaction_diagnostics().activations,
        routed_activations
    );
    assert!(!harness.has_flag(action, InteractionFlags::PRESSED));
    assert!(!harness.has_flag(other, InteractionFlags::PRESSED));
    let buttons: Vec<_> = harness
        .input_events()
        .iter()
        .filter_map(|event| match event.kind {
            UiEventKind::Input(InputEvent::PointerButton { state, .. }) => {
                Some((event.target, state))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        buttons,
        vec![
            (action, ButtonState::Pressed),
            (action, ButtonState::Released)
        ],
        "the down and captured release belong to the button under the pointer before queued motion"
    );
}

#[test]
fn inactive_and_discarded_motion_supply_position_for_the_first_click_after_activation() {
    for queued_before_blur in [false, true] {
        let mut harness = Harness::new();
        let action = harness.click("Action");
        let editor = harness.named("Editor");
        let position = harness.point(editor);
        let activations = harness.activations();
        if queued_before_blur {
            harness
                .runtime
                .queue_input(InputEvent::mouse_moved(position));
        }
        harness.deactivate();
        harness.clear_events();
        if !queued_before_blur {
            harness
                .runtime
                .queue_input(InputEvent::mouse_moved(position));
        }
        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Pressed,
        ));
        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Released,
        ));
        harness.flush();
        assert!(harness.input_events().is_empty());
        assert!(!harness.has_flag(editor, InteractionFlags::FOCUSED));

        harness.activate();
        assert!(harness.has_flag(action, InteractionFlags::FOCUSED));
        harness.clear_events();
        // Native button events need the most recently observed position even without new motion.
        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Pressed,
        ));
        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Released,
        ));
        harness.flush();

        assert!(harness.has_flag(editor, InteractionFlags::FOCUSED));
        assert!(!harness.has_flag(action, InteractionFlags::FOCUSED));
        assert_eq!(harness.activations(), activations);
        let events = harness.input_events();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| event.target == editor
            && matches!(
                event.kind,
                UiEventKind::Input(InputEvent::PointerButton { .. })
            )));
    }
}
