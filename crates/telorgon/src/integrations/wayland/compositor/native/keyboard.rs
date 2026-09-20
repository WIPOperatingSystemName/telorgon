use super::*;

impl NativeState {
    pub(super) fn set_keyboard_focus(
        &mut self,
        seat_id: u32,
        surface: Option<WaylandSurfaceId>,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        if let Some(pending) = self.suspended_focus.get_mut(&seat_id) {
            pending.keyboard = Some(surface);
            return Ok(());
        }
        let previous = self
            .core
            .seats
            .get(&seat_id)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?
            .keyboard_focus;
        if previous.map(|focus| focus.surface) == surface {
            return Ok(());
        }
        if let Some(previous) = previous {
            let surface_resource = self.surface_resource(previous.surface)?;
            for resource in self.resources_for_client(
                previous.client,
                |kind| matches!(kind, ResourceKind::Keyboard(candidate) if candidate == seat_id),
            )? {
                self.post_event(
                    resource,
                    "wl_keyboard",
                    "leave",
                    &mut [
                        ffi::wl_argument { u: serial },
                        ffi::wl_argument {
                            o: surface_resource,
                        },
                    ],
                )?;
            }
        }
        let focus = if let Some(surface) = surface {
            let client = self
                .core
                .world
                .surface_owner(surface)
                .ok_or_else(|| NativeCompositorError::new("unknown focus surface"))?;
            self.core
                .serials
                .issue(
                    serial,
                    client,
                    crate::integrations::wayland::compositor::SerialKind::KeyboardEnter,
                    Some(surface),
                )
                .map_err(error)?;
            let surface_resource = self.surface_resource(surface)?;
            let keys = self
                .core
                .seats
                .get(&seat_id)
                .expect("seat checked")
                .pressed_keys()
                .to_vec();
            let modifiers = self
                .core
                .seats
                .get(&seat_id)
                .expect("seat checked")
                .keyboard_modifiers();
            let mut keys = ffi::wl_array {
                size: std::mem::size_of_val(keys.as_slice()),
                alloc: std::mem::size_of_val(keys.as_slice()),
                data: keys.as_ptr().cast_mut().cast::<c_void>(),
            };
            for resource in self.resources_for_client(
                client,
                |kind| matches!(kind, ResourceKind::Keyboard(candidate) if candidate == seat_id),
            )? {
                self.post_event(
                    resource,
                    "wl_keyboard",
                    "enter",
                    &mut [
                        ffi::wl_argument { u: serial },
                        ffi::wl_argument {
                            o: surface_resource,
                        },
                        ffi::wl_argument { a: &mut keys },
                    ],
                )?;
                self.post_keyboard_modifiers(resource, serial, modifiers)?;
            }
            Some(crate::integrations::wayland::compositor::KeyboardFocus {
                client,
                surface,
                enter_serial: serial,
            })
        } else {
            None
        };
        self.core
            .seats
            .get_mut(&seat_id)
            .expect("seat checked")
            .keyboard_focus = focus;
        self.update_shortcut_inhibitors()?;
        if let Some(focus) = focus {
            self.send_selection_to_client(seat_id, focus.client)?;
        }
        Ok(())
    }

    pub(super) fn keyboard_keymap(
        &mut self,
        seat_id: u32,
        keymap: &OwnedFd,
        size: u32,
    ) -> Result<(), NativeCompositorError> {
        if size == 0 {
            return Err(NativeCompositorError::new("keyboard keymap is empty"));
        }
        self.keyboard_keymaps
            .insert(seat_id, (keymap.try_clone().map_err(error)?, size));
        for resource in self.resources_for_kind(
            |kind| matches!(kind, ResourceKind::Keyboard(candidate) if candidate == seat_id),
        )? {
            self.post_event(
                resource,
                "wl_keyboard",
                "keymap",
                &mut [
                    ffi::wl_argument { u: 1 },
                    ffi::wl_argument {
                        h: keymap.as_raw_fd(),
                    },
                    ffi::wl_argument { u: size },
                ],
            )?;
        }
        Ok(())
    }

    pub(super) fn send_keyboard_initial(
        &self,
        resource: ResourceRef<'_>,
        seat_id: u32,
        client: ClientId,
    ) -> Result<(), NativeCompositorError> {
        if let Some((keymap, size)) = self.keyboard_keymaps.get(&seat_id) {
            self.post_event(
                resource,
                "wl_keyboard",
                "keymap",
                &mut [
                    ffi::wl_argument { u: 1 },
                    ffi::wl_argument {
                        h: keymap.as_raw_fd(),
                    },
                    ffi::wl_argument { u: *size },
                ],
            )?;
        }
        if resource.version() >= 4 {
            self.post_event(
                resource,
                "wl_keyboard",
                "repeat_info",
                &mut [ffi::wl_argument { i: 25 }, ffi::wl_argument { i: 600 }],
            )?;
        }
        if let Some(focus) = self
            .core
            .seats
            .get(&seat_id)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?
            .keyboard_focus
            .filter(|focus| focus.client == client)
        {
            let surface_resource = self.surface_resource(focus.surface)?;
            let seat = self.core.seats.get(&seat_id).expect("seat checked");
            let keys = seat.pressed_keys().to_vec();
            let modifiers = seat.keyboard_modifiers();
            let mut keys = ffi::wl_array {
                size: std::mem::size_of_val(keys.as_slice()),
                alloc: std::mem::size_of_val(keys.as_slice()),
                data: keys.as_ptr().cast_mut().cast::<c_void>(),
            };
            self.post_event(
                resource,
                "wl_keyboard",
                "enter",
                &mut [
                    ffi::wl_argument {
                        u: focus.enter_serial,
                    },
                    ffi::wl_argument {
                        o: surface_resource,
                    },
                    ffi::wl_argument { a: &mut keys },
                ],
            )?;
            self.post_keyboard_modifiers(resource, focus.enter_serial, modifiers)?;
        }
        Ok(())
    }

    pub(super) fn keyboard_key(
        &mut self,
        seat_id: u32,
        time: u32,
        key: u32,
        state: crate::integrations::wayland::compositor::ButtonState,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        // Device queues can contain late events after access has been revoked.
        if self.suspended_focus.contains_key(&seat_id) {
            return Ok(());
        }

        let focus = {
            let seat = self
                .core
                .seats
                .get_mut(&seat_id)
                .ok_or_else(|| NativeCompositorError::new("unknown seat"))?;
            // Stale releases and duplicate presses are not input evidence. In
            // particular, do not mint a serial or send them to a new focus.
            if !seat.set_key(key, state) {
                return Ok(());
            }
            seat.keyboard_focus
        };
        let Some(focus) = focus else {
            return Ok(());
        };
        self.core
            .serials
            .issue(
                serial,
                focus.client,
                crate::integrations::wayland::compositor::SerialKind::KeyboardKey,
                Some(focus.surface),
            )
            .map_err(error)?;
        let wire_state = u32::from(matches!(
            state,
            crate::integrations::wayland::compositor::ButtonState::Pressed
        ));
        for resource in self.resources_for_client(
            focus.client,
            |kind| matches!(kind, ResourceKind::Keyboard(candidate) if candidate == seat_id),
        )? {
            self.post_event(
                resource,
                "wl_keyboard",
                "key",
                &mut [
                    ffi::wl_argument { u: serial },
                    ffi::wl_argument { u: time },
                    ffi::wl_argument { u: key },
                    ffi::wl_argument { u: wire_state },
                ],
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn keyboard_modifiers(
        &mut self,
        seat_id: u32,
        serial: u32,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) -> Result<(), NativeCompositorError> {
        // Device queues can contain late events after access has been revoked.
        if self.suspended_focus.contains_key(&seat_id) {
            return Ok(());
        }

        let focus = {
            let seat = self
                .core
                .seats
                .get_mut(&seat_id)
                .ok_or_else(|| NativeCompositorError::new("unknown seat"))?;
            seat.set_keyboard_modifiers(depressed, latched, locked, group);
            seat.keyboard_focus
        };
        let Some(focus) = focus else {
            return Ok(());
        };
        for resource in self.resources_for_client(
            focus.client,
            |kind| matches!(kind, ResourceKind::Keyboard(candidate) if candidate == seat_id),
        )? {
            self.post_keyboard_modifiers(resource, serial, (depressed, latched, locked, group))?;
        }
        Ok(())
    }

    pub(super) fn post_keyboard_modifiers(
        &self,
        resource: ResourceRef<'_>,
        serial: u32,
        modifiers: (u32, u32, u32, u32),
    ) -> Result<(), NativeCompositorError> {
        self.post_event(
            resource,
            "wl_keyboard",
            "modifiers",
            &mut [
                ffi::wl_argument { u: serial },
                ffi::wl_argument { u: modifiers.0 },
                ffi::wl_argument { u: modifiers.1 },
                ffi::wl_argument { u: modifiers.2 },
                ffi::wl_argument { u: modifiers.3 },
            ],
        )
    }

    pub(super) fn dispatch_xwayland_keyboard_grab(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "grab_keyboard" {
            return Err(unsupported_request(request));
        }
        if !self
            .xwayland
            .as_ref()
            .is_some_and(|access| access.allows(resource.client().identity()))
        {
            return Err(NativeCompositorError::new(
                "unauthorized Xwayland keyboard grab",
            ));
        }
        let surface = self.surface_from_resource(
            request
                .object(1)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing grab surface"))?,
        )?;
        let seat_resource = request
            .object(2)
            .map_err(error)?
            .ok_or_else(|| NativeCompositorError::new("missing grab seat"))?;
        let ResourceKind::Seat(seat) = self.resource_kind(seat_resource)? else {
            return Err(NativeCompositorError::new("grab target is not a seat"));
        };
        // Honor only an already-focused, mapped modern Xwayland surface. Never
        // steal focus to satisfy an X11 grab. Cancellation requires a fresh request.
        let active = !self.revoked_shortcuts.contains(&(seat, surface))
            && self
                .core
                .seats
                .get(&seat)
                .and_then(|seat| seat.keyboard_focus)
                .is_some_and(|focus| focus.surface == surface)
            && self.core.world.surface(surface).is_some_and(|surface| {
                let snapshot = surface.snapshot();
                snapshot.role == Some(SurfaceRole::Xwayland)
                    && snapshot.xwayland_serial.is_some()
                    && snapshot.attachment.is_some()
            });
        let object = self.peek_next_object()?;
        self.create_resource(
            resource.client(),
            context.client,
            "zwp_xwayland_keyboard_grab_v1",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::XwaylandKeyboardGrab(object),
            true,
        )?;
        self.xwayland_keyboard_grabs
            .insert(object, (seat, surface, active));
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_shortcut_inhibit(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "inhibit_shortcuts" {
            return Err(unsupported_request(request));
        }
        let surface = self.surface_from_resource(
            request
                .object(1)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing inhibitor surface"))?,
        )?;
        let seat_resource = request
            .object(2)
            .map_err(error)?
            .ok_or_else(|| NativeCompositorError::new("missing inhibitor seat"))?;
        let ResourceKind::Seat(seat) = self.resource_kind(seat_resource)? else {
            return Err(NativeCompositorError::new("inhibitor target is not a seat"));
        };
        if self
            .shortcut_inhibitors
            .values()
            .any(|(candidate, target, _)| *candidate == seat && *target == surface)
        {
            return Err(NativeCompositorError::new(
                "shortcuts already inhibited for this seat and surface",
            ));
        }
        let object = self.peek_next_object()?;
        self.create_resource(
            resource.client(),
            context.client,
            "zwp_keyboard_shortcuts_inhibitor_v1",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::ShortcutInhibitor(object),
            true,
        )?;
        self.shortcut_inhibitors
            .insert(object, (seat, surface, false));
        self.update_shortcut_inhibitors()?;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn update_shortcut_inhibitors(&mut self) -> Result<(), NativeCompositorError> {
        // Unlike native inhibitors, cancelled X11 grab objects never reactivate.
        for (seat, surface, active) in self.xwayland_keyboard_grabs.values_mut() {
            *active &= self
                .core
                .seats
                .get(seat)
                .and_then(|seat| seat.keyboard_focus)
                .is_some_and(|focus| focus.surface == *surface)
                && self
                    .core
                    .world
                    .surface(*surface)
                    .is_some_and(|surface| surface.snapshot().attachment.is_some());
        }
        let mut activated = Vec::new();
        for (object, (seat, surface, active)) in &mut self.shortcut_inhibitors {
            let eligible = !self.revoked_shortcuts.contains(&(*seat, *surface))
                && self
                    .core
                    .seats
                    .get(seat)
                    .and_then(|seat| seat.keyboard_focus)
                    .is_some_and(|focus| focus.surface == *surface)
                && self.core.world.surface(*surface).is_some_and(|surface| {
                    let snapshot = surface.snapshot();
                    snapshot.attachment.is_some() && snapshot.role != Some(SurfaceRole::SessionLock)
                });
            if eligible && !*active {
                activated.push(*object);
            }
            // Focus/unmap deactivation is deliberately silent per the protocol.
            *active = eligible;
        }
        for object in activated {
            let resource = self
                .resource_for_kind(
                    |kind| matches!(kind, ResourceKind::ShortcutInhibitor(id) if id == object),
                )?
                .ok_or_else(|| NativeCompositorError::new("shortcut inhibitor is absent"))?;
            self.post_event(
                resource,
                "zwp_keyboard_shortcuts_inhibitor_v1",
                "active",
                &mut [],
            )?;
        }
        Ok(())
    }

    pub(super) fn dispatch_idle_inhibit_manager(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "create_inhibitor" {
            return Err(unsupported_request(request));
        }
        let surface = self.surface_from_resource(
            request
                .object(1)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing wl_surface"))?,
        )?;
        let object = self.peek_next_object()?;
        self.create_resource(
            resource.client(),
            context.client,
            "zwp_idle_inhibitor_v1",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::IdleInhibitor(object),
            true,
        )?;
        self.idle_inhibitors.insert(object, surface);
        Ok(DispatchOutcome::default())
    }
}
