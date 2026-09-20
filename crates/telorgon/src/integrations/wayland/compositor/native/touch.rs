use super::*;

impl NativeState {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn touch_down(
        &mut self,
        seat_id: u32,
        surface: WaylandSurfaceId,
        time_milliseconds: u32,
        touch_id: i32,
        position: crate::foundation::PointF,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        // Device queues can contain late events after access has been revoked.
        if self.suspended_focus.contains_key(&seat_id) {
            return Ok(());
        }

        if touch_id < 0 || self.touch_points.contains_key(&(seat_id, touch_id)) {
            return Err(NativeCompositorError::new(
                "invalid or duplicate touch identity",
            ));
        }
        let seat = self
            .core
            .seats
            .get(&seat_id)
            .ok_or_else(|| NativeCompositorError::new("unknown seat"))?;
        if !seat.capabilities.touch {
            return Err(NativeCompositorError::new("seat has no touch capability"));
        }
        let client = self
            .core
            .world
            .surface_owner(surface)
            .ok_or_else(|| NativeCompositorError::new("unknown touch surface"))?;
        self.core
            .serials
            .issue(
                serial,
                client,
                crate::integrations::wayland::compositor::SerialKind::TouchDown,
                Some(surface),
            )
            .map_err(error)?;
        let surface_resource = self.surface_resource(surface)?;
        for resource in self.resources_for_client(
            client,
            |kind| matches!(kind, ResourceKind::Touch(candidate) if candidate == seat_id),
        )? {
            self.post_event(
                resource,
                "wl_touch",
                "down",
                &mut [
                    ffi::wl_argument { u: serial },
                    ffi::wl_argument {
                        u: time_milliseconds,
                    },
                    ffi::wl_argument {
                        o: surface_resource,
                    },
                    ffi::wl_argument { i: touch_id },
                    ffi::wl_argument {
                        f: fixed(position.x),
                    },
                    ffi::wl_argument {
                        f: fixed(position.y),
                    },
                ],
            )?;
            self.post_event(resource, "wl_touch", "frame", &mut [])?;
        }
        self.touch_points.insert(
            (seat_id, touch_id),
            NativeTouchPoint {
                client,
                surface,
                down_serial: serial,
            },
        );
        Ok(())
    }

    pub(super) fn touch_motion(
        &self,
        seat_id: u32,
        time_milliseconds: u32,
        touch_id: i32,
        position: crate::foundation::PointF,
    ) -> Result<(), NativeCompositorError> {
        // Device queues can contain late events after access has been revoked.
        if self.suspended_focus.contains_key(&seat_id) {
            return Ok(());
        }

        let point = self
            .touch_points
            .get(&(seat_id, touch_id))
            .copied()
            .ok_or_else(|| NativeCompositorError::new("unknown touch identity"))?;
        for resource in self.resources_for_client(
            point.client,
            |kind| matches!(kind, ResourceKind::Touch(candidate) if candidate == seat_id),
        )? {
            self.post_event(
                resource,
                "wl_touch",
                "motion",
                &mut [
                    ffi::wl_argument {
                        u: time_milliseconds,
                    },
                    ffi::wl_argument { i: touch_id },
                    ffi::wl_argument {
                        f: fixed(position.x),
                    },
                    ffi::wl_argument {
                        f: fixed(position.y),
                    },
                ],
            )?;
            self.post_event(resource, "wl_touch", "frame", &mut [])?;
        }
        Ok(())
    }

    pub(super) fn touch_up(
        &mut self,
        seat_id: u32,
        time_milliseconds: u32,
        touch_id: i32,
        serial: u32,
    ) -> Result<(), NativeCompositorError> {
        // Device queues can contain late events after access has been revoked.
        if self.suspended_focus.contains_key(&seat_id) {
            return Ok(());
        }

        let point = self
            .touch_points
            .remove(&(seat_id, touch_id))
            .ok_or_else(|| NativeCompositorError::new("unknown touch identity"))?;
        self.core
            .serials
            .issue(
                serial,
                point.client,
                crate::integrations::wayland::compositor::SerialKind::DataDevice,
                Some(point.surface),
            )
            .map_err(error)?;
        for resource in self.resources_for_client(
            point.client,
            |kind| matches!(kind, ResourceKind::Touch(candidate) if candidate == seat_id),
        )? {
            self.post_event(
                resource,
                "wl_touch",
                "up",
                &mut [
                    ffi::wl_argument { u: serial },
                    ffi::wl_argument {
                        u: time_milliseconds,
                    },
                    ffi::wl_argument { i: touch_id },
                ],
            )?;
            self.post_event(resource, "wl_touch", "frame", &mut [])?;
        }
        Ok(())
    }

    pub(super) fn touch_cancel(&mut self, seat_id: u32) -> Result<(), NativeCompositorError> {
        let clients = self
            .touch_points
            .iter()
            .filter_map(|((seat, _), point)| (*seat == seat_id).then_some(point.client))
            .collect::<BTreeSet<_>>();
        self.touch_points.retain(|(seat, _), _| *seat != seat_id);
        for client in clients {
            for resource in self.resources_for_client(
                client,
                |kind| matches!(kind, ResourceKind::Touch(candidate) if candidate == seat_id),
            )? {
                self.post_event(resource, "wl_touch", "cancel", &mut [])?;
            }
        }
        Ok(())
    }
}
