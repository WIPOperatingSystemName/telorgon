use super::*;

impl InteractionRouter {
    pub(super) fn visual_pointer_button(
        &mut self,
        ui: &mut MountedUi,
        pointer: PointerId,
        button: PointerButton,
        state: ButtonState,
        raw_hit: Option<NodeId>,
    ) -> bool {
        let pressed = if button == PointerButton::PRIMARY && state == ButtonState::Pressed {
            visual_nodes(ui)
                .into_iter()
                .filter(|node| visual_press_enabled(ui, *node) && visual_hit(ui, *node, raw_hit))
                .collect()
        } else {
            Vec::new()
        };
        let route = self.pointers.entry(pointer).or_default();
        route.raw_hovered = raw_hit;
        if button == PointerButton::PRIMARY {
            route.visual_pressed = pressed;
        }
        self.publish_visual_interaction(ui)
    }

    pub(super) fn sync_visual_presses(&mut self, ui: &MountedUi) {
        for route in self.pointers.values_mut() {
            // A disabled or removed owner cannot rearm when it becomes available again.
            route
                .visual_pressed
                .retain(|node| visual_press_enabled(ui, *node));
        }
    }

    pub(super) fn publish_visual_interaction(&mut self, ui: &mut MountedUi) -> bool {
        let mut changed = false;
        for node in visual_nodes(ui) {
            let interaction = *ui.interactions.get(node).unwrap();
            let enabled = visual_enabled(ui, node);
            let hovered = if interaction.behavior == ControlBehavior::Scroll
                && !interaction.visual_hover
                && !interaction.hover_within
            {
                // Scroll viewports retain their ordinary control hover until they opt into
                // visual hover. This also restores it after a self-only effect is removed.
                self.pointers
                    .values()
                    .any(|route| route.hovered == Some(node))
            } else {
                enabled
                    && (interaction.visual_hover || interaction.hover_within)
                    && self
                        .pointers
                        .values()
                        .any(|route| visual_hit(ui, node, route.raw_hovered))
            };
            let pressed = enabled
                && interaction.visual_press
                && self.pointers.values().any(|route| {
                    route.visual_pressed.contains(&node) && visual_hit(ui, node, route.raw_hovered)
                });
            changed |= self.publish_flag(ui, node, InteractionFlags::HOVERED, hovered);
            changed |= self.publish_flag(ui, node, InteractionFlags::PRESSED, pressed);
        }
        changed
    }
}

fn visual_nodes(ui: &MountedUi) -> Vec<NodeId> {
    ui.nodes
        .alive()
        .iter()
        .copied()
        .filter(|node| {
            ui.interactions.get(*node).is_some_and(|interaction| {
                interaction.behavior == ControlBehavior::Scroll
                    || (interaction.behavior == ControlBehavior::None
                        && (interaction.visual_hover
                            || interaction.visual_press
                            || interaction.hover_within
                            || interaction.flags.contains(InteractionFlags::HOVERED)
                            || interaction.flags.contains(InteractionFlags::PRESSED)))
            })
        })
        .collect()
}

fn visual_press_enabled(ui: &MountedUi, node: NodeId) -> bool {
    ui.interactions.get(node).is_some_and(|interaction| {
        matches!(
            interaction.behavior,
            ControlBehavior::None | ControlBehavior::Scroll
        ) && interaction.visual_press
    }) && visual_enabled(ui, node)
}

fn visual_enabled(ui: &MountedUi, node: NodeId) -> bool {
    let mut current = Some(node);
    while let Some(node) = current {
        let Some(core) = ui.nodes.core(node) else {
            return false;
        };
        if ui
            .interactions
            .get(node)
            .is_some_and(|interaction| !interaction.enabled || !interaction.visible)
        {
            return false;
        }
        current = core.parent;
    }
    true
}

fn visual_hit(ui: &MountedUi, node: NodeId, hit: Option<NodeId>) -> bool {
    hit.is_some_and(|hit| {
        ui.nodes.contains(hit)
            && (hit == node
                || (ui
                    .interactions
                    .get(node)
                    .is_some_and(|interaction| interaction.hover_within)
                    && ui.is_descendant_or_self(hit, node)))
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture, has};
    use super::*;
    use crate::ui::{BoxStyle, LayoutStyle, MountWriter};

    fn scroll_fixture() -> (MountedUi, NodeId, NodeId) {
        let mut ui = MountedUi::default();
        let mut scroll = None;
        let mut label = None;
        MountWriter::<()>::new(&mut ui).root(
            BoxStyle::default(),
            LayoutStyle::default(),
            |writer| {
                scroll = Some(
                    writer
                        .scroll(BoxStyle::default(), LayoutStyle::default(), |writer| {
                            label = Some(
                                writer
                                    .text("Scroll content", crate::ColorRgba8::default(), 14.0)
                                    .node,
                            );
                        })
                        .node,
                );
            },
        );
        (ui, scroll.unwrap(), label.unwrap())
    }

    #[test]
    fn scroll_visual_effects_are_opt_in_and_do_not_activate_or_capture() {
        let (mut ui, scroll, label) = scroll_fixture();
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(47);
        let ordinary = router.pointer_moved(&mut ui, pointer, PointF::default(), Some(label));
        assert_eq!(ordinary.target, Some(scroll));
        assert!(has(&ui, scroll, InteractionFlags::HOVERED));

        ui.set_visual_interaction(scroll, true, true);
        router.pointer_moved(&mut ui, pointer, PointF::default(), Some(label));
        assert!(!has(&ui, scroll, InteractionFlags::HOVERED));
        router.pointer_button(
            &mut ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(label),
        );
        assert!(!has(&ui, scroll, InteractionFlags::PRESSED));
        router.pointer_button(
            &mut ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Released,
            Some(label),
        );

        router.pointer_moved(&mut ui, pointer, PointF::default(), Some(scroll));
        assert!(has(&ui, scroll, InteractionFlags::HOVERED));
        let pressed = router.pointer_button(
            &mut ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(scroll),
        );
        assert!(has(&ui, scroll, InteractionFlags::PRESSED));
        assert!(pressed.activation.is_none());
        assert_eq!(pressed.target, Some(scroll));
        assert!(router.pointers[&pointer].captured.is_none());
        assert!(router.focused().is_none());
        let released = router.pointer_button(
            &mut ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Released,
            Some(scroll),
        );
        assert!(!has(&ui, scroll, InteractionFlags::PRESSED));
        assert!(released.activation.is_none());
        assert_eq!(
            ui.interactions.get(scroll).unwrap().behavior,
            ControlBehavior::Scroll
        );
        assert_eq!(ui.nearest_control(label), Some(scroll));
    }

    #[test]
    fn scroll_hover_within_and_effect_removal_preserve_ordinary_scroll_hover() {
        let (mut ui, scroll, label) = scroll_fixture();
        ui.set_visual_interaction(scroll, true, true);
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(48);
        router.pointer_moved(&mut ui, pointer, PointF::default(), Some(label));
        assert!(!has(&ui, scroll, InteractionFlags::HOVERED));
        ui.set_hover_within(scroll, true);
        router.pointer_button(
            &mut ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(label),
        );
        assert!(has(&ui, scroll, InteractionFlags::HOVERED));
        assert!(has(&ui, scroll, InteractionFlags::PRESSED));
        router.cancel_pointer(&mut ui, pointer);
        assert!(!has(&ui, scroll, InteractionFlags::PRESSED));

        ui.set_hover_within(scroll, false);
        router.sync(&mut ui);
        assert!(!has(&ui, scroll, InteractionFlags::HOVERED));
        ui.set_visual_interaction(scroll, false, false);
        router.sync(&mut ui);
        assert!(has(&ui, scroll, InteractionFlags::HOVERED));
        router.pointer_button(
            &mut ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(label),
        );
        assert!(!has(&ui, scroll, InteractionFlags::PRESSED));
        router.view_deactivated(&mut ui);
        assert!(!has(&ui, scroll, InteractionFlags::HOVERED));
    }

    #[test]
    fn passive_effects_do_not_register_controls_or_leak_to_siblings() {
        let mut fixture = fixture();
        fixture
            .ui
            .set_visual_interaction(fixture.first_label, true, true);
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(40);
        router.pointer_moved(
            &mut fixture.ui,
            pointer,
            PointF::default(),
            Some(fixture.first_label),
        );
        assert!(has(
            &fixture.ui,
            fixture.first_label,
            InteractionFlags::HOVERED
        ));
        assert!(!has(&fixture.ui, fixture.second, InteractionFlags::HOVERED));
        let press = router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(fixture.first_label),
        );
        assert!(has(
            &fixture.ui,
            fixture.first_label,
            InteractionFlags::PRESSED
        ));
        assert_eq!(press.target, Some(fixture.first));
        assert_eq!(router.focused(), Some(fixture.first));
        assert_eq!(router.pointers[&pointer].captured, Some(fixture.first));
        let interaction = fixture.ui.interactions.get(fixture.first_label).unwrap();
        assert!(!interaction.focusable);
        assert_eq!(interaction.behavior, ControlBehavior::None);
        let release = router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Released,
            Some(fixture.first_label),
        );
        assert!(!has(
            &fixture.ui,
            fixture.first_label,
            InteractionFlags::PRESSED
        ));
        assert_eq!(release.activation.map(|item| item.0), Some(fixture.first));
    }

    #[test]
    fn passive_container_descendant_effects_require_hover_within() {
        let mut fixture = fixture();
        let parent = fixture
            .ui
            .nodes
            .core(fixture.first)
            .unwrap()
            .parent
            .unwrap();
        fixture.ui.set_visual_interaction(parent, true, true);
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(41);
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
        assert!(!has(&fixture.ui, parent, InteractionFlags::HOVERED));
        assert!(!has(&fixture.ui, parent, InteractionFlags::PRESSED));
        router.cancel_pointer(&mut fixture.ui, pointer);
        fixture.ui.set_hover_within(parent, true);
        router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(fixture.first_label),
        );
        assert!(has(&fixture.ui, parent, InteractionFlags::HOVERED));
        assert!(has(&fixture.ui, parent, InteractionFlags::PRESSED));
        assert_eq!(router.pointers[&pointer].captured, Some(fixture.first));
        router.view_deactivated(&mut fixture.ui);
        assert!(!has(&fixture.ui, parent, InteractionFlags::HOVERED));
        assert!(!has(&fixture.ui, parent, InteractionFlags::PRESSED));
    }

    #[test]
    fn passive_press_cancellation_and_disabled_ancestors_cannot_rearm() {
        for cancel in [
            |router: &mut InteractionRouter, ui: &mut MountedUi, pointer| {
                router.cancel_pointer(ui, pointer)
            },
            |router: &mut InteractionRouter, ui: &mut MountedUi, pointer| {
                router.capture_lost(ui, pointer)
            },
            |router: &mut InteractionRouter, ui: &mut MountedUi, pointer| {
                router.gesture_claimed(ui, pointer, CompetingGesture::Drag)
            },
            |router: &mut InteractionRouter, ui: &mut MountedUi, _| router.view_deactivated(ui),
        ] {
            let mut fixture = fixture();
            fixture
                .ui
                .set_visual_interaction(fixture.first_label, true, true);
            let mut router = InteractionRouter::default();
            let pointer = PointerId::new(42);
            router.pointer_button(
                &mut fixture.ui,
                pointer,
                PointerButton::PRIMARY,
                ButtonState::Pressed,
                Some(fixture.first_label),
            );
            assert!(has(
                &fixture.ui,
                fixture.first_label,
                InteractionFlags::PRESSED
            ));
            assert!(cancel(&mut router, &mut fixture.ui, pointer));
            router.pointer_moved(
                &mut fixture.ui,
                pointer,
                PointF::default(),
                Some(fixture.first_label),
            );
            assert!(!has(
                &fixture.ui,
                fixture.first_label,
                InteractionFlags::PRESSED
            ));
        }

        let mut fixture = fixture();
        fixture
            .ui
            .set_visual_interaction(fixture.first_label, true, true);
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(43);
        router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(fixture.first_label),
        );
        fixture.ui.set_disabled(fixture.first, true);
        router.sync(&mut fixture.ui);
        assert!(!has(
            &fixture.ui,
            fixture.first_label,
            InteractionFlags::HOVERED
        ));
        assert!(!has(
            &fixture.ui,
            fixture.first_label,
            InteractionFlags::PRESSED
        ));
        fixture.ui.set_disabled(fixture.first, false);
        router.sync(&mut fixture.ui);
        assert!(has(
            &fixture.ui,
            fixture.first_label,
            InteractionFlags::HOVERED
        ));
        assert!(!has(
            &fixture.ui,
            fixture.first_label,
            InteractionFlags::PRESSED
        ));
    }

    #[test]
    fn passive_only_nodes_have_no_activation_capture_or_keyboard_focus() {
        let mut fixture = fixture();
        let parent = fixture
            .ui
            .nodes
            .core(fixture.first)
            .unwrap()
            .parent
            .unwrap();
        fixture.ui.set_visual_interaction(parent, true, true);
        let mut router = InteractionRouter::default();
        let pointer = PointerId::new(44);
        let press = router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(parent),
        );
        assert!(has(&fixture.ui, parent, InteractionFlags::PRESSED));
        assert!(press.activation.is_none());
        assert!(router.pointers[&pointer].captured.is_none());
        assert!(router.focused().is_none());
        router.pointer_moved(&mut fixture.ui, pointer, PointF::default(), None);
        assert!(!has(&fixture.ui, parent, InteractionFlags::PRESSED));
        router.pointer_moved(&mut fixture.ui, pointer, PointF::default(), Some(parent));
        assert!(has(&fixture.ui, parent, InteractionFlags::PRESSED));
        router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Released,
            None,
        );
        assert!(!has(&fixture.ui, parent, InteractionFlags::PRESSED));
    }

    #[test]
    fn passive_pointer_owners_cancel_independently_and_ignore_removed_nodes() {
        let mut fixture = fixture();
        fixture
            .ui
            .set_visual_interaction(fixture.first_label, true, true);
        let mut router = InteractionRouter::default();
        for pointer in [PointerId::new(45), PointerId::new(46)] {
            router.pointer_button(
                &mut fixture.ui,
                pointer,
                PointerButton::PRIMARY,
                ButtonState::Pressed,
                Some(fixture.first_label),
            );
        }
        router.cancel_pointer(&mut fixture.ui, PointerId::new(45));
        assert!(has(
            &fixture.ui,
            fixture.first_label,
            InteractionFlags::PRESSED
        ));
        router.capture_lost(&mut fixture.ui, PointerId::new(46));
        assert!(!has(
            &fixture.ui,
            fixture.first_label,
            InteractionFlags::PRESSED
        ));

        router.pointer_button(
            &mut fixture.ui,
            PointerId::new(45),
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(fixture.first_label),
        );
        fixture.ui.remove(fixture.first_label);
        router.sync(&mut fixture.ui);
        assert!(
            router.pointers[&PointerId::new(45)]
                .visual_pressed
                .is_empty()
        );
        let released = router.pointer_button(
            &mut fixture.ui,
            PointerId::new(45),
            PointerButton::PRIMARY,
            ButtonState::Released,
            Some(fixture.first_label),
        );
        assert!(released.activation.is_none());
    }
}
