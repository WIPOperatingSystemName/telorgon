use crate::core::PointF;
use std::collections::BTreeMap;

use crate::compositor_wayland::{ClientId, WaylandSurfaceId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeatCapabilities {
    pub pointer: bool,
    pub keyboard: bool,
    pub touch: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerFocus {
    pub client: ClientId,
    pub surface: WaylandSurfaceId,
    pub position: PointF,
    pub enter_serial: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyboardFocus {
    pub client: ClientId,
    pub surface: WaylandSurfaceId,
    pub enter_serial: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Released,
    Pressed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorImage {
    Hidden,
    ClientSurface {
        surface: WaylandSurfaceId,
        hotspot_x: i32,
        hotspot_y: i32,
    },
    Shape(u32),
    TelorgonDefault,
}

#[derive(Clone, Copy, Debug)]
enum PointerButtonOwner {
    Client(PointerFocus),
    Compositor,
    // Keep the physical press until release, even when its recipient disappears.
    Suppressed,
}

#[derive(Debug)]
pub struct SeatState {
    pub name: String,
    pub capabilities: SeatCapabilities,
    pub pointer_focus: Option<PointerFocus>,
    pub keyboard_focus: Option<KeyboardFocus>,
    pub cursor: CursorImage,
    pressed_keys: Vec<u32>,
    pressed_buttons: Vec<u32>,
    button_owners: BTreeMap<u32, PointerButtonOwner>,
    keyboard_modifiers: (u32, u32, u32, u32),
}

impl SeatState {
    pub fn new(name: impl Into<String>, capabilities: SeatCapabilities) -> Self {
        Self {
            name: name.into(),
            capabilities,
            pointer_focus: None,
            keyboard_focus: None,
            cursor: CursorImage::TelorgonDefault,
            pressed_keys: Vec::new(),
            pressed_buttons: Vec::new(),
            button_owners: BTreeMap::new(),
            keyboard_modifiers: (0, 0, 0, 0),
        }
    }

    /// Physical state is invalid after input devices have been suspended.
    /// Protocol owners must send leave/cancel notifications around this reset.
    pub(crate) fn reset_input(&mut self) {
        self.pressed_keys.clear();
        self.pressed_buttons.clear();
        self.button_owners.clear();
        self.keyboard_modifiers = (0, 0, 0, 0);
        self.cursor = CursorImage::TelorgonDefault;
    }

    /// Security-boundary reset; the host retains physical ownership until release.
    pub(crate) fn cancel_keyboard_keys(&mut self) {
        self.pressed_keys.clear();
        self.keyboard_modifiers = (0, 0, 0, 0);
    }

    pub fn set_key(&mut self, key: u32, state: ButtonState) -> bool {
        update_pressed(&mut self.pressed_keys, key, state)
    }

    pub fn set_button(&mut self, button: u32, state: ButtonState) -> bool {
        update_pressed(&mut self.pressed_buttons, button, state)
    }

    pub fn pressed_keys(&self) -> &[u32] {
        &self.pressed_keys
    }

    pub fn pressed_buttons(&self) -> &[u32] {
        &self.pressed_buttons
    }

    pub(crate) fn accepts_cursor(&self, client: ClientId, serial: u32) -> bool {
        self.pointer_focus
            .is_some_and(|focus| focus.client == client && focus.enter_serial == serial)
    }

    /// The implicit grab belongs to the first client press until all its buttons are up.
    pub(crate) fn pointer_grab_focus(&self) -> Option<PointerFocus> {
        self.button_owners.values().find_map(|owner| match owner {
            PointerButtonOwner::Client(focus)
                if self.accepts_cursor(focus.client, focus.enter_serial)
                    && self
                        .pointer_focus
                        .is_some_and(|current| current.surface == focus.surface) =>
            {
                Some(*focus)
            }
            _ => None,
        })
    }

    pub(crate) fn compositor_owns_button(&self, button: u32) -> bool {
        matches!(
            self.button_owners.get(&button),
            Some(PointerButtonOwner::Compositor)
        )
    }

    /// Update physical state and return a client only for an owned, matched event.
    pub(crate) fn pointer_button_target(
        &mut self,
        button: u32,
        state: ButtonState,
        compositor_owned: bool,
    ) -> Option<PointerFocus> {
        let owner = match state {
            ButtonState::Pressed => {
                if self.pressed_buttons.contains(&button) {
                    return None;
                }
                let owner = if compositor_owned {
                    PointerButtonOwner::Compositor
                } else if let Some(focus) = self.pointer_grab_focus() {
                    PointerButtonOwner::Client(focus)
                } else if !self.pressed_buttons.is_empty() {
                    PointerButtonOwner::Suppressed
                } else {
                    self.pointer_focus
                        .map_or(PointerButtonOwner::Suppressed, PointerButtonOwner::Client)
                };
                self.set_button(button, state);
                self.button_owners.insert(button, owner);
                owner
            }
            ButtonState::Released => {
                self.set_button(button, state);
                self.button_owners.remove(&button)?
            }
        };
        match owner {
            PointerButtonOwner::Client(focus)
                if self.accepts_cursor(focus.client, focus.enter_serial)
                    && self
                        .pointer_focus
                        .is_some_and(|current| current.surface == focus.surface) =>
            {
                Some(focus)
            }
            _ => None,
        }
    }

    pub(crate) fn cancel_client_pointer_grab(&mut self) {
        for owner in self.button_owners.values_mut() {
            if matches!(owner, PointerButtonOwner::Client(_)) {
                *owner = PointerButtonOwner::Suppressed;
            }
        }
    }

    pub(crate) fn cancel_pointer_buttons(&mut self) {
        self.button_owners
            .values_mut()
            .for_each(|owner| *owner = PointerButtonOwner::Suppressed);
    }

    pub fn keyboard_modifiers(&self) -> (u32, u32, u32, u32) {
        self.keyboard_modifiers
    }

    pub fn set_keyboard_modifiers(
        &mut self,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) {
        self.keyboard_modifiers = (depressed, latched, locked, group);
    }

    pub fn remove_client(&mut self, client: ClientId) {
        if self
            .pointer_focus
            .is_some_and(|focus| focus.client == client)
        {
            self.cancel_client_pointer_grab();
            self.pointer_focus = None;
            self.cursor = CursorImage::TelorgonDefault;
        }
        if self
            .keyboard_focus
            .is_some_and(|focus| focus.client == client)
        {
            self.keyboard_focus = None;
        }
    }

    pub fn remove_surface(&mut self, surface: WaylandSurfaceId) {
        if self
            .pointer_focus
            .is_some_and(|focus| focus.surface == surface)
        {
            self.cancel_client_pointer_grab();
            self.pointer_focus = None;
            self.cursor = CursorImage::TelorgonDefault;
        }
        if self
            .keyboard_focus
            .is_some_and(|focus| focus.surface == surface)
        {
            self.keyboard_focus = None;
        }
        if matches!(
            self.cursor,
            CursorImage::ClientSurface {
                surface: cursor_surface,
                ..
            } if cursor_surface == surface
        ) {
            self.cursor = CursorImage::TelorgonDefault;
        }
    }
}

fn update_pressed(values: &mut Vec<u32>, value: u32, state: ButtonState) -> bool {
    match (
        values.iter().position(|candidate| *candidate == value),
        state,
    ) {
        (None, ButtonState::Pressed) => {
            values.push(value);
            true
        }
        (Some(index), ButtonState::Released) => {
            values.swap_remove(index);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focused_seat() -> (SeatState, PointerFocus) {
        let focus = PointerFocus {
            client: ClientId::from_raw(1).unwrap(),
            surface: WaylandSurfaceId::from_raw(7).unwrap(),
            position: PointF::default(),
            enter_serial: 166,
        };
        let mut seat = SeatState::new("seat0", SeatCapabilities::default());
        seat.pointer_focus = Some(focus);
        (seat, focus)
    }

    #[test]
    fn cursor_authorization_uses_current_enter_not_button_or_historical_serials() {
        let (mut seat, focus) = focused_seat();
        let mut ledger = crate::compositor_wayland::SerialLedger::new(1).unwrap();
        ledger
            .issue(
                166,
                focus.client,
                crate::compositor_wayland::SerialKind::PointerEnter,
                Some(focus.surface),
            )
            .unwrap();
        ledger
            .issue(
                167,
                focus.client,
                crate::compositor_wayland::SerialKind::PointerButton,
                Some(focus.surface),
            )
            .unwrap();
        assert!(seat.accepts_cursor(focus.client, 166));
        assert!(!seat.accepts_cursor(focus.client, 167));
        assert!(!seat.accepts_cursor(ClientId::from_raw(2).unwrap(), 166));
        seat.pointer_focus = Some(PointerFocus {
            enter_serial: 168,
            ..focus
        });
        assert!(!seat.accepts_cursor(focus.client, 166));
        assert!(seat.accepts_cursor(focus.client, 168));
        seat.pointer_focus = None;
        assert!(!seat.accepts_cursor(focus.client, 168));
    }

    #[test]
    fn decoration_press_and_release_never_reach_a_reentered_client() {
        let (mut seat, focus) = focused_seat();
        seat.pointer_focus = None;
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Pressed, true),
            None
        );
        seat.pointer_focus = Some(focus);
        assert!(seat.compositor_owns_button(272));
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Released, false),
            None
        );
        assert!(!seat.compositor_owns_button(272));
        assert!(seat.pressed_buttons().is_empty());
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Pressed, false),
            Some(focus)
        );
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Released, false),
            Some(focus)
        );
    }

    #[test]
    fn client_grab_lasts_through_multiple_buttons_and_rejects_duplicate_edges() {
        let (mut seat, focus) = focused_seat();
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Released, false),
            None
        );
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Pressed, false),
            Some(focus)
        );
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Pressed, false),
            None
        );
        assert_eq!(
            seat.pointer_button_target(273, ButtonState::Pressed, false),
            Some(focus)
        );
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Released, false),
            Some(focus)
        );
        assert_eq!(seat.pointer_grab_focus(), Some(focus));
        assert_eq!(
            seat.pointer_button_target(273, ButtonState::Released, false),
            Some(focus)
        );
        assert_eq!(seat.pointer_grab_focus(), None);
        assert!(seat.pressed_buttons().is_empty());
    }

    #[test]
    fn destroyed_surface_or_client_does_not_transfer_held_buttons() {
        for remove_client in [false, true] {
            let (mut seat, focus) = focused_seat();
            seat.pointer_button_target(272, ButtonState::Pressed, false);
            if remove_client {
                seat.remove_client(focus.client);
            } else {
                seat.remove_surface(focus.surface);
            }
            assert_eq!(seat.pointer_grab_focus(), None);
            let next = PointerFocus {
                client: ClientId::from_raw(2).unwrap(),
                surface: WaylandSurfaceId::from_raw(8).unwrap(),
                enter_serial: 170,
                ..focus
            };
            seat.pointer_focus = Some(next);
            assert_eq!(
                seat.pointer_button_target(272, ButtonState::Released, false),
                None
            );
            assert!(seat.pressed_buttons().is_empty());
            assert_eq!(
                seat.pointer_button_target(272, ButtonState::Pressed, false),
                Some(next)
            );
        }
    }

    #[test]
    fn explicit_grab_and_session_lock_suppress_releases_until_buttons_are_up() {
        let (mut seat, focus) = focused_seat();
        seat.pointer_button_target(272, ButtonState::Pressed, false);
        seat.cancel_client_pointer_grab();
        assert_eq!(seat.pointer_grab_focus(), None);
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Released, false),
            None
        );
        seat.pointer_button_target(272, ButtonState::Pressed, true);
        seat.cancel_pointer_buttons();
        seat.pointer_focus = Some(PointerFocus {
            enter_serial: 200,
            ..focus
        });
        assert!(!seat.compositor_owns_button(272));
        assert_eq!(
            seat.pointer_button_target(273, ButtonState::Pressed, false),
            None
        );
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Released, false),
            None
        );
        assert_eq!(
            seat.pointer_button_target(273, ButtonState::Released, false),
            None
        );
        assert!(seat.pressed_buttons().is_empty());
    }

    #[test]
    fn press_without_focus_and_focus_replacement_do_not_leak_releases() {
        let (mut seat, focus) = focused_seat();
        seat.pointer_focus = None;
        seat.pointer_button_target(272, ButtonState::Pressed, false);
        seat.pointer_focus = Some(focus);
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Released, false),
            None
        );
        seat.pointer_button_target(272, ButtonState::Pressed, false);
        seat.pointer_focus = Some(PointerFocus {
            enter_serial: 200,
            ..focus
        });
        assert_eq!(
            seat.pointer_button_target(272, ButtonState::Released, false),
            None
        );
        assert!(seat.pressed_buttons().is_empty());
    }

    #[test]
    fn repeated_button_state_does_not_duplicate_pressed_state() {
        let mut seat = SeatState::new("seat0", SeatCapabilities::default());
        assert!(seat.set_button(0x110, ButtonState::Pressed));
        assert!(!seat.set_button(0x110, ButtonState::Pressed));
        assert_eq!(seat.pressed_buttons(), &[0x110]);
        assert!(seat.set_button(0x110, ButtonState::Released));
    }

    #[test]
    fn destroying_a_focused_surface_clears_focus_and_its_cursor() {
        let client = ClientId::from_raw(1).unwrap();
        let surface = WaylandSurfaceId::from_raw(7).unwrap();
        let mut seat = SeatState::new("seat0", SeatCapabilities::default());
        seat.pointer_focus = Some(PointerFocus {
            client,
            surface,
            position: PointF::default(),
            enter_serial: 1,
        });
        seat.keyboard_focus = Some(KeyboardFocus {
            client,
            surface,
            enter_serial: 2,
        });
        seat.cursor = CursorImage::ClientSurface {
            surface,
            hotspot_x: 0,
            hotspot_y: 0,
        };

        seat.remove_surface(surface);

        assert!(seat.pointer_focus.is_none());
        assert!(seat.keyboard_focus.is_none());
        assert_eq!(seat.cursor, CursorImage::TelorgonDefault);
    }

    #[test]
    fn keyboard_modifiers_are_retained_for_the_next_focus_enter() {
        let mut seat = SeatState::new("seat0", SeatCapabilities::default());
        seat.set_keyboard_modifiers(1, 2, 4, 3);

        assert_eq!(seat.keyboard_modifiers(), (1, 2, 4, 3));
    }
}
