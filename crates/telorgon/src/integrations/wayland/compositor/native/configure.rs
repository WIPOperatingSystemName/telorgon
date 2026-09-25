use super::*;

#[cfg(test)]
#[path = "configure_tests.rs"]
mod tests;

impl NativeState {
    pub(super) fn send_initial_configure(
        &mut self,
        surface: WaylandSurfaceId,
    ) -> Result<(), NativeCompositorError> {
        let serial = unsafe { ffi::wl_display_next_serial(self.display.as_ptr()) };
        if serial == 0 {
            return Err(NativeCompositorError::new(
                "libwayland returned a zero configure serial",
            ));
        }
        let decoration = self.pending_decoration_mode(surface);
        let states = if self.toplevels.contains_key(&surface) {
            decoration_states(self.decoration_policy, decoration, Default::default())
        } else {
            Default::default()
        };
        self.core
            .xdg_surface_mut(surface)
            .ok_or_else(|| NativeCompositorError::new("unknown xdg_surface"))?
            .queue_configure(XdgConfigure {
                serial,
                size: None,
                bounds: None,
                states,
                decoration,
            })
            .map_err(error)?;
        if self.toplevels.contains_key(&surface)
            && let Some(toplevel_object) = self
            .resources
            .iter()
            .find_map(|(object, identity)| {
                let resource = unsafe { ResourceRef::from_raw(*identity as *mut ffi::wl_resource) }?;
                matches!(self.resource_kind(resource).ok()?, ResourceKind::XdgToplevel(candidate) if candidate == surface)
                    .then_some(*object)
            })
            && let Some(identity) = self.resources.get(&toplevel_object).copied()
            && let Some(resource) = unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) }
        {
            if resource.version() >= 5 {
                // Clients use this mandatory initial event to expose their window controls.
                // The host handles maximize, fullscreen, and minimize, but no window menu.
                let mut capabilities = [2_u32, 3, 4];
                let mut array = ffi::wl_array {
                    size: std::mem::size_of_val(&capabilities),
                    alloc: std::mem::size_of_val(&capabilities),
                    data: capabilities.as_mut_ptr().cast(),
                };
                self.post_event(resource, "xdg_toplevel", "wm_capabilities", &mut [
                    ffi::wl_argument { a: &mut array },
                ])?;
            }
            let mut state_values = wire_states(states, resource.version());
            let mut states = ffi::wl_array {
                size: state_values.len() * std::mem::size_of::<u32>(),
                alloc: state_values.len() * std::mem::size_of::<u32>(),
                data: state_values.as_mut_ptr().cast(),
            };
            self.post_event(
                resource,
                "xdg_toplevel",
                "configure",
                &mut [
                    ffi::wl_argument { i: 0 },
                    ffi::wl_argument { i: 0 },
                    ffi::wl_argument { a: &mut states },
                ],
            )?;
        }
        if self.popups.contains_key(&surface) {
            self.send_popup_configure(surface, None)?;
        }
        let object = *self
            .xdg_resources
            .get(&surface)
            .ok_or_else(|| NativeCompositorError::new("xdg_surface resource is absent"))?;
        let identity = *self
            .resources
            .get(&object)
            .ok_or_else(|| NativeCompositorError::new("xdg_surface resource is absent"))?;
        let resource = unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) }
            .ok_or_else(|| NativeCompositorError::new("xdg_surface resource is stale"))?;
        self.post_event(
            resource,
            "xdg_surface",
            "configure",
            &mut [ffi::wl_argument { u: serial }],
        )?;
        self.initial_configures.insert(surface);
        super::super::diagnostics::event(
            surface.get(),
            "configure",
            format_args!("initial serial={serial} size=client-choice decoration={decoration:?}"),
        );
        Ok(())
    }

    pub(super) fn send_toplevel_configure(
        &mut self,
        surface: WaylandSurfaceId,
        size: Option<crate::foundation::SizeI>,
        states: crate::integrations::wayland::compositor::ToplevelState,
    ) -> Result<u32, NativeCompositorError> {
        if !self.toplevels.contains_key(&surface) {
            return Err(NativeCompositorError::new("surface is not an xdg_toplevel"));
        }
        let (toplevel_identity, toplevel_version) = {
            let resource = self
                .resource_for_kind(
                    |kind| matches!(kind, ResourceKind::XdgToplevel(candidate) if candidate == surface),
                )?
                .ok_or_else(|| NativeCompositorError::new("xdg_toplevel resource is absent"))?;
            (resource.identity(), resource.version())
        };
        let xdg_surface_object = *self
            .xdg_resources
            .get(&surface)
            .ok_or_else(|| NativeCompositorError::new("xdg_surface resource is absent"))?;
        let xdg_surface_identity = *self
            .resources
            .get(&xdg_surface_object)
            .ok_or_else(|| NativeCompositorError::new("xdg_surface resource is absent"))?;
        let xdg_surface =
            unsafe { ResourceRef::from_raw(xdg_surface_identity as *mut ffi::wl_resource) }
                .ok_or_else(|| NativeCompositorError::new("xdg_surface resource is stale"))?;
        let serial = unsafe { ffi::wl_display_next_serial(self.display.as_ptr()) };
        if serial == 0 {
            return Err(NativeCompositorError::new(
                "libwayland returned a zero configure serial",
            ));
        }
        let decoration = self.pending_decoration_mode(surface);
        self.requested_toplevel_states.insert(surface, states);
        let states = decoration_states(self.decoration_policy, decoration, states);
        self.core
            .xdg_surface_mut(surface)
            .ok_or_else(|| NativeCompositorError::new("unknown xdg_surface"))?
            .queue_configure(XdgConfigure {
                serial,
                size,
                bounds: None,
                states,
                decoration,
            })
            .map_err(error)?;
        let mut state_values = wire_states(states, toplevel_version);
        let mut state_array = ffi::wl_array {
            size: state_values.len() * std::mem::size_of::<u32>(),
            alloc: state_values.len() * std::mem::size_of::<u32>(),
            data: state_values.as_mut_ptr().cast(),
        };
        let size = size.unwrap_or_default();
        let toplevel = unsafe { ResourceRef::from_raw(toplevel_identity as *mut ffi::wl_resource) }
            .ok_or_else(|| NativeCompositorError::new("xdg_toplevel resource is stale"))?;
        self.post_event(
            toplevel,
            "xdg_toplevel",
            "configure",
            &mut [
                ffi::wl_argument { i: size.width },
                ffi::wl_argument { i: size.height },
                ffi::wl_argument {
                    a: &mut state_array,
                },
            ],
        )?;
        self.post_event(
            xdg_surface,
            "xdg_surface",
            "configure",
            &mut [ffi::wl_argument { u: serial }],
        )?;
        super::super::diagnostics::event(
            surface.get(),
            "configure",
            format_args!(
                "serial={serial} size={size:?} states={states:?} decoration={decoration:?}"
            ),
        );
        Ok(serial)
    }

    pub(super) fn send_popup_configure(
        &self,
        surface: WaylandSurfaceId,
        repositioned: Option<u32>,
    ) -> Result<(), NativeCompositorError> {
        let popup = self
            .popups
            .get(&surface)
            .ok_or_else(|| NativeCompositorError::new("unknown xdg_popup"))?;
        let geometry = popup_geometry(popup.positioner);
        let resource = self
            .resource_for_kind(
                |kind| matches!(kind, ResourceKind::XdgPopup(candidate) if candidate == surface),
            )?
            .ok_or_else(|| NativeCompositorError::new("xdg_popup resource is absent"))?;
        self.post_event(
            resource,
            "xdg_popup",
            "configure",
            &mut [
                ffi::wl_argument { i: geometry.x },
                ffi::wl_argument { i: geometry.y },
                ffi::wl_argument { i: geometry.width },
                ffi::wl_argument { i: geometry.height },
            ],
        )?;
        if let Some(token) = repositioned {
            self.post_event(
                resource,
                "xdg_popup",
                "repositioned",
                &mut [ffi::wl_argument { u: token }],
            )?;
        }
        Ok(())
    }
}

fn decoration_states(
    policy: crate::DecorationPolicy,
    decoration: crate::integrations::wayland::compositor::DecorationMode,
    mut states: crate::integrations::wayland::compositor::ToplevelState,
) -> crate::integrations::wayland::compositor::ToplevelState {
    // This is only a client styling hint. Host tile membership and geometry stay unchanged.
    if policy.tiled_client_decorations
        && decoration == crate::integrations::wayland::compositor::DecorationMode::ClientSide
        && !states.fullscreen
    {
        states.tiled_left = true;
        states.tiled_right = true;
        states.tiled_top = true;
        states.tiled_bottom = true;
    }
    states
}

fn wire_states(
    states: crate::integrations::wayland::compositor::ToplevelState,
    version: u32,
) -> Vec<u32> {
    let mut state_values = Vec::<u32>::with_capacity(9);
    if states.maximized {
        state_values.push(1);
    }
    if states.fullscreen {
        state_values.push(2);
    }
    if states.resizing {
        state_values.push(3);
    }
    if states.activated {
        state_values.push(4);
    }
    if version >= 2 {
        if states.tiled_left {
            state_values.push(5);
        }
        if states.tiled_right {
            state_values.push(6);
        }
        if states.tiled_top {
            state_values.push(7);
        }
        if states.tiled_bottom {
            state_values.push(8);
        }
    }
    if version >= 6 && states.suspended {
        state_values.push(9);
    }
    state_values
}
