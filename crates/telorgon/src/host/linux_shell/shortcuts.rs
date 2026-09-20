use std::collections::BTreeSet;

use crate::host::application::{ShellKeyAction, ShellKeyEvent};

/// Remember ownership until release, even if modifiers or focus change meanwhile.
#[derive(Default)]
pub(super) struct ShortcutKeys {
    pressed: BTreeSet<u32>,
    consumed: BTreeSet<u32>,
}

impl ShortcutKeys {
    /// A security boundary revokes delivery, but not physical press ownership.
    pub(super) fn suppress_held(&mut self) {
        self.consumed.extend(self.pressed.iter().copied());
    }

    /// Return physical presses for XKB release and clear ownership after seat loss.
    pub(super) fn reset(&mut self) -> BTreeSet<u32> {
        self.consumed.clear();
        std::mem::take(&mut self.pressed)
    }

    pub(super) fn route(
        &mut self,
        event: ShellKeyEvent,
        pressed: bool,
        locked: bool,
        handler: Option<&mut (dyn FnMut(ShellKeyEvent) -> ShellKeyAction + 'static)>,
    ) -> ShellKeyAction {
        if !pressed {
            self.pressed.remove(&event.keycode);
            return if self.consumed.remove(&event.keycode) {
                ShellKeyAction::Consume
            } else {
                ShellKeyAction::Forward
            };
        }
        if !self.pressed.insert(event.keycode) {
            return if self.consumed.contains(&event.keycode) {
                ShellKeyAction::Consume
            } else {
                ShellKeyAction::Forward
            };
        }
        // Reserve physical Escape regardless of the configured XKB layout or
        // user shortcut handler. A lock never turns this into focus/activation.
        let action = if event.keycode == 1 && event.control && event.alt && event.shift {
            ShellKeyAction::ReleaseCapture
        } else if locked {
            ShellKeyAction::Forward
        } else {
            handler.map_or(ShellKeyAction::Forward, |handler| handler(event))
        };
        if action != ShellKeyAction::Forward {
            self.consumed.insert(event.keycode);
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::application::{KeyBindings, KeyChord, ShortcutKey};
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn lock_boundary_suppresses_held_keys_until_release_without_retriggering_handlers() {
        let mut keys = ShortcutKeys::default();
        let event = ShellKeyEvent {
            keycode: 20,
            ..Default::default()
        };
        assert_eq!(
            keys.route(event, true, false, None),
            ShellKeyAction::Forward
        );
        keys.suppress_held();
        assert_eq!(
            keys.route(event, true, true, None),
            ShellKeyAction::Consume
        );
        keys.suppress_held(); // Unlock before the physical release.
        let mut forbidden = |_| panic!("held key must not retrigger a shortcut after unlock");
        assert_eq!(
            keys.route(event, true, false, Some(&mut forbidden)),
            ShellKeyAction::Consume
        );
        assert_eq!(
            keys.route(event, false, false, Some(&mut forbidden)),
            ShellKeyAction::Consume
        );
        assert_eq!(
            keys.route(event, true, false, None),
            ShellKeyAction::Forward
        );
    }

    #[test]
    fn seat_reset_releases_physical_keys_and_allows_fresh_shortcuts() {
        let mut keys = ShortcutKeys::default();
        let event = ShellKeyEvent {
            keycode: 20,
            ..Default::default()
        };
        let mut handler = |_| ShellKeyAction::Consume;
        assert_eq!(
            keys.route(event, true, false, Some(&mut handler)),
            ShellKeyAction::Consume
        );
        assert_eq!(keys.reset(), BTreeSet::from([20]));
        assert!(keys.reset().is_empty());
        assert_eq!(
            keys.route(event, true, false, Some(&mut handler)),
            ShellKeyAction::Consume
        );
    }

    #[test]
    fn emergency_chord_preempts_handlers_and_keeps_release_owned_after_lock() {
        let mut keys = ShortcutKeys::default();
        let mut handler = |_| panic!("reserved chord must not reach configurable shortcuts");
        let event = ShellKeyEvent {
            keycode: 1,
            keysym: 0,
            control: true,
            alt: true,
            shift: true,
            ..Default::default()
        };
        for locked in [false, true] {
            assert_eq!(
                keys.route(event, true, locked, Some(&mut handler)),
                ShellKeyAction::ReleaseCapture
            );
            assert_eq!(
                keys.route(event, true, locked, Some(&mut handler)),
                ShellKeyAction::Consume
            );
            assert_eq!(
                keys.route(
                    ShellKeyEvent {
                        control: false,
                        ..event
                    },
                    false,
                    !locked,
                    Some(&mut handler)
                ),
                ShellKeyAction::Consume
            );
        }
    }

    #[test]
    fn named_bindings_invoke_once_and_preserve_capture_and_lock_isolation() {
        thread_local! {
            static CALLS: Cell<usize> = const { Cell::new(0) };
        }
        fn launcher() {
            CALLS.with(|calls| calls.set(calls.get() + 1));
        }
        fn terminal() {
            CALLS.with(|calls| calls.set(calls.get() + 10));
        }
        CALLS.with(|calls| calls.set(0));
        let bindings = KeyBindings::new()
            .bind(KeyChord::new(ShortcutKey::Space).super_key(), launcher)
            .bind(KeyChord::new(ShortcutKey::Enter).super_key(), terminal);
        let mut handler = move |event| bindings.handle(event);
        let mut keys = ShortcutKeys::default();
        let event = ShellKeyEvent {
            keycode: 57,
            keysym: 0x20,
            logo: true,
            ..Default::default()
        };
        assert_eq!(
            keys.route(event, true, false, Some(&mut handler)),
            ShellKeyAction::Consume
        );
        assert_eq!(
            keys.route(event, true, false, Some(&mut handler)),
            ShellKeyAction::Consume
        );
        // A modifier change and session lock must not leak the captured release.
        assert_eq!(
            keys.route(
                ShellKeyEvent {
                    logo: false,
                    ..event
                },
                false,
                true,
                Some(&mut handler)
            ),
            ShellKeyAction::Consume
        );
        assert_eq!(
            keys.route(event, true, true, Some(&mut handler)),
            ShellKeyAction::Forward
        );
        assert_eq!(
            keys.route(event, false, true, Some(&mut handler)),
            ShellKeyAction::Forward
        );
        CALLS.with(|calls| assert_eq!(calls.get(), 1));
        assert_eq!(
            keys.route(
                ShellKeyEvent {
                    keycode: 28,
                    keysym: 0xff0d,
                    ..event
                },
                true,
                false,
                Some(&mut handler)
            ),
            ShellKeyAction::Consume
        );
        CALLS.with(|calls| assert_eq!(calls.get(), 11));
    }

    #[test]
    fn consumed_press_repeat_and_release_stay_out_of_clients() {
        let count = Rc::new(Cell::new(0));
        let calls = count.clone();
        let mut handler = move |_| {
            calls.set(calls.get() + 1);
            ShellKeyAction::Consume
        };
        let mut keys = ShortcutKeys::default();
        let mut event = ShellKeyEvent {
            keycode: 20,
            control: true,
            ..Default::default()
        };
        assert_eq!(
            keys.route(event, true, false, Some(&mut handler)),
            ShellKeyAction::Consume
        );
        event.control = false;
        assert_eq!(
            keys.route(event, true, false, Some(&mut handler)),
            ShellKeyAction::Consume
        );
        assert_eq!(
            keys.route(event, false, true, Some(&mut handler)),
            ShellKeyAction::Consume
        );
        assert_eq!(count.get(), 1);
        assert_eq!(
            keys.route(event, true, false, Some(&mut handler)),
            ShellKeyAction::Consume
        );
        assert_eq!(count.get(), 2);
    }

    #[test]
    fn ordinary_keys_and_locked_sessions_forward_both_edges() {
        let event = ShellKeyEvent {
            keycode: 16,
            ..Default::default()
        };
        let mut keys = ShortcutKeys::default();
        let mut forbidden = |_| panic!("locked session must not invoke shortcuts");
        assert_eq!(
            keys.route(event, true, true, Some(&mut forbidden)),
            ShellKeyAction::Forward
        );
        assert_eq!(
            keys.route(event, false, true, Some(&mut forbidden)),
            ShellKeyAction::Forward
        );
        assert_eq!(
            keys.route(event, true, false, None),
            ShellKeyAction::Forward
        );
        let mut consume = |_| ShellKeyAction::Consume;
        assert_eq!(
            keys.route(event, true, false, Some(&mut consume)),
            ShellKeyAction::Forward
        );
        assert_eq!(
            keys.route(event, false, false, Some(&mut consume)),
            ShellKeyAction::Forward
        );
    }

    #[test]
    fn quit_is_returned_to_the_owner_loop() {
        let mut keys = ShortcutKeys::default();
        let mut handler = |_| ShellKeyAction::Quit;
        assert_eq!(
            keys.route(ShellKeyEvent::default(), true, false, Some(&mut handler)),
            ShellKeyAction::Quit
        );
    }
}
