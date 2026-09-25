use super::*;

impl NativeState {
    pub(super) fn destroy_context(&mut self, context: &ResourceContext) {
        match context.kind {
            ResourceKind::ForeignToplevelList => {
                self.foreign_toplevel.lists.remove(&context.object);
            }
            ResourceKind::ForeignToplevelHandle => {
                self.foreign_toplevel.handles.remove(&context.object);
            }
            ResourceKind::ImageCopyCaptureSession(id) => {
                self.capture.sessions.remove(&id);
            }
            ResourceKind::ImageCopyCaptureFrame(id) => {
                self.capture.frames.remove(&id);
            }
            _ => {}
        }
        self.resources.remove(&context.object);
        self.entered_outputs
            .retain(|(_, output)| *output != context.object);
        if let ResourceKind::Surface(surface) = context.kind {
            self.mapped_outputs.remove(&surface);
            self.surface_outputs.remove(&surface);
            self.entered_outputs
                .retain(|(candidate, _)| *candidate != surface);
        }
        let abort_drag = self
            .active_drag
            .as_ref()
            .is_some_and(|drag| match context.kind {
                ResourceKind::DataSource(source) => drag.source == Some(source),
                ResourceKind::Surface(surface) => {
                    drag.origin == surface
                        || drag
                            .target
                            .as_ref()
                            .is_some_and(|target| target.surface == surface)
                }
                _ => false,
            });
        if abort_drag && let Some(drag) = self.active_drag.take() {
            if let Some(target) = &drag.target {
                let _ = self.send_drag_leave(target, drag.source);
            }
            if !matches!(context.kind, ResourceKind::DataSource(_))
                && let Some(source) = drag.source
            {
                let _ = self.cancel_data_source(source);
            }
            self.finish_drag(drag.icon);
        }
        if let ResourceKind::Surface(surface) = context.kind
            && let Some(drag) = self.active_drag.as_mut()
            && drag.icon == Some(surface)
        {
            drag.icon = None;
        }
        match context.kind {
            ResourceKind::Surface(surface) => {
                self.revoke_suspended_focus(surface);
                self.pointer_press_serials
                    .retain(|_, (_, focus)| focus.surface != surface);
                self.callbacks.remove(&surface);
                self.committed_callbacks
                    .retain(|(candidate, _), _| *candidate != surface);
                self.pending_presentation_feedbacks.remove(&surface);
                self.committed_presentation_feedbacks
                    .retain(|(candidate, _), _| *candidate != surface);
                self.initial_configures.remove(&surface);
                self.xdg_resources.remove(&surface);
                self.decorations.remove(&surface);
                self.toplevels.remove(&surface);
                self.requested_toplevel_states.remove(&surface);
                self.committed_decorations.remove(&surface);
                self.pending_toplevel_icons.remove(&surface);
                self.committed_toplevel_icons.remove(&surface);
                self.viewports.remove(&surface);
                self.synchronized_surfaces.remove(&surface);
                self.pending_acquire_fences.remove(&surface);
                self.pending_releases.remove(&surface);
                // Committed uses belong to the renderer/queued publication, not to
                // wl_surface's lifetime. Retire their fences and release objects
                // through the normal completion path, including after destruction.
                self.core.buffer_uses.cancel_surface(surface);
                self.touch_points
                    .retain(|_, point| point.surface != surface);
                self.idle_inhibitors
                    .retain(|_, candidate| *candidate != surface);
                self.xwayland_keyboard_grabs
                    .retain(|_, (_, target, _)| *target != surface);
                self.shortcut_inhibitors
                    .retain(|_, (_, target, _)| *target != surface);
                self.revoked_shortcuts
                    .retain(|(_, target)| *target != surface);
                self.pointer_constraints
                    .retain(|_, constraint| constraint.surface != surface);
                self.pointer_capture_releases
                    .retain(|_, released| *released != surface);
                self.session_lock_surfaces.remove(&surface);
                let _ = self.core.destroy_surface(context.client, surface);
            }
            ResourceKind::Region(object) => {
                self.regions.remove(&object);
            }
            ResourceKind::ShmPool(object) => {
                self.shm_pools.remove(&object);
            }
            ResourceKind::Buffer(buffer) => {
                let affected = self
                    .toplevel_icons
                    .iter()
                    .filter(|(_, icon)| icon.buffers.values().any(|candidate| *candidate == buffer))
                    .map(|(object, _)| *object)
                    .collect::<Vec<_>>();
                for object in affected {
                    if let Some(identity) = self.resources.get(&object).copied()
                        && let Some(icon) =
                            unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) }
                    {
                        icon.post_error(3, "an icon wl_buffer was destroyed before its icon");
                    }
                }
                self.destroyed_buffers.insert(buffer, context.client);
            }
            ResourceKind::LinuxBufferParams(object) => {
                self.dmabuf_params.remove(&object);
            }
            ResourceKind::DataSource(source) | ResourceKind::PrimarySource(source) => {
                self.finished_drag_sources.remove(&source);
                if self.core.data_devices.remove_source(source) {
                    let focused = self
                        .core
                        .seats
                        .iter()
                        .filter_map(|(seat, state)| {
                            state.keyboard_focus.map(|focus| (*seat, focus.client))
                        })
                        .collect::<Vec<_>>();
                    for (seat, client) in focused {
                        let _ = self.send_selection_to_client(seat, client);
                    }
                }
            }
            ResourceKind::DataOffer(offer) | ResourceKind::PrimaryOffer(offer) => {
                self.core.data_devices.remove_offer(offer);
            }
            ResourceKind::Subsurface(surface) => {
                let _ = self.core.subsurfaces.remove(surface);
            }
            ResourceKind::XdgSurface(surface) => {
                self.xdg_resources.remove(&surface);
            }
            ResourceKind::ToplevelDecoration(surface) | ResourceKind::KdeDecoration(surface) => {
                self.remove_decoration(surface, context.object);
            }
            ResourceKind::XdgToplevel(surface) => {
                if let Some(object) = self.decorations.get(&surface).and_then(|state| state.xdg) {
                    self.remove_decoration(surface, object);
                }
                self.toplevels.remove(&surface);
                self.requested_toplevel_states.remove(&surface);
                self.committed_decorations.remove(&surface);
                self.pending_toplevel_icons.remove(&surface);
                self.committed_toplevel_icons.remove(&surface);
            }
            ResourceKind::ToplevelIcon(object) => {
                self.toplevel_icons.remove(&object);
            }
            ResourceKind::XdgPositioner(object) => {
                self.positioners.remove(&object);
            }
            ResourceKind::XdgPopup(surface) => {
                self.popups.remove(&surface);
            }
            ResourceKind::Viewport(surface) => {
                self.viewports.remove(&surface);
            }
            ResourceKind::PresentationFeedback(surface) => {
                if let Some(feedbacks) = self.pending_presentation_feedbacks.get_mut(&surface) {
                    feedbacks.retain(|object| *object != context.object);
                }
                for ((candidate, _), feedbacks) in &mut self.committed_presentation_feedbacks {
                    if *candidate == surface {
                        feedbacks.retain(|object| *object != context.object);
                    }
                }
            }
            ResourceKind::ActivationToken(object) => {
                self.activation_tokens.remove(&object);
            }
            ResourceKind::SessionLock(object) => {
                if let Some(lock) = self.session_locks.remove(&object)
                    && self.active_session_lock == Some(object)
                {
                    self.active_session_lock = None;
                    if !lock.locked_event_sent {
                        self.core
                            .queue_action(CompositorAction::SessionLockCancelled(object));
                    }
                }
            }
            ResourceKind::SessionLockSurface(surface) => {
                self.session_lock_surfaces.remove(&surface);
            }
            ResourceKind::XwaylandKeyboardGrab(object) => {
                self.xwayland_keyboard_grabs.remove(&object);
            }
            ResourceKind::ShortcutInhibitor(object) => {
                self.shortcut_inhibitors.remove(&object);
            }
            ResourceKind::IdleInhibitor(object) => {
                self.idle_inhibitors.remove(&object);
            }
            ResourceKind::LockedPointer(object) | ResourceKind::ConfinedPointer(object) => {
                self.pointer_constraints.remove(&object);
            }
            ResourceKind::SurfaceSynchronization(surface) => {
                self.synchronized_surfaces.remove(&surface);
                self.pending_acquire_fences.remove(&surface);
                // get_release creates an independent object; destroying the
                // synchronization object must not revoke its next-commit release.
            }
            ResourceKind::ExplicitBufferRelease(surface) => {
                self.pending_releases
                    .retain(|candidate, object| *candidate != surface || *object != context.object);
                self.committed_releases.retain(|(candidate, _), object| {
                    *candidate != surface || *object != context.object
                });
            }
            ResourceKind::Callback(surface) => {
                if let Some(callbacks) = self.callbacks.get_mut(&surface) {
                    callbacks.retain(|object| *object != context.object);
                }
                for ((candidate, _), callbacks) in &mut self.committed_callbacks {
                    if *candidate == surface {
                        callbacks.retain(|object| *object != context.object);
                    }
                }
            }
            _ => {}
        }
        let _ = self.core.objects.remove(context.client, context.object);
        if self.core.objects.client_len(context.client) == 0 {
            self.touch_points
                .retain(|_, point| point.client != context.client);
            let _ = self.core.disconnect_client(context.client);
            self.clients.retain(|_, client| *client != context.client);
        }
        self.collect_destroyed_buffers();
    }
}
