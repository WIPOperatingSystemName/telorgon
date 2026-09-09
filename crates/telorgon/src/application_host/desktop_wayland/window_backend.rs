//! Protocol dispatch for the shared desktop frame/policy model.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WindowBackend {
    Wayland,
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
    X11(crate::xwayland::association::XWindow),
}

pub(super) fn close(
    surface: WaylandSurfaceId,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    wayland: &mut NativeCompositor<'_>,
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))] compatibility: Option<
        &mut compatibility::Compatibility,
    >,
) -> AppResult<()> {
    match windows.get(&surface).and_then(|window| window.backend) {
        Some(WindowBackend::Wayland) => wayland.close_toplevel(surface).map_err(app_error)?,
        #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
        Some(WindowBackend::X11(_)) => {
            if let Some(host) = compatibility {
                host.close_surface(surface);
            }
        }
        None => {}
    }
    Ok(())
}

/// Native focus schedules xdg state; X11 focus waits for its checked ICCCM
/// commands before granting the associated Wayland surface keyboard delivery.
pub(super) fn focus(
    display: &Display,
    wayland: &mut NativeCompositor<'_>,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    configure_scheduler: &mut ConfigureScheduler,
    stacking_order: &mut Vec<WaylandSurfaceId>,
    surface: Option<WaylandSurfaceId>,
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))] compatibility: Option<
        &mut compatibility::Compatibility,
    >,
) -> AppResult<()> {
    focus_toplevel(
        display,
        wayland,
        windows,
        configure_scheduler,
        stacking_order,
        surface,
    )?;
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
    if let Some(host) = compatibility {
        host.request_focus(surface, windows);
    }
    Ok(())
}

pub(super) fn flush(
    display: &Display,
    wayland: &mut NativeCompositor<'_>,
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    configure_scheduler: &mut ConfigureScheduler,
    resize_budget_available: &mut bool,
    config: &LinuxDesktopConfig,
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))] compatibility: Option<
        &mut compatibility::Compatibility,
    >,
) -> AppResult<()> {
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
    if let Some(host) = compatibility {
        host.flush_windows(windows, config);
    }
    #[cfg(not(all(feature = "desktop-xwayland", target_env = "gnu")))]
    let _ = config;
    flush_resize_configures(
        display,
        wayland,
        windows,
        configure_scheduler,
        resize_budget_available,
    )
}
