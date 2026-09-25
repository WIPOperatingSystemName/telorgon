use super::*;

impl NativeState {
    pub(super) fn dispatch_session_lock_manager(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "lock" {
            return Err(unsupported_request(request));
        }
        let object = self.peek_next_object()?;
        let lock_resource = self.create_resource(
            resource.client(),
            context.client,
            "ext_session_lock_v1",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::SessionLock(object),
            true,
        )?;
        let denied = self.active_session_lock.is_some();
        self.session_locks.insert(
            object,
            NativeSessionLock {
                client: context.client,
                locked_event_sent: false,
                finished_event_sent: denied,
            },
        );
        if denied {
            self.post_event(lock_resource, "ext_session_lock_v1", "finished", &mut [])?;
        } else {
            self.active_session_lock = Some(object);
            self.clipboard_turn(true);
            self.refresh_capture_sources()?;
            self.core
                .queue_action(CompositorAction::SessionLockRequested(object));
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_session_lock(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        object: ProtocolObjectId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        let lock = self
            .session_locks
            .get(&object)
            .ok_or_else(|| NativeCompositorError::new("unknown session lock"))?;
        if lock.client != context.client {
            return Err(NativeCompositorError::new(
                "session lock belongs to another client",
            ));
        }
        match request.message().name {
            "destroy" => {
                if lock.locked_event_sent {
                    return Err(NativeCompositorError::new(
                        "locked session must use unlock_and_destroy",
                    ));
                }
                if self.active_session_lock == Some(object) {
                    self.active_session_lock = None;
                    self.core
                        .queue_action(CompositorAction::SessionLockCancelled(object));
                }
                Ok(DispatchOutcome {
                    destroy_self: true,
                    ..DispatchOutcome::default()
                })
            }
            "unlock_and_destroy" => {
                if !lock.locked_event_sent {
                    return Err(NativeCompositorError::new(
                        "session cannot unlock before the locked event",
                    ));
                }
                if self.active_session_lock == Some(object) {
                    self.active_session_lock = None;
                }
                self.secure_session_locked = false;
                self.core
                    .queue_action(CompositorAction::SessionUnlockRequested(object));
                Ok(DispatchOutcome {
                    destroy_self: true,
                    ..DispatchOutcome::default()
                })
            }
            "get_lock_surface" => {
                if lock.finished_event_sent {
                    return Err(NativeCompositorError::new(
                        "finished session lock cannot create surfaces",
                    ));
                }
                let surface = self.surface_from_resource(
                    request
                        .object(1)
                        .map_err(error)?
                        .ok_or_else(|| NativeCompositorError::new("missing lock surface"))?,
                )?;
                let output_resource = request
                    .object(2)
                    .map_err(error)?
                    .ok_or_else(|| NativeCompositorError::new("missing lock output"))?;
                let ResourceKind::Output(output) = self.resource_kind(output_resource)? else {
                    return Err(NativeCompositorError::new(
                        "session-lock target is not a wl_output",
                    ));
                };
                if self
                    .session_lock_surfaces
                    .values()
                    .any(|candidate| candidate.lock == object && candidate.output == output)
                {
                    return Err(NativeCompositorError::new(
                        "session lock already has a surface for this output",
                    ));
                }
                let candidate = self
                    .core
                    .world
                    .surface(surface)
                    .ok_or_else(|| NativeCompositorError::new("unknown lock wl_surface"))?;
                if candidate.snapshot().role.is_some() {
                    return Err(NativeCompositorError::new(
                        "session-lock wl_surface already has a role",
                    ));
                }
                if candidate.snapshot().revision != 1
                    || candidate.snapshot().attachment.is_some()
                    || candidate.pending().attachment.is_some()
                {
                    return Err(NativeCompositorError::new(
                        "session-lock wl_surface was already constructed",
                    ));
                }
                self.surface_mut(surface)?
                    .assign_role(SurfaceRole::SessionLock)
                    .map_err(error)?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "ext_session_lock_surface_v1",
                    1,
                    request.new_id(0).map_err(error)?,
                    ResourceKind::SessionLockSurface(surface),
                    true,
                )?;
                self.session_lock_surfaces.insert(
                    surface,
                    NativeSessionLockSurface {
                        lock: object,
                        output,
                        pending_configures: VecDeque::new(),
                        last_acked: None,
                    },
                );
                self.send_session_lock_configure(surface)?;
                Ok(DispatchOutcome::default())
            }
            _ => Err(unsupported_request(request)),
        }
    }

    pub(super) fn dispatch_session_lock_surface(
        &mut self,
        surface: WaylandSurfaceId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "ack_configure" {
            return Err(unsupported_request(request));
        }
        let serial = request.uint(0).map_err(error)?;
        let lock_surface = self
            .session_lock_surfaces
            .get_mut(&surface)
            .ok_or_else(|| NativeCompositorError::new("unknown session-lock surface"))?;
        if lock_surface
            .last_acked
            .is_some_and(|(acked, _)| acked == serial)
        {
            return Err(NativeCompositorError::new(
                "session-lock configure serial was already acknowledged",
            ));
        }
        let index = lock_surface
            .pending_configures
            .iter()
            .position(|(candidate, _)| *candidate == serial)
            .ok_or_else(|| NativeCompositorError::new("invalid session-lock configure serial"))?;
        let acknowledged = lock_surface.pending_configures[index];
        lock_surface.pending_configures.drain(..=index);
        lock_surface.last_acked = Some(acknowledged);
        Ok(DispatchOutcome::default())
    }

    pub(super) fn send_session_lock_configure(
        &mut self,
        surface: WaylandSurfaceId,
    ) -> Result<(), NativeCompositorError> {
        let output = self
            .session_lock_surfaces
            .get(&surface)
            .ok_or_else(|| NativeCompositorError::new("unknown session-lock surface"))?
            .output;
        if self.retired_outputs.contains(&output) {
            return Ok(());
        }
        let size = self.output_logical_size(output)?;
        let serial = unsafe { ffi::wl_display_next_serial(self.display.as_ptr()) };
        if serial == 0 {
            return Err(NativeCompositorError::new(
                "libwayland returned a zero lock configure serial",
            ));
        }
        let lock_surface = self
            .session_lock_surfaces
            .get_mut(&surface)
            .expect("checked above");
        while lock_surface.pending_configures.len() >= 64 {
            lock_surface.pending_configures.pop_front();
        }
        lock_surface.pending_configures.push_back((serial, size));
        let resource = self
            .resource_for_kind(
                |kind| matches!(kind, ResourceKind::SessionLockSurface(candidate) if candidate == surface),
            )?
            .ok_or_else(|| NativeCompositorError::new("lock-surface resource is absent"))?;
        self.post_event(
            resource,
            "ext_session_lock_surface_v1",
            "configure",
            &mut [
                ffi::wl_argument { u: serial },
                ffi::wl_argument {
                    u: size.width as u32,
                },
                ffi::wl_argument {
                    u: size.height as u32,
                },
            ],
        )
    }

    pub(super) fn session_lock_frame_presented(
        &mut self,
        object: ProtocolObjectId,
    ) -> Result<(), NativeCompositorError> {
        if self.active_session_lock != Some(object) {
            return Err(NativeCompositorError::new(
                "presented session lock is not active",
            ));
        }
        let lock = self
            .session_locks
            .get(&object)
            .ok_or_else(|| NativeCompositorError::new("session lock object is absent"))?;
        if lock.finished_event_sent {
            return Err(NativeCompositorError::new("session lock was denied"));
        }
        if lock.locked_event_sent {
            return Ok(());
        }
        let resource = self
            .resource_for_kind(
                |kind| matches!(kind, ResourceKind::SessionLock(candidate) if candidate == object),
            )?
            .ok_or_else(|| NativeCompositorError::new("session lock resource is absent"))?;
        self.post_event(resource, "ext_session_lock_v1", "locked", &mut [])?;
        self.session_locks
            .get_mut(&object)
            .expect("checked above")
            .locked_event_sent = true;
        self.secure_session_locked = true;
        Ok(())
    }
}
