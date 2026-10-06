use std::collections::HashMap;

use crate::foundation::PointF;
use crate::input::{
    Activation, ActivationInput, ActivationStateMachine, ActivationTransition, ButtonState,
    CompetingGesture, KeyEvent, LogicalKey, NamedKey, PointerButton, PointerCaptureRequest,
    PointerId, ValueChangePhase,
};
use crate::graphics::scene::NodeId;
use crate::ui::{ControlBehavior, InteractionFlags, MountedUi};

mod passive;
mod focus;
mod focus_state;
mod view_lifecycle;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InteractionDiagnostics {
    pub state_publications: u64,
    pub activations: u64,
    pub cancellations: u64,
    pub stale_owners_rejected: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FocusChange {
    pub old: Option<NodeId>,
    pub new: Option<NodeId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PointerRouting {
    pub target: Option<NodeId>,
    pub activation: Option<(NodeId, Activation)>,
    pub value: Option<(NodeId, ValueChangePhase)>,
    pub focus: Option<FocusChange>,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeyRouting {
    pub activation: Option<(NodeId, Activation)>,
    pub changed: bool,
}

#[derive(Clone, Debug, Default)]
struct PointerRoute {
    position: PointF,
    raw_hovered: Option<NodeId>,
    hovered: Option<NodeId>,
    captured: Option<NodeId>,
    visual_pressed: Vec<NodeId>,
}

/// Per-view owner for hover, capture, activation, and focus publication.
///
/// Every activation machine belongs to one generational control root. Pointer routes only retain
/// pointer-local hover/capture, so adjacent controls and simultaneous contacts cannot share an arm
/// or pressed state.
#[derive(Default)]
pub struct InteractionRouter {
    pointers: HashMap<PointerId, PointerRoute>,
    controls: HashMap<NodeId, ActivationStateMachine>,
    focused: Option<NodeId>,
    focus_scope: Option<NodeId>,
    scope_restore: Vec<(NodeId, Option<NodeId>)>,
    suspended_focus: Option<(NodeId, bool)>,
    focus_visible: bool,
    deactivated: bool,
    always_show_focus: bool,
    diagnostics: InteractionDiagnostics,
}

impl InteractionRouter {
    pub fn diagnostics(&self) -> InteractionDiagnostics {
        self.diagnostics
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    pub fn pointer_position(&self, pointer: PointerId) -> Option<PointF> {
        self.pointers.get(&pointer).map(|route| route.position)
    }

    pub(crate) fn observe_pointer_position(&mut self, pointer: PointerId, position: PointF) {
        self.pointers.entry(pointer).or_default().position = position;
    }

    pub(crate) fn pointer_moved(
        &mut self,
        ui: &mut MountedUi,
        pointer: PointerId,
        position: PointF,
        raw_hit: Option<NodeId>,
    ) -> PointerRouting {
        self.observe_pointer_position(pointer, position);
        if !self.is_active() {
            return PointerRouting::default();
        }
        let synchronized = self.sync(ui);
        let hit = raw_hit.and_then(|node| ui.nearest_control(node));
        let hovered_hit = hit.filter(|node| self.behavior(ui, *node).is_some());
        let previous_hover = self.pointers.get(&pointer).and_then(|route| route.hovered);
        {
            let route = self.pointers.entry(pointer).or_default();
            route.position = position;
            route.raw_hovered = raw_hit;
            route.hovered = hovered_hit;
        }

        let mut routing = PointerRouting {
            target: self
                .pointers
                .get(&pointer)
                .and_then(|route| route.captured)
                .or(hit)
                .or(raw_hit),
            changed: synchronized,
            ..PointerRouting::default()
        };
        if previous_hover != hovered_hit {
            if let Some(old) = previous_hover {
                let still_hovered = self
                    .pointers
                    .iter()
                    .any(|(id, route)| *id != pointer && route.hovered == Some(old));
                routing.changed |=
                    self.publish_flag(ui, old, InteractionFlags::HOVERED, still_hovered);
            }
            if let Some(new) = hovered_hit {
                routing.changed |= self.publish_flag(ui, new, InteractionFlags::HOVERED, true);
            }
        }

        routing.changed |= self.publish_visual_interaction(ui);
        let Some(captured) = self.pointers.get(&pointer).and_then(|route| route.captured) else {
            return routing;
        };
        if self.behavior(ui, captured) == Some(ControlBehavior::TextInput) {
            return routing;
        }
        let inside = raw_hit.is_some_and(|node| ui.is_descendant_or_self(node, captured));
        let outcome = self.handle_activation(
            ui,
            captured,
            ActivationInput::PointerMoved { pointer, inside },
        );
        routing.changed |= outcome.changed;
        routing.activation = outcome.activation.map(|activation| (captured, activation));
        if self.behavior(ui, captured) == Some(ControlBehavior::Value) {
            routing.value = Some((captured, ValueChangePhase::Update));
        }
        routing
    }

    pub(crate) fn pointer_button(
        &mut self,
        ui: &mut MountedUi,
        pointer: PointerId,
        button: PointerButton,
        state: ButtonState,
        raw_hit: Option<NodeId>,
    ) -> PointerRouting {
        if !self.is_active() {
            return PointerRouting::default();
        }
        let synchronized = self.sync(ui);
        let hit = raw_hit.and_then(|node| ui.nearest_control(node));
        let visual_changed = self.visual_pointer_button(ui, pointer, button, state, raw_hit);
        let captured = self.pointers.get(&pointer).and_then(|route| route.captured);
        let control = captured.or(hit);
        let mut routing = PointerRouting {
            target: control.or(raw_hit),
            changed: synchronized | visual_changed,
            ..PointerRouting::default()
        };
        let Some(target) = control else {
            if state == ButtonState::Pressed && button == PointerButton::PRIMARY {
                let focus = self.set_focus(ui, None, false);
                if focus.old != focus.new {
                    routing.focus = Some(focus);
                    routing.changed = true;
                }
            }
            return routing;
        };

        if state == ButtonState::Pressed && button == PointerButton::PRIMARY {
            let focus = self.set_focus(ui, Some(target), false);
            if focus.old != focus.new {
                routing.focus = Some(focus);
                routing.changed = true;
            }
        }

        let Some(behavior) = self.behavior(ui, target) else {
            return routing;
        };
        if behavior == ControlBehavior::TextInput && button == PointerButton::PRIMARY {
            self.set_capture(pointer, target, match state {
                ButtonState::Pressed => PointerCaptureRequest::Capture(pointer),
                ButtonState::Released => PointerCaptureRequest::Release(pointer),
            });
            return routing;
        }
        if !matches!(behavior, ControlBehavior::Activate | ControlBehavior::Value) {
            return routing;
        }
        let inside = raw_hit.is_some_and(|node| ui.is_descendant_or_self(node, target));
        let input = match state {
            ButtonState::Pressed => ActivationInput::PointerDown { pointer, button },
            ButtonState::Released => ActivationInput::PointerUp {
                pointer,
                button,
                inside,
            },
        };
        let outcome = self.handle_activation(ui, target, input);
        routing.changed |= outcome.changed;
        if let Some(capture) = outcome.capture {
            self.set_capture(pointer, target, capture);
        }
        if behavior == ControlBehavior::Activate {
            routing.activation = outcome.activation.map(|activation| (target, activation));
        }
        if behavior == ControlBehavior::Value && button == PointerButton::PRIMARY {
            routing.value = Some((
                target,
                if state == ButtonState::Pressed {
                    ValueChangePhase::Begin
                } else {
                    ValueChangePhase::Commit
                },
            ));
        }
        routing
    }

    pub(crate) fn key(&mut self, ui: &mut MountedUi, key: &KeyEvent) -> KeyRouting {
        if !self.is_active() {
            return KeyRouting::default();
        }
        self.sync(ui);
        let Some(target) = self.focused else {
            return KeyRouting::default();
        };
        self.focus_visible = true;
        let focus_changed = self.publish_flag(ui, target, InteractionFlags::FOCUS_VISIBLE, true);
        if self.behavior(ui, target) != Some(ControlBehavior::Activate) {
            return KeyRouting {
                changed: focus_changed,
                ..KeyRouting::default()
            };
        }
        let LogicalKey::Named(named) = key.logical_key else {
            return KeyRouting {
                changed: focus_changed,
                ..KeyRouting::default()
            };
        };
        let input = match (named, key.state) {
            (NamedKey::Enter, ButtonState::Pressed) => {
                Some(ActivationInput::EnterDown { repeat: key.repeat })
            }
            (NamedKey::Space, ButtonState::Pressed) => {
                Some(ActivationInput::SpaceDown { repeat: key.repeat })
            }
            (NamedKey::Space, ButtonState::Released) => Some(ActivationInput::SpaceUp),
            _ => None,
        };
        let Some(input) = input else {
            return KeyRouting {
                changed: focus_changed,
                ..KeyRouting::default()
            };
        };
        let outcome = self.handle_activation(ui, target, input);
        KeyRouting {
            activation: outcome.activation.map(|activation| (target, activation)),
            changed: outcome.changed | focus_changed,
        }
    }

    pub(crate) fn cancel_pointer(&mut self, ui: &mut MountedUi, pointer: PointerId) -> bool {
        self.cancel_pointer_with(ui, pointer, |pointer| ActivationInput::PointerCancelled {
            pointer,
        })
    }

    pub(crate) fn capture_lost(&mut self, ui: &mut MountedUi, pointer: PointerId) -> bool {
        self.cancel_pointer_with(ui, pointer, |pointer| ActivationInput::PointerCaptureLost {
            pointer,
        })
    }

    pub(crate) fn gesture_claimed(
        &mut self,
        ui: &mut MountedUi,
        pointer: PointerId,
        gesture: CompetingGesture,
    ) -> bool {
        self.cancel_pointer_with(ui, pointer, |pointer| {
            ActivationInput::PointerGestureClaimed { pointer, gesture }
        })
    }

    pub(crate) fn sync(&mut self, ui: &mut MountedUi) -> bool {
        let stale_controls: Vec<_> = self
            .controls
            .keys()
            .copied()
            .filter(|node| {
                !ui.nodes.contains(*node)
                    || !ui.interactions.get(*node).is_some_and(|interaction| {
                        matches!(
                            interaction.behavior,
                            ControlBehavior::Activate | ControlBehavior::Value
                        )
                    })
            })
            .collect();
        for node in &stale_controls {
            if let Some(machine) = self.controls.get_mut(node) {
                let outcome = machine.handle(ActivationInput::Unmount);
                if matches!(outcome.transition, ActivationTransition::Cancelled { .. }) {
                    self.diagnostics.cancellations += 1;
                }
            }
        }
        for node in &stale_controls {
            self.controls.remove(node);
        }
        let mut changed = !stale_controls.is_empty();

        let disabled_controls: Vec<_> = self.controls.keys().copied().filter(|node| {
            !self.control_available(ui, *node)
        }).collect();
        for node in disabled_controls {
            changed |= self.handle_activation(ui, node, ActivationInput::SetEnabled(false)).changed;
        }

        let pointer_ids: Vec<_> = self.pointers.keys().copied().collect();
        for pointer in pointer_ids {
            let Some((hovered, captured)) = self.pointers.get(&pointer)
                .map(|route| (route.hovered, route.captured)) else {
                continue;
            };
            if let Some(hovered) = hovered
                && (!ui.nodes.contains(hovered) || self.behavior(ui, hovered).is_none())
            {
                changed |= self.publish_flag(ui, hovered, InteractionFlags::HOVERED, false);
                if let Some(route) = self.pointers.get_mut(&pointer) {
                    route.hovered = None;
                }
                self.diagnostics.stale_owners_rejected += 1;
            }
            if let Some(captured) = captured {
                let eligible = ui.interactions.get(captured).is_some_and(|interaction| {
                    self.control_available(ui, captured)
                        && matches!(
                            interaction.behavior,
                            ControlBehavior::Activate | ControlBehavior::Value | ControlBehavior::TextInput
                        )
                });
                if !eligible {
                    let outcome =
                        self.handle_activation(ui, captured, ActivationInput::SetEnabled(false));
                    changed |= outcome.changed;
                    if let Some(route) = self.pointers.get_mut(&pointer) {
                        route.captured = None;
                    }
                    self.diagnostics.stale_owners_rejected += 1;
                }
            }
        }

        if self.focused.is_some_and(|node| !self.focus_eligible(ui, node)) {
            let focus = self.set_focus(ui, None, false);
            changed |= focus.old != focus.new;
            self.diagnostics.stale_owners_rejected += 1;
        }
        self.sync_visual_presses(ui);
        changed | self.publish_visual_interaction(ui)
    }

    fn behavior(&self, ui: &MountedUi, node: NodeId) -> Option<ControlBehavior> {
        ui.interactions.get(node).and_then(|interaction| {
            (self.control_available(ui, node)
                && interaction.behavior != ControlBehavior::None)
                .then_some(interaction.behavior)
        })
    }

    fn ensure_machine(&mut self, ui: &MountedUi, node: NodeId) -> bool {
        let enabled = self.control_available(ui, node);
        self.controls
            .entry(node)
            .or_insert_with(|| ActivationStateMachine::new(enabled));
        enabled
    }

    fn handle_activation(
        &mut self,
        ui: &mut MountedUi,
        node: NodeId,
        input: ActivationInput,
    ) -> ActivationRouting {
        let enabled = self.ensure_machine(ui, node);
        let machine = self
            .controls
            .get_mut(&node)
            .expect("machine was inserted above");
        let outcome = if matches!(input, ActivationInput::SetEnabled(_)) {
            machine.handle(input)
        } else if machine.enabled() != enabled {
            let synchronized = machine.handle(ActivationInput::SetEnabled(enabled));
            if enabled {
                machine.handle(input)
            } else {
                synchronized
            }
        } else if enabled {
            machine.handle(input)
        } else {
            return ActivationRouting::default();
        };
        // Value controls remain engaged throughout a captured drag, including outside
        // their bounds. Activating controls still disarm visually when the pointer leaves.
        let value_control = ui.interactions.get(node)
            .is_some_and(|interaction| interaction.behavior == ControlBehavior::Value);
        let pressed = self.controls.get(&node).is_some_and(|machine| {
            if value_control { machine.is_armed() } else { machine.is_visually_armed() }
        });
        let changed = self.publish_flag(ui, node, InteractionFlags::PRESSED, pressed);
        let activation = match outcome.transition {
            ActivationTransition::Activated(activation) => {
                self.diagnostics.activations += 1;
                Some(activation)
            }
            ActivationTransition::Cancelled { .. } => {
                self.diagnostics.cancellations += 1;
                None
            }
            _ => None,
        };
        ActivationRouting {
            activation,
            capture: match outcome.capture {
                PointerCaptureRequest::None => None,
                capture => Some(capture),
            },
            changed,
        }
    }

    fn set_capture(&mut self, pointer: PointerId, target: NodeId, request: PointerCaptureRequest) {
        let route = self.pointers.entry(pointer).or_default();
        match request {
            PointerCaptureRequest::None => {}
            PointerCaptureRequest::Capture(owner) if owner == pointer => {
                route.captured = Some(target)
            }
            PointerCaptureRequest::Capture(_) => {}
            PointerCaptureRequest::Release(owner) if owner == pointer => route.captured = None,
            PointerCaptureRequest::Release(_) => {}
        }
    }

    fn cancel_pointer_with(
        &mut self,
        ui: &mut MountedUi,
        pointer: PointerId,
        input: impl FnOnce(PointerId) -> ActivationInput,
    ) -> bool {
        let captured = self.pointers.get(&pointer).and_then(|route| route.captured);
        let mut changed = false;
        if let Some(captured) = captured {
            changed |= self.handle_activation(ui, captured, input(pointer)).changed;
        }
        if let Some(route) = self.pointers.get_mut(&pointer) {
            route.captured = None;
            route.visual_pressed.clear();
        }
        changed | self.publish_visual_interaction(ui)
    }

    fn publish_flag(
        &mut self,
        ui: &mut MountedUi,
        node: NodeId,
        flag: InteractionFlags,
        enabled: bool,
    ) -> bool {
        let changed = ui.route_interaction_flag(node, flag, enabled);
        self.diagnostics.state_publications += u64::from(changed);
        changed
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ActivationRouting {
    activation: Option<Activation>,
    capture: Option<PointerCaptureRequest>,
    changed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::ColorRgba8;
    use crate::ui::{BoxStyle, LayoutStyle, MountWriter};

    pub(super) struct Fixture {
        pub(super) ui: MountedUi,
        pub(super) first: NodeId,
        pub(super) first_label: NodeId,
        pub(super) second: NodeId,
    }

    pub(super) fn fixture() -> Fixture {
        let mut ui = MountedUi::default();
        let mut first = None;
        let mut first_label = None;
        let mut second = None;
        MountWriter::<()>::new(&mut ui).root(
            BoxStyle::default(),
            LayoutStyle::default(),
            |writer| {
                let control = writer.button_node(BoxStyle::default(), |writer| {
                    first_label = Some(writer.text("first", ColorRgba8::default(), 14.0).node);
                });
                first = Some(control.node);
                second = Some(
                    writer
                        .button_node(BoxStyle::default(), |writer| {
                            writer.text("second", ColorRgba8::default(), 14.0);
                        })
                        .node,
                );
            },
        );
        Fixture {
            ui,
            first: first.unwrap(),
            first_label: first_label.unwrap(),
            second: second.unwrap(),
        }
    }

    pub(super) fn has(ui: &MountedUi, node: NodeId, flag: InteractionFlags) -> bool {
        ui.interactions
            .get(node)
            .is_some_and(|interaction| interaction.flags.contains(flag))
    }

    #[test]
    fn hover_within_is_opt_in_and_independent_of_styles() {
        let mut fixture = fixture();
        let parent = fixture.ui.nodes.core(fixture.first).unwrap().parent.unwrap();
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(10);
        router.pointer_moved(&mut fixture.ui, pointer, PointF::default(), Some(fixture.first_label));
        assert!(!has(&fixture.ui, parent, InteractionFlags::HOVERED));

        fixture.ui.set_hover_within(parent, true);
        router.sync(&mut fixture.ui);
        assert!(has(&fixture.ui, parent, InteractionFlags::HOVERED));
        assert!(has(&fixture.ui, fixture.first, InteractionFlags::HOVERED));
        assert!(!has(&fixture.ui, parent, InteractionFlags::PRESSED));

        router.pointer_moved(&mut fixture.ui, pointer, PointF::default(), Some(fixture.second));
        assert!(has(&fixture.ui, parent, InteractionFlags::HOVERED));
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::HOVERED));
        assert!(has(&fixture.ui, fixture.second, InteractionFlags::HOVERED));

        fixture.ui.set_hover_within(parent, false);
        router.sync(&mut fixture.ui);
        assert!(!has(&fixture.ui, parent, InteractionFlags::HOVERED));
        assert!(has(&fixture.ui, fixture.second, InteractionFlags::HOVERED));
    }

    #[test]
    fn hover_within_tracks_multiple_pointers_and_clears_on_deactivation() {
        let mut fixture = fixture();
        let parent = fixture.ui.nodes.core(fixture.first).unwrap().parent.unwrap();
        fixture.ui.set_hover_within(parent, true);
        let mut router = InteractionRouter::default();
        for (id, target) in [(11, fixture.first), (12, fixture.second)] {
            router.pointer_moved(&mut fixture.ui, PointerId::new(id), PointF::default(), Some(target));
        }
        router.pointer_moved(&mut fixture.ui, PointerId::new(11), PointF::default(), None);
        assert!(has(&fixture.ui, parent, InteractionFlags::HOVERED));
        router.view_deactivated(&mut fixture.ui);
        assert!(!has(&fixture.ui, parent, InteractionFlags::HOVERED));
    }

    #[test]
    fn child_hits_publish_only_to_the_registered_control_root() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        router.pointer_moved(
            &mut fixture.ui,
            PointerId::new(1),
            PointF { x: 2.0, y: 2.0 },
            Some(fixture.first_label),
        );
        assert!(has(&fixture.ui, fixture.first, InteractionFlags::HOVERED));
        assert!(!has(
            &fixture.ui,
            fixture.first_label,
            InteractionFlags::HOVERED
        ));
        assert!(!has(&fixture.ui, fixture.second, InteractionFlags::HOVERED));
    }

    #[test]
    fn pressed_capture_rearms_without_leaking_to_the_sibling() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(2);
        router.pointer_moved(
            &mut fixture.ui,
            pointer,
            PointF::default(),
            Some(fixture.first_label),
        );
        router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(fixture.first_label),
        );
        assert!(has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));

        router.pointer_moved(
            &mut fixture.ui,
            pointer,
            PointF::default(),
            Some(fixture.second),
        );
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
        assert!(!has(&fixture.ui, fixture.second, InteractionFlags::PRESSED));

        router.pointer_moved(
            &mut fixture.ui,
            pointer,
            PointF::default(),
            Some(fixture.first_label),
        );
        assert!(has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
        let released = router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Released,
            Some(fixture.first_label),
        );
        assert_eq!(released.activation.map(|item| item.0), Some(fixture.first));
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
    }

    #[test]
    fn value_control_stays_pressed_during_outside_drag_until_release_or_cancel() {
        for cancel in [false, true] {
            let mut fixture = fixture();
            fixture.ui.set_control_behavior(fixture.first, ControlBehavior::Value);
            let mut router = InteractionRouter::default();
            let pointer = PointerId::new(22);
            router.pointer_button(
                &mut fixture.ui, pointer, PointerButton::PRIMARY,
                ButtonState::Pressed, Some(fixture.first),
            );
            let moved = router.pointer_moved(
                &mut fixture.ui, pointer, PointF::default(), Some(fixture.second),
            );
            assert_eq!(moved.value, Some((fixture.first, ValueChangePhase::Update)));
            assert!(has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
            assert!(!has(&fixture.ui, fixture.second, InteractionFlags::PRESSED));
            if cancel {
                router.capture_lost(&mut fixture.ui, pointer);
            } else {
                router.pointer_button(
                    &mut fixture.ui, pointer, PointerButton::PRIMARY,
                    ButtonState::Released, Some(fixture.second),
                );
            }
            assert!(!has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
            assert!(router.pointers[&pointer].captured.is_none());
        }
    }

    #[test]
    fn secondary_button_never_arms_or_activates() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(3);
        let down = router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::SECONDARY,
            ButtonState::Pressed,
            Some(fixture.first),
        );
        let up = router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::SECONDARY,
            ButtonState::Released,
            Some(fixture.first),
        );
        assert!(down.activation.is_none() && up.activation.is_none());
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
    }

    #[test]
    fn deactivation_clears_hover_before_forgetting_pointer_routes() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(4);
        router.pointer_moved(
            &mut fixture.ui,
            pointer,
            PointF::default(),
            Some(fixture.first),
        );
        assert!(has(&fixture.ui, fixture.first, InteractionFlags::HOVERED));
        assert!(router.view_deactivated(&mut fixture.ui));
        router.pointer_moved(&mut fixture.ui, pointer, PointF::default(), None);
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::HOVERED));
        router.pointer_moved(
            &mut fixture.ui,
            pointer,
            PointF::default(),
            Some(fixture.first),
        );
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::HOVERED));
        router.view_activated(&mut fixture.ui);
        router.pointer_moved(
            &mut fixture.ui,
            pointer,
            PointF::default(),
            Some(fixture.first),
        );
        assert!(has(&fixture.ui, fixture.first, InteractionFlags::HOVERED));
    }

    #[test]
    fn hover_is_aggregated_across_pointers_without_cross_control_state() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        for pointer in [PointerId::new(4), PointerId::new(5)] {
            router.pointer_moved(
                &mut fixture.ui,
                pointer,
                PointF::default(),
                Some(fixture.first),
            );
        }
        router.pointer_moved(
            &mut fixture.ui,
            PointerId::new(4),
            PointF::default(),
            Some(fixture.second),
        );
        assert!(has(&fixture.ui, fixture.first, InteractionFlags::HOVERED));
        assert!(has(&fixture.ui, fixture.second, InteractionFlags::HOVERED));
        router.pointer_moved(&mut fixture.ui, PointerId::new(5), PointF::default(), None);
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::HOVERED));
        assert!(has(&fixture.ui, fixture.second, InteractionFlags::HOVERED));
    }

    #[test]
    fn disabling_a_captured_control_clears_transient_state_without_activation() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(6);
        router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(fixture.first),
        );
        assert!(has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
        fixture
            .ui
            .interactions
            .get_mut(fixture.first)
            .unwrap()
            .set_enabled(false);
        assert!(router.sync(&mut fixture.ui));
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
        router.pointer_moved(&mut fixture.ui, pointer, PointF::default(), Some(fixture.first));
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::HOVERED));
        let release = router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Released,
            Some(fixture.first),
        );
        assert!(release.activation.is_none());
    }

    #[test]
    fn lost_capture_and_gesture_handoff_cancel_without_activation() {
        for cancel in [
            |router: &mut InteractionRouter, ui: &mut MountedUi, pointer| {
                router.capture_lost(ui, pointer)
            },
            |router: &mut InteractionRouter, ui: &mut MountedUi, pointer| {
                router.gesture_claimed(ui, pointer, CompetingGesture::Drag)
            },
        ] {
            let mut fixture = fixture();
            let mut router = InteractionRouter::default();
            let pointer = PointerId::new(7);
            router.pointer_button(
                &mut fixture.ui,
                pointer,
                PointerButton::PRIMARY,
                ButtonState::Pressed,
                Some(fixture.first),
            );
            assert!(has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
            assert!(cancel(&mut router, &mut fixture.ui, pointer));
            assert!(!has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
            assert!(
                router
                    .pointer_button(
                        &mut fixture.ui,
                        pointer,
                        PointerButton::PRIMARY,
                        ButtonState::Released,
                        Some(fixture.first),
                    )
                    .activation
                    .is_none()
            );
        }
    }

    #[test]
    fn disabling_keyboard_armed_button_cancels_pending_space_release() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        router.set_focus(&mut fixture.ui, Some(fixture.first), true);
        let mut space = KeyEvent::new(crate::input::PhysicalKey::UNIDENTIFIED, ButtonState::Pressed)
            .with_logical_key(LogicalKey::Named(NamedKey::Space));
        assert!(router.key(&mut fixture.ui, &space).activation.is_none());
        assert!(has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));

        fixture.ui.set_disabled(fixture.first, true);
        router.sync(&mut fixture.ui);
        assert_eq!(router.focused(), None);
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
        fixture.ui.set_disabled(fixture.first, false);
        router.sync(&mut fixture.ui);

        let pointer = PointerId::new(23);
        let mut activations = 0;
        for state in [ButtonState::Pressed, ButtonState::Released] {
            let routed = router.pointer_button(
                &mut fixture.ui, pointer, PointerButton::PRIMARY,
                state, Some(fixture.first_label),
            );
            if let Some((target, _)) = routed.activation {
                assert_eq!(target, fixture.first);
                activations += 1;
            }
        }
        assert_eq!(activations, 1, "reenabled button must accept one normal click");
        space.state = ButtonState::Released;
        assert!(router.key(&mut fixture.ui, &space).activation.is_none());
        assert_eq!(router.diagnostics().activations, 1);
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
    }

    #[test]
    fn removed_captured_control_is_rejected_by_generation() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(8);
        router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(fixture.first),
        );
        fixture.ui.remove(fixture.first);
        assert!(router.sync(&mut fixture.ui));
        assert!(
            router
                .pointer_button(
                    &mut fixture.ui,
                    pointer,
                    PointerButton::PRIMARY,
                    ButtonState::Released,
                    Some(fixture.first),
                )
                .activation
                .is_none()
        );
        assert!(router.diagnostics().stale_owners_rejected > 0);
    }
}
