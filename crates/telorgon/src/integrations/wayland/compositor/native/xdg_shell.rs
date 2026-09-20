use super::*;

impl NativeState {
    pub(super) fn dispatch_xdg_wm_base(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "create_positioner" => {
                let object = self.peek_next_object()?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "xdg_positioner",
                    resource.version(),
                    request.new_id(0).map_err(error)?,
                    ResourceKind::XdgPositioner(object),
                    true,
                )?;
                self.positioners
                    .insert(object, NativeXdgPositioner::default());
            }
            "get_xdg_surface" => {
                let surface = self.surface_from_resource(
                    request
                        .object(1)
                        .map_err(error)?
                        .ok_or_else(|| NativeCompositorError::new("missing wl_surface"))?,
                )?;
                if self.surface_mut(surface)?.snapshot().attachment.is_some() {
                    return Err(NativeCompositorError::new(
                        "xdg_surface was created with a buffer attached",
                    ));
                }
                let object = self.peek_next_object()?;
                self.core
                    .create_xdg_surface(context.client, surface, object, resource.version())
                    .map_err(error)?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "xdg_surface",
                    resource.version(),
                    request.new_id(0).map_err(error)?,
                    ResourceKind::XdgSurface(surface),
                    false,
                )?;
                self.xdg_resources.insert(surface, object);
            }
            "pong" => {
                let _ = request.uint(0).map_err(error)?;
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_xdg_positioner(
        &mut self,
        object: ProtocolObjectId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        let positioner = self
            .positioners
            .get_mut(&object)
            .ok_or_else(|| NativeCompositorError::new("unknown xdg_positioner"))?;
        match request.message().name {
            "set_size" => {
                positioner.size = Some(crate::foundation::SizeI {
                    width: request.int(0).map_err(error)?,
                    height: request.int(1).map_err(error)?,
                });
            }
            "set_anchor_rect" => {
                positioner.anchor_rect = Some(RectI {
                    x: request.int(0).map_err(error)?,
                    y: request.int(1).map_err(error)?,
                    width: request.int(2).map_err(error)?,
                    height: request.int(3).map_err(error)?,
                });
            }
            "set_anchor" => positioner.anchor = request.uint(0).map_err(error)?,
            "set_gravity" => positioner.gravity = request.uint(0).map_err(error)?,
            "set_constraint_adjustment" => {
                positioner.constraint_adjustment = request.uint(0).map_err(error)?;
            }
            "set_offset" => {
                positioner.offset = PointI {
                    x: request.int(0).map_err(error)?,
                    y: request.int(1).map_err(error)?,
                };
            }
            "set_reactive" => positioner.reactive = true,
            "set_parent_size" => {
                positioner.parent_size = Some(crate::foundation::SizeI {
                    width: request.int(0).map_err(error)?,
                    height: request.int(1).map_err(error)?,
                });
            }
            "set_parent_configure" => {
                positioner.parent_configure = Some(request.uint(0).map_err(error)?);
            }
            _ => return Err(unsupported_request(request)),
        }
        // Validate every field that can be checked before the two required fields are complete.
        if positioner.anchor > 8
            || positioner.gravity > 8
            || positioner.constraint_adjustment & !0x3f != 0
        {
            return Err(NativeCompositorError::new(
                "invalid xdg_positioner enum or flags",
            ));
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_xdg_surface(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        surface: WaylandSurfaceId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "get_toplevel" => {
                self.surface_mut(surface)?
                    .assign_role(SurfaceRole::XdgToplevel)
                    .map_err(error)?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "xdg_toplevel",
                    resource.version(),
                    request.new_id(0).map_err(error)?,
                    ResourceKind::XdgToplevel(surface),
                    true,
                )?;
                self.toplevels.insert(surface, XdgToplevelState::default());
            }
            "ack_configure" => {
                self.core
                    .xdg_surface_mut(surface)
                    .ok_or_else(|| NativeCompositorError::new("unknown xdg_surface"))?
                    .ack_configure(request.uint(0).map_err(error)?)
                    .map_err(error)?;
            }
            "set_window_geometry" => self
                .core
                .xdg_surface_mut(surface)
                .ok_or_else(|| NativeCompositorError::new("unknown xdg_surface"))?
                .set_window_geometry(RectI {
                    x: request.int(0).map_err(error)?,
                    y: request.int(1).map_err(error)?,
                    width: request.int(2).map_err(error)?,
                    height: request.int(3).map_err(error)?,
                })
                .map_err(error)?,
            "get_popup" => {
                let parent = request
                    .object(1)
                    .map_err(error)?
                    .map(|resource| match self.resource_kind(resource)? {
                        ResourceKind::XdgSurface(parent) => Ok(parent),
                        _ => Err(NativeCompositorError::new(
                            "xdg_popup parent is not an xdg_surface",
                        )),
                    })
                    .transpose()?;
                let positioner_resource = request
                    .object(2)
                    .map_err(error)?
                    .ok_or_else(|| NativeCompositorError::new("missing xdg_positioner"))?;
                let ResourceKind::XdgPositioner(positioner_object) =
                    self.resource_kind(positioner_resource)?
                else {
                    return Err(NativeCompositorError::new(
                        "popup positioner is not an xdg_positioner",
                    ));
                };
                let positioner = self
                    .positioners
                    .get(&positioner_object)
                    .copied()
                    .ok_or_else(|| NativeCompositorError::new("unknown xdg_positioner"))?
                    .finish()?;
                self.surface_mut(surface)?
                    .assign_role(SurfaceRole::XdgPopup)
                    .map_err(error)?;
                self.create_resource(
                    resource.client(),
                    context.client,
                    "xdg_popup",
                    resource.version(),
                    request.new_id(0).map_err(error)?,
                    ResourceKind::XdgPopup(surface),
                    true,
                )?;
                self.popups.insert(
                    surface,
                    crate::integrations::wayland::compositor::XdgPopupState {
                        parent,
                        positioner,
                        grabbed: false,
                        reposition_token: None,
                    },
                );
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_xdg_popup(
        &mut self,
        context: &ResourceContext,
        surface: WaylandSurfaceId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        match request.message().name {
            "grab" => {
                let serial = request.uint(1).map_err(error)?;
                self.core
                    .serials
                    .consume(
                        context.client,
                        serial,
                        &[
                            crate::integrations::wayland::compositor::SerialKind::PointerButton,
                            crate::integrations::wayland::compositor::SerialKind::TouchDown,
                        ],
                        self.popups.get(&surface).and_then(|popup| popup.parent),
                    )
                    .map_err(error)?;
                self.popups
                    .get_mut(&surface)
                    .ok_or_else(|| NativeCompositorError::new("unknown xdg_popup"))?
                    .grabbed = true;
            }
            "reposition" => {
                let positioner_resource = request
                    .object(0)
                    .map_err(error)?
                    .ok_or_else(|| NativeCompositorError::new("missing xdg_positioner"))?;
                let ResourceKind::XdgPositioner(positioner_object) =
                    self.resource_kind(positioner_resource)?
                else {
                    return Err(NativeCompositorError::new(
                        "popup reposition object is not an xdg_positioner",
                    ));
                };
                let positioner = self
                    .positioners
                    .get(&positioner_object)
                    .copied()
                    .ok_or_else(|| NativeCompositorError::new("unknown xdg_positioner"))?
                    .finish()?;
                let token = request.uint(1).map_err(error)?;
                let popup = self
                    .popups
                    .get_mut(&surface)
                    .ok_or_else(|| NativeCompositorError::new("unknown xdg_popup"))?;
                popup.positioner = positioner;
                popup.reposition_token = Some(token);
                self.send_popup_configure(surface, Some(token))?;
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_xdg_toplevel(
        &mut self,
        context: &ResourceContext,
        surface: WaylandSurfaceId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if !self.toplevels.contains_key(&surface) {
            return Err(NativeCompositorError::new("unknown xdg_toplevel"));
        }
        match request.message().name {
            "set_title" => self
                .toplevels
                .get_mut(&surface)
                .expect("checked above")
                .set_title(c_string(request, 0)?)
                .map_err(error)?,
            "set_app_id" => self
                .toplevels
                .get_mut(&surface)
                .expect("checked above")
                .set_application_id(c_string(request, 0)?)
                .map_err(error)?,
            "set_min_size" => {
                let minimum = crate::foundation::SizeI {
                    width: request.int(0).map_err(error)?,
                    height: request.int(1).map_err(error)?,
                };
                let maximum = self
                    .toplevels
                    .get(&surface)
                    .expect("checked above")
                    .maximum_size;
                self.toplevels
                    .get_mut(&surface)
                    .expect("checked above")
                    .set_size_constraints(Some(minimum), maximum)
                    .map_err(error)?;
            }
            "set_max_size" => {
                let maximum = crate::foundation::SizeI {
                    width: request.int(0).map_err(error)?,
                    height: request.int(1).map_err(error)?,
                };
                let minimum = self
                    .toplevels
                    .get(&surface)
                    .expect("checked above")
                    .minimum_size;
                self.toplevels
                    .get_mut(&surface)
                    .expect("checked above")
                    .set_size_constraints(minimum, Some(maximum))
                    .map_err(error)?;
            }
            "move" | "resize" => {
                // Both requests carry `(seat, serial, ...)`; resize alone appends its edge.
                let serial = request.uint(1).map_err(error)?;
                self.core
                    .serials
                    .consume(
                        context.client,
                        serial,
                        &[crate::integrations::wayland::compositor::SerialKind::PointerButton],
                        Some(surface),
                    )
                    .map_err(error)?;
                if request.message().name == "move" {
                    self.core
                        .queue_action(CompositorAction::MoveToplevel(surface));
                } else {
                    let edge = resize_edge(request.uint(2).map_err(error)?)?;
                    self.core
                        .queue_action(CompositorAction::ResizeToplevel { surface, edge });
                }
            }
            "set_maximized" => {
                self.core.queue_action(CompositorAction::MaximizeToplevel {
                    surface,
                    maximized: true,
                });
            }
            "unset_maximized" => {
                self.core.queue_action(CompositorAction::MaximizeToplevel {
                    surface,
                    maximized: false,
                });
            }
            "set_fullscreen" => {
                let output = request
                    .object(0)
                    .map_err(error)?
                    .map(|resource| match self.resource_kind(resource)? {
                        ResourceKind::Output(output) => Ok(output),
                        _ => Err(NativeCompositorError::new(
                            "fullscreen target is not a wl_output",
                        )),
                    })
                    .transpose()?;
                self.core
                    .queue_action(CompositorAction::FullscreenToplevel {
                        surface,
                        fullscreen: true,
                        output,
                    });
            }
            "unset_fullscreen" => {
                self.core
                    .queue_action(CompositorAction::FullscreenToplevel {
                        surface,
                        fullscreen: false,
                        output: None,
                    });
            }
            "set_minimized" => {
                self.core
                    .queue_action(CompositorAction::MinimizeToplevel(surface));
            }
            "set_parent" => {
                let parent = request
                    .object(0)
                    .map_err(error)?
                    .map(|resource| match self.resource_kind(resource)? {
                        ResourceKind::XdgToplevel(parent) => Ok(parent),
                        _ => Err(NativeCompositorError::new(
                            "toplevel parent is not an xdg_toplevel",
                        )),
                    })
                    .transpose()?;
                let mut ancestor = parent;
                let mut visited = BTreeSet::new();
                while let Some(candidate) = ancestor {
                    if candidate == surface || !visited.insert(candidate) {
                        return Err(NativeCompositorError::new(
                            "toplevel parent relationship would form a cycle",
                        ));
                    }
                    ancestor = self
                        .toplevels
                        .get(&candidate)
                        .ok_or_else(|| NativeCompositorError::new("unknown toplevel parent"))?
                        .parent;
                }
                self.toplevels
                    .get_mut(&surface)
                    .expect("checked above")
                    .parent = parent;
            }
            "show_window_menu" => {
                let serial = request.uint(1).map_err(error)?;
                self.core
                    .serials
                    .consume(
                        context.client,
                        serial,
                        &[crate::integrations::wayland::compositor::SerialKind::PointerButton],
                        Some(surface),
                    )
                    .map_err(error)?;
            }
            _ => return Err(unsupported_request(request)),
        }
        Ok(DispatchOutcome::default())
    }
}
