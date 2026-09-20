use super::*;

impl NativeState {
    pub(super) fn drag_motion(
        &mut self,
        seat_id: u32,
        target: Option<WaylandSurfaceId>,
        time: u32,
        position: crate::foundation::PointF,
    ) -> Result<(), NativeCompositorError> {
        let (drag_seat, source, current) = self
            .active_drag
            .as_ref()
            .map(|drag| {
                (
                    drag.seat,
                    drag.source,
                    drag.target.as_ref().map(|target| target.surface),
                )
            })
            .ok_or_else(|| NativeCompositorError::new("no drag is active"))?;
        if drag_seat != seat_id {
            return Err(NativeCompositorError::new("drag belongs to another seat"));
        }
        if let Some(surface) = target
            && self.core.world.surface(surface).is_none()
        {
            return Err(NativeCompositorError::new("unknown drag target surface"));
        }

        if current != target {
            if let Some(previous) = self
                .active_drag
                .as_mut()
                .and_then(|drag| drag.target.take())
            {
                self.send_drag_leave(&previous, source)?;
            }
            let Some(surface) = target else {
                return Ok(());
            };
            self.enter_drag_target(seat_id, surface, position)?;
            return Ok(());
        }

        let device_objects = self
            .active_drag
            .as_ref()
            .and_then(|drag| drag.target.as_ref())
            .map(|target| target.devices.clone())
            .unwrap_or_default();
        for object in device_objects {
            let Some(device) = self.resource_for_object(object)? else {
                continue;
            };
            self.post_event(
                device,
                "wl_data_device",
                "motion",
                &mut [
                    ffi::wl_argument { u: time },
                    ffi::wl_argument {
                        f: fixed(position.x),
                    },
                    ffi::wl_argument {
                        f: fixed(position.y),
                    },
                ],
            )?;
        }
        Ok(())
    }

    pub(super) fn enter_drag_target(
        &mut self,
        seat_id: u32,
        surface: WaylandSurfaceId,
        position: crate::foundation::PointF,
    ) -> Result<(), NativeCompositorError> {
        let client = self
            .core
            .world
            .surface_owner(surface)
            .ok_or_else(|| NativeCompositorError::new("drag target has no owner"))?;
        let source = self.active_drag.as_ref().and_then(|drag| drag.source);
        let devices = self
            .resources_for_client(
                client,
                |kind| matches!(kind, ResourceKind::DataDevice(candidate) if candidate == seat_id),
            )?
            .into_iter()
            .map(|resource| self.protocol_object_for_resource(resource))
            .collect::<Result<Vec<_>, _>>()?;
        if devices.is_empty() {
            return Ok(());
        }

        let serial = unsafe { ffi::wl_display_next_serial(self.display.as_ptr()) };
        self.core
            .serials
            .issue(
                serial,
                client,
                crate::integrations::wayland::compositor::SerialKind::DataDevice,
                Some(surface),
            )
            .map_err(error)?;
        let surface_resource = self.surface_resource(surface)?;
        let mut offers = Vec::new();
        for device_object in &devices {
            let Some(device_identity) = self.resources.get(device_object).copied() else {
                continue;
            };
            let offer = source
                .map(|source| self.create_drag_offer(device_identity, client, source))
                .transpose()?;
            let Some(device) =
                (unsafe { ResourceRef::from_raw(device_identity as *mut ffi::wl_resource) })
            else {
                continue;
            };
            let offer_resource = offer
                .map(|(object, identity)| {
                    offers.push(object);
                    identity as *mut ffi::wl_resource
                })
                .unwrap_or(std::ptr::null_mut());
            self.post_event(
                device,
                "wl_data_device",
                "enter",
                &mut [
                    ffi::wl_argument { u: serial },
                    ffi::wl_argument {
                        o: surface_resource,
                    },
                    ffi::wl_argument {
                        f: fixed(position.x),
                    },
                    ffi::wl_argument {
                        f: fixed(position.y),
                    },
                    ffi::wl_argument { o: offer_resource },
                ],
            )?;
        }
        if let Some(drag) = self.active_drag.as_mut() {
            drag.target = Some(NativeDragTarget {
                surface,
                devices,
                offers,
            });
        }
        Ok(())
    }

    pub(super) fn send_drag_leave(
        &self,
        target: &NativeDragTarget,
        source: Option<ProtocolObjectId>,
    ) -> Result<(), NativeCompositorError> {
        for object in &target.devices {
            let Some(device) = self.resource_for_object(*object)? else {
                continue;
            };
            self.post_event(device, "wl_data_device", "leave", &mut [])?;
        }
        if let Some(source) = source
            && let Ok(source_resource) = self.data_source_resource(source)
        {
            self.post_event(
                source_resource,
                "wl_data_source",
                "target",
                &mut [ffi::wl_argument {
                    s: std::ptr::null(),
                }],
            )?;
        }
        Ok(())
    }

    pub(super) fn drop_drag(&mut self, seat_id: u32) -> Result<(), NativeCompositorError> {
        let Some(drag) = self.active_drag.take() else {
            return Ok(());
        };
        if drag.seat != seat_id {
            self.active_drag = Some(drag);
            return Err(NativeCompositorError::new("drag belongs to another seat"));
        }

        let accepted = drag.source.is_none()
            || drag.target.as_ref().is_some_and(|target| {
                target.offers.iter().any(|offer| {
                    self.core.data_devices.offer(*offer).is_some_and(|offer| {
                        offer.accepted_mime_type.is_some()
                            && offer.selected_action
                                != crate::integrations::wayland::compositor::DataAction::NONE
                    })
                })
            });
        if accepted {
            let mut has_version_3_offer = false;
            if let Some(target) = &drag.target {
                for object in &target.devices {
                    let Some(device) = self.resource_for_object(*object)? else {
                        continue;
                    };
                    self.post_event(device, "wl_data_device", "drop", &mut [])?;
                }
                for object in &target.offers {
                    if let Some(offer) = self.core.data_devices.offer_mut(*object)
                        && offer.accepted_mime_type.is_some()
                        && offer.selected_action
                            != crate::integrations::wayland::compositor::DataAction::NONE
                    {
                        offer.dropped = true;
                        has_version_3_offer |= self
                            .resources
                            .get(object)
                            .copied()
                            .and_then(|identity| unsafe {
                                ResourceRef::from_raw(identity as *mut ffi::wl_resource)
                            })
                            .is_some_and(|resource| resource.version() >= 3);
                    }
                }
            }
            if let Some(source) = drag.source {
                let finish_legacy =
                    !has_version_3_offer && self.finished_drag_sources.insert(source);
                if let Ok(source_resource) = self.data_source_resource(source)
                    && source_resource.version() >= 3
                {
                    self.post_event(
                        source_resource,
                        "wl_data_source",
                        "dnd_drop_performed",
                        &mut [],
                    )?;
                    if finish_legacy {
                        self.post_event(
                            source_resource,
                            "wl_data_source",
                            "dnd_finished",
                            &mut [],
                        )?;
                    }
                }
            }
        } else {
            if let Some(target) = &drag.target {
                self.send_drag_leave(target, drag.source)?;
            }
            if let Some(source) = drag.source {
                self.cancel_data_source(source)?;
            }
        }
        self.finish_drag(drag.icon);
        Ok(())
    }

    pub(super) fn cancel_drag(&mut self, seat_id: u32) -> Result<(), NativeCompositorError> {
        let Some(drag) = self.active_drag.take() else {
            return Ok(());
        };
        if drag.seat != seat_id {
            self.active_drag = Some(drag);
            return Err(NativeCompositorError::new("drag belongs to another seat"));
        }
        if let Some(target) = &drag.target {
            self.send_drag_leave(target, drag.source)?;
        }
        if let Some(source) = drag.source {
            self.cancel_data_source(source)?;
        }
        self.finish_drag(drag.icon);
        Ok(())
    }

    pub(super) fn finish_drag(&mut self, icon: Option<WaylandSurfaceId>) {
        self.core.data_devices.finish_drag();
        self.core
            .queue_action(CompositorAction::FinishDrag { icon });
    }

    pub(super) fn create_drag_offer(
        &mut self,
        device_identity: usize,
        client: ClientId,
        source_object: ProtocolObjectId,
    ) -> Result<(ProtocolObjectId, usize), NativeCompositorError> {
        let device = unsafe {
            ResourceRef::from_raw(device_identity as *mut ffi::wl_resource)
                .ok_or_else(|| NativeCompositorError::new("wl_data_device resource is absent"))?
        };
        let source = self
            .core
            .data_devices
            .source(source_object)
            .cloned()
            .ok_or_else(|| NativeCompositorError::new("drag source is absent"))?;
        let object = self.peek_next_object()?;
        let offer_resource = self.create_resource(
            device.client(),
            client,
            "wl_data_offer",
            device.version(),
            0,
            ResourceKind::DataOffer(object),
            true,
        )?;
        let legacy = offer_resource.version() < 3;
        let source_actions = drag_source_actions(
            self.data_source_resource(source_object)?.version(),
            offer_resource.version(),
            source.actions,
        );
        if let Err(cause) = self.core.data_devices.create_offer(
            crate::integrations::wayland::compositor::DataOffer {
                object,
                source: source_object,
                target: client,
                drag: true,
                accepted_mime_type: None,
                source_actions,
                target_actions: if legacy {
                    crate::integrations::wayland::compositor::DataAction::COPY
                } else {
                    crate::integrations::wayland::compositor::DataAction::NONE
                },
                preferred_action: if legacy {
                    crate::integrations::wayland::compositor::DataAction::COPY
                } else {
                    crate::integrations::wayland::compositor::DataAction::NONE
                },
                selected_action: crate::integrations::wayland::compositor::DataAction::NONE,
                dropped: false,
                finished: false,
            },
        ) {
            unsafe { offer_resource.destroy() };
            return Err(error(cause));
        }
        self.post_event(
            device,
            "wl_data_device",
            "data_offer",
            &mut [ffi::wl_argument {
                o: offer_resource.identity() as *mut ffi::wl_resource,
            }],
        )?;
        for mime in &source.mime_types {
            let mime = protocol_string(mime.as_str());
            self.post_event(
                offer_resource,
                "wl_data_offer",
                "offer",
                &mut [ffi::wl_argument { s: mime.as_ptr() }],
            )?;
        }
        if !legacy {
            self.post_event(
                offer_resource,
                "wl_data_offer",
                "source_actions",
                &mut [ffi::wl_argument {
                    u: u32::from(source_actions.bits()),
                }],
            )?;
        }
        Ok((object, offer_resource.identity()))
    }
}
