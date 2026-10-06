use super::*;

impl InteractionRouter {
    pub(crate) fn is_active(&self) -> bool {
        !self.deactivated
    }

    pub(super) fn effectively_visible(&self, ui: &MountedUi, node: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(node) = current {
            let Some(core) = ui.nodes.core(node) else {
                return false;
            };
            if ui
                .interactions
                .get(node)
                .is_some_and(|state| !state.visible)
            {
                return false;
            }
            current = core.parent;
        }
        true
    }

    pub(super) fn control_available(&self, ui: &MountedUi, node: NodeId) -> bool {
        ui.interactions.get(node).is_some_and(|state| state.enabled)
            && self.effectively_visible(ui, node)
    }

    pub(super) fn is_focus_target(&self, ui: &MountedUi, node: NodeId) -> bool {
        self.control_available(ui, node)
            && ui
                .interactions
                .get(node)
                .is_some_and(|state| state.focusable && state.behavior != ControlBehavior::None)
    }

    pub(crate) fn focus_eligible(&self, ui: &MountedUi, node: NodeId) -> bool {
        self.is_active() && self.in_focus_scope(ui, node) && self.is_focus_target(ui, node)
    }

    pub fn set_always_show_focus(&mut self, ui: &mut MountedUi, always: bool) -> bool {
        if self.always_show_focus == always {
            return false;
        }
        self.always_show_focus = always;
        let Some(focused) = self.focused else {
            return false;
        };
        self.publish_flag(
            ui,
            focused,
            InteractionFlags::FOCUS_VISIBLE,
            self.focus_visible || always,
        )
    }

    pub(crate) fn set_focus(
        &mut self,
        ui: &mut MountedUi,
        target: Option<NodeId>,
        focus_visible: bool,
    ) -> FocusChange {
        let target = target.filter(|node| self.focus_eligible(ui, *node));
        self.focus_visible = target.is_some() && focus_visible;
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
            if self.controls.contains_key(&old) {
                self.handle_activation(ui, old, ActivationInput::FocusLost);
            }
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
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture, has};
    use super::*;

    #[test]
    fn hiding_an_ancestor_cancels_keyboard_arm_and_revealing_it_accepts_a_fresh_click() {
        let mut fixture = fixture();
        let parent = fixture
            .ui
            .nodes
            .core(fixture.first)
            .unwrap()
            .parent
            .unwrap();
        fixture.ui.set_focus_scope(parent, false);
        let mut router = InteractionRouter::default();
        router.set_focus(&mut fixture.ui, Some(fixture.first), true);
        let mut space = KeyEvent::new(
            crate::input::PhysicalKey::UNIDENTIFIED,
            ButtonState::Pressed,
        )
        .with_logical_key(LogicalKey::Named(NamedKey::Space));
        router.key(&mut fixture.ui, &space);
        fixture.ui.interactions.get_mut(parent).unwrap().visible = false;
        router.sync(&mut fixture.ui);
        assert_eq!(router.focused(), None);
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
        fixture.ui.interactions.get_mut(parent).unwrap().visible = true;
        let pointer = PointerId::new(51);
        router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Pressed,
            Some(fixture.first_label),
        );
        let release = router.pointer_button(
            &mut fixture.ui,
            pointer,
            PointerButton::PRIMARY,
            ButtonState::Released,
            Some(fixture.first_label),
        );
        assert_eq!(
            release.activation.map(|(node, _)| node),
            Some(fixture.first)
        );
        space.state = ButtonState::Released;
        assert!(router.key(&mut fixture.ui, &space).activation.is_none());
        assert_eq!(router.diagnostics().activations, 1);
    }

    #[test]
    fn making_a_control_passive_drops_its_focus_and_keyboard_activation() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        router.set_focus(&mut fixture.ui, Some(fixture.first), true);
        fixture
            .ui
            .set_control_behavior(fixture.first, ControlBehavior::None);
        router.sync(&mut fixture.ui);
        let enter = KeyEvent::new(
            crate::input::PhysicalKey::UNIDENTIFIED,
            ButtonState::Pressed,
        )
        .with_logical_key(LogicalKey::Named(NamedKey::Enter));
        assert!(router.key(&mut fixture.ui, &enter).activation.is_none());
        assert_eq!(router.focused(), None);
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::FOCUSED));
    }
}
