use super::*;

impl NativeState {
    pub(super) fn collect_destroyed_buffers(&mut self) {
        let retired: Vec<_> = self
            .destroyed_buffers
            .iter()
            .filter_map(|(&buffer, &client)| {
                let referenced =
                    self.core
                        .world
                        .client_surfaces(client)
                        .into_iter()
                        .any(|surface| {
                            let state = self.core.world.surface(surface).expect("listed surface");
                            state
                                .snapshot()
                                .attachment
                                .is_some_and(|attachment| attachment.buffer == buffer)
                                || state
                                    .pending()
                                    .attachment
                                    .flatten()
                                    .is_some_and(|attachment| attachment.buffer == buffer)
                                || self.core.subsurfaces.cached_buffer(surface) == Some(buffer)
                        });
                (!referenced).then_some((buffer, client))
            })
            .collect();
        for (buffer, client) in retired {
            self.destroyed_buffers.remove(&buffer);
            self.buffer_files.remove(&buffer);
            self.dmabuf_files.remove(&buffer);
            let _ = self.core.destroy_buffer(client, buffer);
        }
    }

    pub(super) fn commit_surface(
        &mut self,
        surface: WaylandSurfaceId,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if let Some(observer) = &self.timing_observer {
            let state = self.core.world.surface(surface);
            observer(TimingEvent::CommitRequested {
                surface: surface.get(),
                current_revision: state.map_or(0, |s| s.snapshot().revision),
                attaches_buffer: state.is_some_and(|s| s.pending().attachment.flatten().is_some()),
                callbacks: self.callbacks.get(&surface).map_or(0, Vec::len),
            });
        }
        if let Some(state) = self.core.world.surface(surface) {
            super::super::diagnostics::event(surface.get(), "commit", format_args!(
                "role={:?} parent={:?} revision={} attachment={:?} scale={:?} damage={} buffer_damage={}",
                state.snapshot().role, self.core.subsurfaces.parent(surface), state.snapshot().revision,
                state.pending().attachment, state.pending().buffer_scale,
                state.pending().damage.len(), state.pending().buffer_damage.len()));
        }
        let pending_buffer = self
            .surface_mut(surface)?
            .pending()
            .attachment
            .flatten()
            .map(|attachment| attachment.buffer);
        if let Some(xdg) = self.core.xdg_surface_mut(surface) {
            xdg.validate_buffer_commit(pending_buffer.is_some())
                .map_err(error)?;
        }
        if let Some(lock_surface) = self.session_lock_surfaces.get(&surface) {
            if lock_surface.last_acked.is_none() {
                return Err(NativeCompositorError::new(
                    "session-lock surface committed before its first configure ack",
                ));
            }
            let resulting_buffer = match self.surface_mut(surface)?.pending().attachment {
                Some(attachment) => attachment.map(|attachment| attachment.buffer),
                None => self
                    .core
                    .world
                    .surface(surface)
                    .and_then(|surface| surface.snapshot().attachment)
                    .map(|attachment| attachment.buffer),
            };
            if resulting_buffer.is_none() {
                return Err(NativeCompositorError::new(
                    "session-lock surface committed a null buffer",
                ));
            }
        }
        if pending_buffer.is_none()
            && (self.pending_acquire_fences.contains_key(&surface)
                || self.pending_releases.contains_key(&surface))
        {
            return Err(NativeCompositorError::new(
                "explicit synchronization requires a buffer in the same commit",
            ));
        }
        if self.core.subsurfaces.parent(surface).is_some() {
            let pending = self.surface_mut(surface)?.pending().clone();
            if self
                .core
                .subsurfaces
                .stage_or_release(surface, pending)
                .map_err(error)?
                .is_none()
            {
                super::super::diagnostics::event(surface.get(), "cached", format_args!("waiting for synchronized parent commit"));
                return Ok(DispatchOutcome::default());
            }
        }
        let outcome = self.surface_mut(surface)?.commit().map_err(error)?;
        if let Some((acknowledged_configure, window_geometry)) = self
            .core
            .xdg_surface_mut(surface)
            .map(|xdg_surface| xdg_surface.commit_state())
        {
            self.commit_decoration_mode(surface, acknowledged_configure);
            self.surface_mut(surface)?
                .apply_xdg_commit_state(acknowledged_configure, window_geometry);
        }
        if let Some(icon) = self.pending_toplevel_icons.remove(&surface) {
            match icon {
                PendingToplevelIcon::Reset => {
                    self.committed_toplevel_icons.remove(&surface);
                }
                PendingToplevelIcon::Icon(icon) => {
                    self.committed_toplevel_icons.insert(surface, icon);
                }
            }
        }
        self.commit_viewport_state(surface)?;
        if let Some(expected) = self
            .session_lock_surfaces
            .get(&surface)
            .and_then(|surface| surface.last_acked.map(|(_, size)| size))
            && self.surface_logical_size(surface)? != expected
        {
            return Err(NativeCompositorError::new(
                "session-lock surface dimensions do not match its acknowledged configure",
            ));
        }
        self.commit_feedback_state(surface, outcome.revision);
        if let Some(fence) = self.pending_acquire_fences.remove(&surface) {
            self.committed_acquire_fences
                .insert((surface, outcome.revision), fence);
        }
        if let Some(release) = self.pending_releases.remove(&surface) {
            self.committed_releases
                .insert((surface, outcome.revision), release);
        }
        if !outcome.mapped {
            self.revoke_suspended_focus(surface);
        }
        self.update_surface_output(surface, outcome.mapped)?;
        self.core.queue_action(if outcome.mapped {
            CompositorAction::PublishSurface(surface)
        } else {
            CompositorAction::WithdrawSurface(surface)
        });
        if self.core.xdg_surface_mut(surface).is_some()
            && !self.initial_configures.contains(&surface)
            && !outcome.mapped
        {
            self.send_initial_configure(surface)?;
        }
        for (child, commit) in self.core.subsurfaces.release_children(surface) {
            super::super::diagnostics::event(child.get(), "released", format_args!("ancestor={surface:?}"));
            self.surface_mut(child)?.stage(commit).map_err(error)?;
            let child_outcome = self.surface_mut(child)?.commit().map_err(error)?;
            self.commit_viewport_state(child)?;
            self.commit_feedback_state(child, child_outcome.revision);
            if !child_outcome.mapped {
                self.revoke_suspended_focus(child);
            }
            self.update_surface_output(child, child_outcome.mapped)?;
            self.core.queue_action(if child_outcome.mapped {
                CompositorAction::PublishSurface(child)
            } else {
                CompositorAction::WithdrawSurface(child)
            });
        }
        self.update_shortcut_inhibitors()?;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn commit_feedback_state(&mut self, surface: WaylandSurfaceId, revision: u64) {
        if let Some(callbacks) = self.callbacks.remove(&surface)
            && !callbacks.is_empty()
        {
            self.committed_callbacks
                .entry((surface, revision))
                .or_default()
                .extend(callbacks);
        }
        if let Some(feedbacks) = self.pending_presentation_feedbacks.remove(&surface)
            && !feedbacks.is_empty()
        {
            self.committed_presentation_feedbacks
                .entry((surface, revision))
                .or_default()
                .extend(feedbacks);
        }
    }

    pub(super) fn commit_viewport_state(
        &mut self,
        surface: WaylandSurfaceId,
    ) -> Result<(), NativeCompositorError> {
        let Some(viewport) = self.viewports.get_mut(&surface) else {
            return self.validate_surface_buffer_geometry(surface, None);
        };
        viewport.commit();
        let current = viewport.current;
        self.validate_surface_buffer_geometry(surface, Some(current))
    }

    pub(super) fn validate_surface_buffer_geometry(
        &self,
        surface: WaylandSurfaceId,
        viewport: Option<ViewportState>,
    ) -> Result<(), NativeCompositorError> {
        let snapshot = self
            .core
            .world
            .surface(surface)
            .ok_or_else(|| NativeCompositorError::new("unknown wl_surface"))?
            .snapshot();
        let Some(attachment) = snapshot.attachment else {
            return Ok(());
        };
        let buffer_size = match self
            .core
            .buffer(attachment.buffer)
            .ok_or_else(|| NativeCompositorError::new("surface buffer is absent"))?
        {
            BufferDescriptor::Shm(buffer) => buffer.size,
            BufferDescriptor::DmaBuf(buffer) => buffer.size,
        };
        let transformed = transformed_size(buffer_size, snapshot.buffer_transform);
        if transformed.width % snapshot.buffer_scale != 0
            || transformed.height % snapshot.buffer_scale != 0
        {
            return Err(NativeCompositorError::new(
                "buffer dimensions are not divisible by wl_surface buffer scale",
            ));
        }
        let natural_width = f64::from(transformed.width / snapshot.buffer_scale);
        let natural_height = f64::from(transformed.height / snapshot.buffer_scale);
        let Some(viewport) = viewport else {
            return Ok(());
        };
        if let Some(source) = viewport.source {
            if source.x + source.width > natural_width || source.y + source.height > natural_height
            {
                return Err(NativeCompositorError::new(
                    "viewport source extends outside the surface buffer",
                ));
            }
            if viewport.destination.is_none()
                && (source.width.fract() != 0.0 || source.height.fract() != 0.0)
            {
                return Err(NativeCompositorError::new(
                    "fractional viewport source requires a destination size",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn surface_logical_size(
        &self,
        surface: WaylandSurfaceId,
    ) -> Result<crate::foundation::SizeI, NativeCompositorError> {
        if let Some(destination) = self
            .viewports
            .get(&surface)
            .and_then(|viewport| viewport.current.destination)
        {
            return Ok(destination);
        }
        if let Some(source) = self
            .viewports
            .get(&surface)
            .and_then(|viewport| viewport.current.source)
        {
            return Ok(crate::foundation::SizeI {
                width: source.width as i32,
                height: source.height as i32,
            });
        }
        let snapshot = self
            .core
            .world
            .surface(surface)
            .ok_or_else(|| NativeCompositorError::new("unknown wl_surface"))?
            .snapshot();
        let attachment = snapshot
            .attachment
            .ok_or_else(|| NativeCompositorError::new("surface has no buffer"))?;
        let buffer_size = match self
            .core
            .buffer(attachment.buffer)
            .ok_or_else(|| NativeCompositorError::new("surface buffer is absent"))?
        {
            BufferDescriptor::Shm(buffer) => buffer.size,
            BufferDescriptor::DmaBuf(buffer) => buffer.size,
        };
        let transformed = transformed_size(buffer_size, snapshot.buffer_transform);
        Ok(crate::foundation::SizeI {
            width: transformed.width / snapshot.buffer_scale,
            height: transformed.height / snapshot.buffer_scale,
        })
    }

    pub(super) fn surface_frame_completed(
        &mut self,
        surface: WaylandSurfaceId,
        through_revision: u64,
        time_milliseconds: u32,
        presented: bool,
    ) -> Result<Vec<usize>, NativeCompositorError> {
        let current_revision = self
            .core
            .world
            .surface(surface)
            .ok_or_else(|| NativeCompositorError::new("unknown wl_surface"))?
            .snapshot()
            .revision;
        if through_revision > current_revision {
            return Err(NativeCompositorError::new(
                "presented surface revision is newer than committed state",
            ));
        }
        let mut identities = self.queue_frame_callbacks(surface, through_revision, time_milliseconds, presented)?;
        let (presented_feedbacks, discarded_feedbacks) = take_surface_feedbacks_through(
            &mut self.committed_presentation_feedbacks,
            surface,
            through_revision,
            presented,
        );
        let callback_count = identities.len();
        let feedback_count = presented_feedbacks.len() + discarded_feedbacks.len();
        super::super::diagnostics::event(surface.get(), "frame", format_args!(
            "revision={through_revision} presented={presented} callbacks={callback_count} feedbacks={feedback_count}"));
        for object in discarded_feedbacks {
            let Some(identity) = self.resources.get(&object).copied() else {
                continue;
            };
            let Some(resource) =
                (unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) })
            else {
                continue;
            };
            self.post_event(resource, "wp_presentation_feedback", "discarded", &mut [])?;
            identities.push(identity);
        }
        if !presented_feedbacks.is_empty() {
            let timestamp = monotonic_timestamp()?;
            self.presentation_sequence = self.presentation_sequence.wrapping_add(1).max(1);
            let sequence = self.presentation_sequence;
            let refresh = self
                .core
                .outputs
                .values()
                .find(|output| output.enabled)
                .map(|output| output.current_mode().refresh_millihertz)
                .map(|millihertz| {
                    u32::try_from(1_000_000_000_000_u64 / u64::from(millihertz)).unwrap_or(u32::MAX)
                })
                .unwrap_or(0);
            for object in presented_feedbacks {
                let Some(identity) = self.resources.get(&object).copied() else {
                    continue;
                };
                let Some(resource) =
                    (unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) })
                else {
                    continue;
                };
                self.post_event(
                    resource,
                    "wp_presentation_feedback",
                    "presented",
                    &mut [
                        ffi::wl_argument {
                            u: (timestamp.seconds >> 32) as u32,
                        },
                        ffi::wl_argument {
                            u: timestamp.seconds as u32,
                        },
                        ffi::wl_argument {
                            u: timestamp.nanoseconds,
                        },
                        ffi::wl_argument { u: refresh },
                        ffi::wl_argument {
                            u: (sequence >> 32) as u32,
                        },
                        ffi::wl_argument { u: sequence as u32 },
                        // The blocking KMS path does not yet expose hardware timestamp proof.
                        ffi::wl_argument { u: 0 },
                    ],
                )?;
                identities.push(identity);
            }
        }
        Ok(identities)
    }
}
