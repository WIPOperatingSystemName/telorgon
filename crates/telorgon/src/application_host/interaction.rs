use std::collections::HashMap;

use crate::core::PointF;
use crate::input::{
    Activation, ActivationInput, ActivationStateMachine, ActivationTransition, ButtonState,
    CompetingGesture, KeyEvent, LogicalKey, NamedKey, PointerButton, PointerCaptureRequest,
    PointerId, ValueChangePhase,
};
use crate::scene::NodeId;
use crate::ui::{ControlBehavior, InteractionFlags, MountedUi};

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

#[derive(Clone, Copy, Debug, Default)]
struct PointerRoute {
    position: PointF,
    raw_hovered: Option<NodeId>,
    hovered: Option<NodeId>,
    captured: Option<NodeId>,
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

    pub fn set_always_show_focus(&mut self, ui: &mut MountedUi, always: bool) -> bool {
        if self.always_show_focus == always {
            return false;
        }
        self.always_show_focus = always;
        let Some(focused) = self.focused else {
            return false;
        };
        self.publish_flag(ui, focused, InteractionFlags::FOCUS_VISIBLE, always)
    }

    pub(crate) fn pointer_moved(
        &mut self,
        ui: &mut MountedUi,
        pointer: PointerId,
        position: PointF,
        raw_hit: Option<NodeId>,
    ) -> PointerRouting {
        self.sync(ui);
        let hit = raw_hit.and_then(|node| ui.nearest_control(node));
        let previous_hover = self.pointers.get(&pointer).and_then(|route| route.hovered);
        {
            let route = self.pointers.entry(pointer).or_default();
            route.position = position;
            route.raw_hovered = raw_hit;
            route.hovered = hit;
        }

        let mut routing = PointerRouting {
            target: self
                .pointers
                .get(&pointer)
                .and_then(|route| route.captured)
                .or(hit)
                .or(raw_hit),
            ..PointerRouting::default()
        };
        if previous_hover != hit {
            if let Some(old) = previous_hover {
                let still_hovered = self
                    .pointers
                    .iter()
                    .any(|(id, route)| *id != pointer && route.hovered == Some(old));
                routing.changed |=
                    self.publish_flag(ui, old, InteractionFlags::HOVERED, still_hovered);
            }
            if let Some(new) = hit {
                routing.changed |= self.publish_flag(ui, new, InteractionFlags::HOVERED, true);
            }
        }

        routing.changed |= self.publish_container_hover(ui);
        let Some(captured) = self.pointers.get(&pointer).and_then(|route| route.captured) else {
            return routing;
        };
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
        self.sync(ui);
        let hit = raw_hit.and_then(|node| ui.nearest_control(node));
        let captured = self.pointers.get(&pointer).and_then(|route| route.captured);
        let control = captured.or(hit);
        let mut routing = PointerRouting {
            target: control.or(raw_hit),
            ..PointerRouting::default()
        };
        let Some(target) = control else {
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
        self.sync(ui);
        let Some(target) = self.focused else {
            return KeyRouting::default();
        };
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

    pub(crate) fn view_deactivated(&mut self, ui: &mut MountedUi) -> bool {
        let controls: Vec<_> = self.controls.keys().copied().collect();
        let mut changed = false;
        for control in controls {
            changed |= self
                .handle_activation(ui, control, ActivationInput::ViewDeactivated)
                .changed;
        }
        // Clear the published state before discarding the route: subsequent motion
        // otherwise has no previous owner whose hover flag it can remove.
        let hovered: Vec<_> = self
            .pointers
            .values()
            .filter_map(|route| route.hovered)
            .collect();
        for node in hovered {
            changed |= self.publish_flag(ui, node, InteractionFlags::HOVERED, false);
        }
        for route in self.pointers.values_mut() {
            route.captured = None;
            route.hovered = None;
            route.raw_hovered = None;
        }
        changed |= self.publish_container_hover(ui);
        let focus = self.set_focus(ui, None, false);
        changed | (focus.old != focus.new)
    }

    pub(crate) fn set_focus(
        &mut self,
        ui: &mut MountedUi,
        target: Option<NodeId>,
        focus_visible: bool,
    ) -> FocusChange {
        let target = target.filter(|node| {
            ui.interactions.get(*node).is_some_and(|interaction| {
                interaction.focusable
                    && interaction.enabled
                    && interaction.visible
                    && interaction.behavior != ControlBehavior::None
            })
        });
        let old = self.focused;
        if old == target {
            if let Some(target) = target {
                self.publish_flag(
                    ui,
                    target,
                    InteractionFlags::FOCUS_VISIBLE,
                    focus_visible || self.always_show_focus,
                );
            }
            return FocusChange { old, new: target };
        }

        if let Some(old) = old {
            self.handle_activation(ui, old, ActivationInput::FocusLost);
            self.publish_flag(ui, old, InteractionFlags::FOCUSED, false);
            self.publish_flag(ui, old, InteractionFlags::FOCUS_VISIBLE, false);
        }
        self.focused = target;
        if let Some(target) = target {
            self.publish_flag(ui, target, InteractionFlags::FOCUSED, true);
            self.publish_flag(
                ui,
                target,
                InteractionFlags::FOCUS_VISIBLE,
                focus_visible || self.always_show_focus,
            );
        }
        FocusChange { old, new: target }
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

        let pointer_ids: Vec<_> = self.pointers.keys().copied().collect();
        for pointer in pointer_ids {
            let Some(route) = self.pointers.get(&pointer).copied() else {
                continue;
            };
            if route.hovered.is_some_and(|node| !ui.nodes.contains(node)) {
                if let Some(route) = self.pointers.get_mut(&pointer) {
                    route.hovered = None;
                }
                self.diagnostics.stale_owners_rejected += 1;
            }
            if let Some(captured) = route.captured {
                let eligible = ui.interactions.get(captured).is_some_and(|interaction| {
                    interaction.enabled
                        && interaction.visible
                        && matches!(
                            interaction.behavior,
                            ControlBehavior::Activate | ControlBehavior::Value
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

        if self.focused.is_some_and(|node| {
            !ui.interactions.get(node).is_some_and(|interaction| {
                interaction.enabled && interaction.visible && interaction.focusable
            })
        }) {
            let focus = self.set_focus(ui, None, false);
            changed |= focus.old != focus.new;
            self.diagnostics.stale_owners_rejected += 1;
        }
        changed | self.publish_container_hover(ui)
    }

    fn behavior(&self, ui: &MountedUi, node: NodeId) -> Option<ControlBehavior> {
        ui.interactions.get(node).and_then(|interaction| {
            (interaction.enabled
                && interaction.visible
                && interaction.behavior != ControlBehavior::None)
                .then_some(interaction.behavior)
        })
    }

    fn ensure_machine(&mut self, ui: &MountedUi, node: NodeId) -> bool {
        let enabled = ui
            .interactions
            .get(node)
            .is_some_and(|interaction| interaction.enabled && interaction.visible);
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
        let pressed = self
            .controls
            .get(&node)
            .is_some_and(ActivationStateMachine::is_visually_armed);
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
        }
        changed
    }

    fn publish_container_hover(&mut self, ui: &mut MountedUi) -> bool {
        let containers: Vec<_> = ui.nodes.alive().iter().copied()
            .filter(|node| ui.kinds.get(*node) == Some(&crate::NodeKind::Box)
                && ui.interactions.get(*node).is_some_and(|interaction| {
                    interaction.hover_within || interaction.flags.contains(InteractionFlags::HOVERED)
                }))
            .collect();
        let mut changed = false;
        for node in containers {
            let enabled = ui.interactions.get(node).is_some_and(|interaction| {
                interaction.hover_within && interaction.enabled && interaction.visible
            });
            let hovered = enabled && self.pointers.values().any(|route| {
                route.raw_hovered.is_some_and(|hit| {
                    ui.nodes.contains(hit) && ui.is_descendant_or_self(hit, node)
                })
            });
            changed |= self.publish_flag(ui, node, InteractionFlags::HOVERED, hovered);
        }
        changed
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
    use crate::core::ColorRgba8;
    use crate::ui::{BoxStyle, LayoutStyle, MountWriter};

    struct Fixture {
        ui: MountedUi,
        first: NodeId,
        first_label: NodeId,
        second: NodeId,
    }

    fn fixture() -> Fixture {
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

    fn has(ui: &MountedUi, node: NodeId, flag: InteractionFlags) -> bool {
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
