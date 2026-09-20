use super::*;

impl NativeState {
    pub(super) fn dispatch_linux_dmabuf(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if matches!(
            request.message().name,
            "get_default_feedback" | "get_surface_feedback"
        ) {
            if request.message().name == "get_surface_feedback" {
                let surface = request
                    .object(1)
                    .map_err(error)?
                    .ok_or_else(|| NativeCompositorError::new("missing feedback surface"))?;
                self.surface_from_resource(surface)?;
                if surface.client().identity() != resource.client().identity() {
                    return Err(NativeCompositorError::new(
                        "feedback surface belongs to another client",
                    ));
                }
            }
            let feedback = self.create_resource(
                resource.client(),
                context.client,
                "zwp_linux_dmabuf_feedback_v1",
                resource.version(),
                request.new_id(0).map_err(error)?,
                ResourceKind::LinuxDmaBufFeedback,
                true,
            )?;
            // One immutable sampling policy applies to every surface for this display lifetime.
            // There are no later updates, so a feedback object is already inert if its surface dies.
            self.send_dmabuf_feedback(feedback)?;
            return Ok(DispatchOutcome::default());
        }
        if request.message().name != "create_params" {
            return Err(unsupported_request(request));
        }
        let object = self.peek_next_object()?;
        self.create_resource(
            resource.client(),
            context.client,
            "zwp_linux_buffer_params_v1",
            resource.version(),
            request.new_id(0).map_err(error)?,
            ResourceKind::LinuxBufferParams(object),
            true,
        )?;
        self.dmabuf_params
            .insert(object, NativeDmaBufParams::default());
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_linux_buffer_params(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        object: ProtocolObjectId,
        request: &mut IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "add" => {
                let plane = NativeDmaBufPlane {
                    fd: request.take_fd(0).map_err(error)?,
                    offset: request.uint(2).map_err(error)?,
                    stride: request.uint(3).map_err(error)?,
                    modifier: (u64::from(request.uint(4).map_err(error)?) << 32)
                        | u64::from(request.uint(5).map_err(error)?),
                };
                let index = request.uint(1).map_err(error)?;
                let params = self
                    .dmabuf_params
                    .get_mut(&object)
                    .ok_or_else(|| NativeCompositorError::new("unknown DMA-BUF parameters"))?;
                if params.used || index >= 4 || plane.stride == 0 {
                    return Err(NativeCompositorError::new("invalid DMA-BUF plane"));
                }
                if params.planes.insert(index, plane).is_some() {
                    return Err(NativeCompositorError::new("duplicate DMA-BUF plane index"));
                }
            }
            "create" | "create_immed" => {
                let immediate = request.message().name == "create_immed";
                let base = usize::from(immediate);
                let wire_id = if immediate {
                    request.new_id(0).map_err(error)?
                } else {
                    0
                };
                let result = self.finish_dma_buf(
                    resource,
                    context,
                    object,
                    wire_id,
                    request.int(base).map_err(error)?,
                    request.int(base + 1).map_err(error)?,
                    request.uint(base + 2).map_err(error)?,
                    request.uint(base + 3).map_err(error)?,
                );
                match result {
                    Ok(buffer_resource) if !immediate => {
                        self.post_event(
                            resource,
                            "zwp_linux_buffer_params_v1",
                            "created",
                            &mut [ffi::wl_argument {
                                o: buffer_resource.identity() as *mut ffi::wl_resource,
                            }],
                        )?;
                    }
                    Ok(_) => {}
                    Err(_) if !immediate => {
                        self.post_event(resource, "zwp_linux_buffer_params_v1", "failed", &mut [])?;
                    }
                    Err(error) => return Err(error),
                }
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_dma_buf<'client>(
        &mut self,
        params_resource: ResourceRef<'client>,
        context: &ResourceContext,
        object: ProtocolObjectId,
        wire_id: u32,
        width: i32,
        height: i32,
        format: u32,
        flags: u32,
    ) -> Result<ResourceRef<'client>, NativeCompositorError> {
        let params = self
            .dmabuf_params
            .get_mut(&object)
            .ok_or_else(|| NativeCompositorError::new("unknown DMA-BUF parameters"))?;
        if params.used {
            return Err(NativeCompositorError::new(
                "DMA-BUF parameters are single-use",
            ));
        }
        params.used = true;
        if width <= 0 || height <= 0 || flags & !0x7 != 0 || params.planes.is_empty() {
            return Err(NativeCompositorError::new("invalid DMA-BUF metadata"));
        }
        let modifier = params
            .planes
            .first_key_value()
            .map(|(_, plane)| plane.modifier)
            .expect("planes checked");
        if params.planes.iter().any(|(index, plane)| {
            *index as usize >= params.planes.len() || plane.modifier != modifier
        }) || !self.dmabuf_formats.contains(&DmaBufFormat {
            fourcc: format,
            modifier,
        }) {
            return Err(NativeCompositorError::new(
                "unsupported or non-contiguous DMA-BUF plane layout",
            ));
        }
        self.next_buffer = next_nonzero(self.next_buffer)?;
        let buffer = WaylandBufferId::from_raw(self.next_buffer).expect("nonzero");
        let planes = std::mem::take(&mut params.planes);
        let descriptor_planes = planes
            .iter()
            .map(
                |(index, plane)| crate::integrations::wayland::compositor::DmaBufPlane {
                    index: *index as u8,
                    fd_token: u64::try_from(plane.fd.as_raw_fd()).unwrap_or(1).max(1),
                    offset: plane.offset,
                    stride: plane.stride,
                    modifier: plane.modifier,
                },
            )
            .collect();
        let descriptor = crate::integrations::wayland::compositor::DmaBufDescriptor::new(
            crate::foundation::SizeI { width, height },
            format,
            crate::integrations::wayland::compositor::DmaBufFlags {
                y_invert: flags & 1 != 0,
                interlaced: flags & 2 != 0,
                bottom_field_first: flags & 4 != 0,
            },
            descriptor_planes,
        )
        .map_err(error)?;
        let files = planes.into_values().map(|plane| plane.fd).collect();
        self.core
            .register_buffer(context.client, buffer, BufferDescriptor::DmaBuf(descriptor))
            .map_err(error)?;
        let buffer_resource = self.create_resource(
            params_resource.client(),
            context.client,
            "wl_buffer",
            1,
            wire_id,
            ResourceKind::Buffer(buffer),
            true,
        )?;
        self.dmabuf_files.insert(buffer, files);
        Ok(buffer_resource)
    }

    pub(super) fn dispatch_explicit_synchronization(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "get_synchronization" {
            return Err(unsupported_request(request));
        }
        let surface = self.surface_from_resource(
            request
                .object(1)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing wl_surface"))?,
        )?;
        if !self.synchronized_surfaces.insert(surface) {
            return Err(NativeCompositorError::new(
                "surface already has an explicit-synchronization object",
            ));
        }
        self.create_resource(
            resource.client(),
            context.client,
            "zwp_linux_surface_synchronization_v1",
            resource.version(),
            request.new_id(0).map_err(error)?,
            ResourceKind::SurfaceSynchronization(surface),
            true,
        )?;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_surface_synchronization(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        surface: WaylandSurfaceId,
        request: &mut IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "set_acquire_fence" => {
                if self.pending_acquire_fences.contains_key(&surface) {
                    return Err(NativeCompositorError::new(
                        "acquire fence was already set for the pending commit",
                    ));
                }
                self.pending_acquire_fences
                    .insert(surface, request.take_fd(0).map_err(error)?);
            }
            "get_release" => {
                if self.pending_releases.contains_key(&surface) {
                    return Err(NativeCompositorError::new(
                        "buffer release was already requested for the pending commit",
                    ));
                }
                let object = self.peek_next_object()?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "zwp_linux_buffer_release_v1",
                    1,
                    request.new_id(0).map_err(error)?,
                    ResourceKind::ExplicitBufferRelease(surface),
                    true,
                )?;
                self.pending_releases.insert(surface, object);
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn finish_explicit_release(
        &mut self,
        surface: WaylandSurfaceId,
        revision: u64,
        fence: Option<OwnedFd>,
    ) -> Result<Option<usize>, NativeCompositorError> {
        let Some(object) = self.committed_releases.remove(&(surface, revision)) else {
            return Ok(None);
        };
        let identity = self
            .resources
            .get(&object)
            .copied()
            .ok_or_else(|| NativeCompositorError::new("explicit release resource is absent"))?;
        let resource = unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) }
            .ok_or_else(|| NativeCompositorError::new("explicit release resource is stale"))?;
        if let Some(fence) = fence.as_ref() {
            self.post_event(
                resource,
                "zwp_linux_buffer_release_v1",
                "fenced_release",
                &mut [ffi::wl_argument {
                    h: fence.as_raw_fd(),
                }],
            )?;
        } else {
            self.post_event(
                resource,
                "zwp_linux_buffer_release_v1",
                "immediate_release",
                &mut [],
            )?;
        }
        Ok(Some(identity))
    }
}
