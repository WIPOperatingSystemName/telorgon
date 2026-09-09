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
        if window.fullscreen
            || window.minimized
            || (window.maximized && window.restore_geometry.is_none())
        {
            return None;
        }
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
        if window.backend == Some(WindowBackend::Wayland) {
            window.native_configure.resize_anchor = Some(ResizeAnchor::new(
                window.position,
                window.requested_size,
                edge,
            ));
            // A new pointer grab supersedes an older final configure even if that client has not
            // committed a matching buffer yet. A delayed client must never wedge future resizes.
            window.native_configure.resize_final = None;
            // Announce the grab without asking the client to redraw at every intermediate size.
            configure_scheduler.schedule_resize(surface, window.configure_size());
        }
        #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
        if matches!(window.backend, Some(WindowBackend::X11(_))) {
            window
                .resize_preview
                .begin(window.position, window.requested_size, edge);
        }
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
    interaction: &mut WindowInteraction,
    configure_scheduler: &mut ConfigureScheduler,
    pointer_position: PointF,
    output: SizeI,
    config: &LinuxDesktopConfig,
) -> AppResult<()> {
    match *interaction {
        WindowInteraction::Move {
            surface,
            pointer_start,
            position_start,
        } => {
            let Some(window) = windows.get_mut(&surface) else {
                return Ok(());
            };
            if window.maximized {
                // Keep a click (or small pointer jitter) from restoring the window.
                if (pointer_position.x - pointer_start.x)
                    .hypot(pointer_position.y - pointer_start.y)
                    < 4.0
                {
                    return Ok(());
                }
                let Some((_, restored_size)) = window.restore_geometry.take() else {
                    return Ok(());
                };
                let outer_width = window
                    .chrome_outer
                    .map_or(window.requested_size.width, |s| s.width)
                    .max(1);
                let grab_fraction = ((pointer_start.x - window.position.x as f32)
                    / outer_width as f32)
                    .clamp(0.0, 1.0);
                let grab_y = (pointer_start.y - window.position.y as f32).max(0.0);
                window.maximized = false;
                #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
                {
                    window.resize_preview = Default::default();
                }
                window.native_configure.resize_anchor = None;
                window.native_configure.resize_final = None;
                window.requested_size = restored_size;
                window.position = PointI {
                    x: (pointer_position.x - grab_fraction * restored_size.width as f32).round()
                        as i32,
                    y: (pointer_position.y - grab_y).round().max(0.0) as i32,
                };
                // Rebase the ongoing grab so the next motion continues from the restored
                // position rather than jumping back to the maximized origin.
                *interaction = WindowInteraction::Move {
                    surface,
                    pointer_start: pointer_position,
                    position_start: window.position,
                };
                configure_scheduler.schedule_final(surface, restored_size);
                return Ok(());
            }
            let delta = rounded_pointer_delta(pointer_start, pointer_position);
            window.position.x = position_start.x.saturating_add(delta.x).clamp(
                32_i32.saturating_sub(window.requested_size.width),
                output.width - 32,
            );
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
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
    if let Some(window) = windows.get_mut(&surface) {
        if matches!(window.backend, Some(WindowBackend::X11(_))) {
            window.resize_preview.finish();
        }
    }
    if let Some(window) = windows
        .get_mut(&surface)
        .filter(|window| window.backend == Some(WindowBackend::Wayland))
    {
        window.native_configure.resize_final =
            Some(FinalResizeConfigure::pending(window.requested_size));
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
        let Some(window) = windows
            .get(&surface)
            .filter(|window| window.backend == Some(WindowBackend::Wayland))
        else {
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
                .and_then(|window| window.native_configure.resize_final.as_mut())
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
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
    {
        window.resize_preview = Default::default();
    }
    window.native_configure.resize_anchor = None;
    window.native_configure.resize_final = None;
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
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
    if maximized && matches!(window.backend, Some(WindowBackend::X11(_))) {
        window.resize_preview.finish();
    }
    if maximized && window.backend == Some(WindowBackend::Wayland) {
        window.native_configure.resize_final =
            Some(FinalResizeConfigure::pending(window.requested_size));
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
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
    {
        window.resize_preview = Default::default();
    }
    window.native_configure.resize_anchor = None;
    window.native_configure.resize_final = None;
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
