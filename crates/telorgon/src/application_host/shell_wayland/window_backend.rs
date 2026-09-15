//! Protocol dispatch for the shared desktop frame/policy model.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WindowBackend {
    Wayland,
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    X11(crate::xwayland::association::XWindow),
}

pub(super) fn close(
    surface: WaylandSurfaceId,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    wayland: &mut NativeCompositor<'_>,
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))] compatibility: Option<
        &mut compatibility::Compatibility,
    >,
) -> AppResult<()> {
    match windows.get(&surface).and_then(|window| window.backend) {
        Some(WindowBackend::Wayland) => wayland.close_toplevel(surface).map_err(app_error)?,
        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
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
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))] compatibility: Option<
        &mut compatibility::Compatibility,
    >,
) -> AppResult<()> {
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    if let Some(target) = x11_focus_owner(windows, surface) {
        let current = wayland
            .core()
            .seats
            .get(&1)
            .and_then(|seat| seat.keyboard_focus)
            .map(|focus| focus.surface);
        if current == Some(target) {
            if let Some(host) = compatibility {
                host.raise_focused(target, windows, stacking_order);
            }
            return Ok(());
        }
    }
    focus_toplevel(
        display,
        wayland,
        windows,
        configure_scheduler,
        stacking_order,
        surface,
    )?;
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
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
    config: &LinuxShellConfig,
    work_area: RectI,
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))] compatibility: Option<
        &mut compatibility::Compatibility,
    >,
) -> AppResult<()> {
    super::size_policy::apply(wayland, windows, configure_scheduler, config, work_area);
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    if let Some(host) = compatibility {
        host.flush_windows(windows, config);
    }
    #[cfg(not(all(feature = "shell-xwayland", target_env = "gnu")))]
    let _ = config;
    flush_resize_configures(
        display,
        wayland,
        windows,
        configure_scheduler,
        resize_budget_available,
    )
}

#[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
fn x11_focus_owner(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    surface: Option<WaylandSurfaceId>,
) -> Option<WaylandSurfaceId> {
    let mut surface = surface?;
    for _ in 0..windows.len() {
        let window = windows.get(&surface)?;
        if window.minimized {
            return None;
        }
        if window.role == SurfaceRole::Xwayland {
            return Some(surface);
        }
        surface = window.parent?;
    }
    None
}

#[cfg(all(test, feature = "shell-xwayland", target_env = "gnu"))]
mod focus_tests {
    use super::super::client::maximize_preview_tests::test_window;
    use super::*;

    #[test]
    fn clicking_focused_x11_surface_preserves_keyboard_delivery_and_enter_serial() {
        use crate::compositor_wayland::{
            ClientId, ClientLimits, KeyboardFocus, SeatCapabilities, SeatState,
        };
        let display = Display::new().unwrap();
        let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        let surface = WaylandSurfaceId::from_raw(10).unwrap();
        let expected = KeyboardFocus {
            client: ClientId::from_raw(1).unwrap(),
            surface,
            enter_serial: 77,
        };
        let mut seat = SeatState::new(
            "seat0",
            SeatCapabilities {
                keyboard: true,
                ..Default::default()
            },
        );
        seat.keyboard_focus = Some(expected);
        native.core_mut().seats.insert(1, seat);
        let mut window = test_window(
            SizeI {
                width: 100,
                height: 100,
            },
            PointI::default(),
        );
        window.role = SurfaceRole::Xwayland;
        let windows = BTreeMap::from([(surface, window)]);
        let mut scheduler = ConfigureScheduler::default();
        let mut order = vec![surface];
        focus(
            &display,
            &mut native,
            &windows,
            &mut scheduler,
            &mut order,
            Some(surface),
            None,
        )
        .unwrap();
        assert_eq!(native.core().seats[&1].keyboard_focus, Some(expected));
    }

    #[test]
    fn focus_owner_resolves_x11_children_but_rejects_hidden_missing_and_cycles() {
        let root = WaylandSurfaceId::from_raw(10).unwrap();
        let child = WaylandSurfaceId::from_raw(11).unwrap();
        let mut image = test_window(
            SizeI {
                width: 100,
                height: 100,
            },
            PointI::default(),
        );
        image.role = SurfaceRole::Xwayland;
        let mut sub = test_window(
            SizeI {
                width: 10,
                height: 10,
            },
            PointI::default(),
        );
        sub.role = SurfaceRole::Subsurface;
        sub.parent = Some(root);
        let mut windows = BTreeMap::from([(root, image), (child, sub)]);
        assert_eq!(x11_focus_owner(&windows, Some(root)), Some(root));
        assert_eq!(x11_focus_owner(&windows, Some(child)), Some(root));
        windows.get_mut(&root).unwrap().minimized = true;
        assert_eq!(x11_focus_owner(&windows, Some(child)), None);
        windows.get_mut(&root).unwrap().minimized = false;
        windows.get_mut(&root).unwrap().role = SurfaceRole::Subsurface;
        windows.get_mut(&root).unwrap().parent = Some(child);
        assert_eq!(x11_focus_owner(&windows, Some(child)), None);
        windows.remove(&root);
        assert_eq!(x11_focus_owner(&windows, Some(child)), None);
    }
}
