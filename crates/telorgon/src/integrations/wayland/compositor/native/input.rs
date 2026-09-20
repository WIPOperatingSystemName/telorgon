use super::*;

impl NativeCompositor<'_> {
    pub fn set_pointer_focus(
        &mut self,
        seat: u32,
        surface: Option<WaylandSurfaceId>,
        position: crate::foundation::PointF,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        self.state
            .set_pointer_focus(seat, surface, position, serial)
    }

    pub fn pointer_motion(
        &mut self,
        seat: u32,
        time_milliseconds: u32,
        position: crate::foundation::PointF,
    ) -> Result<(), NativeCompositorError> {
        self.state.pointer_motion(seat, time_milliseconds, position)
    }

    pub fn relative_pointer_motion(
        &self,
        seat: u32,
        time_microseconds: u64,
        delta: crate::foundation::PointF,
        unaccelerated: crate::foundation::PointF,
    ) -> Result<(), NativeCompositorError> {
        self.state
            .relative_pointer_motion(seat, time_microseconds, delta, unaccelerated)
    }

    pub fn pointer_button(
        &mut self,
        seat: u32,
        time_milliseconds: u32,
        button: u32,
        state: crate::integrations::wayland::compositor::ButtonState,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        self.state
            .pointer_button(seat, time_milliseconds, button, state, serial)
    }

    pub fn pointer_axis(
        &self,
        seat: u32,
        time_milliseconds: u32,
        horizontal: f64,
        vertical: f64,
        discrete_x: i32,
        discrete_y: i32,
    ) -> Result<(), NativeCompositorError> {
        self.state.pointer_axis(
            seat,
            time_milliseconds,
            horizontal,
            vertical,
            discrete_x,
            discrete_y,
        )
    }

    pub fn drag_active(&self, seat: u32) -> bool {
        self.state
            .active_drag
            .as_ref()
            .is_some_and(|drag| drag.seat == seat)
    }

    pub fn drag_icon(&self, seat: u32) -> Option<WaylandSurfaceId> {
        self.state
            .active_drag
            .as_ref()
            .filter(|drag| drag.seat == seat)
            .and_then(|drag| drag.icon)
    }

    pub fn drag_touch_slot(&self, seat: u32) -> Option<i32> {
        self.state
            .active_drag
            .as_ref()
            .filter(|drag| drag.seat == seat)
            .and_then(|drag| match drag.grab {
                NativeDragGrab::Pointer => None,
                NativeDragGrab::Touch(slot) => Some(slot),
            })
    }

    pub fn drag_motion(
        &mut self,
        seat: u32,
        target: Option<WaylandSurfaceId>,
        time_milliseconds: u32,
        position: crate::foundation::PointF,
    ) -> Result<(), NativeCompositorError> {
        self.state
            .drag_motion(seat, target, time_milliseconds, position)
    }

    pub fn drop_drag(&mut self, seat: u32) -> Result<(), NativeCompositorError> {
        self.state.drop_drag(seat)
    }

    pub fn cancel_drag(&mut self, seat: u32) -> Result<(), NativeCompositorError> {
        self.state.cancel_drag(seat)
    }

    pub fn set_keyboard_focus(
        &mut self,
        seat: u32,
        surface: Option<WaylandSurfaceId>,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        self.state.set_keyboard_focus(seat, surface, serial)
    }

    pub fn keyboard_keymap(
        &mut self,
        seat: u32,
        keymap: &OwnedFd,
        size: u32,
    ) -> Result<(), NativeCompositorError> {
        self.state.keyboard_keymap(seat, keymap, size)
    }

    pub fn keyboard_key(
        &mut self,
        seat: u32,
        time_milliseconds: u32,
        key: u32,
        state: crate::integrations::wayland::compositor::ButtonState,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        self.state
            .keyboard_key(seat, time_milliseconds, key, state, serial)
    }

    pub fn keyboard_modifiers(
        &mut self,
        seat: u32,
        serial: u32,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) -> Result<(), NativeCompositorError> {
        self.state
            .keyboard_modifiers(seat, serial, depressed, latched, locked, group)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn touch_down(
        &mut self,
        seat: u32,
        surface: WaylandSurfaceId,
        time_milliseconds: u32,
        touch_id: i32,
        position: crate::foundation::PointF,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        self.state
            .touch_down(seat, surface, time_milliseconds, touch_id, position, serial)
    }

    pub fn touch_motion(
        &self,
        seat: u32,
        time_milliseconds: u32,
        touch_id: i32,
        position: crate::foundation::PointF,
    ) -> Result<(), NativeCompositorError> {
        self.state
            .touch_motion(seat, time_milliseconds, touch_id, position)
    }

    pub fn touch_up(
        &mut self,
        seat: u32,
        time_milliseconds: u32,
        touch_id: i32,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        self.state
            .touch_up(seat, time_milliseconds, touch_id, serial)
    }

    pub fn touch_cancel(&mut self, seat: u32) -> Result<(), NativeCompositorError> {
        self.state.touch_cancel(seat)
    }

    pub fn shortcuts_inhibited(&self, seat: u32) -> bool {
        self.state
            .shortcut_inhibitors
            .values()
            .any(|(candidate, _, active)| *candidate == seat && *active)
            || self
                .state
                .xwayland_keyboard_grabs
                .values()
                .any(|(candidate, _, active)| *candidate == seat && *active)
    }

    /// User escape revokes this surface's grant. Recreating its protocol object
    /// cannot override the decision; revocation lasts for the surface lifetime.
    pub fn release_shortcut_inhibition(&mut self, seat: u32) -> Result<(), NativeCompositorError> {
        let focus = self
            .state
            .core
            .seats
            .get(&seat)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?
            .keyboard_focus
            .map(|focus| focus.surface);
        if let Some(surface) = focus {
            self.state.revoked_shortcuts.insert((seat, surface));
            for (candidate, target, active) in self.state.xwayland_keyboard_grabs.values_mut() {
                if *candidate == seat && *target == surface {
                    *active = false;
                }
            }
            let objects = self
                .state
                .shortcut_inhibitors
                .iter()
                .filter_map(|(object, (candidate, target, active))| {
                    (*candidate == seat && *target == surface && *active).then_some(*object)
                })
                .collect::<Vec<_>>();
            for object in objects {
                self.state.shortcut_inhibitors.get_mut(&object).unwrap().2 = false;
                let resource = self
                    .state
                    .resource_for_kind(
                        |kind| matches!(kind, ResourceKind::ShortcutInhibitor(id) if id == object),
                    )?
                    .ok_or_else(|| NativeCompositorError::new("shortcut inhibitor is absent"))?;
                self.state.post_event(
                    resource,
                    "zwp_keyboard_shortcuts_inhibitor_v1",
                    "inactive",
                    &mut [],
                )?;
            }
        }
        Ok(())
    }

    pub fn idle_inhibited(&self) -> bool {
        !self.state.idle_inhibitors.is_empty()
    }

    /// Revoke keyboard delivery at a lock boundary. Normal focus changes retain
    /// pressed keys; this explicit boundary must not expose them in a later enter.
    /// The host must suppress the corresponding physical keys until release.
    pub fn cancel_keyboard_input(&mut self, seat: u32) -> Result<(), NativeCompositorError> {
        let serial = unsafe { ffi::wl_display_next_serial(self.state.display.as_ptr()) };
        self.state.set_keyboard_focus(seat, None, serial)?;
        self.state
            .core
            .seats
            .get_mut(&seat)
            .expect("seat checked")
            .cancel_keyboard_keys();
        Ok(())
    }

    /// Clear physical input and protocol capture after the owner suspends devices.
    /// Keyboard leave resets client-side pressed state without inventing key events.
    pub fn suspend_seat_input(&mut self, seat: u32) -> Result<(), NativeCompositorError> {
        if !self.state.core.seats.contains_key(&seat) {
            return Err(NativeCompositorError::new("unknown seat"));
        }
        if self.state.suspended_focus.contains_key(&seat) {
            return Ok(());
        }
        if self
            .state
            .active_drag
            .as_ref()
            .is_some_and(|drag| drag.seat == seat)
        {
            self.state.cancel_drag(seat)?;
        }
        self.state
            .pointer_press_serials
            .retain(|(candidate, _), _| *candidate != seat);
        self.state.touch_cancel(seat)?;
        self.state
            .core
            .seats
            .get_mut(&seat)
            .expect("seat checked")
            .reset_input();
        let serial = unsafe { ffi::wl_display_next_serial(self.state.display.as_ptr()) };
        self.state.set_keyboard_focus(seat, None, serial)?;
        self.state
            .set_pointer_focus(seat, None, crate::foundation::PointF::default(), serial)?;
        self.state
            .suspended_focus
            .insert(seat, SuspendedFocus::default());
        Ok(())
    }

    /// Reopen focus delivery after devices resume. Returns whether policy requested
    /// keyboard focus while suspended, including an explicit request to clear it.
    pub fn resume_seat_input(&mut self, seat: u32) -> Result<bool, NativeCompositorError> {
        if !self.state.core.seats.contains_key(&seat) {
            return Err(NativeCompositorError::new("unknown seat"));
        }
        let Some(pending) = self.state.suspended_focus.remove(&seat) else {
            return Ok(false);
        };
        let serial = unsafe { ffi::wl_display_next_serial(self.state.display.as_ptr()) };
        if let Some(surface) = pending.keyboard {
            let surface =
                surface.filter(|surface| self.state.core.world.surface(*surface).is_some());
            self.state.set_keyboard_focus(seat, surface, serial)?;
        }
        if let Some((surface, position)) = pending.pointer {
            let serial = unsafe { ffi::wl_display_next_serial(self.state.display.as_ptr()) };
            let surface =
                surface.filter(|surface| self.state.core.world.surface(*surface).is_some());
            self.state
                .set_pointer_focus(seat, surface, position, serial)?;
        }
        Ok(pending.keyboard.is_some())
    }

    /// Revoke this seat's pointer constraints until pointer focus leaves the
    /// current surface. Persistent objects may activate again on a later enter;
    /// one-shot objects finish through the normal deactivation path. New objects
    /// on the released surface cannot immediately recapture the pointer.
    pub fn release_pointer_capture(&mut self, seat: u32) -> Result<(), NativeCompositorError> {
        let focus = self
            .state
            .core
            .seats
            .get(&seat)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?
            .pointer_focus
            .map(|focus| focus.surface);
        if let Some(surface) = focus {
            self.state.pointer_capture_releases.insert(seat, surface);
        }
        self.state.update_pointer_constraints(seat, focus)
    }

    pub fn pointer_constraint(&self, seat: u32) -> Option<PointerConstraintState> {
        self.state
            .pointer_constraints
            .values()
            .find(|constraint| constraint.seat == seat && constraint.active)
            .map(|constraint| PointerConstraintState {
                kind: constraint.kind,
                surface: constraint.surface,
                region: constraint.region.clone(),
            })
    }
}
