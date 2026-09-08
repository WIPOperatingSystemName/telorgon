use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum WindowInteraction {
    Move {
        surface: WaylandSurfaceId,
        pointer_start: PointF,
        position_start: PointI,
    },
    Resize {
        surface: WaylandSurfaceId,
        edge: ResizeEdge,
        pointer_start: PointF,
        position_start: PointI,
        size_start: SizeI,
    },
}

impl WindowInteraction {
    pub(super) fn begin_move(
        windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
        surface: WaylandSurfaceId,
        pointer_start: PointF,
    ) -> Option<Self> {
        let window = windows.get(&surface)?;
        Some(Self::Move {
            surface,
            pointer_start,
            position_start: window.position,
        })
    }

    pub(super) fn begin_resize(
        windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
        configure_scheduler: &mut ConfigureScheduler,
        surface: WaylandSurfaceId,
        edge: ResizeEdge,
        pointer_start: PointF,
    ) -> Option<Self> {
        let window = windows.get_mut(&surface)?;
        window.resize_anchor = Some(ResizeAnchor::new(
            window.position,
            window.requested_size,
            edge,
        ));
        // A new pointer grab supersedes an older final configure even if that client has not
        // committed a matching buffer yet. A delayed client must never wedge future resizes.
        window.resize_final = None;
        // Announce the grab without asking the client to redraw at every intermediate size.
        configure_scheduler.schedule_resize(surface, window.configure_size());
        Some(Self::Resize {
            surface,
            edge,
            pointer_start,
            position_start: window.position,
            size_start: window.requested_size,
        })
    }
}

pub(super) fn apply_window_interaction(
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    interaction: WindowInteraction,
    pointer_position: PointF,
    output: SizeI,
    config: &LinuxDesktopConfig,
) -> AppResult<()> {
    match interaction {
        WindowInteraction::Move {
            surface,
            pointer_start,
            position_start,
        } => {
            let Some(window) = windows.get_mut(&surface) else {
                return Ok(());
            };
            let delta = rounded_pointer_delta(pointer_start, pointer_position);
            window.position.x = position_start
                .x
                .saturating_add(delta.x)
                .clamp(32_i32.saturating_sub(window.size.width), output.width - 32);
            window.position.y = position_start
                .y
                .saturating_add(delta.y)
                .clamp(0, output.height - config.titlebar_height.max(1));
        }
        WindowInteraction::Resize {
            surface,
            edge,
            pointer_start,
            position_start,
            size_start,
        } => {
            let delta = rounded_pointer_delta(pointer_start, pointer_position);
            let (position, size) =
                resize_drag_geometry(position_start, size_start, edge, delta, output);
            let Some(window) = windows.get_mut(&surface) else {
                return Ok(());
            };
            window.position = position;
            window.requested_size = size;
        }
    }
    Ok(())
}

pub(super) fn finish_window_interaction(
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    configure_scheduler: &mut ConfigureScheduler,
    interaction: WindowInteraction,
) {
    let WindowInteraction::Resize { surface, .. } = interaction else {
        return;
    };
    if let Some(window) = windows.get_mut(&surface) {
        window.resize_final = Some(FinalResizeConfigure::pending(window.requested_size));
        configure_scheduler.schedule_final(surface, window.requested_size);
    }
}

pub(super) fn flush_resize_configures(
    display: &Display,
    wayland: &mut NativeCompositor<'_>,
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    configure_scheduler: &mut ConfigureScheduler,
    resize_budget_available: &mut bool,
) -> AppResult<()> {
    let pending = configure_scheduler.drain().collect::<Vec<_>>();
    if pending.is_empty() {
        return Ok(());
    }
    for PendingResizeConfigure {
        surface,
        size,
        resizing,
    } in pending
    {
        let Some(window) = windows.get(&surface) else {
            continue;
        };
        let activated = wayland
            .core()
            .seats
            .get(&1)
            .and_then(|seat| seat.keyboard_focus)
            .is_some_and(|focus| focus.surface == surface);
        // XDG configures are superseding state, not a request/response lockstep. Waiting for every
        // earlier ack can deadlock behind a same-size state-only configure; the client's ack of a
        // newer serial validly retires every older configure.
        if resizing && !*resize_budget_available {
            configure_scheduler.defer(PendingResizeConfigure {
                surface,
                size,
                resizing,
            });
            continue;
        }
        let serial = wayland
            .configure_toplevel(
                surface,
                Some(size),
                window_toplevel_states(window, activated, resizing),
            )
            .map_err(app_error)?;
        if !resizing {
            if let Some(final_resize) = windows
                .get_mut(&surface)
                .and_then(|window| window.resize_final.as_mut())
            {
                final_resize.record_sent(size, serial);
            }
        }
        if resizing {
            *resize_budget_available = false;
        }
    }
    // Keep xdg configure delivery ahead of expensive scene preparation and presentation.
    display.flush_clients();
    Ok(())
}

pub(super) fn set_window_maximized(
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    configure_scheduler: &mut ConfigureScheduler,
    surface: WaylandSurfaceId,
    maximized: bool,
    work_area: RectI,
    config: &LinuxDesktopConfig,
) -> AppResult<()> {
    let Some(window) = windows.get_mut(&surface) else {
        return Ok(());
    };
    window.resize_anchor = None;
    window.resize_final = None;
    if maximized {
        if !window.maximized && !window.fullscreen {
            window.restore_geometry = Some((window.position, window.requested_size));
        }
        window.maximized = true;
        window.fullscreen = false;
        window.minimized = false;
        window.position = PointI {
            x: work_area.x,
            y: work_area.y,
        };
        window.requested_size = if window.server_decorated {
            SizeI {
                width: (work_area.width - config.window_border * 2).max(1),
                height: (work_area.height - config.window_border * 2 - config.titlebar_height)
                    .max(1),
            }
        } else {
            SizeI {
                width: work_area.width.max(1),
                height: work_area.height.max(1),
            }
        };
        // Custom frames are measured in their maximized state during refresh_window_frames;
        // that authoritative content extent supersedes this legacy fallback configure.
    } else {
        window.maximized = false;
        if let Some((position, size)) = window.restore_geometry.take() {
            window.position = position;
            window.requested_size = size;
        }
    }
    if maximized {
        window.resize_final = Some(FinalResizeConfigure::pending(window.requested_size));
    }
    configure_scheduler.schedule_final(surface, window.requested_size);
    Ok(())
}

pub(super) fn set_window_fullscreen(
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    configure_scheduler: &mut ConfigureScheduler,
    surface: WaylandSurfaceId,
    fullscreen: bool,
    output: SizeI,
    _config: &LinuxDesktopConfig,
) -> AppResult<()> {
    let Some(window) = windows.get_mut(&surface) else {
        return Ok(());
    };
    window.resize_anchor = None;
    window.resize_final = None;
    if fullscreen {
        if !window.maximized && !window.fullscreen {
            window.restore_geometry = Some((window.position, window.requested_size));
        }
        window.maximized = false;
        window.fullscreen = true;
        window.minimized = false;
        window.position = PointI::default();
        window.requested_size = output;
    } else {
        window.fullscreen = false;
        if let Some((position, size)) = window.restore_geometry.take() {
            window.position = position;
            window.requested_size = size;
        }
    }
    configure_scheduler.schedule_final(surface, window.requested_size);
    Ok(())
}

pub(super) fn window_toplevel_states(
    window: &ClientWindow,
    activated: bool,
    resizing: bool,
) -> ToplevelState {
    ToplevelState {
        maximized: window.maximized,
        fullscreen: window.fullscreen,
        resizing,
        activated,
        ..ToplevelState::default()
    }
}
