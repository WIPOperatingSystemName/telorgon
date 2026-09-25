use super::*;

impl NativeState {
    pub(super) fn source_interface(
        &self,
        resource: ResourceRef<'_>,
    ) -> Result<&'static str, NativeCompositorError> {
        Ok(
            if matches!(
                self.resource_kind(resource)?,
                ResourceKind::PrimarySource(_)
            ) {
                "zwp_primary_selection_source_v1"
            } else {
                "wl_data_source"
            },
        )
    }
    pub(super) fn dispatch_primary(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "create_source" => {
                let object = self.peek_next_object()?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "zwp_primary_selection_source_v1",
                    1,
                    request.new_id(0).map_err(error)?,
                    ResourceKind::PrimarySource(object),
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
            "get_device" => {
                let seat_resource = request
                    .object(1)
                    .map_err(error)?
                    .ok_or_else(|| NativeCompositorError::new("missing seat"))?;
                let ResourceKind::Seat(seat) = self.resource_kind(seat_resource)? else {
                    return Err(NativeCompositorError::new("invalid seat"));
                };
                let device = self.create_resource(
                    resource.client(),
                    context.client,
                    "zwp_primary_selection_device_v1",
                    1,
                    request.new_id(0).map_err(error)?,
                    ResourceKind::PrimaryDevice(seat),
                    true,
                )?;
                if self
                    .core
                    .seats
                    .get(&seat)
                    .and_then(|seat| seat.keyboard_focus)
                    .is_some_and(|focus| focus.client == context.client)
                {
                    self.send_selection_device_kind(device, context.client, true)?;
                }
            }
            "set_selection" => {
                let ResourceKind::PrimaryDevice(seat) = context.kind else {
                    return Err(unsupported_request(request));
                };
                if !self
                    .core
                    .seats
                    .get(&seat)
                    .and_then(|seat| seat.keyboard_focus)
                    .is_some_and(|focus| focus.client == context.client)
                {
                    return Ok(DispatchOutcome::default());
                }
                if self
                    .core
                    .serials
                    .validate(
                        context.client,
                        request.uint(1).map_err(error)?,
                        &[
                            crate::integrations::wayland::compositor::SerialKind::PointerButton,
                            crate::integrations::wayland::compositor::SerialKind::KeyboardKey,
                        ],
                        None,
                    )
                    .is_err()
                {
                    return Ok(DispatchOutcome::default());
                }
                let source = request
                    .object(0)
                    .map_err(error)?
                    .map(|resource| match self.resource_kind(resource)? {
                        ResourceKind::PrimarySource(source) => Ok(source),
                        _ => Err(NativeCompositorError::new("invalid primary source")),
                    })
                    .transpose()?;
                let previous = self.core.data_devices.primary_selection();
                if previous == source {
                    return Ok(DispatchOutcome::default());
                }
                self.core
                    .data_devices
                    .set_primary_selection(context.client, source)
                    .map_err(error)?;
                if let Some(previous) = previous {
                    self.cancel_data_source(previous)?;
                }
                self.send_primary_to_client(seat, context.client, false)?;
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }
    pub(super) fn send_primary_to_client(
        &mut self,
        seat: u32,
        client: ClientId,
        clear: bool,
    ) -> Result<(), NativeCompositorError> {
        let devices: Vec<_> = self
            .resources_for_client(
                client,
                |kind| matches!(kind, ResourceKind::PrimaryDevice(id) if id == seat),
            )?
            .iter()
            .map(|resource| resource.identity())
            .collect();
        for identity in devices {
            let Some(device) =
                (unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) })
            else {
                continue;
            };
            if clear {
                self.post_event(
                    device,
                    "zwp_primary_selection_device_v1",
                    "selection",
                    &mut [ffi::wl_argument {
                        o: std::ptr::null_mut(),
                    }],
                )?;
            } else {
                self.send_selection_device_kind(device, client, true)?;
            }
        }
        Ok(())
    }
}
