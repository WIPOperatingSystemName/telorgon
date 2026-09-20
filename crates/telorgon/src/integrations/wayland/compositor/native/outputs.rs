use super::*;

impl NativeState {
    pub(super) fn dispatch_xdg_output(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "get_xdg_output" {
            return Err(unsupported_request(request));
        }
        let parent = request
            .object(1)
            .map_err(error)?
            .ok_or_else(|| NativeCompositorError::new("missing wl_output"))?;
        let ResourceKind::Output(output_id) = self.resource_kind(parent)? else {
            return Err(NativeCompositorError::new(
                "xdg-output target is not an output",
            ));
        };
        let child = self.create_resource(
            resource.client(),
            context.client,
            "zxdg_output_v1",
            resource.version(),
            request.new_id(0).map_err(error)?,
            ResourceKind::XdgOutput(output_id, self.protocol_object_for_resource(parent)?),
            true,
        )?;
        self.send_xdg_output_description(child, output_id, true, parent.version() >= 2)?;
        if child.version() >= 3 && parent.version() >= 2 {
            self.post_event(parent, "wl_output", "done", &mut [])?;
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn x11_output_scale(&self, resource: ResourceRef<'_>) -> i32 {
        self.xwayland
            .as_ref()
            .filter(|access| access.allows(resource.client().identity()))
            .map_or(1, |access| access.coordinate_scale())
    }

    pub(super) fn send_xdg_output_description(
        &self,
        child: ResourceRef<'_>,
        output_id: u32,
        initial: bool,
        parent_done: bool,
    ) -> Result<(), NativeCompositorError> {
        let output = self
            .core
            .outputs
            .get(&output_id)
            .ok_or_else(|| NativeCompositorError::new("unknown output"))?;
        let density = self.x11_output_scale(child);
        let mut position = output.description.logical_position;
        let mut size = output.logical_size();
        position.x = position.x.saturating_mul(density);
        position.y = position.y.saturating_mul(density);
        size.width = size.width.saturating_mul(density);
        size.height = size.height.saturating_mul(density);
        self.post_event(
            child,
            "zxdg_output_v1",
            "logical_position",
            &mut [
                ffi::wl_argument { i: position.x },
                ffi::wl_argument { i: position.y },
            ],
        )?;
        self.post_event(
            child,
            "zxdg_output_v1",
            "logical_size",
            &mut [
                ffi::wl_argument { i: size.width },
                ffi::wl_argument { i: size.height },
            ],
        )?;
        if child.version() >= 2 {
            let name = protocol_string(&output.description.name);
            let description = protocol_string(&output.description.description);
            if initial {
                self.post_event(
                    child,
                    "zxdg_output_v1",
                    "name",
                    &mut [ffi::wl_argument { s: name.as_ptr() }],
                )?;
            }
            if initial || child.version() >= 3 {
                self.post_event(
                    child,
                    "zxdg_output_v1",
                    "description",
                    &mut [ffi::wl_argument {
                        s: description.as_ptr(),
                    }],
                )?;
            }
        }
        if child.version() < 3 || !parent_done {
            self.post_event(child, "zxdg_output_v1", "done", &mut [])?;
        }
        Ok(())
    }

    pub(super) fn send_output_description(
        &self,
        resource: ResourceRef<'_>,
        output_id: u32,
        initial: bool,
        finish: bool,
    ) -> Result<(), NativeCompositorError> {
        let output = self
            .core
            .outputs
            .get(&output_id)
            .ok_or_else(|| NativeCompositorError::new("unknown output"))?;
        let description = &output.description;
        let make = protocol_string(&description.make);
        let model = protocol_string(&description.model);
        let transform = output_transform_wire(description.transform);
        self.post_event(
            resource,
            "wl_output",
            "geometry",
            &mut [
                ffi::wl_argument {
                    i: description.logical_position.x,
                },
                ffi::wl_argument {
                    i: description.logical_position.y,
                },
                ffi::wl_argument {
                    i: description.physical_millimeters.width,
                },
                ffi::wl_argument {
                    i: description.physical_millimeters.height,
                },
                ffi::wl_argument { i: 0 },
                ffi::wl_argument { s: make.as_ptr() },
                ffi::wl_argument { s: model.as_ptr() },
                ffi::wl_argument { i: transform },
            ],
        )?;
        for (index, mode) in description.modes.iter().enumerate() {
            let mut flags = u32::from(index == output.current_mode);
            if mode.preferred {
                flags |= 2;
            }
            self.post_event(
                resource,
                "wl_output",
                "mode",
                &mut [
                    ffi::wl_argument { u: flags },
                    ffi::wl_argument { i: mode.size.width },
                    ffi::wl_argument {
                        i: mode.size.height,
                    },
                    ffi::wl_argument {
                        i: i32::try_from(mode.refresh_millihertz).unwrap_or(i32::MAX),
                    },
                ],
            )?;
        }
        if resource.version() >= 2 {
            self.post_event(
                resource,
                "wl_output",
                "scale",
                &mut [ffi::wl_argument {
                    i: description.scale.get().ceil() as i32,
                }],
            )?;
        }
        if resource.version() >= 4 {
            let name = protocol_string(&description.name);
            let detail = protocol_string(&description.description);
            if initial {
                self.post_event(
                    resource,
                    "wl_output",
                    "name",
                    &mut [ffi::wl_argument { s: name.as_ptr() }],
                )?;
            }
            self.post_event(
                resource,
                "wl_output",
                "description",
                &mut [ffi::wl_argument { s: detail.as_ptr() }],
            )?;
        }
        if finish && resource.version() >= 2 {
            self.post_event(resource, "wl_output", "done", &mut [])?;
        }
        Ok(())
    }

    pub(super) fn send_seat_description(
        &self,
        resource: ResourceRef<'_>,
        seat_id: u32,
    ) -> Result<(), NativeCompositorError> {
        let seat = self
            .core
            .seats
            .get(&seat_id)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?;
        let capabilities = u32::from(seat.capabilities.pointer)
            | (u32::from(seat.capabilities.keyboard) << 1)
            | (u32::from(seat.capabilities.touch) << 2);
        if resource.version() >= 2 {
            let name = protocol_string(&seat.name);
            self.post_event(
                resource,
                "wl_seat",
                "name",
                &mut [ffi::wl_argument { s: name.as_ptr() }],
            )?;
        }
        self.post_event(
            resource,
            "wl_seat",
            "capabilities",
            &mut [ffi::wl_argument { u: capabilities }],
        )?;
        Ok(())
    }

    /// The desktop currently places all mapped surfaces on its single enabled output.
    /// Track each binding separately so late binds and map/unmap cycles get balanced events.
    pub(super) fn update_surface_output(
        &mut self,
        surface: WaylandSurfaceId,
        mapped: bool,
    ) -> Result<(), NativeCompositorError> {
        let raw = self.surface_resource(surface)?;
        let resource = unsafe { ResourceRef::from_raw(raw) }.expect("live surface resource");
        let context = unsafe { &*resource.user_data().cast::<ResourceContext>() };
        let outputs = self
            .resources_for_client(context.client, |kind| {
                matches!(kind, ResourceKind::Output(_))
            })?
            .into_iter()
            .map(|output| {
                let ctx = unsafe { &*output.user_data().cast::<ResourceContext>() };
                (ctx.object, ctx.kind, output.identity())
            })
            .collect::<Vec<_>>();
        for (object, kind, identity) in outputs {
            let ResourceKind::Output(output) = kind else {
                unreachable!()
            };
            let present = mapped
                && self
                    .core
                    .outputs
                    .get(&output)
                    .is_some_and(|output| output.enabled);
            let entered = self.entered_outputs.contains(&(surface, object));
            if present != entered {
                self.post_event(
                    resource,
                    "wl_surface",
                    if present { "enter" } else { "leave" },
                    &mut [ffi::wl_argument {
                        o: identity as *mut ffi::wl_resource,
                    }],
                )?;
                if present {
                    self.entered_outputs.insert((surface, object));
                } else {
                    self.entered_outputs.remove(&(surface, object));
                }
            }
        }
        if mapped {
            self.mapped_outputs.insert(surface);
        } else {
            self.mapped_outputs.remove(&surface);
        }
        Ok(())
    }

    pub(super) fn output_logical_size(
        &self,
        output: u32,
    ) -> Result<crate::foundation::SizeI, NativeCompositorError> {
        let output = self
            .core
            .outputs
            .get(&output)
            .ok_or_else(|| NativeCompositorError::new("unknown session-lock output"))?;
        let mode = output
            .description
            .modes
            .get(output.current_mode)
            .ok_or_else(|| NativeCompositorError::new("output has no current mode"))?;
        let transformed = match output.description.transform {
            crate::integrations::wayland::compositor::OutputTransform::Rotate90
            | crate::integrations::wayland::compositor::OutputTransform::Rotate270
            | crate::integrations::wayland::compositor::OutputTransform::Flipped90
            | crate::integrations::wayland::compositor::OutputTransform::Flipped270 => {
                crate::foundation::SizeI {
                    width: mode.size.height,
                    height: mode.size.width,
                }
            }
            _ => mode.size,
        };
        Ok(output.description.scale.logical_size(transformed))
    }
}
