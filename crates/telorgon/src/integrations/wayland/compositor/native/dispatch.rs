use super::*;

impl NativeState {
    pub(super) fn bind(
        &mut self,
        client: ClientRef<'_>,
        interface: &str,
        kind: ResourceKind,
        version: u32,
        id: u32,
    ) -> Result<(), NativeCompositorError> {
        self.check_capture_access(kind, client.identity())?;
        if matches!(
            kind,
            ResourceKind::XwaylandShell | ResourceKind::XwaylandKeyboardGrabManager
        ) && !self
            .xwayland
            .as_ref()
            .is_some_and(|access| access.allows(client.identity()))
        {
            return Err(NativeCompositorError::new(
                "unauthorized Xwayland shell bind",
            ));
        }
        let client_id = self.ensure_client(client)?;
        let resource =
            self.create_resource(client, client_id, interface, version, id, kind, true)?;
        if matches!(kind, ResourceKind::ForeignToplevelList) {
            self.bind_foreign_toplevel_list(resource)?;
        }
        if interface == "wl_shm" {
            for format in [0_u32, 1_u32] {
                self.post_event(
                    resource,
                    "wl_shm",
                    "format",
                    &mut [ffi::wl_argument { u: format }],
                )?;
            }
        }
        if interface == "wp_presentation" {
            self.post_event(
                resource,
                "wp_presentation",
                "clock_id",
                &mut [ffi::wl_argument { u: 1 }],
            )?;
        }
        if interface == "xdg_toplevel_icon_manager_v1" {
            for size in [16_i32, 24, 32, 48, 64] {
                self.post_event(
                    resource,
                    "xdg_toplevel_icon_manager_v1",
                    "icon_size",
                    &mut [ffi::wl_argument { i: size }],
                )?;
            }
            self.post_event(resource, "xdg_toplevel_icon_manager_v1", "done", &mut [])?;
        }
        if let ResourceKind::Output(output) = kind {
            self.send_output_description(resource, output, true, true)?;
            for surface in self.mapped_outputs.iter().copied().collect::<Vec<_>>() {
                self.update_surface_output(surface, true)?;
            }
        }
        if let ResourceKind::Seat(seat) = kind {
            self.send_seat_description(resource, seat)?;
        }
        if matches!(kind, ResourceKind::LinuxDmaBuf) && version == 3 {
            for format in &self.dmabuf_formats {
                self.post_event(
                    resource,
                    "zwp_linux_dmabuf_v1",
                    "modifier",
                    &mut [
                        ffi::wl_argument { u: format.fourcc },
                        ffi::wl_argument {
                            u: (format.modifier >> 32) as u32,
                        },
                        ffi::wl_argument {
                            u: format.modifier as u32,
                        },
                    ],
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn ensure_client(
        &mut self,
        client: ClientRef<'_>,
    ) -> Result<ClientId, NativeCompositorError> {
        if let Some(client) = self.clients.get(&client.identity()) {
            return Ok(*client);
        }
        self.next_client = next_nonzero(self.next_client)?;
        let id = ClientId::from_raw(self.next_client).expect("nonzero");
        self.core.connect_client(id).map_err(error)?;
        self.clients.insert(client.identity(), id);
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_resource<'client>(
        &mut self,
        client: ClientRef<'client>,
        client_id: ClientId,
        interface: &str,
        version: u32,
        wire_id: u32,
        kind: ResourceKind,
        register: bool,
    ) -> Result<ResourceRef<'client>, NativeCompositorError> {
        self.next_object = next_nonzero(self.next_object)?;
        let object = ProtocolObjectId::from_raw(self.next_object).expect("nonzero");
        let interface_descriptor = self
            .protocol
            .interface(interface)
            .ok_or_else(|| NativeCompositorError::new(format!("missing interface {interface}")))?;
        let version = version.min(interface_descriptor.version as u32);
        let resource = unsafe { client.create_resource(interface_descriptor, version, wire_id) }
            .map_err(error)?;
        if register {
            self.core
                .objects
                .insert(
                    object,
                    ObjectMetadata {
                        owner: client_id,
                        kind: kind.object_kind(),
                        version,
                    },
                )
                .map_err(error)?;
        }
        let context = Box::new(ResourceContext {
            state: self,
            object,
            client: client_id,
            interface: interface.to_owned(),
            kind,
        });
        let context = Box::into_raw(context);
        unsafe {
            resource.set_dispatcher(
                Some(dispatch_resource),
                std::ptr::null(),
                context.cast::<c_void>(),
                Some(destroy_resource),
            )
        };
        self.resources.insert(object, resource.identity());
        Ok(resource)
    }

    pub(super) fn dispatch(
        &mut self,
        resource: ResourceRef<'_>,
        context: *const ResourceContext,
        kind: ResourceKind,
        request: &mut IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        self.check_capture_access(kind, resource.client().identity())?;
        let context = unsafe { &*context };
        if let ResourceKind::SessionLock(object) = kind
            && request.message().destructor
        {
            return self.dispatch_session_lock(resource, context, object, request);
        }
        if request.message().destructor || request.message().name == "destroy" {
            return Ok(DispatchOutcome {
                destroy_self: true,
                ..DispatchOutcome::default()
            });
        }
        match kind {
            ResourceKind::ForeignToplevelList => {
                self.dispatch_foreign_toplevel_list(resource, context, request)
            }
            ResourceKind::ForeignToplevelHandle => Err(unsupported_request(request)),
            ResourceKind::ImageCopyCaptureManager => {
                self.dispatch_copy_capture_manager(resource, context, request)
            }
            ResourceKind::ImageCopyCaptureSession(id) => {
                self.dispatch_copy_capture_session(resource, context, id, request)
            }
            ResourceKind::ImageCopyCaptureFrame(id) => {
                self.dispatch_copy_capture_frame(resource, id, request)
            }
            ResourceKind::OutputCaptureSourceManager => {
                self.dispatch_output_capture_source(resource, context, request)
            }
            ResourceKind::OutputCaptureSource(_) => Err(unsupported_request(request)),
            ResourceKind::XwaylandShell => self.dispatch_xwayland_shell(resource, context, request),
            ResourceKind::XwaylandSurface(surface) => {
                self.dispatch_xwayland_surface(resource, surface, request)
            }
            ResourceKind::Compositor => self.dispatch_compositor(resource, context, request),
            ResourceKind::Surface(surface) => {
                self.dispatch_surface(resource, context, surface, request)
            }
            ResourceKind::Region(object) => self.dispatch_region(object, request),
            ResourceKind::Shm => self.dispatch_shm(resource, context, request),
            ResourceKind::ShmPool(object) => {
                self.dispatch_shm_pool(resource, context, object, request)
            }
            ResourceKind::Buffer(_) | ResourceKind::Callback(_) => {
                Err(NativeCompositorError::new("resource has no requests"))
            }
            ResourceKind::Output(_) => Err(NativeCompositorError::new(
                "wl_output has no non-destructor requests",
            )),
            ResourceKind::Seat(seat) => self.dispatch_seat(resource, context, seat, request),
            ResourceKind::Pointer(seat) => self.dispatch_pointer(context, seat, request),
            ResourceKind::Keyboard(_) | ResourceKind::Touch(_) => Err(NativeCompositorError::new(
                "input resource has no non-destructor requests",
            )),
            ResourceKind::DataDeviceManager => {
                self.dispatch_data_device_manager(resource, context, request)
            }
            ResourceKind::DataDevice(seat) => self.dispatch_data_device(context, seat, request),
            ResourceKind::DataSource(source) => self.dispatch_data_source(source, request),
            ResourceKind::DataOffer(offer) => {
                self.dispatch_data_offer(resource, context, offer, request)
            }
            ResourceKind::LinuxDmaBuf => self.dispatch_linux_dmabuf(resource, context, request),
            ResourceKind::LinuxDmaBufFeedback => Err(unsupported_request(request)),
            ResourceKind::LinuxBufferParams(object) => {
                self.dispatch_linux_buffer_params(resource, context, object, request)
            }
            ResourceKind::DecorationManager => {
                self.dispatch_decoration_manager(resource, context, request)
            }
            ResourceKind::ToplevelDecoration(surface) => {
                self.dispatch_toplevel_decoration(resource, surface, request)
            }
            ResourceKind::CursorShapeManager => {
                self.dispatch_cursor_shape_manager(resource, context, request)
            }
            ResourceKind::CursorShapeDevice(seat) => {
                self.dispatch_cursor_shape_device(context, seat, request)
            }
            ResourceKind::ToplevelIconManager => {
                self.dispatch_toplevel_icon_manager(resource, context, request)
            }
            ResourceKind::ToplevelIcon(object) => {
                self.dispatch_toplevel_icon(resource, object, request)
            }
            ResourceKind::FractionalScaleManager => {
                self.dispatch_fractional_scale_manager(resource, context, request)
            }
            ResourceKind::FractionalScale => Err(NativeCompositorError::new(
                "fractional-scale object has no non-destructor requests",
            )),
            ResourceKind::Viewporter => self.dispatch_viewporter(resource, context, request),
            ResourceKind::Viewport(surface) => self.dispatch_viewport(surface, request),
            ResourceKind::Presentation => self.dispatch_presentation(resource, context, request),
            ResourceKind::PresentationFeedback(_) => Err(NativeCompositorError::new(
                "presentation-feedback object has no non-destructor requests",
            )),
            ResourceKind::Activation => self.dispatch_activation(resource, context, request),
            ResourceKind::ActivationToken(object) => {
                self.dispatch_activation_token(resource, context, object, request)
            }
            ResourceKind::SessionLockManager => {
                self.dispatch_session_lock_manager(resource, context, request)
            }
            ResourceKind::SessionLock(object) => {
                self.dispatch_session_lock(resource, context, object, request)
            }
            ResourceKind::SessionLockSurface(surface) => {
                self.dispatch_session_lock_surface(surface, request)
            }
            ResourceKind::RelativePointerManager => {
                self.dispatch_relative_pointer_manager(resource, context, request)
            }
            ResourceKind::RelativePointer(_) => Err(NativeCompositorError::new(
                "relative-pointer object has no non-destructor requests",
            )),
            ResourceKind::XdgOutputManager => self.dispatch_xdg_output(resource, context, request),
            ResourceKind::XdgOutput(_, _) => Err(NativeCompositorError::new(
                "xdg-output has no non-destructor requests",
            )),
            ResourceKind::XwaylandKeyboardGrabManager => {
                self.dispatch_xwayland_keyboard_grab(resource, context, request)
            }
            ResourceKind::XwaylandKeyboardGrab(_) => Err(NativeCompositorError::new(
                "Xwayland grab has no non-destructor requests",
            )),
            ResourceKind::ShortcutInhibitManager => {
                self.dispatch_shortcut_inhibit(resource, context, request)
            }
            ResourceKind::ShortcutInhibitor(_) => Err(NativeCompositorError::new(
                "shortcut inhibitor has no non-destructor requests",
            )),
            ResourceKind::IdleInhibitManager => {
                self.dispatch_idle_inhibit_manager(resource, context, request)
            }
            ResourceKind::IdleInhibitor(_) => Err(NativeCompositorError::new(
                "idle-inhibitor object has no non-destructor requests",
            )),
            ResourceKind::PointerConstraints => {
                self.dispatch_pointer_constraints(resource, context, request)
            }
            ResourceKind::LockedPointer(object) => self.dispatch_locked_pointer(object, request),
            ResourceKind::ConfinedPointer(object) => {
                self.dispatch_confined_pointer(object, request)
            }
            ResourceKind::ExplicitSynchronization => {
                self.dispatch_explicit_synchronization(resource, context, request)
            }
            ResourceKind::SurfaceSynchronization(surface) => {
                self.dispatch_surface_synchronization(resource, context, surface, request)
            }
            ResourceKind::ExplicitBufferRelease(_) => Err(NativeCompositorError::new(
                "explicit buffer-release object has no requests",
            )),
            ResourceKind::Subcompositor => self.dispatch_subcompositor(resource, context, request),
            ResourceKind::Subsurface(surface) => self.dispatch_subsurface(surface, request),
            ResourceKind::XdgWmBase => self.dispatch_xdg_wm_base(resource, context, request),
            ResourceKind::XdgPositioner(object) => self.dispatch_xdg_positioner(object, request),
            ResourceKind::XdgSurface(surface) => {
                self.dispatch_xdg_surface(resource, context, surface, request)
            }
            ResourceKind::XdgToplevel(surface) => {
                self.dispatch_xdg_toplevel(context, surface, request)
            }
            ResourceKind::XdgPopup(surface) => self.dispatch_xdg_popup(context, surface, request),
        }
    }

    pub(super) fn resource_for_kind(
        &self,
        predicate: impl Fn(ResourceKind) -> bool,
    ) -> Result<Option<ResourceRef<'_>>, NativeCompositorError> {
        for identity in self.resources.values().copied() {
            let Some(resource) =
                (unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) })
            else {
                continue;
            };
            if predicate(self.resource_kind(resource)?) {
                return Ok(Some(resource));
            }
        }
        Ok(None)
    }

    pub(super) fn resource_for_object(
        &self,
        object: ProtocolObjectId,
    ) -> Result<Option<ResourceRef<'_>>, NativeCompositorError> {
        let Some(identity) = self.resources.get(&object).copied() else {
            return Ok(None);
        };
        let resource = unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) };
        if let Some(resource) = resource {
            let context_object = self.protocol_object_for_resource(resource)?;
            if context_object != object {
                return Err(NativeCompositorError::new(
                    "protocol resource identity is inconsistent",
                ));
            }
        }
        Ok(resource)
    }

    pub(super) fn resources_for_kind(
        &self,
        predicate: impl Fn(ResourceKind) -> bool,
    ) -> Result<Vec<ResourceRef<'_>>, NativeCompositorError> {
        let mut matches = Vec::new();
        for identity in self.resources.values().copied() {
            let Some(resource) =
                (unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) })
            else {
                continue;
            };
            if predicate(self.resource_kind(resource)?) {
                matches.push(resource);
            }
        }
        Ok(matches)
    }

    pub(super) fn resources_for_client(
        &self,
        client: ClientId,
        predicate: impl Fn(ResourceKind) -> bool,
    ) -> Result<Vec<ResourceRef<'_>>, NativeCompositorError> {
        let mut matches = Vec::new();
        for identity in self.resources.values().copied() {
            let Some(resource) =
                (unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) })
            else {
                continue;
            };
            let pointer = resource.user_data().cast::<ResourceContext>();
            if pointer.is_null() {
                continue;
            }
            let context = unsafe { &*pointer };
            if context.client == client && predicate(context.kind) {
                matches.push(resource);
            }
        }
        Ok(matches)
    }

    pub(super) fn post_event(
        &self,
        resource: ResourceRef<'_>,
        interface: &str,
        event: &str,
        arguments: &mut [ffi::wl_argument],
    ) -> Result<(), NativeCompositorError> {
        let (opcode, schema) = self
            .protocol
            .interface_schema(interface)
            .and_then(|interface| interface.event_named(event))
            .ok_or_else(|| NativeCompositorError::new(format!("missing {interface}.{event}")))?;
        if schema.arguments.len() != arguments.len() {
            return Err(NativeCompositorError::new("event argument count mismatch"));
        }
        if schema.since > resource.version() {
            return Err(NativeCompositorError::new(format!(
                "event {interface}.{event} requires version {}, resource has version {}",
                schema.since,
                resource.version()
            )));
        }
        unsafe { resource.post_event(opcode, arguments) };
        Ok(())
    }

    pub(super) fn resource_kind(
        &self,
        resource: ResourceRef<'_>,
    ) -> Result<ResourceKind, NativeCompositorError> {
        let context = resource.user_data().cast::<ResourceContext>();
        if context.is_null() {
            return Err(NativeCompositorError::new(
                "resource is not owned by Telorgon",
            ));
        }
        let context = unsafe { &*context };
        if !std::ptr::eq(context.state, self) {
            return Err(NativeCompositorError::new(
                "resource belongs to another compositor",
            ));
        }
        Ok(context.kind)
    }

    pub(super) fn protocol_object_for_resource(
        &self,
        resource: ResourceRef<'_>,
    ) -> Result<ProtocolObjectId, NativeCompositorError> {
        let context = resource.user_data().cast::<ResourceContext>();
        if context.is_null() {
            return Err(NativeCompositorError::new(
                "resource is not owned by Telorgon",
            ));
        }
        let context = unsafe { &*context };
        if !std::ptr::eq(context.state, self) {
            return Err(NativeCompositorError::new(
                "resource belongs to another compositor",
            ));
        }
        Ok(context.object)
    }

    pub(super) fn peek_next_object(&self) -> Result<ProtocolObjectId, NativeCompositorError> {
        let next = self
            .next_object
            .checked_add(1)
            .filter(|value| *value != 0)
            .ok_or_else(|| NativeCompositorError::new("protocol object identity exhausted"))?;
        Ok(ProtocolObjectId::from_raw(next).expect("nonzero"))
    }
}
