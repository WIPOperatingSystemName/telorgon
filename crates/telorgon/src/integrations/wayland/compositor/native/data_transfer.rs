use super::*;

impl NativeState {
    pub(super) fn dispatch_data_device_manager(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "create_data_source" => {
                let object = self.peek_next_object()?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "wl_data_source",
                    resource.version(),
                    request.new_id(0).map_err(error)?,
                    ResourceKind::DataSource(object),
                    true,
                )?;
                self.core
                    .data_devices
                    .create_source(crate::integrations::wayland::compositor::DataSource {
                        owner: context.client,
                        object,
                        mime_types: Vec::new(),
                        actions: crate::integrations::wayland::compositor::DataAction::NONE,
                        actions_set: false,
                        used: false,
                    })
                    .map_err(error)?;
            }
            "get_data_device" => {
                let seat_resource = request
                    .object(1)
                    .map_err(error)?
                    .ok_or_else(|| NativeCompositorError::new("missing wl_seat"))?;
                let ResourceKind::Seat(seat) = self.resource_kind(seat_resource)? else {
                    return Err(NativeCompositorError::new(
                        "data-device target is not a wl_seat",
                    ));
                };
                let data_device = self.create_resource(
                    resource.client(),
                    context.client,
                    "wl_data_device",
                    resource.version(),
                    request.new_id(0).map_err(error)?,
                    ResourceKind::DataDevice(seat),
                    true,
                )?;
                let focused = self
                    .core
                    .seats
                    .get(&seat)
                    .and_then(|seat| seat.keyboard_focus)
                    .is_some_and(|focus| focus.client == context.client);
                if focused {
                    self.send_selection_to_device(data_device, context.client)?;
                }
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_data_source(
        &mut self,
        source: ProtocolObjectId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        let source_object = source;
        let source = self
            .core
            .data_devices
            .source_mut(source)
            .ok_or_else(|| NativeCompositorError::new("unknown wl_data_source"))?;
        match request.message().name {
            "offer" => source
                .offer(
                    crate::integrations::wayland::compositor::MimeType::new(c_string(request, 0)?)
                        .map_err(error)?,
                )
                .map_err(error)?,
            "set_actions" => {
                let actions = crate::integrations::wayland::compositor::DataAction::from_protocol(
                    request.uint(0).map_err(error)?,
                )
                .ok_or_else(|| NativeCompositorError::new("invalid data-source actions"))?;
                if source.set_actions(actions).is_err() {
                    self.data_source_resource(source_object)?
                        .post_error(1, "data-source actions are already set or source is used");
                    return Ok(DispatchOutcome::default());
                }
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_data_device(
        &mut self,
        context: &ResourceContext,
        seat: u32,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "set_selection" => {
                let Some(focus) = self.core.seats.get(&seat).and_then(|seat| seat.keyboard_focus) else { return Ok(DispatchOutcome::default()); };
                if focus.client != context.client { return Ok(DispatchOutcome::default()); }
                let serial = request.uint(1).map_err(error)?;
                if self.core.serials.validate(context.client, serial, &[
                    crate::integrations::wayland::compositor::SerialKind::PointerButton,
                    crate::integrations::wayland::compositor::SerialKind::KeyboardKey,
                ], None).is_err() { return Ok(DispatchOutcome::default()); }
                let source = request
                    .object(0)
                    .map_err(error)?
                    .map(|resource| self.data_source_from_resource(resource))
                    .transpose()?;
                let previous = self.core.data_devices.selection();
                if previous == source { return Ok(DispatchOutcome::default()); }
                self.core
                    .data_devices
                    .set_selection(context.client, source)
                    .map_err(error)?;
                if let Some(previous) = previous {
                    self.cancel_data_source(previous)?;
                }
                self.send_selection_to_client(seat, context.client)?;
            }
            "start_drag" => {
                // A serial issued before device revocation cannot restart capture.
                if self.suspended_focus.contains_key(&seat) {
                    return Ok(DispatchOutcome::default());
                }
                let source = request
                    .object(0)
                    .map_err(error)?
                    .map(|resource| self.data_source_from_resource(resource))
                    .transpose()?;
                let origin = self.surface_from_resource(
                    request
                        .object(1)
                        .map_err(error)?
                        .ok_or_else(|| NativeCompositorError::new("missing drag origin"))?,
                )?;
                if self.core.world.surface_owner(origin) != Some(context.client) {
                    return Err(NativeCompositorError::new(
                        "drag origin belongs to another client",
                    ));
                }
                let serial = request.uint(3).map_err(error)?;
                let grab_serial = self
                    .core
                    .serials
                    .validate(
                        context.client,
                        serial,
                        &[
                            crate::integrations::wayland::compositor::SerialKind::PointerButton,
                            crate::integrations::wayland::compositor::SerialKind::TouchDown,
                        ],
                        Some(origin),
                    )
                    .map_err(error)?;
                let grab = match grab_serial.kind {
                    crate::integrations::wayland::compositor::SerialKind::PointerButton => {
                        if !self
                            .core
                            .seats
                            .get(&seat)
                            .and_then(|seat| seat.pointer_grab_focus())
                            .is_some_and(|focus| {
                                focus.client == context.client && focus.surface == origin
                            })
                        {
                            return Ok(DispatchOutcome::default());
                        }
                        let live_serial = self.pointer_press_serials.iter().any(
                            |(&(candidate, button), &(issued, focus))| {
                                candidate == seat
                                    && issued == serial
                                    && focus.client == context.client
                                    && focus.surface == origin
                                    && self.core.seats.get(&seat).is_some_and(|state| {
                                        state.pressed_buttons().contains(&button)
                                            && state.pointer_grab_focus().is_some_and(|current| {
                                                current.enter_serial == focus.enter_serial
                                            })
                                    })
                            },
                        );
                        if !live_serial {
                            return Ok(DispatchOutcome::default());
                        }
                        NativeDragGrab::Pointer
                    }
                    crate::integrations::wayland::compositor::SerialKind::TouchDown => {
                        let slot = self
                            .touch_points
                            .iter()
                            .find_map(|((candidate_seat, slot), point)| {
                                (*candidate_seat == seat
                                    && point.client == context.client
                                    && point.surface == origin
                                    && point.down_serial == serial)
                                    .then_some(*slot)
                            })
                            .ok_or_else(|| {
                                NativeCompositorError::new(
                                    "touch drag serial has no active touch point",
                                )
                            })?;
                        NativeDragGrab::Touch(slot)
                    }
                    _ => unreachable!("serial kind was constrained above"),
                };
                self.core
                    .serials
                    .consume(context.client, serial, &[grab_serial.kind], Some(origin))
                    .map_err(error)?;
                if let Some(source) = source {
                    let source_resource = self.data_source_resource(source)?;
                    let source_state = self
                        .core
                        .data_devices
                        .source(source)
                        .ok_or_else(|| NativeCompositorError::new("unknown drag source"))?;
                    if source_resource.version() >= 3 && !source_state.actions_set {
                        return Err(NativeCompositorError::new(
                            "version 3 drag source did not set its actions",
                        ));
                    }
                }
                let icon = request
                    .object(2)
                    .map_err(error)?
                    .map(|resource| self.surface_from_resource(resource))
                    .transpose()?;
                if let Some(icon) = icon {
                    if self.core.world.surface_owner(icon) != Some(context.client) {
                        return Err(NativeCompositorError::new(
                            "drag icon belongs to another client",
                        ));
                    }
                    self.surface_mut(icon)?
                        .assign_role(SurfaceRole::DragIcon)
                        .map_err(error)?;
                }
                self.core
                    .data_devices
                    .start_drag(context.client, source, origin)
                    .map_err(error)?;
                self.active_drag = Some(NativeDrag {
                    seat,
                    source,
                    origin,
                    icon,
                    grab,
                    target: None,
                });
                self.core
                    .queue_action(CompositorAction::StartDrag { seat, origin, icon });
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_data_offer(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        offer: ProtocolObjectId,
        request: &mut IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "accept" => {
                let is_drag = self
                    .core
                    .data_devices
                    .offer(offer)
                    .is_some_and(|offer| offer.drag);
                if !is_drag {
                    return Err(NativeCompositorError::new(
                        "selection offers cannot be accepted as drag offers",
                    ));
                }
                let target_surface = self
                    .active_drag
                    .as_ref()
                    .and_then(|drag| drag.target.as_ref())
                    .filter(|target| target.offers.contains(&offer))
                    .map(|target| target.surface)
                    .ok_or_else(|| NativeCompositorError::new("data offer is no longer active"))?;
                self.core
                    .serials
                    .validate(
                        context.client,
                        request.uint(0).map_err(error)?,
                        &[crate::integrations::wayland::compositor::SerialKind::DataDevice],
                        Some(target_surface),
                    )
                    .map_err(error)?;
                let accepted = request
                    .string(1)
                    .map_err(error)?
                    .map(|value| {
                        crate::integrations::wayland::compositor::MimeType::new(
                            value.to_string_lossy().into_owned(),
                        )
                        .map_err(error)
                    })
                    .transpose()?;
                if let Some(mime) = &accepted {
                    let source = self
                        .core
                        .data_devices
                        .offer(offer)
                        .and_then(|offer| self.core.data_devices.source(offer.source))
                        .ok_or_else(|| NativeCompositorError::new("unknown data offer source"))?;
                    if !source.mime_types.contains(mime) {
                        return Err(NativeCompositorError::new(
                            "accepted MIME type was not offered",
                        ));
                    }
                }
                let (source, drag) = {
                    let offer = self
                        .core
                        .data_devices
                        .offer_mut(offer)
                        .ok_or_else(|| NativeCompositorError::new("unknown wl_data_offer"))?;
                    offer.accepted_mime_type = accepted.clone();
                    if offer.drag && resource.version() < 3 {
                        offer.target_actions =
                            crate::integrations::wayland::compositor::DataAction::COPY;
                        offer.selected_action = if accepted.is_some() {
                            crate::integrations::wayland::compositor::DataAction::COPY
                        } else {
                            crate::integrations::wayland::compositor::DataAction::NONE
                        };
                    }
                    (offer.source, offer.drag)
                };
                if drag {
                    let source_resource = self.data_source_resource(source)?;
                    let mime = accepted.as_ref().map(|mime| protocol_string(mime.as_str()));
                    self.post_event(
                        source_resource,
                        "wl_data_source",
                        "target",
                        &mut [ffi::wl_argument {
                            s: mime.as_ref().map_or(std::ptr::null(), |mime| mime.as_ptr()),
                        }],
                    )?;
                }
            }
            "receive" => {
                if self.active_session_lock.is_some() || self.secure_session_locked {
                    let _fd = request.take_fd(1).map_err(error)?;
                    return Ok(DispatchOutcome::default());
                }
                let mime =
                    crate::integrations::wayland::compositor::MimeType::new(c_string(request, 0)?)
                        .map_err(error)?;
                let Some(source) = self.core.data_devices.offer(offer)
                    .and_then(|offer| self.core.data_devices.source(offer.source)).cloned() else {
                        let _fd = request.take_fd(1).map_err(error)?;
                        return Ok(DispatchOutcome::default());
                    };
                if !source.mime_types.contains(&mime) {
                    return Err(NativeCompositorError::new(
                        "requested MIME type was not offered",
                    ));
                }
                let fd = request.take_fd(1).map_err(error)?;
                if self.send_host_clipboard(source.object, mime.as_str(), fd.try_clone().map_err(error)?)? {
                    return Ok(DispatchOutcome::default());
                }
                let source_resource = self.data_source_resource(source.object)?;
                let mime = protocol_string(mime.as_str());
                self.post_event(
                    source_resource,
                    self.source_interface(source_resource)?,
                    "send",
                    &mut [
                        ffi::wl_argument { s: mime.as_ptr() },
                        ffi::wl_argument { h: fd.as_raw_fd() },
                    ],
                )?;
            }
            "set_actions" => {
                if !self
                    .core
                    .data_devices
                    .offer(offer)
                    .is_some_and(|offer| offer.drag)
                {
                    return Err(NativeCompositorError::new(
                        "selection offers do not negotiate drag actions",
                    ));
                }
                let actions = crate::integrations::wayland::compositor::DataAction::from_protocol(
                    request.uint(0).map_err(error)?,
                )
                .ok_or_else(|| NativeCompositorError::new("invalid data-offer actions"))?;
                let preferred =
                    crate::integrations::wayland::compositor::DataAction::from_protocol(
                        request.uint(1).map_err(error)?,
                    )
                    .filter(|action| {
                        [
                            crate::integrations::wayland::compositor::DataAction::NONE,
                            crate::integrations::wayland::compositor::DataAction::COPY,
                            crate::integrations::wayland::compositor::DataAction::MOVE,
                            crate::integrations::wayland::compositor::DataAction::ASK,
                        ]
                        .contains(action)
                    })
                    .ok_or_else(|| NativeCompositorError::new("invalid preferred data action"))?;
                if preferred != crate::integrations::wayland::compositor::DataAction::NONE
                    && !actions.contains(preferred)
                {
                    return Err(NativeCompositorError::new(
                        "preferred data action is not in the accepted set",
                    ));
                }
                let source = {
                    let offer = self
                        .core
                        .data_devices
                        .offer_mut(offer)
                        .ok_or_else(|| NativeCompositorError::new("unknown wl_data_offer"))?;
                    offer.target_actions = actions;
                    offer.preferred_action = preferred;
                    offer.source
                };
                let selected = self.core.data_devices.choose_action(offer).map_err(error)?;
                self.post_event(
                    resource,
                    "wl_data_offer",
                    "action",
                    &mut [ffi::wl_argument {
                        u: u32::from(selected.bits()),
                    }],
                )?;
                if let Ok(source_resource) = self.data_source_resource(source)
                    && source_resource.version() >= 3
                {
                    self.post_event(
                        source_resource,
                        "wl_data_source",
                        "action",
                        &mut [ffi::wl_argument {
                            u: u32::from(selected.bits()),
                        }],
                    )?;
                }
            }
            "finish" => {
                let source = {
                    let offer = self
                        .core
                        .data_devices
                        .offer_mut(offer)
                        .ok_or_else(|| NativeCompositorError::new("unknown wl_data_offer"))?;
                    if !offer.drag
                        || !offer.dropped
                        || offer.finished
                        || offer.selected_action
                            == crate::integrations::wayland::compositor::DataAction::NONE
                    {
                        return Err(NativeCompositorError::new(
                            "data offer cannot be finished in its current state",
                        ));
                    }
                    offer.finished = true;
                    offer.source
                };
                let first_finish = self.finished_drag_sources.insert(source);
                if first_finish
                    && let Ok(source_resource) = self.data_source_resource(source)
                    && source_resource.version() >= 3
                {
                    self.post_event(source_resource, "wl_data_source", "dnd_finished", &mut [])?;
                }
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn data_source_from_resource(
        &self,
        resource: ResourceRef<'_>,
    ) -> Result<ProtocolObjectId, NativeCompositorError> {
        let ResourceKind::DataSource(source) = self.resource_kind(resource)? else {
            return Err(NativeCompositorError::new(
                "resource is not a wl_data_source",
            ));
        };
        Ok(source)
    }

    pub(super) fn data_source_resource(
        &self,
        source: ProtocolObjectId,
    ) -> Result<ResourceRef<'_>, NativeCompositorError> {
        self.resource_for_kind(
            |kind| matches!(kind, ResourceKind::DataSource(candidate) | ResourceKind::PrimarySource(candidate) if candidate == source),
        )?
        .ok_or_else(|| NativeCompositorError::new("wl_data_source resource is absent"))
    }

    pub(super) fn cancel_data_source(
        &self,
        source: ProtocolObjectId,
    ) -> Result<(), NativeCompositorError> {
        if let Some(resource) = self.resource_for_kind(
            |kind| matches!(kind, ResourceKind::DataSource(candidate) | ResourceKind::PrimarySource(candidate) if candidate == source),
        )? {
            self.post_event(resource, self.source_interface(resource)?, "cancelled", &mut [])?;
        }
        Ok(())
    }

    pub(super) fn send_selection_to_client(
        &mut self,
        seat: u32,
        client: ClientId,
    ) -> Result<(), NativeCompositorError> {
        self.send_primary_to_client(seat, client, false)?;
        let devices = self
            .resources_for_client(
                client,
                |kind| matches!(kind, ResourceKind::DataDevice(candidate) if candidate == seat),
            )?
            .into_iter()
            .map(ResourceRef::identity)
            .collect::<Vec<_>>();
        for identity in devices {
            let Some(device) =
                (unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) })
            else {
                continue;
            };
            self.send_selection_to_device(device, client)?;
        }
        Ok(())
    }

    pub(super) fn clear_selection_for_client(
        &mut self,
        seat: u32,
        client: ClientId,
    ) -> Result<(), NativeCompositorError> {
        self.send_primary_to_client(seat, client, true)?;
        self.core.data_devices.remove_offers_for_target(client);
        for resource in self.resources_for_client(
            client,
            |kind| matches!(kind, ResourceKind::DataDevice(candidate) if candidate == seat),
        )? {
            self.post_event(
                resource,
                "wl_data_device",
                "selection",
                &mut [ffi::wl_argument {
                    o: std::ptr::null_mut(),
                }],
            )?;
        }
        Ok(())
    }

    pub(super) fn send_selection_to_device(
        &mut self,
        device: ResourceRef<'_>,
        client: ClientId,
    ) -> Result<(), NativeCompositorError> {
        self.send_selection_device_kind(device, client, false)
    }
    pub(super) fn send_selection_device_kind(&mut self, device: ResourceRef<'_>, client: ClientId, primary: bool) -> Result<(), NativeCompositorError> {
        let device_interface = if primary { "zwp_primary_selection_device_v1" } else { "wl_data_device" };
        let offer_interface = if primary { "zwp_primary_selection_offer_v1" } else { "wl_data_offer" };
        let selected = if primary { self.core.data_devices.primary_selection() } else { self.core.data_devices.selection() };
        let Some(selection) = selected else {
            return self.post_event(
                device,
                device_interface,
                "selection",
                &mut [ffi::wl_argument {
                    o: std::ptr::null_mut(),
                }],
            );
        };
        let source = self
            .core
            .data_devices
            .source(selection)
            .cloned()
            .ok_or_else(|| NativeCompositorError::new("selection source is absent"))?;
        let object = self.peek_next_object()?;
        let offer_resource = self.create_resource(
            device.client(),
            client,
            offer_interface,
            device.version(),
            0,
            if primary { ResourceKind::PrimaryOffer(object) } else { ResourceKind::DataOffer(object) },
            true,
        )?;
        if let Err(cause) = self.core.data_devices.create_offer(
            crate::integrations::wayland::compositor::DataOffer {
                object,
                source: selection,
                target: client,
                drag: false,
                accepted_mime_type: None,
                source_actions: source.actions,
                target_actions: crate::integrations::wayland::compositor::DataAction::NONE,
                preferred_action: crate::integrations::wayland::compositor::DataAction::NONE,
                selected_action: crate::integrations::wayland::compositor::DataAction::NONE,
                dropped: false,
                finished: false,
            },
        ) {
            unsafe { offer_resource.destroy() };
            return Err(error(cause));
        }
        self.post_event(
            device,
            device_interface,
            "data_offer",
            &mut [ffi::wl_argument {
                o: offer_resource.identity() as *mut ffi::wl_resource,
            }],
        )?;
        for mime in &source.mime_types {
            let mime = protocol_string(mime.as_str());
            self.post_event(
                offer_resource,
                offer_interface,
                "offer",
                &mut [ffi::wl_argument { s: mime.as_ptr() }],
            )?;
        }
        self.post_event(
            device,
            device_interface,
            "selection",
            &mut [ffi::wl_argument {
                o: offer_resource.identity() as *mut ffi::wl_resource,
            }],
        )
    }
}
