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

/// Compositor control clicks share press/release semantics across client backends.
#[derive(Default)]
pub(super) struct DecorationClick(Option<(WaylandSurfaceId, DecorationHit)>);
impl DecorationClick {
    pub fn press(&mut self, surface: WaylandSurfaceId, hit: DecorationHit) {
        self.0 = Some((surface, hit));
    }
    pub fn release(
        &mut self,
        over: Option<(WaylandSurfaceId, DecorationHit)>,
    ) -> Option<(WaylandSurfaceId, DecorationHit)> {
        self.0.take().filter(|pressed| Some(*pressed) == over)
    }
    pub fn cancel(&mut self) {
        self.0 = None;
    }
    pub fn cancel_surface(&mut self, surface: WaylandSurfaceId) {
        if self.0.is_some_and(|(id, _)| id == surface) {
            self.cancel();
        }
    }
}

#[cfg(test)]
mod decoration_click_tests {
    use super::*;
    #[test]
    fn controls_require_matching_press_and_release_and_fire_only_once() {
        let a = WaylandSurfaceId::from_raw(1).unwrap();
        let b = WaylandSurfaceId::from_raw(2).unwrap();
        for control in [
            DecorationHit::Close,
            DecorationHit::Maximize,
            DecorationHit::Minimize,
        ] {
            let mut click = DecorationClick::default();
            assert_eq!(click.release(Some((a, control))), None);
            click.press(a, control);
            assert_eq!(click.release(Some((a, control))), Some((a, control)));
            assert_eq!(click.release(Some((a, control))), None);
            for outside in [None, Some((a, DecorationHit::Titlebar)), Some((b, control))] {
                click.press(a, control);
                assert_eq!(click.release(outside), None);
                assert_eq!(click.release(Some((a, control))), None);
            }
            click.press(a, control);
            click.cancel_surface(a);
            assert_eq!(click.release(Some((a, control))), None);
            click.press(a, control);
            click.cancel();
            assert_eq!(click.release(Some((a, control))), None);
        }
    }
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
    config: &LinuxShellConfig,
) -> AppResult<()> {
    let grab = wayland
        .core()
        .seats
        .get(&1)
        .and_then(|seat| seat.pointer_grab_focus())
        .map(|focus| focus.surface)
        .filter(|surface| {
            windows.get(surface).is_some_and(|window| {
                client::surface_tree_visible(windows, *surface)
                    && (window.role == SurfaceRole::SessionLock) == session_locked
            })
        });
    let next = grab
        .or_else(|| hit_test_surface(&wayland.core().world, windows, stacking_order, position, config, session_locked))
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
    config: &LinuxShellConfig,
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
    config: &LinuxShellConfig,
) -> AppResult<()> {
    if wayland.drag_active(1) && wayland.drag_touch_slot(1).is_none() {
        let target = hit_test_surface(&wayland.core().world, windows, stacking_order, position, config, session_locked);
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
            SurfaceRole::XdgToplevel if !window.hidden_on_primary() => return Some(candidate),
            SurfaceRole::XdgPopup | SurfaceRole::Subsurface => {
                candidate = window.parent?;
            }
            _ => return None,
        }
    }
    None
}

pub(super) fn hit_test_decoration(
    world: &crate::integrations::wayland::compositor::WaylandWorld,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &[WaylandSurfaceId],
    position: PointF,
    config: &LinuxShellConfig,
    icons: &[(String, Layer)],
) -> Option<(WaylandSurfaceId, DecorationHit)> {
    for surface in stacking_order.iter().rev() {
        let Some(window) = windows.get(surface) else {
            continue;
        };
        let position = window
            .motion_input
            .map_or(position, |input| input.map(position));
        if window.role == SurfaceRole::Xwayland
            && window.backend.is_none()
            && !window.hidden_on_primary()
            && surface_placement(window, window.position, config).contains(position)
        {
            // An unmanaged menu above a frame owns its pixels; do not activate
            // the underlying titlebar/control through it.
            return None;
        }
        if window.role == SurfaceRole::XdgToplevel && !window_has_frame(window)
            && !window.fullscreen && !window.hidden_on_primary()
            && surface_placement(window, window.position, config).contains(position)
            && surface_accepts_input(world, windows, *surface, position, config)
        {
            // A foreground client's resize margin wins over a background server frame.
            return None;
        }
        if window.backend.is_some() && !window.hidden_on_primary() {
            let content = window_content_rect(window, window.position, config);
            if position.x >= content.x as f32
                && position.x < content.right() as f32
                && position.y >= content.y as f32
                && position.y < content.bottom() as f32
            {
                // A veil is compositor content, not a hit target for a lower window's controls.
                if window.resize_veil_active()
                    || window.motion_input.is_some_and(|i| i.block_content)
                {
                    return Some((*surface, DecorationHit::Frame));
                }
                if !window_has_frame(window) {
                    return None;
                }
            }
        }
        if window.backend.is_none() || window.hidden_on_primary() || !window_has_frame(window) {
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
        let border = window_border_width(window, config).max(0);
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
        if let Some(edge) = edge
            && !window.maximized
        {
            return Some((*surface, DecorationHit::Resize(edge)));
        }
        if window_has_titlebar(window) && local.y < border + config.titlebar_height {
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
    config: &LinuxShellConfig,
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
    let next = decoration_pointer_request(&wayland.core().world, frames, windows, stacking_order, position, config, icons)
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
    world: &crate::integrations::wayland::compositor::WaylandWorld,
    frames: &BTreeMap<WaylandSurfaceId, WindowFrameLayer>,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &[WaylandSurfaceId],
    position: PointF,
    config: &LinuxShellConfig,
    icons: &[(String, Layer)],
) -> Option<PointerRequest> {
    let (surface, hit) = hit_test_decoration(world, windows, stacking_order, position, config, icons)?;
    // Tiled resize cursors belong to the shared-divider controller, including wider hit regions.
    if windows.get(&surface).is_some_and(|w| w.tile.is_some())
        && matches!(hit, DecorationHit::Resize(_))
    {
        return Some(PointerRequest::Semantic(PointerIcon::Default));
    }
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

fn surface_accepts_input(
    world: &crate::integrations::wayland::compositor::WaylandWorld,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    surface: WaylandSurfaceId,
    position: PointF,
    config: &LinuxShellConfig,
) -> bool {
    let Some(state) = world.surface(surface) else { return false };
    state.snapshot().input_region.as_ref().is_none_or(|region| {
        let local = surface_local_position(windows, surface, position, config);
        region.rectangles().iter().any(|rect| {
            let x = f64::from(local.x);
            let y = f64::from(local.y);
            x >= f64::from(rect.x) && y >= f64::from(rect.y)
                && x < f64::from(rect.x) + f64::from(rect.width)
                && y < f64::from(rect.y) + f64::from(rect.height)
        })
    })
}

pub(super) fn hit_test_surface(
    world: &crate::integrations::wayland::compositor::WaylandWorld,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &[WaylandSurfaceId],
    position: PointF,
    config: &LinuxShellConfig,
    session_locked: bool,
) -> Option<WaylandSurfaceId> {
    // A foreground frame can overlap a background client's content. Give the frame
    // exclusive pointer ownership so leaving it sends a fresh enter to that client,
    // even when the client had focus before the pointer crossed the frame.
    if !session_locked
        && hit_test_decoration(world, windows, stacking_order, position, config, &[]).is_some()
    {
        return None;
    }
    stacking_order
        .iter()
        .rev()
        .filter_map(|surface| windows.get(surface).map(|window| (*surface, window)))
        .filter(|(surface, _)| {
            resize_veil_owner(windows, *surface).is_none_or(|owner| owner == *surface)
        })
        // Test committed protocol state before selecting an occluding surface. An empty
        // input region makes a rendering subsurface transparent to input, not to painting.
        .filter(|(surface, _)| {
            surface_accepts_input(world, windows, *surface, position, config)
        })
        .find(|(surface, window)| {
            let position = window.motion_input.map_or(position, |input|input.map(position));
            let origin = surface_tree_position(windows, *surface, config);
            let target = if window.role == SurfaceRole::XdgToplevel
                && !window_has_frame(window) && !window.fullscreen
            {
                // GTK resize handles live in the surface margins outside xdg geometry.
                // The committed input region above excludes noninteractive shadow pixels.
                surface_placement(window, origin, config).target
            } else {
                window_content_rect(window, origin, config)
            };
            window.role != SurfaceRole::Cursor
                && client::surface_tree_visible(windows, *surface)
                && (window.role == SurfaceRole::SessionLock) == session_locked
                && position.x >= target.x as f32
                && position.y >= target.y as f32
                && position.x < target.right() as f32
                && position.y < target.bottom() as f32
        })
        // The first window owns its whole live slot, including resize padding. Do not send
        // out-of-buffer coordinates to it or let padding click through to a lower window.
        .filter(|(surface, window)| {
            let position = window.motion_input.map_or(position, |input|input.map(position));
            !window.motion_input.is_some_and(|i|i.block_content) && resize_veil_owner(windows, *surface).is_none()
                && surface_placement(window, surface_tree_position(windows, *surface, config), config).contains(position)
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
pub(super) fn test_input_world(surfaces: &[WaylandSurfaceId]) -> crate::integrations::wayland::compositor::WaylandWorld {
    use crate::integrations::wayland::compositor::{ClientId, WaylandWorld};
    let mut world = WaylandWorld::default();
    let client = ClientId::from_raw(1).unwrap();
    world.add_client(client).unwrap();
    for surface in surfaces {
        world.create_surface(client, *surface).unwrap();
    }
    world
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_resize_margins_receive_input_but_transparent_shadows_do_not() {
        use super::super::client::maximize_preview_tests::test_window;
        use crate::integrations::wayland::compositor::Region;
        let back = WaylandSurfaceId::from_raw(1).unwrap();
        let front = WaylandSurfaceId::from_raw(2).unwrap();
        let stack = [back, front];
        let config = LinuxShellConfig::default();
        let mut world = test_input_world(&stack);
        let mut background = test_window(SizeI { width: 800, height: 600 }, PointI::default());
        background.server_decorated = false;
        let mut firefox = test_window(SizeI { width: 440, height: 330 }, PointI { x: 100, y: 100 });
        firefox.server_decorated = false;
        firefox.window_geometry = RectI { x: 20, y: 15, width: 400, height: 300 };
        // Five pixels of active resize margin surround the declared window geometry.
        world.surface_mut(front).unwrap().set_input_region(Some(Region::from_rectangles(vec![
            RectI { x: 15, y: 10, width: 410, height: 310 },
        ]).unwrap()));
        world.surface_mut(front).unwrap().commit().unwrap();
        let mut windows = BTreeMap::from([(back, background), (front, firefox)]);
        for point in [
            PointF { x: 97.0, y: 200.0 }, PointF { x: 502.0, y: 200.0 },
            PointF { x: 200.0, y: 97.0 }, PointF { x: 200.0, y: 402.0 },
            PointF { x: 97.0, y: 97.0 }, PointF { x: 502.0, y: 402.0 },
        ] {
            assert_eq!(hit_test_surface(&world, &windows, &stack, point, &config, false), Some(front), "{point:?}");
        }
        for point in [PointF { x: 90.0, y: 200.0 }, PointF { x: 200.0, y: 410.0 }] {
            assert_eq!(hit_test_surface(&world, &windows, &stack, point, &config, false), Some(back));
        }
        let background = windows.get_mut(&back).unwrap();
        background.server_decorated = true;
        background.position = PointI { x: 50, y: 90 };
        let margin = PointF { x: 97.0, y: 97.0 };
        assert_eq!(hit_test_surface(&world, &windows, &stack, margin, &config, false), Some(front));
        assert_eq!(hit_test_decoration(&world, &windows, &stack, margin, &config, &[]), None);
        let shadow = PointF { x: 90.0, y: 97.0 };
        assert!(matches!(hit_test_decoration(&world, &windows, &stack, shadow, &config, &[]), Some((owner, _)) if owner == back));
    }

    #[test]
    fn committed_input_regions_route_through_firefox_rendering_subsurface() {
        use super::super::client::maximize_preview_tests::test_window;
        use crate::integrations::wayland::compositor::Region;
        let root = WaylandSurfaceId::from_raw(7).unwrap();
        let child = WaylandSurfaceId::from_raw(4).unwrap();
        let stack = [root, child];
        let config = LinuxShellConfig::default();
        let mut world = test_input_world(&stack);
        let size = SizeI { width: 400, height: 300 };
        let mut parent = test_window(size, PointI { x: 48, y: 48 });
        parent.server_decorated = false;
        parent.window_geometry.x = 26;
        parent.window_geometry.y = 23;
        let mut content = test_window(size, PointI::default());
        content.role = SurfaceRole::Subsurface;
        content.backend = None;
        content.parent = Some(root);
        content.offset = PointI { x: 26, y: 23 };
        let windows = BTreeMap::from([(root, parent), (child, content)]);
        let pointer = PointF { x: 100.0, y: 100.0 };
        let hit = |world: &crate::integrations::wayland::compositor::WaylandWorld, point| {
            hit_test_surface(world, &windows, &stack, point, &config, false)
        };
        assert_eq!(hit(&world, pointer), Some(child)); // Default region is infinite.
        world.surface_mut(child).unwrap().set_input_region(Some(Region::empty()));
        assert_eq!(hit(&world, pointer), Some(child)); // Pending state is not effective.
        world.surface_mut(child).unwrap().commit().unwrap();
        assert_eq!(hit(&world, pointer), Some(root));

        // Regions use surface-local logical coordinates, independent of buffer scale.
        let state = world.surface_mut(child).unwrap();
        state.set_buffer_scale(2).unwrap();
        state.set_input_region(Some(Region::from_rectangles(vec![RectI {
            x: 50, y: 50, width: 10, height: 10,
        }]).unwrap()));
        state.commit().unwrap();
        assert_eq!(hit(&world, pointer), Some(child)); // Child-local (52, 52).
        assert_eq!(hit(&world, PointF { x: 97.5, y: 100.0 }), Some(root));
        assert_eq!(hit(&world, PointF { x: 108.0, y: 100.0 }), Some(root));
        assert_eq!(hit(&world, PointF { x: 100.0, y: 108.0 }), Some(root));

        world.surface_mut(child).unwrap().set_input_region(None);
        world.surface_mut(child).unwrap().commit().unwrap();
        assert_eq!(hit(&world, PointF { x: 108.0, y: 100.0 }), Some(child));
        for surface in stack {
            world.surface_mut(surface).unwrap().set_input_region(Some(Region::empty()));
            world.surface_mut(surface).unwrap().commit().unwrap();
        }
        assert_eq!(hit(&world, pointer), None);
    }

    #[test]
    fn foreground_resize_border_revokes_background_pointer_focus() {
        use super::super::client::maximize_preview_tests::test_window;
        let back = WaylandSurfaceId::from_raw(1).unwrap();
        let front = WaylandSurfaceId::from_raw(2).unwrap();
        let world = test_input_world(&[back, front]);
        let config = LinuxShellConfig::default();
        let mut windows = BTreeMap::from([
            (
                back,
                test_window(
                    SizeI {
                        width: 800,
                        height: 600,
                    },
                    PointI::default(),
                ),
            ),
            (
                front,
                test_window(
                    SizeI {
                        width: 200,
                        height: 200,
                    },
                    PointI { x: 200, y: 200 },
                ),
            ),
        ]);
        let backends = [
            WindowBackend::Wayland,
            #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
            WindowBackend::X11(crate::integrations::x11::association::XWindow {
                generation: 1,
                xid: 10,
                incarnation: 1,
            }),
        ];
        for backend in backends {
            for window in windows.values_mut() {
                window.backend = Some(backend);
                #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                if matches!(backend, WindowBackend::X11(_)) {
                    window.role = SurfaceRole::Xwayland;
                }
            }
            let stack = [back, front];
            let border = PointF { x: 200.0, y: 280.0 };
            let behind = PointF { x: 180.0, y: 280.0 };
            assert!(
                matches!(hit_test_decoration(&world, &windows, &stack, border, &config, &[]),
            Some((id, DecorationHit::Resize(_))) if id == front)
            );
            assert_eq!(
                hit_test_surface(&world, &windows, &stack, border, &config, false),
                None
            );
            assert_eq!(
                hit_test_surface(&world, &windows, &stack, behind, &config, false),
                Some(back)
            );
            assert!(pointer_focus_requires_transition(None, Some(back)));
        }
    }

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
