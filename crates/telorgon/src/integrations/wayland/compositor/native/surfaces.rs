use super::*;

impl NativeState {
    pub(super) fn dispatch_compositor(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "create_surface" => {
                self.next_surface = next_nonzero(self.next_surface)?;
                let surface = WaylandSurfaceId::from_raw(self.next_surface).expect("nonzero");
                self.core
                    .world
                    .create_surface(context.client, surface)
                    .map_err(error)?;
                let created = self.create_resource(
                    resource.client(),
                    context.client,
                    "wl_surface",
                    resource.version(),
                    request.new_id(0).map_err(error)?,
                    ResourceKind::Surface(surface),
                    true,
                )?;
                if created.version() >= 6 {
                    let scale = self
                        .core
                        .outputs
                        .values()
                        .find(|output| output.enabled)
                        .map_or(1, |output| output.description.scale.get().ceil() as i32);
                    self.post_event(
                        created,
                        "wl_surface",
                        "preferred_buffer_scale",
                        &mut [ffi::wl_argument { i: scale }],
                    )?;
                }
            }
            "create_region" => {
                let object = self.peek_next_object()?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "wl_region",
                    1,
                    request.new_id(0).map_err(error)?,
                    ResourceKind::Region(object),
                    true,
                )?;
                self.regions.insert(object, Vec::new());
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_surface(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        surface: WaylandSurfaceId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "attach" => {
                let buffer = request
                    .object(0)
                    .map_err(error)?
                    .map(|buffer| self.resource_kind(buffer))
                    .transpose()?
                    .map(|kind| match kind {
                        ResourceKind::Buffer(buffer) => Ok(buffer),
                        _ => Err(NativeCompositorError::new(
                            "wl_surface.attach object is not a buffer",
                        )),
                    })
                    .transpose()?;
                let offset = PointI {
                    x: request.int(1).map_err(error)?,
                    y: request.int(2).map_err(error)?,
                };
                if resource.version() >= 5 && offset != PointI::default() {
                    return Err(NativeCompositorError::new(
                        "wl_surface v5 attach offset must be zero",
                    ));
                }
                self.surface_mut(surface)?
                    .attach(buffer.map(|buffer| BufferAttachment { buffer, offset }));
            }
            "damage" | "damage_buffer" => {
                let rect = RectI {
                    x: request.int(0).map_err(error)?,
                    y: request.int(1).map_err(error)?,
                    width: request.int(2).map_err(error)?,
                    height: request.int(3).map_err(error)?,
                };
                if request.message().name == "damage_buffer" {
                    self.surface_mut(surface)?
                        .damage_buffer(rect)
                        .map_err(error)?;
                } else {
                    self.surface_mut(surface)?.damage(rect).map_err(error)?;
                }
            }
            "frame" => {
                let object = self.peek_next_object()?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "wl_callback",
                    1,
                    request.new_id(0).map_err(error)?,
                    ResourceKind::Callback(surface),
                    true,
                )?;
                self.callbacks.entry(surface).or_default().push(object);
            }
            "set_opaque_region" | "set_input_region" => {
                let region = request
                    .object(0)
                    .map_err(error)?
                    .map(|region| self.region_from_resource(region))
                    .transpose()?;
                if request.message().name == "set_opaque_region" {
                    self.surface_mut(surface)?.set_opaque_region(region);
                } else {
                    self.surface_mut(surface)?.set_input_region(region);
                }
            }
            "set_buffer_transform" => {
                let transform = match request.int(0).map_err(error)? {
                    0 => BufferTransform::Normal,
                    1 => BufferTransform::Rotate90,
                    2 => BufferTransform::Rotate180,
                    3 => BufferTransform::Rotate270,
                    4 => BufferTransform::Flipped,
                    5 => BufferTransform::Flipped90,
                    6 => BufferTransform::Flipped180,
                    7 => BufferTransform::Flipped270,
                    _ => return Err(NativeCompositorError::new("invalid buffer transform")),
                };
                self.surface_mut(surface)?.set_buffer_transform(transform);
            }
            "set_buffer_scale" => self
                .surface_mut(surface)?
                .set_buffer_scale(request.int(0).map_err(error)?)
                .map_err(error)?,
            "offset" => {
                // wl_surface.offset is represented by the next attachment offset in the current
                // Telorgon surface profile. A pending attachment is required for it to take effect.
                let offset = PointI {
                    x: request.int(0).map_err(error)?,
                    y: request.int(1).map_err(error)?,
                };
                let pending = self.surface_mut(surface)?.pending().attachment.flatten();
                if let Some(mut attachment) = pending {
                    attachment.offset = offset;
                    self.surface_mut(surface)?.attach(Some(attachment));
                }
            }
            "commit" => return self.commit_surface(surface),
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_region(
        &mut self,
        object: ProtocolObjectId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        let rectangle = RectI {
            x: request.int(0).map_err(error)?,
            y: request.int(1).map_err(error)?,
            width: request.int(2).map_err(error)?,
            height: request.int(3).map_err(error)?,
        };
        if rectangle.width <= 0 || rectangle.height <= 0 {
            // Empty region operations have no effect. In particular, clients can
            // send zero-sized rectangles while updating input/opaque regions;
            // wl_region does not define a fatal invalid-size protocol error.
            return Ok(DispatchOutcome::default());
        }
        let rectangles = self
            .regions
            .get_mut(&object)
            .ok_or_else(|| NativeCompositorError::new("unknown region"))?;
        match request.message().name {
            "add" => {
                if rectangles.len() >= Region::MAX_RECTANGLES {
                    return Err(NativeCompositorError::new(
                        "region rectangle limit exceeded",
                    ));
                }
                rectangles.push(rectangle);
            }
            "subtract" => {
                let mut difference = Vec::with_capacity(rectangles.len().saturating_mul(2));
                for current in rectangles.drain(..) {
                    difference.extend(subtract_rectangle(current, rectangle));
                    if difference.len() > Region::MAX_RECTANGLES {
                        return Err(NativeCompositorError::new(
                            "region rectangle limit exceeded by subtraction",
                        ));
                    }
                }
                *rectangles = difference;
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_shm(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &mut IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "create_pool" {
            return Err(unsupported_request(request));
        }
        let object = self.peek_next_object()?;
        let pool = ShmPool::new(
            request.int(2).map_err(error)?,
            ClientLimits::default().maximum_buffer_bytes,
        )
        .map_err(error)?;
        let fd = request.take_fd(1).map_err(error)?;
        if fd_size(&fd).map_err(error)? < pool.size as u64 {
            return Err(NativeCompositorError::new(
                "shared-memory pool size exceeds the backing file",
            ));
        }
        self.create_resource(
            resource.client(),
            context.client,
            "wl_shm_pool",
            resource.version(),
            request.new_id(0).map_err(error)?,
            ResourceKind::ShmPool(object),
            true,
        )?;
        self.shm_pools.insert(
            object,
            NativeShmPool {
                owner: context.client,
                fd,
                pool,
            },
        );
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_shm_pool(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        object: ProtocolObjectId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "resize" => {
                let new_size = request.int(0).map_err(error)?;
                let pool = self
                    .shm_pools
                    .get_mut(&object)
                    .ok_or_else(|| NativeCompositorError::new("unknown SHM pool"))?;
                if fd_size(&pool.fd).map_err(error)? < new_size.max(0) as u64 {
                    return Err(NativeCompositorError::new(
                        "resized shared-memory pool exceeds the backing file",
                    ));
                }
                pool.pool
                    .resize(new_size, ClientLimits::default().maximum_buffer_bytes)
                    .map_err(error)?;
            }
            "create_buffer" => {
                let pool = self
                    .shm_pools
                    .get(&object)
                    .ok_or_else(|| NativeCompositorError::new("unknown SHM pool"))?;
                if pool.owner != context.client {
                    return Err(NativeCompositorError::new("SHM pool ownership mismatch"));
                }
                let format = match request.uint(5).map_err(error)? {
                    0 => ShmFormat::Argb8888,
                    1 => ShmFormat::Xrgb8888,
                    value => ShmFormat::Other(value),
                };
                let descriptor = ShmBuffer::new(
                    pool.pool,
                    request.int(1).map_err(error)?,
                    request.int(2).map_err(error)?,
                    request.int(3).map_err(error)?,
                    request.int(4).map_err(error)?,
                    format,
                )
                .map_err(error)?;
                let fd = pool.fd.try_clone().map_err(error)?;
                self.next_buffer = next_nonzero(self.next_buffer)?;
                let buffer = WaylandBufferId::from_raw(self.next_buffer).expect("nonzero");
                self.core
                    .register_buffer(context.client, buffer, BufferDescriptor::Shm(descriptor))
                    .map_err(error)?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "wl_buffer",
                    1,
                    request.new_id(0).map_err(error)?,
                    ResourceKind::Buffer(buffer),
                    true,
                )?;
                self.buffer_files.insert(buffer, fd);
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_subcompositor(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "get_subsurface" {
            return Err(unsupported_request(request));
        }
        let child = self.surface_from_resource(
            request
                .object(1)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing child surface"))?,
        )?;
        let parent = self.surface_from_resource(
            request
                .object(2)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing parent surface"))?,
        )?;
        self.surface_mut(child)?
            .assign_role(SurfaceRole::Subsurface)
            .map_err(error)?;
        self.core.subsurfaces.add(child, parent).map_err(error)?;
        self.create_resource(
            resource.client(),
            context.client,
            "wl_subsurface",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::Subsurface(child),
            true,
        )?;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_subsurface(
        &mut self,
        surface: WaylandSurfaceId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "set_position" => self
                .core
                .subsurfaces
                .set_position(
                    surface,
                    crate::integrations::wayland::compositor::SubsurfacePosition {
                        offset: PointI {
                            x: request.int(0).map_err(error)?,
                            y: request.int(1).map_err(error)?,
                        },
                        above: self.core.subsurfaces.position(surface).and_then(|position| position.above),
                    },
                )
                .map_err(error)?,
            "set_sync" => {
                self.core
                    .subsurfaces
                    .set_synchronized(surface, true)
                    .map_err(error)?;
            }
            "set_desync" => {
                if let Some(commit) = self
                    .core
                    .subsurfaces
                    .set_synchronized(surface, false)
                    .map_err(error)?
                {
                    self.surface_mut(surface)?.stage(commit).map_err(error)?;
                    self.commit_surface(surface)?;
                }
            }
            "place_above" | "place_below" => {
                // Sibling ordering is policy-visible; exact ordering is applied by the shell scene.
                let sibling = self.surface_from_resource(
                    request
                        .object(0)
                        .map_err(error)?
                        .ok_or_else(|| NativeCompositorError::new("missing sibling"))?,
                )?;
                let position = crate::integrations::wayland::compositor::SubsurfacePosition {
                    offset: self.core.subsurfaces.position(surface).map_or(PointI::default(), |position| position.offset),
                    above: (request.message().name == "place_above").then_some(sibling),
                };
                self.core
                    .subsurfaces
                    .set_position(surface, position)
                    .map_err(error)?;
            }
            _ => return Err(unsupported_request(request)),
        }
        super::super::diagnostics::event(surface.get(), "subsurface", format_args!(
            "request={} parent={:?} position={:?} synchronized={}", request.message().name,
            self.core.subsurfaces.parent(surface), self.core.subsurfaces.position(surface),
            self.core.subsurfaces.effectively_synchronized(surface)));
        Ok(DispatchOutcome::default())
    }

    pub(super) fn surface_resource(
        &self,
        surface: WaylandSurfaceId,
    ) -> Result<*mut ffi::wl_resource, NativeCompositorError> {
        self.resource_for_kind(
            |kind| matches!(kind, ResourceKind::Surface(candidate) if candidate == surface),
        )?
        .map(|resource| resource.identity() as *mut ffi::wl_resource)
        .ok_or_else(|| NativeCompositorError::new("wl_surface resource is absent"))
    }

    pub(super) fn surface_from_resource(
        &self,
        resource: ResourceRef<'_>,
    ) -> Result<WaylandSurfaceId, NativeCompositorError> {
        match self.resource_kind(resource)? {
            ResourceKind::Surface(surface) => Ok(surface),
            _ => Err(NativeCompositorError::new("resource is not a wl_surface")),
        }
    }

    pub(super) fn region_from_resource(
        &self,
        resource: ResourceRef<'_>,
    ) -> Result<Region, NativeCompositorError> {
        let ResourceKind::Region(object) = self.resource_kind(resource)? else {
            return Err(NativeCompositorError::new("resource is not a wl_region"));
        };
        Region::from_rectangles(
            self.regions
                .get(&object)
                .cloned()
                .ok_or_else(|| NativeCompositorError::new("unknown region"))?,
        )
        .map_err(error)
    }

    pub(super) fn surface_mut(
        &mut self,
        surface: WaylandSurfaceId,
    ) -> Result<&mut crate::integrations::wayland::compositor::SurfaceState, NativeCompositorError>
    {
        self.core
            .world
            .surface_mut(surface)
            .ok_or_else(|| NativeCompositorError::new("unknown wl_surface"))
    }
}
