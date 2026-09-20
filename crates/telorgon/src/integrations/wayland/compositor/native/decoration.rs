use super::*;

impl NativeState {
    pub(super) fn dispatch_decoration_manager(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "get_toplevel_decoration" {
            return Err(unsupported_request(request));
        }
        let toplevel_resource = request
            .object(1)
            .map_err(error)?
            .ok_or_else(|| NativeCompositorError::new("missing xdg_toplevel"))?;
        let ResourceKind::XdgToplevel(surface) = self.resource_kind(toplevel_resource)? else {
            return Err(NativeCompositorError::new(
                "decoration target is not an xdg_toplevel",
            ));
        };
        let decoration = self.create_resource(
            resource.client(),
            context.client,
            "zxdg_toplevel_decoration_v1",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::ToplevelDecoration(surface),
            true,
        )?;
        self.toplevels
            .get_mut(&surface)
            .ok_or_else(|| NativeCompositorError::new("unknown xdg_toplevel"))?
            .decoration = crate::integrations::wayland::compositor::DecorationMode::ServerSide;
        self.post_event(
            decoration,
            "zxdg_toplevel_decoration_v1",
            "configure",
            &mut [ffi::wl_argument { u: 2 }],
        )?;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_toplevel_decoration(
        &mut self,
        resource: ResourceRef<'_>,
        surface: WaylandSurfaceId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        let mode = match request.message().name {
            "set_mode" => match request.uint(0).map_err(error)? {
                1 => crate::integrations::wayland::compositor::DecorationMode::ClientSide,
                2 => crate::integrations::wayland::compositor::DecorationMode::ServerSide,
                _ => return Err(NativeCompositorError::new("invalid decoration mode")),
            },
            "unset_mode" => crate::integrations::wayland::compositor::DecorationMode::ServerSide,
            _ => return Err(unsupported_request(request)),
        };
        let mode = if self.decoration_policy.server_decorated(
            mode == crate::integrations::wayland::compositor::DecorationMode::ServerSide,
        ) {
            crate::integrations::wayland::compositor::DecorationMode::ServerSide
        } else {
            crate::integrations::wayland::compositor::DecorationMode::ClientSide
        };
        self.toplevels
            .get_mut(&surface)
            .ok_or_else(|| NativeCompositorError::new("unknown xdg_toplevel"))?
            .decoration = mode;
        self.post_event(
            resource,
            "zxdg_toplevel_decoration_v1",
            "configure",
            &mut [ffi::wl_argument {
                u: if mode == crate::integrations::wayland::compositor::DecorationMode::ServerSide {
                    2
                } else {
                    1
                },
            }],
        )?;
        if self.initial_configures.contains(&surface) {
            let latest = self
                .core
                .xdg_surface_mut(surface)
                .and_then(|xdg| xdg.latest_configure());
            self.send_toplevel_configure(
                surface,
                latest.and_then(|configure| configure.size),
                latest.map_or_else(Default::default, |configure| configure.states),
            )?;
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_toplevel_icon_manager(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "create_icon" => {
                let object = self.peek_next_object()?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "xdg_toplevel_icon_v1",
                    1,
                    request.new_id(0).map_err(error)?,
                    ResourceKind::ToplevelIcon(object),
                    true,
                )?;
                self.toplevel_icons
                    .insert(object, NativeToplevelIcon::default());
            }
            "set_icon" => {
                let toplevel_resource = request
                    .object(0)
                    .map_err(error)?
                    .ok_or_else(|| NativeCompositorError::new("missing xdg_toplevel"))?;
                let ResourceKind::XdgToplevel(surface) = self.resource_kind(toplevel_resource)?
                else {
                    return Err(NativeCompositorError::new(
                        "icon target is not an xdg_toplevel",
                    ));
                };
                if !self.toplevels.contains_key(&surface) {
                    return Err(NativeCompositorError::new("unknown xdg_toplevel"));
                }
                let icon = request
                    .object(1)
                    .map_err(error)?
                    .map(|resource| match self.resource_kind(resource)? {
                        ResourceKind::ToplevelIcon(object) => Ok(object),
                        _ => Err(NativeCompositorError::new(
                            "set_icon object is not an xdg_toplevel_icon_v1",
                        )),
                    })
                    .transpose()?;
                let Some(icon) = icon else {
                    self.pending_toplevel_icons
                        .insert(surface, PendingToplevelIcon::Reset);
                    return Ok(DispatchOutcome::default());
                };
                let (name, buffers) = {
                    let icon = self
                        .toplevel_icons
                        .get_mut(&icon)
                        .ok_or_else(|| NativeCompositorError::new("unknown toplevel icon"))?;
                    icon.immutable = true;
                    (icon.name.clone(), icon.buffers.clone())
                };
                if name.is_none() && buffers.is_empty() {
                    self.pending_toplevel_icons
                        .insert(surface, PendingToplevelIcon::Reset);
                    return Ok(DispatchOutcome::default());
                }
                let mut images = Vec::with_capacity(buffers.len());
                for ((_, scale), buffer) in buffers {
                    images.push(ToplevelIconImage {
                        buffer,
                        scale,
                        image: self.snapshot_shm_buffer(buffer)?,
                    });
                }
                self.toplevel_icon_revision = self.toplevel_icon_revision.wrapping_add(1).max(1);
                self.pending_toplevel_icons.insert(
                    surface,
                    PendingToplevelIcon::Icon(ToplevelIconSnapshot {
                        revision: self.toplevel_icon_revision,
                        name,
                        images,
                    }),
                );
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_toplevel_icon(
        &mut self,
        resource: ResourceRef<'_>,
        object: ProtocolObjectId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        let immutable = self
            .toplevel_icons
            .get(&object)
            .ok_or_else(|| NativeCompositorError::new("unknown toplevel icon"))?
            .immutable;
        if immutable {
            resource.post_error(2, "the toplevel icon is immutable after assignment");
            return Ok(DispatchOutcome::default());
        }
        match request.message().name {
            "set_name" => {
                let name = c_string(request, 0)?;
                if name.len() > 4_096 || name.contains('\0') {
                    return Err(NativeCompositorError::new("invalid toplevel icon name"));
                }
                self.toplevel_icons
                    .get_mut(&object)
                    .expect("icon was checked above")
                    .name = Some(name);
            }
            "add_buffer" => {
                let buffer_resource = request
                    .object(0)
                    .map_err(error)?
                    .ok_or_else(|| NativeCompositorError::new("missing icon wl_buffer"))?;
                let ResourceKind::Buffer(buffer) = self.resource_kind(buffer_resource)? else {
                    resource.post_error(1, "icon object is not a wl_buffer");
                    return Ok(DispatchOutcome::default());
                };
                let scale = request.int(1).map_err(error)?;
                let descriptor = self.core.buffer(buffer);
                let Some(BufferDescriptor::Shm(descriptor)) = descriptor else {
                    resource.post_error(1, "icon buffer must use wl_shm");
                    return Ok(DispatchOutcome::default());
                };
                if scale <= 0 || descriptor.size.width != descriptor.size.height {
                    resource.post_error(1, "icon buffer must be square with a positive scale");
                    return Ok(DispatchOutcome::default());
                }
                self.toplevel_icons
                    .get_mut(&object)
                    .expect("icon was checked above")
                    .buffers
                    .insert((descriptor.size.width, scale), buffer);
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn snapshot_shm_buffer(
        &self,
        buffer: WaylandBufferId,
    ) -> Result<ShmImage, NativeCompositorError> {
        let BufferDescriptor::Shm(descriptor) = self
            .core
            .buffer(buffer)
            .ok_or_else(|| NativeCompositorError::new("unknown Wayland buffer"))?
        else {
            return Err(NativeCompositorError::new(
                "toplevel icon buffer is not shared memory",
            ));
        };
        let fd = self
            .buffer_files
            .get(&buffer)
            .ok_or_else(|| NativeCompositorError::new("icon SHM buffer has no backing file"))?
            .try_clone()
            .map_err(error)?;
        let file = std::fs::File::from(fd);
        let length = descriptor.stride as usize * descriptor.size.height as usize;
        let mut pixels = vec![0_u8; length];
        let mut read = 0;
        while read < pixels.len() {
            let count = file
                .read_at(&mut pixels[read..], descriptor.offset as u64 + read as u64)
                .map_err(error)?;
            if count == 0 {
                return Err(NativeCompositorError::new(
                    "icon shared-memory buffer ended before its declared extent",
                ));
            }
            read += count;
        }
        Ok(ShmImage {
            descriptor: *descriptor,
            pixels,
        })
    }
}
