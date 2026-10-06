use super::*;

impl InteractionRouter {
    pub(crate) fn view_deactivated(&mut self, ui: &mut MountedUi) -> bool {
        if self.deactivated {
            return false;
        }
        self.suspended_focus = self.focused.map(|node| (node, self.focus_visible));
        self.deactivated = true;
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
            route.visual_pressed.clear();
        }
        changed |= self.publish_visual_interaction(ui);
        let focus = self.set_focus(ui, None, false);
        changed | (focus.old != focus.new)
    }

    pub(crate) fn view_activated(&mut self, ui: &mut MountedUi) -> FocusChange {
        let old = self.focused;
        if !self.deactivated {
            return FocusChange { old, new: old };
        }
        self.deactivated = false;
        // Pending scopes and explicit autofocus take priority over the previous owner.
        // Keep the suspended owner available to seed a newly mounted scope's return point.
        self.sync_focus_requests(ui);
        let suspended = self.suspended_focus.take();
        if self.focused.is_none()
            && let Some((target, focus_visible)) = suspended
        {
            self.set_focus(ui, Some(target), focus_visible);
        }
        FocusChange {
            old,
            new: self.focused,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture, has};
    use super::*;

    #[test]
    fn repeated_lifecycle_notifications_preserve_owner_and_keyboard_modality() {
        for focus_visible in [false, true] {
            let mut fixture = fixture();
            let mut router = InteractionRouter::default();
            router.set_focus(&mut fixture.ui, Some(fixture.first), focus_visible);
            assert!(router.view_deactivated(&mut fixture.ui));
            assert!(!router.view_deactivated(&mut fixture.ui));
            assert_eq!(router.suspended_focus, Some((fixture.first, focus_visible)));
            assert!(!has(&fixture.ui, fixture.first, InteractionFlags::FOCUSED));
            let change = router.view_activated(&mut fixture.ui);
            assert_eq!(
                change,
                FocusChange {
                    old: None,
                    new: Some(fixture.first)
                }
            );
            assert!(has(&fixture.ui, fixture.first, InteractionFlags::FOCUSED));
            assert_eq!(
                has(&fixture.ui, fixture.first, InteractionFlags::FOCUS_VISIBLE),
                focus_visible
            );
            assert_eq!(
                router.view_activated(&mut fixture.ui),
                FocusChange {
                    old: Some(fixture.first),
                    new: Some(fixture.first),
                }
            );
            assert!(router.suspended_focus.is_none());
        }
    }

    #[test]
    fn accessibility_focus_setting_does_not_replace_saved_keyboard_modality() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        router.set_always_show_focus(&mut fixture.ui, true);
        router.set_focus(&mut fixture.ui, Some(fixture.first), false);
        assert!(has(
            &fixture.ui,
            fixture.first,
            InteractionFlags::FOCUS_VISIBLE
        ));
        router.view_deactivated(&mut fixture.ui);
        router.set_always_show_focus(&mut fixture.ui, false);
        router.view_activated(&mut fixture.ui);
        assert!(has(&fixture.ui, fixture.first, InteractionFlags::FOCUSED));
        assert!(!has(
            &fixture.ui,
            fixture.first,
            InteractionFlags::FOCUS_VISIBLE
        ));
    }

    #[test]
    fn deactivation_cancels_pointer_and_keyboard_arms_without_restoring_transients() {
        for keyboard in [false, true] {
            let mut fixture = fixture();
            let mut router = InteractionRouter::default();
            let pointer = PointerId::new(31);
            router.pointer_moved(
                &mut fixture.ui,
                pointer,
                PointF::default(),
                Some(fixture.first),
            );
            if keyboard {
                router.set_focus(&mut fixture.ui, Some(fixture.first), true);
                router.key(
                    &mut fixture.ui,
                    &KeyEvent::new(
                        crate::input::PhysicalKey::UNIDENTIFIED,
                        ButtonState::Pressed,
                    )
                    .with_logical_key(LogicalKey::Named(NamedKey::Space)),
                );
            } else {
                router.pointer_button(
                    &mut fixture.ui,
                    pointer,
                    PointerButton::PRIMARY,
                    ButtonState::Pressed,
                    Some(fixture.first),
                );
            }
            assert!(has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
            router.view_deactivated(&mut fixture.ui);
            assert!(!has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
            assert!(!has(&fixture.ui, fixture.first, InteractionFlags::HOVERED));
            assert!(router.pointers[&pointer].captured.is_none());
            router.view_activated(&mut fixture.ui);
            let activation = if keyboard {
                router
                    .key(
                        &mut fixture.ui,
                        &KeyEvent::new(
                            crate::input::PhysicalKey::UNIDENTIFIED,
                            ButtonState::Released,
                        )
                        .with_logical_key(LogicalKey::Named(NamedKey::Space)),
                    )
                    .activation
            } else {
                router
                    .pointer_button(
                        &mut fixture.ui,
                        pointer,
                        PointerButton::PRIMARY,
                        ButtonState::Released,
                        Some(fixture.first),
                    )
                    .activation
            };
            assert!(activation.is_none());
            assert!(!has(&fixture.ui, fixture.first, InteractionFlags::PRESSED));
        }
    }

    #[test]
    fn queued_autofocus_has_priority_over_the_suspended_owner() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        router.set_focus(&mut fixture.ui, Some(fixture.first), false);
        router.view_deactivated(&mut fixture.ui);
        fixture.ui.request_focus(fixture.second);
        router.sync_focus_requests(&mut fixture.ui);
        assert!(
            fixture
                .ui
                .interactions
                .get(fixture.second)
                .unwrap()
                .focus_requested
        );
        assert_eq!(
            router.view_activated(&mut fixture.ui).new,
            Some(fixture.second)
        );
        assert!(
            !fixture
                .ui
                .interactions
                .get(fixture.second)
                .unwrap()
                .focus_requested
        );
        assert!(!has(&fixture.ui, fixture.first, InteractionFlags::FOCUSED));
    }

    #[test]
    fn modal_opened_while_inactive_receives_focus_and_retains_its_return_owner() {
        let mut fixture = fixture();
        let mut router = InteractionRouter::default();
        router.set_focus(&mut fixture.ui, Some(fixture.first), true);
        router.view_deactivated(&mut fixture.ui);
        fixture.ui.set_focus_scope(fixture.second, true);
        assert_eq!(
            router.view_activated(&mut fixture.ui).new,
            Some(fixture.second)
        );
        assert_eq!(
            router.scope_restore.last(),
            Some(&(fixture.second, Some(fixture.first)))
        );
        fixture.ui.set_focus_scope(fixture.second, false);
        assert_eq!(
            router.sync_focus_requests(&mut fixture.ui).new,
            Some(fixture.first)
        );
    }

    #[test]
    fn removed_or_disabled_suspended_owners_are_not_restored() {
        for remove in [false, true] {
            let mut fixture = fixture();
            let mut router = InteractionRouter::default();
            router.set_focus(&mut fixture.ui, Some(fixture.first), true);
            router.view_deactivated(&mut fixture.ui);
            if remove {
                fixture.ui.remove(fixture.first);
            } else {
                fixture.ui.set_disabled(fixture.first, true);
            }
            assert_eq!(router.view_activated(&mut fixture.ui).new, None);
            assert!(router.suspended_focus.is_none());
        }
    }
}
