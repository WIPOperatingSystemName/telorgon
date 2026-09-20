use super::*;

impl NativeState {
    pub(super) fn dispatch_xwayland_shell(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if !self
            .xwayland
            .as_ref()
            .is_some_and(|access| access.allows(resource.client().identity()))
        {
            return Err(NativeCompositorError::new(
                "unauthorized Xwayland shell request",
            ));
        }
        if request.message().name != "get_xwayland_surface" {
            return Err(unsupported_request(request));
        }
        let surface = self.surface_from_resource(
            request
                .object(1)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing Xwayland wl_surface"))?,
        )?;
        if self.surface_mut(surface)?.assign_xwayland_role().is_err() {
            resource.post_error(0, "wl_surface already has a role");
            return Ok(DispatchOutcome::default());
        }
        self.create_resource(
            resource.client(),
            context.client,
            "xwayland_surface_v1",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::XwaylandSurface(surface),
            true,
        )?;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_xwayland_surface(
        &mut self,
        resource: ResourceRef<'_>,
        surface: WaylandSurfaceId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        let access = self
            .xwayland
            .as_ref()
            .filter(|access| access.allows(resource.client().identity()))
            .cloned()
            .ok_or_else(|| NativeCompositorError::new("unauthorized Xwayland surface request"))?;
        if request.message().name != "set_serial" {
            return Err(unsupported_request(request));
        }
        let serial = u64::from(request.uint(0).map_err(error)?)
            | (u64::from(request.uint(1).map_err(error)?) << 32);
        if serial == 0 || serial <= access.last_serial.get() {
            resource.post_error(1, "Xwayland serial must be nonzero and strictly increasing");
            return Ok(DispatchOutcome::default());
        }
        if let Err(error) = self.surface_mut(surface)?.set_xwayland_serial(serial) {
            resource.post_error(0, &error.to_string());
            return Ok(DispatchOutcome::default());
        }
        access.last_serial.set(serial);
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_fractional_scale_manager(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "get_fractional_scale" {
            return Err(unsupported_request(request));
        }
        let _surface = self.surface_from_resource(
            request
                .object(1)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing wl_surface"))?,
        )?;
        let scale = self.create_resource(
            resource.client(),
            context.client,
            "wp_fractional_scale_v1",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::FractionalScale,
            true,
        )?;
        let preferred = self
            .core
            .outputs
            .values()
            .find(|output| output.enabled)
            .map_or(120, |output| {
                (output.description.scale.get() * 120.0).round() as u32
            });
        self.post_event(
            scale,
            "wp_fractional_scale_v1",
            "preferred_scale",
            &mut [ffi::wl_argument { u: preferred }],
        )?;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_viewporter(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "get_viewport" {
            return Err(unsupported_request(request));
        }
        let surface = self.surface_from_resource(
            request
                .object(1)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing wl_surface"))?,
        )?;
        if self.viewports.contains_key(&surface) {
            return Err(NativeCompositorError::new(
                "surface already has a viewport object",
            ));
        }
        self.create_resource(
            resource.client(),
            context.client,
            "wp_viewport",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::Viewport(surface),
            true,
        )?;
        self.viewports.insert(surface, NativeViewport::default());
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_viewport(
        &mut self,
        surface: WaylandSurfaceId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        let viewport = self
            .viewports
            .get_mut(&surface)
            .ok_or_else(|| NativeCompositorError::new("unknown wp_viewport"))?;
        match request.message().name {
            "set_source" => {
                let values = [
                    request.fixed(0).map_err(error)?,
                    request.fixed(1).map_err(error)?,
                    request.fixed(2).map_err(error)?,
                    request.fixed(3).map_err(error)?,
                ];
                viewport.pending_source = if values == [-256; 4] {
                    Some(None)
                } else {
                    let [x, y, width, height] = values.map(|value| f64::from(value) / 256.0);
                    if x < 0.0 || y < 0.0 || width <= 0.0 || height <= 0.0 {
                        return Err(NativeCompositorError::new(
                            "viewport source rectangle is invalid",
                        ));
                    }
                    Some(Some(ViewportSource {
                        x,
                        y,
                        width,
                        height,
                    }))
                };
            }
            "set_destination" => {
                let width = request.int(0).map_err(error)?;
                let height = request.int(1).map_err(error)?;
                viewport.pending_destination = if width == -1 && height == -1 {
                    Some(None)
                } else if width > 0 && height > 0 && width <= 32_768 && height <= 32_768 {
                    Some(Some(crate::foundation::SizeI { width, height }))
                } else {
                    return Err(NativeCompositorError::new(
                        "viewport destination size is invalid",
                    ));
                };
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_presentation(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "feedback" {
            return Err(unsupported_request(request));
        }
        let surface = self.surface_from_resource(
            request
                .object(0)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing wl_surface"))?,
        )?;
        let object = self.peek_next_object()?;
        self.create_resource(
            resource.client(),
            context.client,
            "wp_presentation_feedback",
            resource.version(),
            request.new_id(1).map_err(error)?,
            ResourceKind::PresentationFeedback(surface),
            true,
        )?;
        self.pending_presentation_feedbacks
            .entry(surface)
            .or_default()
            .push(object);
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_activation(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "get_activation_token" => {
                let object = self.peek_next_object()?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "xdg_activation_token_v1",
                    1,
                    request.new_id(0).map_err(error)?,
                    ResourceKind::ActivationToken(object),
                    true,
                )?;
                self.activation_tokens
                    .insert(object, NativeActivationToken::default());
            }
            "activate" => {
                let token = c_string(request, 0)?;
                let surface =
                    self.surface_from_resource(request.object(1).map_err(error)?.ok_or_else(
                        || NativeCompositorError::new("missing activation surface"),
                    )?)?;
                let Some(grant) = self.activation_grants.remove(&token) else {
                    return Ok(DispatchOutcome::default());
                };
                self.activation_order
                    .retain(|candidate| candidate != &token);
                if grant.authorized
                    && self.core.world.surface(surface).is_some_and(|surface| {
                        surface.snapshot().role == Some(SurfaceRole::XdgToplevel)
                    })
                {
                    self.core.queue_action(CompositorAction::ActivateSurface {
                        surface,
                        application_id: grant.application_id,
                        source_surface: grant.source_surface,
                    });
                }
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_activation_token(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        object: ProtocolObjectId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if self
            .activation_tokens
            .get(&object)
            .is_some_and(|token| token.committed)
        {
            return Err(NativeCompositorError::new(
                "activation token was already committed",
            ));
        }
        match request.message().name {
            "set_serial" => {
                let serial = request.uint(0).map_err(error)?;
                let seat_resource = request
                    .object(1)
                    .map_err(error)?
                    .ok_or_else(|| NativeCompositorError::new("missing activation seat"))?;
                let ResourceKind::Seat(seat) = self.resource_kind(seat_resource)? else {
                    return Err(NativeCompositorError::new(
                        "activation serial object is not a wl_seat",
                    ));
                };
                self.activation_tokens
                    .get_mut(&object)
                    .ok_or_else(|| NativeCompositorError::new("unknown activation token"))?
                    .serial = Some((seat, serial));
            }
            "set_app_id" => {
                let application_id = c_string(request, 0)?;
                if application_id.len() > 4_096 {
                    return Err(NativeCompositorError::new(
                        "activation application id exceeds 4096 bytes",
                    ));
                }
                self.activation_tokens
                    .get_mut(&object)
                    .ok_or_else(|| NativeCompositorError::new("unknown activation token"))?
                    .application_id = Some(application_id);
            }
            "set_surface" => {
                let surface =
                    self.surface_from_resource(request.object(0).map_err(error)?.ok_or_else(
                        || NativeCompositorError::new("missing requesting surface"),
                    )?)?;
                self.activation_tokens
                    .get_mut(&object)
                    .ok_or_else(|| NativeCompositorError::new("unknown activation token"))?
                    .surface = Some(surface);
            }
            "commit" => {
                let (serial, application_id, source_surface) = {
                    let token = self
                        .activation_tokens
                        .get_mut(&object)
                        .ok_or_else(|| NativeCompositorError::new("unknown activation token"))?;
                    token.committed = true;
                    (token.serial, token.application_id.clone(), token.surface)
                };
                let authorized = serial.is_some_and(|(seat, serial)| {
                    self.core.seats.contains_key(&seat)
                        && self
                            .core
                            .serials
                            .consume(
                                context.client,
                                serial,
                                &[
                                    crate::integrations::wayland::compositor::SerialKind::PointerEnter,
                                    crate::integrations::wayland::compositor::SerialKind::PointerButton,
                                    crate::integrations::wayland::compositor::SerialKind::KeyboardEnter,
                                    crate::integrations::wayland::compositor::SerialKind::KeyboardKey,
                                    crate::integrations::wayland::compositor::SerialKind::TouchDown,
                                ],
                                source_surface,
                            )
                            .is_ok()
                });
                let handle = loop {
                    let candidate = activation_token_handle()?;
                    if !self.activation_grants.contains_key(&candidate) {
                        break candidate;
                    }
                };
                const MAX_ACTIVATION_GRANTS: usize = 1_024;
                while self.activation_order.len() >= MAX_ACTIVATION_GRANTS {
                    if let Some(expired) = self.activation_order.pop_front() {
                        self.activation_grants.remove(&expired);
                    }
                }
                self.activation_order.push_back(handle.clone());
                self.activation_grants.insert(
                    handle.clone(),
                    NativeActivationGrant {
                        authorized,
                        application_id,
                        source_surface,
                    },
                );
                let handle = protocol_string(&handle);
                self.post_event(
                    resource,
                    "xdg_activation_token_v1",
                    "done",
                    &mut [ffi::wl_argument { s: handle.as_ptr() }],
                )?;
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_pointer_constraints(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        let kind = match request.message().name {
            "lock_pointer" => PointerConstraintKind::Locked,
            "confine_pointer" => PointerConstraintKind::Confined,
            _ => return Err(unsupported_request(request)),
        };
        let surface = self.surface_from_resource(
            request
                .object(1)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing constraint surface"))?,
        )?;
        let pointer = request
            .object(2)
            .map_err(error)?
            .ok_or_else(|| NativeCompositorError::new("missing wl_pointer"))?;
        let ResourceKind::Pointer(seat) = self.resource_kind(pointer)? else {
            return Err(NativeCompositorError::new(
                "constraint target is not a wl_pointer",
            ));
        };
        if self.pointer_constraints.values().any(|constraint| {
            constraint.seat == seat && constraint.surface == surface && !constraint.finished
        }) {
            return Err(NativeCompositorError::new(
                "pointer already has a constraint for this surface",
            ));
        }
        let region = request
            .object(3)
            .map_err(error)?
            .map(|resource| self.region_from_resource(resource))
            .transpose()?;
        let persistent = match request.uint(4).map_err(error)? {
            1 => false,
            2 => true,
            _ => return Err(NativeCompositorError::new("invalid constraint lifetime")),
        };
        let object = self.peek_next_object()?;
        let (interface, resource_kind) = match kind {
            PointerConstraintKind::Locked => {
                ("zwp_locked_pointer_v1", ResourceKind::LockedPointer(object))
            }
            PointerConstraintKind::Confined => (
                "zwp_confined_pointer_v1",
                ResourceKind::ConfinedPointer(object),
            ),
        };
        self.create_resource(
            resource.client(),
            context.client,
            interface,
            1,
            request.new_id(0).map_err(error)?,
            resource_kind,
            true,
        )?;
        self.pointer_constraints.insert(
            object,
            NativePointerConstraint {
                seat,
                surface,
                kind,
                region,
                cursor_hint: None,
                persistent,
                active: false,
                finished: false,
            },
        );
        let focus = self
            .core
            .seats
            .get(&seat)
            .and_then(|seat| seat.pointer_focus)
            .map(|focus| focus.surface);
        self.update_pointer_constraints(seat, focus)?;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_locked_pointer(
        &mut self,
        object: ProtocolObjectId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        let region = if request.message().name == "set_region" {
            Some(
                request
                    .object(0)
                    .map_err(error)?
                    .map(|resource| self.region_from_resource(resource))
                    .transpose()?,
            )
        } else {
            None
        };
        let constraint = self
            .pointer_constraints
            .get_mut(&object)
            .ok_or_else(|| NativeCompositorError::new("unknown locked pointer"))?;
        match request.message().name {
            "set_region" => constraint.region = region.expect("set above"),
            "set_cursor_position_hint" => {
                constraint.cursor_hint = Some(crate::foundation::PointF {
                    x: request.fixed(0).map_err(error)? as f32 / 256.0,
                    y: request.fixed(1).map_err(error)? as f32 / 256.0,
                });
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_confined_pointer(
        &mut self,
        object: ProtocolObjectId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "set_region" {
            return Err(unsupported_request(request));
        }
        let region = request
            .object(0)
            .map_err(error)?
            .map(|resource| self.region_from_resource(resource))
            .transpose()?;
        self.pointer_constraints
            .get_mut(&object)
            .ok_or_else(|| NativeCompositorError::new("unknown confined pointer"))?
            .region = region;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn update_pointer_constraints(
        &mut self,
        seat: u32,
        focus: Option<WaylandSurfaceId>,
    ) -> Result<(), NativeCompositorError> {
        if self
            .pointer_capture_releases
            .get(&seat)
            .is_some_and(|surface| focus != Some(*surface))
        {
            self.pointer_capture_releases.remove(&seat);
        }
        let released = self.pointer_capture_releases.contains_key(&seat);
        let mut transitions = Vec::new();
        for (object, constraint) in &mut self.pointer_constraints {
            if constraint.seat != seat || constraint.finished {
                continue;
            }
            let activate = !released
                && focus == Some(constraint.surface)
                && constraint
                    .region
                    .as_ref()
                    .is_none_or(|region| !region.rectangles().is_empty());
            if activate != constraint.active {
                constraint.active = activate;
                if !activate && !constraint.persistent {
                    constraint.finished = true;
                }
                transitions.push((*object, constraint.kind, activate));
            }
        }
        for (object, kind, active) in transitions {
            let resource = self
                .resource_for_kind(|candidate| match (candidate, kind) {
                    (ResourceKind::LockedPointer(candidate), PointerConstraintKind::Locked)
                    | (
                        ResourceKind::ConfinedPointer(candidate),
                        PointerConstraintKind::Confined,
                    ) => candidate == object,
                    _ => false,
                })?
                .ok_or_else(|| NativeCompositorError::new("pointer constraint is absent"))?;
            let (interface, event) = match (kind, active) {
                (PointerConstraintKind::Locked, true) => ("zwp_locked_pointer_v1", "locked"),
                (PointerConstraintKind::Locked, false) => ("zwp_locked_pointer_v1", "unlocked"),
                (PointerConstraintKind::Confined, true) => ("zwp_confined_pointer_v1", "confined"),
                (PointerConstraintKind::Confined, false) => {
                    ("zwp_confined_pointer_v1", "unconfined")
                }
            };
            self.post_event(resource, interface, event, &mut [])?;
        }
        Ok(())
    }
}
