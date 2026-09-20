use super::*;

impl NativeState {
    pub(super) fn revoke_suspended_focus(&mut self, surface: WaylandSurfaceId) {
        for pending in self.suspended_focus.values_mut() {
            if pending.keyboard == Some(Some(surface)) {
                pending.keyboard = Some(None);
            }
            if let Some((target, _)) = &mut pending.pointer
                && *target == Some(surface)
            {
                *target = None;
            }
        }
    }

    pub(super) fn set_pointer_focus(
        &mut self,
        seat_id: u32,
        surface: Option<WaylandSurfaceId>,
        position: crate::foundation::PointF,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        if let Some(pending) = self.suspended_focus.get_mut(&seat_id) {
            pending.pointer = Some((surface, position));
            return Ok(());
        }
        let previous = self
            .core
            .seats
            .get(&seat_id)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?
            .pointer_focus;
        if let Some(previous) = previous {
            if let Some(surface_resource) = self
                .resource_for_kind(
                    |kind| matches!(kind, ResourceKind::Surface(candidate) if candidate == previous.surface),
                )?
                .map(|resource| resource.identity() as *mut ffi::wl_resource)
            {
                for resource in self.resources_for_client(
                    previous.client,
                    |kind| matches!(kind, ResourceKind::Pointer(candidate) if candidate == seat_id),
                )? {
                    self.post_event(
                        resource,
                        "wl_pointer",
                        "leave",
                        &mut [
                            ffi::wl_argument { u: serial },
                            ffi::wl_argument {
                                o: surface_resource,
                            },
                        ],
                    )?;
                    self.pointer_frame(resource)?;
                }
            }
            self.clear_selection_for_client(seat_id, previous.client)?;
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
                    crate::integrations::wayland::compositor::SerialKind::PointerEnter,
                    Some(surface),
                )
                .map_err(error)?;
            let surface_resource = self.surface_resource(surface)?;
            for resource in self.resources_for_client(
                client,
                |kind| matches!(kind, ResourceKind::Pointer(candidate) if candidate == seat_id),
            )? {
                self.post_event(
                    resource,
                    "wl_pointer",
                    "enter",
                    &mut [
                        ffi::wl_argument { u: serial },
                        ffi::wl_argument {
                            o: surface_resource,
                        },
                        ffi::wl_argument {
                            f: fixed(position.x),
                        },
                        ffi::wl_argument {
                            f: fixed(position.y),
                        },
                    ],
                )?;
                self.pointer_frame(resource)?;
            }
            Some(crate::integrations::wayland::compositor::PointerFocus {
                client,
                surface,
                position,
                enter_serial: serial,
            })
        } else {
            None
        };
        let seat = self.core.seats.get_mut(&seat_id).expect("seat checked");
        seat.cancel_client_pointer_grab();
        seat.pointer_focus = focus;
        // A client cursor is scoped to the focus that authorized it. Do not retain it while the
        // new focus decides which cursor to install, or after the old surface has been destroyed.
        seat.cursor = crate::integrations::wayland::compositor::CursorImage::TelorgonDefault;
        self.update_pointer_constraints(seat_id, focus.map(|focus| focus.surface))?;
        Ok(())
    }

    pub(super) fn pointer_motion(
        &mut self,
        seat_id: u32,
        time: u32,
        position: crate::foundation::PointF,
    ) -> Result<(), NativeCompositorError> {
        // Device queues can contain late events after access has been revoked.
        if self.suspended_focus.contains_key(&seat_id) {
            return Ok(());
        }

        let focus = self
            .core
            .seats
            .get(&seat_id)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?
            .pointer_focus
            .ok_or_else(|| NativeCompositorError::new("pointer has no focused surface"))?;
        for resource in self.resources_for_client(
            focus.client,
            |kind| matches!(kind, ResourceKind::Pointer(candidate) if candidate == seat_id),
        )? {
            self.post_event(
                resource,
                "wl_pointer",
                "motion",
                &mut [
                    ffi::wl_argument { u: time },
                    ffi::wl_argument {
                        f: fixed(position.x),
                    },
                    ffi::wl_argument {
                        f: fixed(position.y),
                    },
                ],
            )?;
            self.pointer_frame(resource)?;
        }
        self.core
            .seats
            .get_mut(&seat_id)
            .expect("seat checked")
            .pointer_focus
            .as_mut()
            .expect("focus checked")
            .position = position;
        Ok(())
    }

    pub(super) fn relative_pointer_motion(
        &self,
        seat_id: u32,
        time_microseconds: u64,
        delta: crate::foundation::PointF,
        unaccelerated: crate::foundation::PointF,
    ) -> Result<(), NativeCompositorError> {
        // Device queues can contain late events after access has been revoked.
        if self.suspended_focus.contains_key(&seat_id) {
            return Ok(());
        }

        let focus = self
            .core
            .seats
            .get(&seat_id)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?
            .pointer_focus
            .ok_or_else(|| NativeCompositorError::new("pointer has no focused surface"))?;
        for resource in self.resources_for_client(
            focus.client,
            |kind| matches!(kind, ResourceKind::RelativePointer(candidate) if candidate == seat_id),
        )? {
            self.post_event(
                resource,
                "zwp_relative_pointer_v1",
                "relative_motion",
                &mut [
                    ffi::wl_argument {
                        u: (time_microseconds >> 32) as u32,
                    },
                    ffi::wl_argument {
                        u: time_microseconds as u32,
                    },
                    ffi::wl_argument { f: fixed(delta.x) },
                    ffi::wl_argument { f: fixed(delta.y) },
                    ffi::wl_argument {
                        f: fixed(unaccelerated.x),
                    },
                    ffi::wl_argument {
                        f: fixed(unaccelerated.y),
                    },
                ],
            )?;
        }
        Ok(())
    }

    pub(super) fn pointer_button(
        &mut self,
        seat_id: u32,
        time: u32,
        button: u32,
        state: crate::integrations::wayland::compositor::ButtonState,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        // Device queues can contain late events after access has been revoked.
        if self.suspended_focus.contains_key(&seat_id) {
            return Ok(());
        }

        if state == crate::integrations::wayland::compositor::ButtonState::Released {
            self.pointer_press_serials.remove(&(seat_id, button));
        }
        let focus = self
            .core
            .seats
            .get_mut(&seat_id)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?
            .pointer_button_target(button, state, false);
        let Some(focus) = focus else {
            return Ok(());
        };
        self.core
            .serials
            .issue(
                serial,
                focus.client,
                crate::integrations::wayland::compositor::SerialKind::PointerButton,
                Some(focus.surface),
            )
            .map_err(error)?;
        if state == crate::integrations::wayland::compositor::ButtonState::Pressed {
            self.pointer_press_serials
                .insert((seat_id, button), (serial, focus));
        }
        let wire_state = u32::from(matches!(
            state,
            crate::integrations::wayland::compositor::ButtonState::Pressed
        ));
        for resource in self.resources_for_client(
            focus.client,
            |kind| matches!(kind, ResourceKind::Pointer(candidate) if candidate == seat_id),
        )? {
            self.post_event(
                resource,
                "wl_pointer",
                "button",
                &mut [
                    ffi::wl_argument { u: serial },
                    ffi::wl_argument { u: time },
                    ffi::wl_argument { u: button },
                    ffi::wl_argument { u: wire_state },
                ],
            )?;
            self.pointer_frame(resource)?;
        }
        Ok(())
    }

    pub(super) fn pointer_axis(
        &self,
        seat_id: u32,
        time: u32,
        horizontal: f64,
        vertical: f64,
        discrete_x: i32,
        discrete_y: i32,
    ) -> Result<(), NativeCompositorError> {
        // Device queues can contain late events after access has been revoked.
        if self.suspended_focus.contains_key(&seat_id) {
            return Ok(());
        }

        let focus = self
            .core
            .seats
            .get(&seat_id)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?
            .pointer_focus
            .ok_or_else(|| NativeCompositorError::new("pointer has no focused surface"))?;
        for resource in self.resources_for_client(
            focus.client,
            |kind| matches!(kind, ResourceKind::Pointer(candidate) if candidate == seat_id),
        )? {
            if resource.version() >= 5 {
                self.post_event(
                    resource,
                    "wl_pointer",
                    "axis_source",
                    &mut [ffi::wl_argument { u: 0 }],
                )?;
            }
            for (axis, value, discrete) in [
                (0_u32, vertical, discrete_y),
                (1_u32, horizontal, discrete_x),
            ] {
                if value == 0.0 && discrete == 0 {
                    continue;
                }
                self.post_event(
                    resource,
                    "wl_pointer",
                    "axis",
                    &mut [
                        ffi::wl_argument { u: time },
                        ffi::wl_argument { u: axis },
                        ffi::wl_argument {
                            f: fixed_f64(value),
                        },
                    ],
                )?;
                if resource.version() >= 8 {
                    self.post_event(
                        resource,
                        "wl_pointer",
                        "axis_value120",
                        &mut [
                            ffi::wl_argument { u: axis },
                            ffi::wl_argument {
                                i: discrete.saturating_mul(120),
                            },
                        ],
                    )?;
                } else if resource.version() >= 5 && discrete != 0 {
                    self.post_event(
                        resource,
                        "wl_pointer",
                        "axis_discrete",
                        &mut [
                            ffi::wl_argument { u: axis },
                            ffi::wl_argument { i: discrete },
                        ],
                    )?;
                }
            }
            self.pointer_frame(resource)?;
        }
        Ok(())
    }

    pub(super) fn pointer_frame(
        &self,
        resource: ResourceRef<'_>,
    ) -> Result<(), NativeCompositorError> {
        if resource.version() >= 5 {
            self.post_event(resource, "wl_pointer", "frame", &mut [])?;
        }
        Ok(())
    }

    pub(super) fn dispatch_seat(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        seat: u32,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        let state = self
            .core
            .seats
            .get(&seat)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?;
        let (interface, kind, enabled) = match request.message().name {
            "get_pointer" => (
                "wl_pointer",
                ResourceKind::Pointer(seat),
                state.capabilities.pointer,
            ),
            "get_keyboard" => (
                "wl_keyboard",
                ResourceKind::Keyboard(seat),
                state.capabilities.keyboard,
            ),
            "get_touch" => (
                "wl_touch",
                ResourceKind::Touch(seat),
                state.capabilities.touch,
            ),
            _ => return Err(unsupported_request(request)),
        };
        if !enabled {
            return Err(NativeCompositorError::new(
                "requested input capability is unavailable",
            ));
        }
        let input_resource = self.create_resource(
            resource.client(),
            context.client,
            interface,
            resource.version(),
            request.new_id(0).map_err(error)?,
            kind,
            true,
        )?;
        if matches!(kind, ResourceKind::Keyboard(_)) {
            self.send_keyboard_initial(input_resource, seat, context.client)?;
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_pointer(
        &mut self,
        context: &ResourceContext,
        seat: u32,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "set_cursor" {
            return Err(unsupported_request(request));
        }
        let serial = request.uint(0).map_err(error)?;
        if !self
            .core
            .seats
            .get(&seat)
            .is_some_and(|seat| seat.accepts_cursor(context.client, serial))
        {
            return Ok(DispatchOutcome::default());
        }
        let cursor = request
            .object(1)
            .map_err(error)?
            .map(|resource| self.surface_from_resource(resource))
            .transpose()?;
        let cursor = match cursor {
            Some(surface) => {
                self.surface_mut(surface)?
                    .assign_role(SurfaceRole::Cursor)
                    .map_err(error)?;
                crate::integrations::wayland::compositor::CursorImage::ClientSurface {
                    surface,
                    hotspot_x: request.int(2).map_err(error)?,
                    hotspot_y: request.int(3).map_err(error)?,
                }
            }
            None => crate::integrations::wayland::compositor::CursorImage::Hidden,
        };
        self.core
            .seats
            .get_mut(&seat)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?
            .cursor = cursor;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_cursor_shape_manager(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "get_pointer" {
            return Err(unsupported_request(request));
        }
        let pointer = request
            .object(1)
            .map_err(error)?
            .ok_or_else(|| NativeCompositorError::new("missing wl_pointer"))?;
        let ResourceKind::Pointer(seat) = self.resource_kind(pointer)? else {
            return Err(NativeCompositorError::new(
                "cursor-shape target is not a wl_pointer",
            ));
        };
        self.create_resource(
            resource.client(),
            context.client,
            "wp_cursor_shape_device_v1",
            resource.version(),
            request.new_id(0).map_err(error)?,
            ResourceKind::CursorShapeDevice(seat),
            true,
        )?;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_cursor_shape_device(
        &mut self,
        context: &ResourceContext,
        seat: u32,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "set_shape" {
            return Err(unsupported_request(request));
        }
        let serial = request.uint(0).map_err(error)?;
        let shape = request.uint(1).map_err(error)?;
        if !(1..=36).contains(&shape) {
            return Err(NativeCompositorError::new("invalid cursor shape"));
        }
        // A stale or unrelated serial is a harmless no-op, not a protocol error. The
        // current enter remains valid even after it ages out of the general serial ledger.
        if !self
            .core
            .seats
            .get(&seat)
            .is_some_and(|seat| seat.accepts_cursor(context.client, serial))
        {
            return Ok(DispatchOutcome::default());
        }
        self.core.seats.get_mut(&seat).expect("seat checked").cursor =
            crate::integrations::wayland::compositor::CursorImage::Shape(shape);
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_relative_pointer_manager(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "get_relative_pointer" {
            return Err(unsupported_request(request));
        }
        let pointer = request
            .object(1)
            .map_err(error)?
            .ok_or_else(|| NativeCompositorError::new("missing wl_pointer"))?;
        let ResourceKind::Pointer(seat) = self.resource_kind(pointer)? else {
            return Err(NativeCompositorError::new(
                "relative-pointer target is not a wl_pointer",
            ));
        };
        self.create_resource(
            resource.client(),
            context.client,
            "zwp_relative_pointer_v1",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::RelativePointer(seat),
            true,
        )?;
        Ok(DispatchOutcome::default())
    }
}
