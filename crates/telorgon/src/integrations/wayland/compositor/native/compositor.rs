use super::*;

impl<'display> NativeCompositor<'display> {
    /// Enable Xwayland shell and keyboard-grab globals with a dedicated-client policy. This
    /// does not start Xwayland or expose X11 windows to desktop policy/rendering.
    pub fn new_with_xwayland(
        display: &'display Display,
        limits: ClientLimits,
        access: Rc<XwaylandAccess>,
    ) -> Result<Self, NativeCompositorError> {
        let interface = NativeProtocol::desktop()
            .interface("xwayland_shell_v1")
            .ok_or_else(|| NativeCompositorError::new("missing Xwayland shell descriptor"))?;
        if access.display.get() != display.native_handle().as_ptr() as usize {
            return Err(NativeCompositorError::new(
                "Xwayland filter was not configured on this display",
            ));
        }
        let mut compositor = Self::new(display, limits)?;
        compositor.state.xwayland = Some(access);
        let mut context = Box::new(BindContext {
            state: &mut *compositor.state,
            interface: "xwayland_shell_v1",
            kind: ResourceKind::XwaylandShell,
        });
        let global = unsafe {
            display.create_global(
                interface,
                1,
                (&mut *context as *mut BindContext).cast(),
                Some(bind_global),
            )
        }
        .map_err(error)?;
        compositor.bind_contexts.push(context);
        compositor.globals.push(global);
        compositor.add_dynamic_global(
            display,
            "zwp_xwayland_keyboard_grab_manager_v1",
            ResourceKind::XwaylandKeyboardGrabManager,
        )?;
        Ok(compositor)
    }
    pub fn new(
        display: &'display Display,
        limits: ClientLimits,
    ) -> Result<Self, NativeCompositorError> {
        let protocol = NativeProtocol::desktop();
        let mut state = Box::new(NativeState {
            xwayland: None,
            capture_access: None,
            foreign_toplevel: foreign_toplevel::NativeForeignToplevelState::default(),
            capture: capture::NativeCaptureState::default(),
            display: display.native_handle(),
            protocol,
            core: CompositorCore::new(limits).map_err(error)?,
            output_revision: 0,
            retired_outputs: BTreeSet::new(),
            manual_outputs: BTreeSet::new(),
            clients: BTreeMap::new(),
            resources: BTreeMap::new(),
            mapped_outputs: BTreeSet::new(),
            surface_outputs: BTreeMap::new(),
            entered_outputs: BTreeSet::new(),
            regions: BTreeMap::new(),
            shm_pools: BTreeMap::new(),
            buffer_files: BTreeMap::new(),
            dmabuf_files: BTreeMap::new(),
            destroyed_buffers: BTreeMap::new(),
            callbacks: BTreeMap::new(),
            committed_callbacks: BTreeMap::new(),
            timing_observer: None,
            pending_presentation_feedbacks: BTreeMap::new(),
            committed_presentation_feedbacks: BTreeMap::new(),
            xdg_resources: BTreeMap::new(),
            toplevels: BTreeMap::new(),
            requested_toplevel_states: BTreeMap::new(),
            decoration_policy: crate::DecorationPolicy::DEFAULT,
            decorations: BTreeMap::new(),
            committed_decorations: BTreeMap::new(),
            toplevel_icons: BTreeMap::new(),
            pending_toplevel_icons: BTreeMap::new(),
            committed_toplevel_icons: BTreeMap::new(),
            positioners: BTreeMap::new(),
            popups: BTreeMap::new(),
            viewports: BTreeMap::new(),
            dmabuf_formats: Vec::new(),
            dmabuf_feedback: None,
            dmabuf_params: BTreeMap::new(),
            keyboard_keymaps: BTreeMap::new(),
            touch_points: BTreeMap::new(),
            active_drag: None,
            finished_drag_sources: BTreeSet::new(),
            xwayland_keyboard_grabs: BTreeMap::new(),
            shortcut_inhibitors: BTreeMap::new(),
            revoked_shortcuts: BTreeSet::new(),
            idle_inhibitors: BTreeMap::new(),
            pointer_constraints: BTreeMap::new(),
            pointer_capture_releases: BTreeMap::new(),
            suspended_focus: BTreeMap::new(),
            pointer_press_serials: BTreeMap::new(),
            activation_tokens: BTreeMap::new(),
            activation_grants: BTreeMap::new(),
            activation_order: VecDeque::new(),
            session_locks: BTreeMap::new(),
            session_lock_surfaces: BTreeMap::new(),
            active_session_lock: None,
            secure_session_locked: false,
            synchronized_surfaces: BTreeSet::new(),
            pending_acquire_fences: BTreeMap::new(),
            pending_releases: BTreeMap::new(),
            committed_acquire_fences: BTreeMap::new(),
            committed_releases: BTreeMap::new(),
            initial_configures: BTreeSet::new(),
            next_client: 0,
            clipboard: None,
            next_object: 0,
            next_surface: 0,
            next_buffer: 0,
            presentation_sequence: 0,
            toplevel_icon_revision: 0,
        });
        let state_pointer = (&mut *state) as *mut NativeState;
        let mut bind_contexts = Vec::new();
        let mut globals = Vec::new();
        for (interface_name, kind, maximum_version) in IMPLEMENTED_GLOBALS {
            let interface = state
                .protocol
                .interface(interface_name)
                .ok_or_else(|| NativeCompositorError::new(format!("missing {interface_name}")))?;
            let advertised =
                crate::integrations::wayland::server::protocol::interface(interface_name)
                    .map(|profile| profile.advertised_version)
                    .unwrap_or(1)
                    .min(interface.version as u32)
                    .min(*maximum_version);
            let mut bind = Box::new(BindContext {
                state: state_pointer,
                interface: interface_name,
                kind: *kind,
            });
            let data = (&mut *bind as *mut BindContext).cast::<c_void>();
            let global =
                unsafe { display.create_global(interface, advertised, data, Some(bind_global)) }
                    .map_err(error)?;
            bind_contexts.push(bind);
            globals.push(global);
        }
        Ok(Self {
            state,
            bind_contexts,
            globals,
        })
    }

    pub(crate) fn x11_surface_scale(&self, surface: WaylandSurfaceId) -> i32 {
        let Some(access) = &self.state.xwayland else {
            return 1;
        };
        let owner = self.state.core.world.surface_owner(surface);
        if self
            .state
            .clients
            .iter()
            .any(|(identity, client)| Some(*client) == owner && access.allows(*identity))
        {
            access.coordinate_scale()
        } else {
            1
        }
    }
    pub(crate) fn surface_process_id(&self, surface: WaylandSurfaceId) -> Option<u32> {
        self.state.resource_for_kind(|kind| {
            matches!(kind, ResourceKind::Surface(candidate) if candidate == surface)
        }).ok().flatten().and_then(|resource| {
            u32::try_from(resource.client().credentials().pid).ok()
        })
    }

    pub fn core(&self) -> &CompositorCore {
        &self.state.core
    }

    pub fn core_mut(&mut self) -> &mut CompositorCore {
        &mut self.state.core
    }

    pub fn advertised_globals(&self) -> usize {
        debug_assert_eq!(self.bind_contexts.len(), self.globals.len());
        self.bind_contexts.iter().filter(|context| {
            !matches!(context.kind, ResourceKind::Output(id) if self.state.retired_outputs.contains(&id))
        }).count()
    }

    pub fn add_output(
        &mut self,
        display: &'display Display,
        id: u32,
        output: crate::integrations::wayland::compositor::OutputState,
    ) -> Result<(), NativeCompositorError> {
        output.description.clone().validate().map_err(error)?;
        if self.state.core.outputs.len() + self.state.retired_outputs.len()
            >= outputs::MAX_OUTPUT_GLOBALS
        {
            return Err(NativeCompositorError::new(
                "output global lifetime limit reached",
            ));
        }
        if id == 0
            || self.state.retired_outputs.contains(&id)
            || output.current_mode >= output.description.modes.len()
            || self.state.core.outputs.contains_key(&id)
            || self
                .state
                .core
                .outputs
                .values()
                .any(|existing| existing.description.name == output.description.name)
        {
            return Err(NativeCompositorError::new(
                "invalid or duplicate output identity/name",
            ));
        }
        let revision = self
            .state
            .output_revision
            .checked_add(1)
            .ok_or_else(|| NativeCompositorError::new("output layout revision exhausted"))?;
        self.state.core.outputs.insert(id, output);
        if let Err(error) = self.add_dynamic_global(display, "wl_output", ResourceKind::Output(id))
        {
            self.state.core.outputs.remove(&id);
            return Err(error);
        }
        self.state.output_revision = revision;
        Ok(())
    }

    /// Publish a changed registered output as one owner-thread update. Names,
    /// available modes and enabled state belong to global reconstruction and
    /// cannot change here. Returns false without events for an identical snapshot.
    pub fn update_output(
        &mut self,
        id: u32,
        output: crate::integrations::wayland::compositor::OutputState,
    ) -> Result<bool, NativeCompositorError> {
        self.update_outputs([(id, output)])
    }

    pub fn output_snapshot(
        &self,
    ) -> crate::integrations::wayland::compositor::OutputLayoutSnapshot {
        crate::integrations::wayland::compositor::OutputLayoutSnapshot::new(
            self.state.output_revision,
            self.state.core.outputs.clone(),
        )
    }

    /// Validate every replacement before changing any output. Successful changed
    /// batches advance the revision once, even when several outputs move together.
    /// Callers must use these APIs rather than mutate core.outputs directly.
    pub fn update_outputs(
        &mut self,
        replacements: impl IntoIterator<
            Item = (u32, crate::integrations::wayland::compositor::OutputState),
        >,
    ) -> Result<bool, NativeCompositorError> {
        let mut pending = BTreeMap::new();
        let mut changed = BTreeSet::new();
        for (id, output) in replacements {
            output.description.clone().validate().map_err(error)?;
            let previous = self
                .state
                .core
                .outputs
                .get(&id)
                .ok_or_else(|| NativeCompositorError::new("unknown output"))?;
            if output.current_mode >= output.description.modes.len()
                || output.description.name != previous.description.name
                || output.description.modes != previous.description.modes
                || output.enabled != previous.enabled
            {
                return Err(NativeCompositorError::new(
                    "output change requires global reconstruction",
                ));
            }
            if previous != &output {
                changed.insert(id);
            }
            if pending.insert(id, output).is_some() {
                return Err(NativeCompositorError::new(
                    "duplicate output in layout update",
                ));
            }
        }
        if changed.is_empty() {
            return Ok(false);
        }
        let revision = self
            .state
            .output_revision
            .checked_add(1)
            .ok_or_else(|| NativeCompositorError::new("output layout revision exhausted"))?;
        let previous_scale = self
            .state
            .core
            .outputs
            .values()
            .find(|output| output.enabled)
            .map(|output| output.description.scale);
        self.state.core.outputs.extend(pending);
        self.state.refresh_capture_sources()?;
        self.state.output_revision = revision;
        let current_scale = self
            .state
            .core
            .outputs
            .values()
            .find(|output| output.enabled)
            .map(|output| output.description.scale);
        let preferred_scale_changed = previous_scale != current_scale;
        let outputs = self.state.resources_for_kind(
            |kind| matches!(kind, ResourceKind::Output(candidate) if changed.contains(&candidate)),
        )?;
        for resource in &outputs {
            let ResourceKind::Output(id) = self.state.resource_kind(*resource)? else {
                unreachable!()
            };
            self.state
                .send_output_description(*resource, id, false, false)?;
        }
        for child in self.state.resources_for_kind(
            |kind| matches!(kind, ResourceKind::XdgOutput(candidate, _) if changed.contains(&candidate)),
        )? {
            let ResourceKind::XdgOutput(id, parent) = self.state.resource_kind(child)? else {
                unreachable!()
            };
            let parent_done = self
                .state
                .resource_for_object(parent)?
                .is_some_and(|parent| parent.version() >= 2);
            self.state
                .send_xdg_output_description(child, id, false, parent_done)?;
        }
        // Complete only after all core and logical events for this snapshot.
        for resource in outputs {
            if resource.version() >= 2 {
                self.state
                    .post_event(resource, "wl_output", "done", &mut [])?;
            }
        }
        if preferred_scale_changed {
            self.state
                .send_preferred_output_scale(current_scale.map_or(1.0, |scale| scale.get()))?;
        }
        Ok(true)
    }

    pub fn add_seat(
        &mut self,
        display: &'display Display,
        id: u32,
        seat: crate::integrations::wayland::compositor::SeatState,
    ) -> Result<(), NativeCompositorError> {
        if id == 0 || self.state.core.seats.contains_key(&id) {
            return Err(NativeCompositorError::new(
                "invalid or duplicate seat identity",
            ));
        }
        self.state.core.seats.insert(id, seat);
        self.add_dynamic_global(display, "wl_seat", ResourceKind::Seat(id))
    }

    pub fn add_linux_dmabuf(
        &mut self,
        display: &'display Display,
        formats: Vec<DmaBufFormat>,
    ) -> Result<(), NativeCompositorError> {
        self.add_linux_dmabuf_inner(display, formats, None)
    }

    /// Advertise DMA-BUF v4 with immutable default and per-surface allocation feedback.
    /// `main_device` must be the DRM device whose renderer supplied `formats`, encoded as dev_t.
    /// Both primary and render node identities are accepted by the protocol. This single-device
    /// profile advertises sampling only, never direct scanout. Capabilities cannot change in place.
    pub fn add_linux_dmabuf_with_feedback(
        &mut self,
        display: &'display Display,
        formats: Vec<DmaBufFormat>,
        main_device: libc::dev_t,
    ) -> Result<(), NativeCompositorError> {
        self.add_linux_dmabuf_inner(display, formats, Some(main_device))
    }

    fn add_linux_dmabuf_inner(
        &mut self,
        display: &'display Display,
        mut formats: Vec<DmaBufFormat>,
        main_device: Option<libc::dev_t>,
    ) -> Result<(), NativeCompositorError> {
        if !self.state.dmabuf_formats.is_empty() {
            return Err(NativeCompositorError::new(
                "DMA-BUF global is already configured",
            ));
        }
        formats.sort_unstable_by_key(|format| (format.fourcc, format.modifier));
        formats.dedup();
        if formats.is_empty() {
            return Err(NativeCompositorError::new(
                "DMA-BUF cannot be advertised without an importable format",
            ));
        }
        let feedback = main_device
            .map(|device| DmaBufFeedback::new(device, &formats))
            .transpose()?;
        self.add_dynamic_global_version(
            display,
            "zwp_linux_dmabuf_v1",
            ResourceKind::LinuxDmaBuf,
            if feedback.is_some() { 4 } else { 3 },
        )?;
        self.state.dmabuf_formats = formats;
        self.state.dmabuf_feedback = feedback;
        Ok(())
    }

    pub fn add_explicit_synchronization(
        &mut self,
        display: &'display Display,
    ) -> Result<(), NativeCompositorError> {
        self.add_dynamic_global_version(
            display,
            "zwp_linux_explicit_synchronization_v1",
            ResourceKind::ExplicitSynchronization,
            2,
        )
    }

    pub fn take_acquire_fence(
        &mut self,
        surface: WaylandSurfaceId,
        revision: u64,
    ) -> Option<OwnedFd> {
        self.state
            .committed_acquire_fences
            .remove(&(surface, revision))
    }

    pub fn finish_explicit_release(
        &mut self,
        surface: WaylandSurfaceId,
        revision: u64,
        fence: Option<OwnedFd>,
    ) -> Result<bool, NativeCompositorError> {
        let identity = self
            .state
            .finish_explicit_release(surface, revision, fence)?;
        if let Some(identity) = identity
            && let Some(resource) =
                unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) }
        {
            unsafe { resource.destroy() };
        }
        Ok(identity.is_some())
    }

    pub fn read_dma_buf(
        &self,
        buffer: WaylandBufferId,
    ) -> Result<DmaBufImage, NativeCompositorError> {
        let BufferDescriptor::DmaBuf(descriptor) = self
            .state
            .core
            .buffer(buffer)
            .ok_or_else(|| NativeCompositorError::new("unknown Wayland buffer"))?
        else {
            return Err(NativeCompositorError::new("buffer is not a DMA-BUF"));
        };
        let planes = self
            .state
            .dmabuf_files
            .get(&buffer)
            .ok_or_else(|| NativeCompositorError::new("DMA-BUF plane storage is absent"))?
            .iter()
            .map(OwnedFd::try_clone)
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        Ok(DmaBufImage {
            descriptor: descriptor.clone(),
            planes,
        })
    }

    fn add_dynamic_global(
        &mut self,
        display: &'display Display,
        interface_name: &'static str,
        kind: ResourceKind,
    ) -> Result<(), NativeCompositorError> {
        let version = self
            .state
            .protocol
            .interface(interface_name)
            .ok_or_else(|| NativeCompositorError::new(format!("missing {interface_name}")))?
            .version as u32;
        self.add_dynamic_global_version(display, interface_name, kind, version)
    }

    fn add_dynamic_global_version(
        &mut self,
        display: &'display Display,
        interface_name: &'static str,
        kind: ResourceKind,
        maximum_version: u32,
    ) -> Result<(), NativeCompositorError> {
        let state_pointer = (&mut *self.state) as *mut NativeState;
        let interface = self
            .state
            .protocol
            .interface(interface_name)
            .ok_or_else(|| NativeCompositorError::new(format!("missing {interface_name}")))?;
        let advertised = crate::integrations::wayland::server::protocol::interface(interface_name)
            .map(|profile| profile.advertised_version)
            .unwrap_or(1)
            .min(interface.version as u32)
            .min(maximum_version);
        let mut bind = Box::new(BindContext {
            state: state_pointer,
            interface: interface_name,
            kind,
        });
        let data = (&mut *bind as *mut BindContext).cast::<c_void>();
        let global =
            unsafe { display.create_global(interface, advertised, data, Some(bind_global)) }
                .map_err(error)?;
        self.bind_contexts.push(bind);
        self.globals.push(global);
        Ok(())
    }

    pub fn duplicate_shm_fd(
        &self,
        buffer: WaylandBufferId,
    ) -> Result<OwnedFd, NativeCompositorError> {
        self.state
            .buffer_files
            .get(&buffer)
            .ok_or_else(|| NativeCompositorError::new("buffer is not backed by shared memory"))?
            .try_clone()
            .map_err(error)
    }

    /// Captures immutable SHM metadata plus a duplicated FD for copying outside protocol state.
    pub fn shm_buffer_reader(
        &self,
        buffer: WaylandBufferId,
    ) -> Result<ShmBufferReader, NativeCompositorError> {
        let BufferDescriptor::Shm(descriptor) = self
            .state
            .core
            .buffer(buffer)
            .ok_or_else(|| NativeCompositorError::new("unknown Wayland buffer"))?
        else {
            return Err(NativeCompositorError::new("buffer is not shared memory"));
        };
        Ok(ShmBufferReader {
            descriptor: *descriptor,
            file: std::fs::File::from(self.duplicate_shm_fd(buffer)?),
        })
    }

    /// Copies one committed SHM buffer into host-owned bytes suitable for a Telorgon image resource.
    pub fn read_shm_buffer(
        &self,
        buffer: WaylandBufferId,
    ) -> Result<ShmImage, NativeCompositorError> {
        self.shm_buffer_reader(buffer)?.read_full()
    }

    /// Copies one buffer-local rectangle from a committed SHM buffer into tightly packed rows.
    pub fn read_shm_buffer_region(
        &self,
        buffer: WaylandBufferId,
        rect: RectI,
    ) -> Result<ShmImageRegion, NativeCompositorError> {
        self.shm_buffer_reader(buffer)?.read_region(rect)
    }

    pub fn configure_toplevel(
        &mut self,
        surface: WaylandSurfaceId,
        size: Option<crate::foundation::SizeI>,
        states: crate::integrations::wayland::compositor::ToplevelState,
    ) -> Result<u32, NativeCompositorError> {
        self.state.send_toplevel_configure(surface, size, states)
    }

    pub fn close_toplevel(&self, surface: WaylandSurfaceId) -> Result<(), NativeCompositorError> {
        let resource = self
            .state
            .resource_for_kind(
                |kind| matches!(kind, ResourceKind::XdgToplevel(candidate) if candidate == surface),
            )?
            .ok_or_else(|| NativeCompositorError::new("xdg_toplevel resource is absent"))?;
        self.state
            .post_event(resource, "xdg_toplevel", "close", &mut [])
    }

    /// Live xdg toplevels, including windows showing an unsaved-document prompt during logout.
    pub fn toplevel_surfaces(&self) -> Vec<WaylandSurfaceId> {
        self.state.toplevels.keys().copied().collect()
    }

    pub fn surface_presented(
        &mut self,
        surface: WaylandSurfaceId,
        through_revision: u64,
        time_milliseconds: u32,
    ) -> Result<(), NativeCompositorError> {
        let identities = self.state.surface_frame_completed(
            surface,
            through_revision,
            time_milliseconds,
            true,
        )?;
        for identity in identities {
            if let Some(resource) =
                unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) }
            {
                unsafe { resource.destroy() };
            }
        }
        Ok(())
    }

    /// Lets an occluded client draw again without claiming its hidden content was presented.
    /// Frame callbacks are pacing hints; presentation feedback stays pending until a displayed
    /// frame either consumes or supersedes it, so older in-flight frames can still report truthfully.
    pub(crate) fn surface_occluded_frame_ready(
        &mut self,
        surface: WaylandSurfaceId,
        through_revision: u64,
        time_milliseconds: u32,
    ) -> Result<(), NativeCompositorError> {
        let identities = self.state.surface_frame_completed(
            surface,
            through_revision,
            time_milliseconds,
            false,
        )?;
        for identity in identities {
            if let Some(resource) =
                unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) }
            {
                unsafe { resource.destroy() };
            }
        }
        Ok(())
    }

    pub fn release_buffer(&self, buffer: WaylandBufferId) -> Result<(), NativeCompositorError> {
        let Some(resource) = self.state.resource_for_kind(
            |kind| matches!(kind, ResourceKind::Buffer(candidate) if candidate == buffer),
        )?
        else {
            // A client may destroy wl_buffer after attach. The duplicated storage FD remains valid,
            // but there is no live protocol object to receive release once the copy completes.
            return Ok(());
        };
        self.state
            .post_event(resource, "wl_buffer", "release", &mut [])
    }

    pub fn popup_placement(
        &self,
        surface: WaylandSurfaceId,
    ) -> Option<(Option<WaylandSurfaceId>, RectI)> {
        self.state
            .popups
            .get(&surface)
            .map(|popup| (popup.parent, popup_geometry(popup.positioner)))
    }

    /// Shell startup configuration; set before accepting client toplevels.
    pub(crate) fn tiled_client_decorations(&self) -> bool {
        self.state.decoration_policy.tiled_client_decorations
    }

    pub(crate) fn set_decoration_policy(&mut self, policy: crate::DecorationPolicy) {
        self.state.decoration_policy = policy;
    }



    pub fn decoration_mode(
        &self,
        surface: WaylandSurfaceId,
    ) -> Option<crate::integrations::wayland::compositor::DecorationMode> {
        self.state.toplevels.get(&surface).map(|_| {
            self.state
                .committed_decorations
                .get(&surface)
                .copied()
                .unwrap_or(crate::integrations::wayland::compositor::DecorationMode::ClientSide)
        })
    }

    /// Returns client-authored metadata used to compose server-side window chrome.
    pub fn toplevel_metadata(
        &self,
        surface: WaylandSurfaceId,
    ) -> Option<&crate::integrations::wayland::compositor::XdgToplevelState> {
        self.state.toplevels.get(&surface)
    }

    /// Returns the icon snapshot applied by the latest `wl_surface.commit` for a toplevel.
    pub fn toplevel_icon(&self, surface: WaylandSurfaceId) -> Option<&ToplevelIconSnapshot> {
        self.state.committed_toplevel_icons.get(&surface)
    }

    pub(crate) fn surface_logical_size(
        &self,
        surface: WaylandSurfaceId,
    ) -> Result<crate::foundation::SizeI, NativeCompositorError> {
        self.state.surface_logical_size(surface)
    }

    pub fn viewport(&self, surface: WaylandSurfaceId) -> Option<ViewportState> {
        self.state
            .viewports
            .get(&surface)
            .map(|viewport| viewport.current)
    }

    /// Completes a pending secure-lock transition after a blank/lock-only frame has reached every
    /// active KMS output.
    pub fn session_lock_frame_presented(
        &mut self,
        lock: ProtocolObjectId,
    ) -> Result<(), NativeCompositorError> {
        self.state.session_lock_frame_presented(lock)
    }

    pub fn session_locked(&self) -> bool {
        self.state.secure_session_locked
    }
}
