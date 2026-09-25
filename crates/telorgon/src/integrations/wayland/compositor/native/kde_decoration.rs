use super::*;
use crate::integrations::wayland::compositor::DecorationMode;

#[derive(Default)]
pub(super) struct SurfaceDecorations {
    pub(super) xdg: Option<ProtocolObjectId>,
    // Some GTK clients create more than one object. The latest request wins; releasing an
    // object restores the most recently requested surviving preference.
    kde: Vec<(ProtocolObjectId, u32)>,
}

impl NativeState {
    pub(super) fn pending_decoration_mode(&self, surface: WaylandSurfaceId) -> DecorationMode {
        let Some(state) = self.decorations.get(&surface) else {
            return DecorationMode::ClientSide;
        };
        // The protocols leave simultaneous use undefined. Prefer xdg's acknowledged contract.
        if state.xdg.is_some() {
            return self
                .toplevels
                .get(&surface)
                .map_or(DecorationMode::ClientSide, |t| t.decoration);
        }
        if state.kde.last().is_some_and(|(_, mode)| *mode == 2) {
            DecorationMode::ServerSide
        } else {
            DecorationMode::ClientSide
        }
    }

    pub(super) fn commit_decoration_mode(
        &mut self,
        surface: WaylandSurfaceId,
        ack: Option<XdgConfigure>,
    ) {
        if !self.toplevels.contains_key(&surface) {
            return;
        }
        let mode = if self
            .decorations
            .get(&surface)
            .is_some_and(|state| state.xdg.is_some())
        {
            let Some(configure) = ack else { return };
            configure.decoration
        } else {
            // KDE has no configure acknowledgement. Apply its preference (or object removal)
            // with the next surface commit, never resurrecting an obsolete xdg configure.
            self.pending_decoration_mode(surface)
        };
        self.committed_decorations.insert(surface, mode);
    }

    pub(super) fn remove_decoration(
        &mut self,
        surface: WaylandSurfaceId,
        object: ProtocolObjectId,
    ) {
        if let Some(state) = self.decorations.get_mut(&surface) {
            if state.xdg == Some(object) {
                state.xdg = None;
            }
            state.kde.retain(|(candidate, _)| *candidate != object);
            if state.xdg.is_none() && state.kde.is_empty() {
                self.decorations.remove(&surface);
            }
        }
    }

    pub(super) fn dispatch_kde_decoration_manager(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "create" {
            return Err(unsupported_request(request));
        }
        let target = request
            .object(1)
            .map_err(error)?
            .ok_or_else(|| NativeCompositorError::new("missing decoration surface"))?;
        let ResourceKind::Surface(surface) = self.resource_kind(target)? else {
            return Err(NativeCompositorError::new(
                "decoration target is not a wl_surface",
            ));
        };
        let object = self.peek_next_object()?;
        let decoration = self.create_resource(
            resource.client(),
            context.client,
            "org_kde_kwin_server_decoration",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::KdeDecoration(surface),
            true,
        )?;
        self.decorations
            .entry(surface)
            .or_default()
            .kde
            .push((object, 2));
        self.post_event(
            decoration,
            "org_kde_kwin_server_decoration",
            "mode",
            &mut [ffi::wl_argument { u: 2 }],
        )?;
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_kde_decoration(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        surface: WaylandSurfaceId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "request_mode" {
            return Err(unsupported_request(request));
        }
        let mode = request.uint(0).map_err(error)?;
        if mode > 2 {
            return Err(NativeCompositorError::new("invalid KDE decoration mode"));
        }
        let state = self
            .decorations
            .get_mut(&surface)
            .filter(|state| state.kde.iter().any(|(id, _)| *id == context.object))
            .ok_or_else(|| NativeCompositorError::new("decoration surface no longer exists"))?;
        state.kde.retain(|(id, _)| *id != context.object);
        state.kde.push((context.object, mode));
        // Unlike xdg-decoration, this protocol requires acknowledging the requested mode,
        // including CSD when the server prefers SSD. Overriding it makes GTK request forever.
        self.post_event(
            resource,
            "org_kde_kwin_server_decoration",
            "mode",
            &mut [ffi::wl_argument { u: mode }],
        )?;
        Ok(DispatchOutcome::default())
    }
}
