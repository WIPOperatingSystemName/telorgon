use super::*;

// Withdrawn globals must retain callback data for clients with racing bind requests on
// Wayland versions without removal acknowledgements. Bound the whole compositor lifetime.
pub(super) const MAX_OUTPUT_GLOBALS: usize = 4096;

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
        if self.retired_outputs.contains(&output_id) {
            return Ok(());
        }
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
        if self.retired_outputs.contains(&output_id) {
            return Ok(());
        }
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

    /// Track each binding separately so late binds and map/unmap cycles get balanced events.
    /// An explicit host assignment overrides the legacy all-enabled-outputs membership.
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
                    .surface_outputs
                    .get(&surface)
                    .map_or(!self.manual_outputs.contains(&output), |outputs| {
                        outputs.contains(&output)
                    })
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

impl NativeCompositor<'_> {
    /// Set client-visible output membership on the compositor owner thread. `Some(&[])`
    /// excludes all outputs; `None` restores automatic membership in enabled outputs.
    /// Assignments survive unmap/remap and are removed when the surface is destroyed.
    ///
    /// Validate the complete list (at most 64 registered, distinct output IDs) before changing
    /// policy. Returns false for unchanged policy. Success queues balanced enter/leave events
    /// for every existing binding; later bindings inherit the same assignment. The normal
    /// display flush delivers them. This controls protocol membership only: the host must
    /// separately place/render the surface and authorize capture.
    pub fn set_surface_outputs(
        &mut self,
        surface: WaylandSurfaceId,
        outputs: Option<&[u32]>,
    ) -> Result<bool, NativeCompositorError> {
        self.state.surface_resource(surface)?;
        let selected = match outputs {
            None => None,
            Some(outputs) => {
                if outputs.len() > 64 {
                    return Err(NativeCompositorError::new("too many surface outputs"));
                }
                let mut selected = BTreeSet::new();
                for &output in outputs {
                    if !self.state.core.outputs.contains_key(&output) || !selected.insert(output) {
                        return Err(NativeCompositorError::new(
                            "unknown or duplicate surface output",
                        ));
                    }
                }
                Some(selected)
            }
        };
        if self.state.surface_outputs.get(&surface) == selected.as_ref() {
            return Ok(false);
        }
        match selected {
            Some(selected) => {
                self.state.surface_outputs.insert(surface, selected);
            }
            None => {
                self.state.surface_outputs.remove(&surface);
            }
        }
        let mapped = self.state.mapped_outputs.contains(&surface);
        self.state.update_surface_output(surface, mapped)?;
        Ok(true)
    }
}

impl NativeState {
    pub(super) fn send_preferred_output_scale(
        &self,
        scale: f32,
    ) -> Result<(), NativeCompositorError> {
        for resource in
            self.resources_for_kind(|kind| matches!(kind, ResourceKind::FractionalScale))?
        {
            self.post_event(
                resource,
                "wp_fractional_scale_v1",
                "preferred_scale",
                &mut [ffi::wl_argument {
                    u: (scale * 120.0).round() as u32,
                }],
            )?;
        }
        for resource in self.resources_for_kind(|kind| matches!(kind, ResourceKind::Surface(_)))? {
            if resource.version() >= 6 {
                self.post_event(
                    resource,
                    "wl_surface",
                    "preferred_buffer_scale",
                    &mut [ffi::wl_argument {
                        i: scale.ceil() as i32,
                    }],
                )?;
            }
        }
        Ok(())
    }
}
impl NativeCompositor<'_> {
    /// Exclude an output from automatic surface membership. Explicit assignments still apply.
    /// Use this for independent displays before dispatching their first client requests.
    pub fn set_output_automatic_membership(
        &mut self,
        id: u32,
        automatic: bool,
    ) -> Result<(), NativeCompositorError> {
        if !self.state.core.outputs.contains_key(&id) {
            return Err(NativeCompositorError::new("unknown output"));
        }
        if automatic {
            self.state.manual_outputs.remove(&id);
        } else {
            self.state.manual_outputs.insert(id);
        }
        for surface in self
            .state
            .mapped_outputs
            .iter()
            .copied()
            .collect::<Vec<_>>()
        {
            self.state.update_surface_output(surface, true)?;
        }
        Ok(())
    }
    /// Next unused output identity, including identities reserved by withdrawn globals.
    /// A value is not a reservation; register it on this owner thread before requesting another.
    pub fn next_output_id(&self) -> Option<u32> {
        if self.state.core.outputs.len() + self.state.retired_outputs.len() >= MAX_OUTPUT_GLOBALS {
            return None;
        }
        self.state
            .core
            .outputs
            .keys()
            .chain(self.state.retired_outputs.iter())
            .copied()
            .max()
            .unwrap_or(0)
            .checked_add(1)
    }

    /// Withdraw an output on the owner thread. The ID is permanently retired; allocate a
    /// fresh ID for a replacement. Returns false for an absent/already-retired output.
    /// Stops direct capture, removes membership, and queues global removal. Existing and
    /// racing bindings remain inert and can be released normally. Flush the display to
    /// deliver queued events. The host must also retire its renderer/portal source.
    ///
    /// At most 4096 outputs can be registered over this compositor's lifetime (including
    /// withdrawn outputs). Old Wayland clients cannot acknowledge removal, so globals and
    /// their callback contexts stay alive until teardown. Exhaustion rejects new registration;
    /// withdrawal remains available. An event-delivery error does not roll retirement back.
    pub fn remove_output(&mut self, id: u32) -> Result<bool, NativeCompositorError> {
        if !self.state.core.outputs.contains_key(&id) {
            return Ok(false);
        }
        let index = self
            .bind_contexts
            .iter()
            .position(
                |context| matches!(context.kind, ResourceKind::Output(output) if output == id),
            )
            .ok_or_else(|| NativeCompositorError::new("output has no registered global"))?;
        let revision = self
            .state
            .output_revision
            .checked_add(1)
            .ok_or_else(|| NativeCompositorError::new("output layout revision exhausted"))?;
        let old_scale = self
            .state
            .core
            .outputs
            .values()
            .find(|output| output.enabled)
            .map(|output| output.description.scale);
        self.state.core.outputs.remove(&id);
        self.state.retired_outputs.insert(id);
        self.state.manual_outputs.remove(&id);
        self.state.output_revision = revision;
        for selected in self.state.surface_outputs.values_mut() {
            selected.remove(&id);
        }
        // Cancel every in-flight capture before notification can fail. Removed IDs never
        // resolve again, including through an old bound capture-source object.
        let mut failure = self.state.retire_capture_output(id).err();
        for surface in self
            .state
            .mapped_outputs
            .iter()
            .copied()
            .collect::<Vec<_>>()
        {
            if let Err(error) = self.state.update_surface_output(surface, true) {
                failure.get_or_insert(error);
            }
        }
        self.globals[index].remove();
        let scale = self
            .state
            .core
            .outputs
            .values()
            .find(|output| output.enabled)
            .map(|output| output.description.scale);
        if old_scale != scale {
            if let Err(error) = self
                .state
                .send_preferred_output_scale(scale.map_or(1.0, |scale| scale.get()))
            {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(true), Err)
    }
}
