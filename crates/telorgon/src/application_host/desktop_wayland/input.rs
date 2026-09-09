use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DecorationHit {
    Frame,
    Titlebar,
    Resize(ResizeEdge),
    Close,
    Maximize,
    Minimize,
    SystemMenu,
    ShellAction(crate::ShellActionId),
}

fn pointer_focus_requires_transition(
    seat_focus: Option<WaylandSurfaceId>,
    next: Option<WaylandSurfaceId>,
) -> bool {
    seat_focus != next
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update_pointer_focus(
    display: &Display,
    wayland: &mut NativeCompositor<'_>,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &[WaylandSurfaceId],
    session_locked: bool,
    current: &mut Option<WaylandSurfaceId>,
    position: PointF,
    config: &LinuxDesktopConfig,
) -> AppResult<()> {
    let grab = wayland
        .core()
        .seats
        .get(&1)
        .and_then(|seat| seat.pointer_grab_focus())
        .map(|focus| focus.surface)
        .filter(|surface| {
            windows.get(surface).is_some_and(|window| {
                !window.minimized && (window.role == SurfaceRole::SessionLock) == session_locked
            })
        });
    let next = grab
        .or_else(|| hit_test_surface(windows, stacking_order, position, config, session_locked))
        .filter(|surface| wayland.core().world.surface(*surface).is_some());
    let seat_focus = wayland
        .core()
        .seats
        .get(&1)
        .and_then(|seat| seat.pointer_focus)
        .map(|focus| focus.surface);
    if pointer_focus_requires_transition(seat_focus, next) {
        let local = next.map_or(position, |surface| {
            surface_local_position(windows, surface, position, config)
        });
        wayland
            .set_pointer_focus(1, next, local, display.next_serial())
            .map_err(app_error)?;
    }
    *current = next;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reconcile_pointer_state(
    display: &Display,
    wayland: &mut NativeCompositor<'_>,
    frames: &mut BTreeMap<WaylandSurfaceId, WindowFrameLayer>,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &[WaylandSurfaceId],
    session_locked: bool,
    current: &mut Option<WaylandSurfaceId>,
    position: PointF,
    config: &LinuxDesktopConfig,
    icons: &[(String, Layer)],
    now: MonotonicInstant,
) -> AppResult<bool> {
    update_pointer_focus(
        display,
        wayland,
        windows,
        stacking_order,
        session_locked,
        current,
        position,
        config,
    )?;
    let repaint = route_frame_pointer_motion(frames, windows, position, session_locked, now);
    set_decoration_pointer_cursor(
        wayland,
        frames,
        windows,
        stacking_order,
        *current,
        position,
        config,
        icons,
    );
    Ok(repaint)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn route_pointer_motion(
    display: &Display,
    wayland: &mut NativeCompositor<'_>,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &[WaylandSurfaceId],
    session_locked: bool,
    current: &mut Option<WaylandSurfaceId>,
    position: PointF,
    time: u32,
    config: &LinuxDesktopConfig,
) -> AppResult<()> {
    if wayland.drag_active(1) && wayland.drag_touch_slot(1).is_none() {
        let target = hit_test_surface(windows, stacking_order, position, config, session_locked);
        let local = target.map_or(position, |surface| {
            surface_local_position(windows, surface, position, config)
        });
        wayland
            .drag_motion(1, target, time, local)
            .map_err(app_error)?;
    } else {
        update_pointer_focus(
            display,
            wayland,
            windows,
            stacking_order,
            session_locked,
            current,
            position,
            config,
        )?;
        if let Some(surface) = *current {
            let local = surface_local_position(windows, surface, position, config);
            let _ = wayland.pointer_motion(1, time, local);
        }
    }
    Ok(())
}

pub(super) fn focus_toplevel(
    display: &Display,
    wayland: &mut NativeCompositor<'_>,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    configure_scheduler: &mut ConfigureScheduler,
    stacking_order: &mut Vec<WaylandSurfaceId>,
    surface: Option<WaylandSurfaceId>,
) -> AppResult<()> {
    let surface = surface
        .and_then(|surface| toplevel_ancestor(windows, surface))
        .filter(|surface| wayland.core().world.surface(*surface).is_some());
    if let Some(surface) = surface {
        raise_toplevel(windows, stacking_order, surface);
    }
    let previous = wayland
        .core()
        .seats
        .get(&1)
        .and_then(|seat| seat.keyboard_focus)
        .map(|focus| focus.surface);
    if previous == surface {
        return Ok(());
    }
    if let Some(previous) = previous
        && wayland
            .core()
            .world
            .surface(previous)
            .is_some_and(|surface| surface.snapshot().role == Some(SurfaceRole::XdgToplevel))
    {
        if let Some(window) = windows.get(&previous) {
            configure_scheduler.schedule_state(
                previous,
                window.configure_size(),
                window.resizing(),
            );
        }
    }
    wayland
        .set_keyboard_focus(1, surface, display.next_serial())
        .map_err(app_error)?;
    if let Some(surface) = surface
        && wayland
            .core()
            .world
            .surface(surface)
            .is_some_and(|surface| surface.snapshot().role == Some(SurfaceRole::XdgToplevel))
    {
        if let Some(window) = windows.get(&surface) {
            configure_scheduler.schedule_state(surface, window.configure_size(), window.resizing());
        }
    }
    Ok(())
}

pub(super) fn raise_toplevel(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &mut Vec<WaylandSurfaceId>,
    surface: WaylandSurfaceId,
) {
    let Some(owner) = toplevel_ancestor(windows, surface) else {
        return;
    };
    // Stable partition keeps popups/subsurfaces above their owner and preserves all
    // unrelated windows. Only explicit activation calls this, never client redraws.
    let mut family: Vec<_> = stacking_order
        .iter()
        .copied()
        .filter(|candidate| toplevel_ancestor(windows, *candidate) == Some(owner))
        .collect();
    // Minimize removes the toplevel from the stack; activation restores its slot.
    if !family.contains(&owner) {
        family.insert(0, owner);
    }
    stacking_order.retain(|candidate| toplevel_ancestor(windows, *candidate) != Some(owner));
    stacking_order.extend(family);
}

fn toplevel_ancestor(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    surface: WaylandSurfaceId,
) -> Option<WaylandSurfaceId> {
    let mut candidate = surface;
    // A valid surface tree cannot contain a parent cycle. The bound also fails closed if corrupt
    // state reaches this host-side cache.
    for _ in 0..=windows.len() {
        let window = windows.get(&candidate)?;
        match window.role {
            SurfaceRole::XdgToplevel if !window.minimized => return Some(candidate),
            SurfaceRole::XdgPopup | SurfaceRole::Subsurface => {
                candidate = window.parent?;
            }
            _ => return None,
        }
    }
    None
}

pub(super) fn hit_test_decoration(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &[WaylandSurfaceId],
    position: PointF,
    config: &LinuxDesktopConfig,
    icons: &[(String, Layer)],
) -> Option<(WaylandSurfaceId, DecorationHit)> {
    for surface in stacking_order.iter().rev() {
        let Some(window) = windows.get(surface) else {
            continue;
        };
        if window.role == SurfaceRole::XdgToplevel && !window.minimized {
            let content = window_content_rect(window, window.position, config);
            if position.x >= content.x as f32
                && position.x < content.right() as f32
                && position.y >= content.y as f32
                && position.y < content.bottom() as f32
            {
                // A veil is compositor content, not a hit target for a lower window's controls.
                if window.resize_anchor.is_some() {
                    return Some((*surface, DecorationHit::Frame));
                }
                if !window_is_decorated(window) {
                    return None;
                }
            }
        }
        if window.role != SurfaceRole::XdgToplevel
            || window.minimized
            || !window_is_decorated(window)
        {
            continue;
        }
        let outer = window
            .chrome_outer
            .unwrap_or_else(|| legacy_window_outer(window, config));
        let local = PointI {
            x: (position.x - window.position.x as f32).floor() as i32,
            y: (position.y - window.position.y as f32).floor() as i32,
        };
        let inside_frame =
            local.x >= 0 && local.y >= 0 && local.x < outer.width && local.y < outer.height;
        if let Some(chrome) = &window.chrome {
            let point = PointF {
                x: position.x - window.position.x as f32,
                y: position.y - window.position.y as f32,
            };
            let role = chrome.hit_test(point.x, point.y);
            // Only explicitly published resize targets may extend beyond the window. Do not
            // reject their outward tolerance before asking the shared chrome hit geometry.
            if !inside_frame
                && !matches!(
                    role,
                    Some(crate::WindowChromeRole::Action(WindowAction::BeginResize(
                        _
                    )))
                )
            {
                continue;
            }
            if chrome.hit_test_content(point.x, point.y) {
                return None;
            }
            let hit = match role {
                Some(crate::WindowChromeRole::DragRegion)
                | Some(crate::WindowChromeRole::Action(WindowAction::BeginMove)) => {
                    DecorationHit::Titlebar
                }
                Some(crate::WindowChromeRole::Action(WindowAction::BeginResize(edge))) => {
                    DecorationHit::Resize(wayland_resize_edge(edge))
                }
                Some(crate::WindowChromeRole::Action(WindowAction::Close)) => DecorationHit::Close,
                Some(crate::WindowChromeRole::Action(WindowAction::Minimize)) => {
                    DecorationHit::Minimize
                }
                Some(crate::WindowChromeRole::Action(WindowAction::ToggleMaximize)) => {
                    DecorationHit::Maximize
                }
                Some(crate::WindowChromeRole::Action(WindowAction::ShowSystemMenu)) => {
                    DecorationHit::SystemMenu
                }
                Some(crate::WindowChromeRole::ShellAction(action)) => {
                    DecorationHit::ShellAction(action)
                }
                Some(
                    crate::WindowChromeRole::Frame
                    | crate::WindowChromeRole::Content
                    | crate::WindowChromeRole::Title
                    | crate::WindowChromeRole::AppIcon,
                )
                | None => DecorationHit::Frame,
            };
            return Some((*surface, hit));
        }
        if !inside_frame {
            continue;
        }
        let border = config.window_border.max(1);
        let left = local.x < border;
        let right = local.x >= outer.width - border;
        let top = local.y < border;
        let bottom = local.y >= outer.height - border;
        let edge = match (left, right, top, bottom) {
            (true, _, true, _) => Some(ResizeEdge::TopLeft),
            (_, true, true, _) => Some(ResizeEdge::TopRight),
            (true, _, _, true) => Some(ResizeEdge::BottomLeft),
            (_, true, _, true) => Some(ResizeEdge::BottomRight),
            (true, _, _, _) => Some(ResizeEdge::Left),
            (_, true, _, _) => Some(ResizeEdge::Right),
            (_, _, true, _) => Some(ResizeEdge::Top),
            (_, _, _, true) => Some(ResizeEdge::Bottom),
            _ => None,
        };
        if let Some(edge) = edge {
            return Some((*surface, DecorationHit::Resize(edge)));
        }
        if local.y < border + config.titlebar_height {
            let icon_extent = config.titlebar_height.clamp(1, 24);
            for (index, (name, hit)) in [
                ("window.close", DecorationHit::Close),
                ("window.maximize", DecorationHit::Maximize),
                ("window.minimize", DecorationHit::Minimize),
            ]
            .into_iter()
            .enumerate()
            {
                if !icons.iter().any(|(candidate, _)| candidate == name) {
                    continue;
                }
                let x = outer.width - border - (index as i32 + 1) * (icon_extent + 4);
                let y = border + (config.titlebar_height - icon_extent) / 2;
                if local.x >= x
                    && local.x < x + icon_extent
                    && local.y >= y
                    && local.y < y + icon_extent
                {
                    return Some((*surface, hit));
                }
            }
            return Some((*surface, DecorationHit::Titlebar));
        }
        return None;
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(super) fn set_decoration_pointer_cursor(
    wayland: &mut NativeCompositor<'_>,
    frames: &BTreeMap<WaylandSurfaceId, WindowFrameLayer>,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &[WaylandSurfaceId],
    client_focus: Option<WaylandSurfaceId>,
    position: PointF,
    config: &LinuxDesktopConfig,
    icons: &[(String, Layer)],
) {
    if wayland
        .core()
        .seats
        .get(&1)
        .is_some_and(|seat| seat.pointer_grab_focus().is_some())
    {
        return;
    }
    let next = decoration_pointer_request(frames, windows, stacking_order, position, config, icons)
        .map(pointer_request_cursor_image)
        .or_else(|| {
            client_focus
                .is_none()
                .then_some(CursorImage::TelorgonDefault)
        });
    let Some(next) = next else {
        return;
    };
    let Some(seat) = wayland.core_mut().seats.get_mut(&1) else {
        return;
    };
    if seat.cursor != next {
        seat.cursor = next;
    }
}

pub(super) fn decoration_pointer_request(
    frames: &BTreeMap<WaylandSurfaceId, WindowFrameLayer>,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &[WaylandSurfaceId],
    position: PointF,
    config: &LinuxDesktopConfig,
    icons: &[(String, Layer)],
) -> Option<PointerRequest> {
    let (surface, hit) = hit_test_decoration(windows, stacking_order, position, config, icons)?;
    if let (Some(frame), Some(window)) = (frames.get(&surface), windows.get(&surface)) {
        let local = PointF {
            x: position.x - window.position.x as f32,
            y: position.y - window.position.y as f32,
        };
        if let Some(region) = window
            .chrome
            .as_ref()
            .and_then(|chrome| chrome.hit_test_region(local.x, local.y))
            && let Some(request) = frame
                .layer
                .runtime
                .ui()
                .pointer_requests
                .get(region.node)
                .copied()
        {
            return Some(request);
        }
        if let Some(request) = frame
            .layer
            .runtime
            .ui()
            .pointer_requests
            .iter()
            .filter_map(|(node, request)| {
                frame
                    .layer
                    .runtime
                    .layout()
                    .computed(node)
                    .and_then(|computed| {
                        (computed.visible_rect.contains(local)
                            && computed.border_rect.contains(local))
                        .then_some((*request, computed.border_rect.area()))
                    })
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(request, _)| request)
        {
            return Some(request);
        }
    }
    Some(match hit {
        DecorationHit::Titlebar => PointerRequest::Semantic(PointerIcon::Move),
        DecorationHit::Resize(edge) => PointerRequest::Semantic(resize_edge_pointer_icon(edge)),
        DecorationHit::Close
        | DecorationHit::Maximize
        | DecorationHit::Minimize
        | DecorationHit::SystemMenu
        | DecorationHit::ShellAction(_) => PointerRequest::Semantic(PointerIcon::Pointer),
        DecorationHit::Frame => PointerRequest::Semantic(PointerIcon::Default),
    })
}

pub(super) fn invoke_shell_action(
    handlers: &[ShellActionHandler],
    action: crate::ShellActionId,
    surface: WaylandSurfaceId,
    frames: &BTreeMap<WaylandSurfaceId, WindowFrameLayer>,
) {
    let Some(handler) = handlers.iter().find(|handler| handler.id() == action) else {
        return;
    };
    let Some(frame) = frames.get(&surface) else {
        return;
    };
    handler.invoke(frame.model.clone());
}

pub(super) fn hit_test_surface(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &[WaylandSurfaceId],
    position: PointF,
    config: &LinuxDesktopConfig,
    session_locked: bool,
) -> Option<WaylandSurfaceId> {
    stacking_order
        .iter()
        .rev()
        .filter_map(|surface| windows.get(surface).map(|window| (*surface, window)))
        .filter(|(surface, _)| {
            resize_veil_owner(windows, *surface).is_none_or(|owner| owner == *surface)
        })
        .find(|(_, window)| {
            let target = window_content_rect(window, window.position, config);
            window.role != SurfaceRole::Cursor
                && !window.minimized
                && (window.role == SurfaceRole::SessionLock) == session_locked
                && position.x >= target.x as f32
                && position.y >= target.y as f32
                && position.x < target.right() as f32
                && position.y < target.bottom() as f32
        })
        // The first window owns its whole live slot, including resize padding. Do not send
        // out-of-buffer coordinates to it or let padding click through to a lower window.
        .filter(|(surface, window)| {
            resize_veil_owner(windows, *surface).is_none()
                && surface_placement(window, window.position, config).contains(position)
                // Rounded corner handles overlap the rectangular content slot. Use the same
                // chrome geometry as cursor/decoration routing so leaving a handle produces a
                // fresh client enter (and a new opportunity to install its cursor).
                && window.chrome.as_ref().is_none_or(|chrome| {
                    chrome.hit_test_content(
                        position.x - window.position.x as f32,
                        position.y - window.position.y as f32,
                    )
                })
        })
        .map(|(surface, _)| surface)
}

pub(super) fn normalized_output_position(normalized: PointF, extent: SizeI) -> PointF {
    PointF {
        x: normalized.x.clamp(0.0, 1.0) * (extent.width - 1) as f32,
        y: normalized.y.clamp(0.0, 1.0) * (extent.height - 1) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seat_focus_is_authoritative_when_the_local_cache_is_stale() {
        let surface = WaylandSurfaceId::from_raw(7).unwrap();
        let stale_local_cache = Some(surface);
        let seat_focus = None;
        let next = Some(surface);

        assert_eq!(stale_local_cache, next);
        assert!(pointer_focus_requires_transition(seat_focus, next));
    }
}
