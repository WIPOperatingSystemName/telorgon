use super::*;

#[test]
fn client_geometry_changes_resize_free_frames_but_preserve_host_targets() {
    let old = SizeI {
        width: 852,
        height: 652,
    };
    let content = SizeI {
        width: 800,
        height: 600,
    };
    let mut window = test_window(old, PointI::default());
    assert_eq!(publication_requested_size(Some(&window), content), content);
    window.requested_size = SizeI {
        width: 900,
        height: 700,
    };
    assert_eq!(
        publication_requested_size(Some(&window), content),
        window.requested_size
    );
    window.requested_size = old;
    window.maximized = true;
    assert_eq!(publication_requested_size(Some(&window), content), old);
    window.maximized = false;
    window.native_configure.resize_final = Some(FinalResizeConfigure::pending(old));
    assert_eq!(publication_requested_size(Some(&window), content), old);
    window.native_configure.resize_final = None;
    window.role = SurfaceRole::Subsurface;
    window.requested_size = SizeI {
        width: 1,
        height: 1,
    };
    assert_eq!(publication_requested_size(Some(&window), content), content);
}

#[test]
fn activation_raises_a_window_family_without_reordering_other_windows() {
    let ids: Vec<_> = (1..=4)
        .map(|id| WaylandSurfaceId::from_raw(id).unwrap())
        .collect();
    let mut windows = BTreeMap::new();
    for id in &ids {
        windows.insert(
            *id,
            test_window(
                SizeI {
                    width: 100,
                    height: 80,
                },
                PointI::default(),
            ),
        );
    }
    let popup = windows.get_mut(&ids[1]).unwrap();
    popup.role = SurfaceRole::XdgPopup;
    popup.parent = Some(ids[0]);
    let mut order = ids.clone();
    super::super::input::raise_toplevel(&windows, &mut order, ids[1]);
    assert_eq!(order, [ids[2], ids[3], ids[0], ids[1]]);
    super::super::input::raise_toplevel(&windows, &mut order, ids[0]);
    assert_eq!(
        order,
        [ids[2], ids[3], ids[0], ids[1]],
        "repeated activation is stable"
    );
    super::super::input::raise_toplevel(&windows, &mut order, ids[2]);
    assert_eq!(order, [ids[3], ids[0], ids[1], ids[2]]);
    windows.get_mut(&ids[3]).unwrap().minimized = true;
    super::super::input::raise_toplevel(&windows, &mut order, ids[3]);
    assert_eq!(order, [ids[3], ids[0], ids[1], ids[2]]);
    order.remove(0);
    windows.get_mut(&ids[3]).unwrap().minimized = false;
    super::super::input::raise_toplevel(&windows, &mut order, ids[3]);
    assert_eq!(order, [ids[0], ids[1], ids[2], ids[3]]);
}

#[test]
fn shm_damage_requires_contiguous_retained_content() {
    let mut window = test_window(
        SizeI {
            width: 10,
            height: 10,
        },
        PointI::default(),
    );
    window.presentation.revision = 7;
    assert!(window.presentation.can_apply_damage(8));
    assert!(!window.presentation.can_apply_damage(9)); // An intervening damaged image was coalesced.
    assert!(!window.presentation.can_apply_damage(7)); // Stale/duplicate publication.
    window.presentation.revision = u64::MAX;
    assert!(!window.presentation.can_apply_damage(0));
}

pub(in crate::host::linux_shell) fn test_window(
    size: SizeI,
    position: PointI,
) -> ClientWindow {
    ClientWindow {
        tile: None,
        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
        tile_size_hints: None,
        motion_style: crate::WindowMotion::none(),
        motion_veil_pending: false,
        tile_resize_hold: false,
        motion_input: None,
        size_policy: Default::default(),
        last_policy_request: None,
        surface_scale: 1,
        desktop_id: None,
        backend: Some(WindowBackend::Wayland),
        frame_title: None,
        application_identity: String::new(),
        application_icon: None,
        role: SurfaceRole::XdgToplevel,
        parent: None,
        offset: PointI::default(),
        server_decorated: true,
        frame_client_decorations: false,
        position,
        window_geometry: RectI {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        },
        requested_size: size,
        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
        resize_preview: Default::default(),
        native_configure: NativeConfigureState::default(),
        restore_geometry: None,
        maximized: false,
        fullscreen: false,
        minimized: false,
        virtual_output: None,
        chrome_outer: None,
        chrome_content_offset: None,
        chrome: None,
        presentation: SurfacePresentation {
            content_ready: true,
            revision: 1,
            size,
            image_size: size,
            alpha_mode: ImageAlphaMode::Opaque,
            pixel_format: ImagePixelFormat::Rgba8,
            pending_image_update: PendingClientImageUpdate::Unchanged,
            pixels: Vec::new(),
        },
    }
}

#[test]
fn maximize_veils_content_without_an_interactive_grab_and_restore_cancels_it() {
    let surface = WaylandSurfaceId::from_raw(42).unwrap();
    let size = SizeI {
        width: 640,
        height: 480,
    };
    let position = PointI { x: 50, y: 60 };
    let mut windows = BTreeMap::from([(surface, test_window(size, position))]);
    let mut scheduler = ConfigureScheduler::default();
    let area = RectI {
        x: 0,
        y: 0,
        width: 1280,
        height: 800,
    };
    assert_eq!(resize_veil_owner(&windows, surface), None);
    set_window_maximized(
        &mut windows,
        &mut scheduler,
        surface,
        true,
        area,
        &LinuxShellConfig::default(),
    )
    .unwrap();
    assert_eq!(resize_veil_owner(&windows, surface), Some(surface));
    let window = windows.get(&surface).unwrap();
    assert!(
        !window.resizing(),
        "maximize must not advertise an interactive pointer resize"
    );
    assert_eq!(
        window.native_configure.resize_final.unwrap().size,
        window.requested_size
    );
    let pending = scheduler.drain().next().unwrap();
    assert!(!pending.resizing);
    assert_eq!(pending.size, window.requested_size);
    set_window_maximized(
        &mut windows,
        &mut scheduler,
        surface,
        false,
        area,
        &LinuxShellConfig::default(),
    )
    .unwrap();
    assert_eq!(resize_veil_owner(&windows, surface), None);
    assert_eq!(windows[&surface].requested_size, size);
    assert_eq!(windows[&surface].position, position);
}

#[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
#[test]
fn both_backends_hold_the_first_placeholder_even_if_redraw_finishes_early() {
    let surface = WaylandSurfaceId::from_raw(42).unwrap();
    for backend in [
        WindowBackend::Wayland,
        WindowBackend::X11(crate::integrations::x11::association::XWindow {
            generation: 1,
            xid: 10,
            incarnation: 1,
        }),
    ] {
        let mut window = test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI::default(),
        );
        window.backend = Some(backend);
        window.motion_style = crate::WindowMotion::smooth();
        let mut windows = BTreeMap::from([(surface, window)]);
        for maximized in [true, false] {
            set_window_maximized(
                &mut windows,
                &mut ConfigureScheduler::default(),
                surface,
                maximized,
                RectI {
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 800,
                },
                &LinuxShellConfig::default(),
            )
            .unwrap();
            let window = windows.get_mut(&surface).unwrap();
            // Model protocol completion occurring before desktop composition gets a turn.
            window.native_configure.resize_final = None;
            window.resize_preview = Default::default();
            assert!(
                window.resize_veil_active(),
                "fast redraw must not skip the placeholder"
            );
            assert!(std::mem::take(&mut window.motion_veil_pending));
            assert!(
                !window.resize_veil_active(),
                "the shared controller now owns the visual handoff"
            );
        }
    }
}
#[test]
fn animated_restore_waits_for_its_own_final_content() {
    let surface = WaylandSurfaceId::from_raw(42).unwrap();
    let original = SizeI {
        width: 640,
        height: 480,
    };
    let mut window = test_window(original, PointI { x: 50, y: 60 });
    window.motion_style = crate::WindowMotion::smooth();
    let mut windows = BTreeMap::from([(surface, window)]);
    let mut scheduler = ConfigureScheduler::default();
    let area = RectI {
        x: 0,
        y: 0,
        width: 1280,
        height: 800,
    };
    for maximized in [true, false] {
        set_window_maximized(
            &mut windows,
            &mut scheduler,
            surface,
            maximized,
            area,
            &LinuxShellConfig::default(),
        )
        .unwrap();
    }
    let window = &windows[&surface];
    assert_eq!(window.requested_size, original);
    let pending = window.native_configure.resize_final.unwrap();
    assert_eq!(pending.size, original);
    assert!(!pending.was_acknowledged());
    assert_eq!(resize_veil_owner(&windows, surface), Some(surface));
    assert!(!window.resizing());
}

#[test]
fn titlebar_drag_restores_saved_size_and_continues_without_a_jump() {
    let surface = WaylandSurfaceId::from_raw(42).unwrap();
    let size = SizeI {
        width: 640,
        height: 480,
    };
    let position = PointI { x: 50, y: 60 };
    let output = SizeI {
        width: 1280,
        height: 800,
    };
    let area = RectI {
        x: 0,
        y: 0,
        width: output.width,
        height: output.height,
    };
    let config = LinuxShellConfig::default();
    for fraction in [0.1, 0.5, 0.9] {
        let mut windows = BTreeMap::from([(surface, test_window(size, position))]);
        let mut scheduler = ConfigureScheduler::default();
        set_window_maximized(&mut windows, &mut scheduler, surface, true, area, &config)
            .unwrap();
        windows.get_mut(&surface).unwrap().chrome_outer = Some(output);
        let _ = scheduler.drain().collect::<Vec<_>>();
        let start = PointF {
            x: fraction * output.width as f32,
            y: 12.0,
        };
        let mut grab = WindowInteraction::begin_move(&windows, surface, start).unwrap();
        apply_window_interaction(
            &mut windows,
            &mut grab,
            &mut scheduler,
            PointF {
                x: start.x + 1.0,
                y: 13.0,
            },
            output,
            &config,
        )
        .unwrap();
        assert!(windows[&surface].maximized, "click/jitter must not restore");
        let moved = PointF {
            x: start.x + 20.0,
            y: 52.0,
        };
        apply_window_interaction(
            &mut windows,
            &mut grab,
            &mut scheduler,
            moved,
            output,
            &config,
        )
        .unwrap();
        let window = &windows[&surface];
        assert!(!window.maximized);
        assert_eq!(window.requested_size, size);
        assert!(window.restore_geometry.is_none());
        assert!(window.native_configure.resize_final.is_none());
        assert_eq!(window.position.y, 40);
        assert_eq!(
            window.position.x,
            (moved.x - fraction * size.width as f32).round() as i32
        );
        let restored_position = window.position;
        let configure = scheduler.drain().next().unwrap();
        assert_eq!(configure.size, size);
        assert!(!configure.resizing);
        apply_window_interaction(
            &mut windows,
            &mut grab,
            &mut scheduler,
            PointF {
                x: moved.x + 10.0,
                y: moved.y + 15.0,
            },
            output,
            &config,
        )
        .unwrap();
        assert_eq!(
            windows[&surface].position,
            PointI {
                x: restored_position.x + 10,
                y: restored_position.y + 15
            }
        );
        finish_window_interaction(&mut windows, &mut scheduler, grab);
        assert!(scheduler.drain().next().is_none());
    }
}

#[test]
fn true_fullscreen_and_minimized_windows_do_not_begin_titlebar_moves() {
    let surface = WaylandSurfaceId::from_raw(42).unwrap();
    let mut windows = BTreeMap::from([(
        surface,
        test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI::default(),
        ),
    )]);
    windows.get_mut(&surface).unwrap().fullscreen = true;
    assert!(WindowInteraction::begin_move(&windows, surface, PointF::default()).is_none());
    windows.get_mut(&surface).unwrap().fullscreen = false;
    windows.get_mut(&surface).unwrap().minimized = true;
    assert!(WindowInteraction::begin_move(&windows, surface, PointF::default()).is_none());
}
