use super::*;

impl NativeState {
    pub(super) fn queue_frame_callbacks(
        &mut self,
        surface: WaylandSurfaceId,
        through_revision: u64,
        time_milliseconds: u32,
        presented: bool,
    ) -> Result<Vec<usize>, NativeCompositorError> {
        let callback_commits =
            take_surface_commits_through(&mut self.committed_callbacks, surface, through_revision);
        let mut identities = Vec::new();
        for object in callback_commits
            .into_iter()
            .flat_map(|(_, callbacks)| callbacks)
        {
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
                "wl_callback",
                "done",
                &mut [ffi::wl_argument {
                    u: time_milliseconds,
                }],
            )?;
            identities.push(identity);
        }
        if !identities.is_empty()
            && let Some(observer) = &self.timing_observer
        {
            observer(TimingEvent::CallbacksQueued {
                surface: surface.get(),
                revision: through_revision,
                count: identities.len(),
                presented,
            });
        }
        Ok(identities)
    }
}

impl NativeCompositor<'_> {
    /// An output refresh permits another client frame even if its latest callback-only
    /// commit has not itself reached scanout. This does not release buffers or complete,
    /// discard, or fabricate presentation feedback for any image revision.
    pub(crate) fn surface_frame_ready(
        &mut self,
        surface: WaylandSurfaceId,
        through_revision: u64,
        time_milliseconds: u32,
    ) -> Result<(), NativeCompositorError> {
        let current = self
            .state
            .core
            .world
            .surface(surface)
            .ok_or_else(|| NativeCompositorError::new("unknown wl_surface"))?
            .snapshot()
            .revision;
        if through_revision > current {
            return Err(NativeCompositorError::new(
                "frame-ready revision is newer than committed state",
            ));
        }
        let identities = self.state.queue_frame_callbacks(
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
}
