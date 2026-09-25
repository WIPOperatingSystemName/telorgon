use super::super::client::maximize_preview_tests::test_window;
use super::*;
use crate::integrations::x11::window::Windows;

fn id() -> XWindow {
    let mut registry = Windows::new(1, 16, 1, 2, 100).unwrap();
    let token = registry.begin_inspection(10).unwrap().unwrap();
    registry
        .finish_inspection(
            token,
            1,
            Geometry {
                x: 100,
                y: 100,
                width: 640,
                height: 480,
                border: 0,
            },
            false,
            true,
        )
        .unwrap();
    registry.get(10).unwrap().id
}

#[test]
fn app_header_move_requires_a_live_press_owned_by_that_surface() {
    use crate::integrations::wayland::compositor::{
        ButtonState, ClientId, PointerFocus, SeatCapabilities, SeatState,
    };
    let surface = WaylandSurfaceId::from_raw(1).unwrap();
    let other = WaylandSurfaceId::from_raw(2).unwrap();
    let mut seat = SeatState::new(
        "test",
        SeatCapabilities {
            pointer: true,
            keyboard: false,
            touch: false,
        },
    );
    seat.pointer_focus = Some(PointerFocus {
        client: ClientId::from_raw(1).unwrap(),
        surface,
        position: PointF::default(),
        enter_serial: 1,
    });
    assert!(!authorize_move_resize(&seat, surface, 1));
    seat.pointer_button_target(0x110, ButtonState::Pressed, false);
    assert!(authorize_move_resize(&seat, surface, 1));
    assert!(!authorize_move_resize(&seat, other, 1));
    assert!(!authorize_move_resize(&seat, surface, 3));
    assert!(!authorize_move_resize(&seat, surface, 0));
    seat.pointer_button_target(0x110, ButtonState::Released, false);
    assert!(!authorize_move_resize(&seat, surface, 1));
}

#[test]
fn client_decorated_window_has_no_shell_resize_hits() {
    let config = LinuxShellConfig::default();
    let surface = WaylandSurfaceId::from_raw(1).unwrap();
    let mut window = test_window(
        SizeI {
            width: 640,
            height: 480,
        },
        PointI { x: 100, y: 100 },
    );
    window.role = SurfaceRole::Xwayland;
    window.backend = Some(WindowBackend::X11(id()));
    apply_decorations(&mut window, false, &config);
    let content = window_content_rect(&window, window.position, &config);
    let border = PointF {
        x: window.position.x as f32 + 1.0,
        y: window.position.y as f32 + 100.0,
    };
    let windows = BTreeMap::from([(surface, window)]);
    assert_eq!(
        hit_test_decoration(&super::super::input::test_input_world(&windows.keys().copied().collect::<Vec<_>>()),
            &windows,
            &[surface],
            PointF {
                x: content.x as f32 + 20.0,
                y: content.y as f32 + 2.0
            },
            &config,
            &[]
        ),
        None
    );
    assert_eq!(hit_test_decoration(&super::super::input::test_input_world(&windows.keys().copied().collect::<Vec<_>>()), &windows, &[surface], border, &config, &[]), None);
}

#[test]
fn decoration_premap_extents_follow_x11_hints() {
    let config = LinuxShellConfig::default();
    for decorated in [false, true] {
        let mut window = test_window(SizeI { width: 640, height: 480 }, PointI::default());
        window.role = SurfaceRole::Xwayland;
        window.backend = Some(WindowBackend::X11(id()));
        window.surface_scale = 3;
        window.server_decorated = decorated;
        assert_eq!(estimated_frame_extents(decorated, false, 3, &config),
            frame_extents(&window, &config));
        assert_eq!(estimated_frame_extents(decorated, true, 3, &config), [0; 4]);
    }
}

#[test]
fn decoration_ownership_changes_preserve_geometry_and_remove_all_csd_extents() {
    let config = LinuxShellConfig::default();
    let mut window = test_window(SizeI { width: 640, height: 480 }, PointI { x: 100, y: 100 });
    window.role = SurfaceRole::Xwayland;
    window.backend = Some(WindowBackend::X11(id()));
    let content = frame_geometry(&window, &config);
    let extents = frame_extents(&window, &config);
    apply_decorations(&mut window, false, &config);
    assert_eq!(frame_geometry(&window, &config), content);
    assert_eq!(frame_extents(&window, &config), [0; 4]);
    assert!(!window_has_frame(&window));
    apply_decorations(&mut window, true, &config);
    assert_eq!(frame_geometry(&window, &config), content);
    assert_eq!(frame_extents(&window, &config), extents);
}

#[test]
fn decoration_changes_preserve_client_geometry_management_and_scaled_extents() {
    let config = LinuxShellConfig::default();
    let surface = WaylandSurfaceId::from_raw(1).unwrap();
    for density in [1, 3] {
        let mut window = test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI { x: 100, y: 100 },
        );
        window.role = SurfaceRole::Xwayland;
        window.surface_scale = density;
        let mut adapter = X11Windows::default();
        let initial = frame_geometry(&window, &config);
        adapter.attach(id(), surface, initial, false, &mut window, &config);
        let content = frame_geometry(&window, &config);
        let extents = frame_extents(&window, &config);
        assert_eq!(
            extents[2],
            ((config.titlebar_height + config.window_border) * density) as u32
        );
        apply_decorations(&mut window, false, &config);
        assert_eq!(frame_geometry(&window, &config), content);
        assert_eq!(
            frame_extents(&window, &config),
            [0; 4]
        );
        assert_eq!(window.backend, Some(WindowBackend::X11(id())));
        assert!(!window.minimized);
        adapter.attach(id(), surface, content, false, &mut window, &config);
        assert!(!window.server_decorated); // Subsequent image sync must not restore chrome.
        apply_decorations(&mut window, true, &config);
        assert_eq!(frame_geometry(&window, &config), content);
        assert_eq!(frame_extents(&window, &config), extents);
        window.fullscreen = true;
        assert_eq!(frame_extents(&window, &config), [0; 4]);
    }
}
#[test]
fn decoration_toggle_keeps_maximized_outer_size_and_custom_extents() {
    let config = LinuxShellConfig::default();
    let mut window = test_window(
        SizeI {
            width: 640,
            height: 480,
        },
        PointI { x: 10, y: 20 },
    );
    window.role = SurfaceRole::Xwayland;
    window.backend = Some(WindowBackend::X11(id()));
    window.chrome_content_offset = Some(PointI { x: 7, y: 45 });
    window.chrome_outer = Some(SizeI {
        width: 659,
        height: 535,
    });
    assert_eq!(frame_extents(&window, &config), [7, 12, 45, 10]);
    window.maximized = true;
    let position = window.position;
    apply_decorations(&mut window, false, &config);
    assert_eq!(window.position, position);
    assert_eq!(
        legacy_window_outer(&window, &config),
        SizeI {
            width: 659,
            height: 535
        }
    );
    assert!(window.chrome_content_offset.is_none());
    assert!(window.chrome_outer.is_none());
    apply_decorations(&mut window, true, &config);
    assert_eq!(window.position, position);
    assert_eq!(
        legacy_window_outer(&window, &config),
        SizeI {
            width: 659,
            height: 535
        }
    );
}

#[test]
fn preferred_pixel_minimum_preserves_grid_aspect_and_fixed_exceptions() {
    use crate::integrations::x11::normal_hints::{AspectRatio, NormalHints};
    let wanted = SizeI {
        width: 300,
        height: 300,
    };
    let preferred = SizeI {
        width: 900,
        height: 600,
    }; // 300x200 logical at 3x
    let available = SizeI {
        width: 1800,
        height: 1200,
    };
    let ordinary = preferred_size(NormalHints::default(), wanted, preferred, available);
    assert_eq!(ordinary, preferred);
    let fixed = NormalHints {
        minimum: Some(wanted),
        maximum: Some(wanted),
        ..Default::default()
    };
    assert_eq!(
        preferred_size(fixed, preferred, preferred, available),
        wanted
    );
    let grid = NormalHints {
        base: Some(SizeI {
            width: 2,
            height: 4,
        }),
        increment: Some(SizeI {
            width: 8,
            height: 16,
        }),
        aspect: Some((
            AspectRatio {
                numerator: 1,
                denominator: 1,
            },
            AspectRatio {
                numerator: 2,
                denominator: 1,
            },
        )),
        ..Default::default()
    };
    let size = preferred_size(grid, wanted, preferred, available);
    assert!(grid.accepts_size(size));
    assert!(size.width >= preferred.width && size.height >= preferred.height);
    assert!(size.width <= available.width && size.height <= available.height);
    let invalid = NormalHints {
        minimum: Some(preferred),
        maximum: Some(wanted),
        ..Default::default()
    };
    assert_eq!(
        preferred_size(invalid, wanted, preferred, available),
        preferred
    );
}
#[test]
fn dense_x11_geometry_round_trips_without_scaling_chrome_or_input_twice() {
    let config = LinuxShellConfig::default();
    let mut window = test_window(
        SizeI {
            width: 900,
            height: 600,
        },
        PointI::default(),
    );
    window.role = SurfaceRole::Xwayland;
    window.surface_scale = 3;
    window.presentation.size = SizeI {
        width: 900,
        height: 600,
    };
    let geometry = Geometry {
        x: 300,
        y: 300,
        width: 900,
        height: 600,
        border: 0,
    };
    let surface = WaylandSurfaceId::from_raw(1).unwrap();
    let mut adapter = X11Windows::default();
    adapter.attach(id(), surface, geometry, false, &mut window, &config);
    assert_eq!(
        window.requested_size,
        SizeI {
            width: 300,
            height: 200
        }
    );
    assert_eq!(frame_geometry(&window, &config), geometry);
    let placement = surface_placement(&window, window.position, &config);
    assert_eq!(
        placement.target,
        RectI {
            x: 100,
            y: 100,
            width: 300,
            height: 200
        }
    );
    let desktop = PointF {
        x: 110.5,
        y: 120.25,
    };
    assert_eq!(
        placement.surface_local(desktop),
        PointF { x: 31.5, y: 60.75 }
    );
    assert_eq!(
        placement.output_position(placement.surface_local(desktop)),
        desktop
    );
    adapter.attach(
        id(),
        surface,
        Geometry {
            x: -90,
            y: 150,
            ..geometry
        },
        true,
        &mut window,
        &config,
    );
    assert_eq!(window.position, PointI { x: -30, y: 50 });
    assert!(!window.server_decorated);
}
#[test]
fn resize_preview_waits_for_checked_matching_new_content_and_can_be_superseded() {
    let old = SizeI {
        width: 640,
        height: 480,
    };
    let target = SizeI {
        width: 800,
        height: 600,
    };
    let mut preview = ResizePreview::default();
    preview.begin(PointI::default(), old, ResizeEdge::BottomRight);
    assert!(preview.active() && preview.dragging());
    assert!(!preview.settle(target, target, 9, false));
    preview.finish();
    preview.submitted(target, 7, false);
    assert!(!preview.dragging());
    assert!(!preview.settle(target, target, 8, true));
    assert!(!preview.settle(old, target, 8, false));
    assert!(!preview.settle(target, old, 8, false));
    assert!(!preview.settle(target, target, 7, false));
    assert!(preview.settle(target, target, 8, false));
    assert!(!preview.active());

    preview.begin(PointI::default(), target, ResizeEdge::TopLeft);
    preview.finish();
    preview.submitted(old, 9, false);
    // Another grab invalidates the old final target, even if it arrives late.
    preview.begin(PointI::default(), old, ResizeEdge::BottomRight);
    assert!(!preview.settle(old, old, 10, false));
    preview.finish();
    assert!(!preview.settle(old, old, 10, false));
    preview.submitted(target, 10, false);
    assert!(!preview.settle(old, old, 11, false));
    assert!(preview.settle(target, target, 11, false));
}

#[test]
fn no_op_resize_can_settle_without_a_new_buffer() {
    let size = SizeI {
        width: 640,
        height: 480,
    };
    let mut preview = ResizePreview::default();
    preview.begin(PointI::default(), size, ResizeEdge::BottomRight);
    preview.finish();
    preview.submitted(size, 7, true);
    assert!(!preview.settle(size, size, 7, true));
    assert!(preview.settle(size, size, 7, false));
}

#[test]
fn x11_final_resize_keeps_frame_paced_clients_running_behind_the_veil() {
    let mut window = test_window(
        SizeI {
            width: 640,
            height: 480,
        },
        PointI::default(),
    );
    window.backend = Some(WindowBackend::X11(id()));
    window.role = SurfaceRole::Xwayland;
    assert!(!window.waiting_for_resize_content());
    window.resize_preview.begin(
        window.position,
        window.requested_size,
        ResizeEdge::BottomRight,
    );
    assert!(!window.waiting_for_resize_content());
    window.resize_preview.finish();
    assert!(window.waiting_for_resize_content());
    assert!(window.native_configure.resize_final.is_none());
    window
        .resize_preview
        .submitted(window.requested_size, 7, false);
    assert!(!window.resize_preview.settle(
        window.requested_size,
        window.requested_size,
        7,
        false
    ));
    assert!(window.waiting_for_resize_content());
    assert!(window.resize_preview.settle(
        window.requested_size,
        window.requested_size,
        8,
        false
    ));
    assert!(!window.waiting_for_resize_content());
}

#[test]
fn diagnostic_hold_preserves_completion_checks_and_has_a_wakeup_deadline() {
    let now = Instant::now();
    let size = SizeI {
        width: 800,
        height: 600,
    };
    let deadline = now + Duration::from_secs(60);
    let mut preview = ResizePreview::Settling {
        anchor: None,
        target: Some(ResizeTarget {
            size,
            revision_after: Some(7),
        }),
        not_before: Some(deadline),
    };
    assert_eq!(preview.hold_deadline(now), Some(deadline));
    assert!(!preview.settle(size, size, 8, false));
    if let ResizePreview::Settling { not_before, .. } = &mut preview {
        *not_before = Some(now - Duration::from_secs(1));
    }
    assert_eq!(preview.hold_deadline(now), None);
    assert!(!preview.settle(size, size, 8, true));
    assert!(!preview.settle(size, size, 7, false));
    assert!(preview.settle(size, size, 8, false));
}

#[test]
fn shared_titlebar_drag_and_coordinates_work_for_both_backends() {
    let id = id();
    let surface = WaylandSurfaceId::from_raw(20).unwrap();
    let config = LinuxShellConfig::default();
    let mut adapter = X11Windows::default();
    for backend in [WindowBackend::Wayland, WindowBackend::X11(id)] {
        let mut image = test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI { x: 100, y: 80 },
        );
        image.backend = Some(backend);
        if matches!(backend, WindowBackend::X11(_)) {
            image.role = SurfaceRole::Xwayland;
            image.backend = None;
            adapter.attach(
                id,
                surface,
                Geometry {
                    x: 100,
                    y: 100,
                    width: 640,
                    height: 480,
                    border: 0,
                },
                false,
                &mut image,
                &config,
            );
        }
        let original = image.position;
        let pointer = PointF {
            x: original.x as f32 + 120.0,
            y: original.y as f32 + config.window_border as f32 + 8.0,
        };
        let mut windows = BTreeMap::from([(surface, image)]);
        assert!(matches!(
            hit_test_decoration(&super::super::input::test_input_world(&windows.keys().copied().collect::<Vec<_>>()), &windows, &[surface], pointer, &config, &[]),
            Some((_, DecorationHit::Titlebar))
        ));
        let mut drag = WindowInteraction::begin_move(&windows, surface, pointer).unwrap();
        let mut scheduler = ConfigureScheduler::default();
        apply_window_interaction(
            &mut windows,
            &mut drag,
            &mut scheduler,
            PointF {
                x: pointer.x + 55.0,
                y: pointer.y + 42.0,
            },
            SizeI {
                width: 1920,
                height: 1080,
            },
            &config,
        )
        .unwrap();
        assert_eq!(
            windows[&surface].position,
            PointI {
                x: original.x + 55,
                y: original.y + 42
            }
        );
        if matches!(backend, WindowBackend::X11(_)) {
            let window = windows.get_mut(&surface).unwrap();
            let moved = window.position;
            // Delayed server notifications must not undo compositor policy.
            adapter.attach(
                id,
                surface,
                Geometry {
                    x: 100,
                    y: 100,
                    width: 640,
                    height: 480,
                    border: 0,
                },
                false,
                window,
                &config,
            );
            assert_eq!(window.position, moved);
            let geometry = frame_geometry(window, &config);
            let offset = window_content_offset(window, &config);
            assert_eq!(i32::from(geometry.x), moved.x + offset.x);
            assert_eq!(i32::from(geometry.y), moved.y + offset.y);
            let local = surface_local_position(
                &windows,
                surface,
                PointF {
                    x: geometry.x as f32 + 10.0,
                    y: geometry.y as f32 + 12.0,
                },
                &config,
            );
            assert_eq!(local, PointF { x: 10.0, y: 12.0 });
        }
        finish_window_interaction(&mut windows, &mut scheduler, drag);
        assert!(scheduler.drain().next().is_none());
    }
}

#[test]
fn x11_resize_maximize_and_minimize_do_not_create_xdg_transactions() {
    let id = id();
    let surface = WaylandSurfaceId::from_raw(20).unwrap();
    let config = LinuxShellConfig::default();
    let mut adapter = X11Windows::default();
    let original = Geometry {
        x: 100,
        y: 100,
        width: 640,
        height: 480,
        border: 0,
    };
    let mut image = test_window(
        SizeI {
            width: 640,
            height: 480,
        },
        PointI::default(),
    );
    image.role = SurfaceRole::Xwayland;
    image.backend = None;
    adapter.attach(id, surface, original, false, &mut image, &config);
    let mut windows = BTreeMap::from([(surface, image)]);
    let mut scheduler = ConfigureScheduler::default();
    let mut drag = WindowInteraction::begin_resize(
        &mut windows,
        &mut scheduler,
        surface,
        ResizeEdge::BottomRight,
        PointF::default(),
    )
    .unwrap();
    apply_window_interaction(
        &mut windows,
        &mut drag,
        &mut scheduler,
        PointF { x: 80.0, y: 60.0 },
        SizeI {
            width: 1920,
            height: 1080,
        },
        &config,
    )
    .unwrap();
    finish_window_interaction(&mut windows, &mut scheduler, drag);
    assert_eq!(
        windows[&surface].requested_size,
        SizeI {
            width: 720,
            height: 540
        }
    );
    assert!(windows[&surface].native_configure.resize_anchor.is_none());
    assert!(windows[&surface].native_configure.resize_final.is_none());
    let saved = (windows[&surface].position, windows[&surface].requested_size);
    let work = RectI {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };
    set_window_maximized(&mut windows, &mut scheduler, surface, true, work, &config).unwrap();
    assert!(windows[&surface].native_configure.resize_final.is_none());
    let maximized_size = windows[&surface].requested_size;
    let preview = &mut windows.get_mut(&surface).unwrap().resize_preview;
    assert!(preview.active());
    preview.submitted(maximized_size, 10, false);
    assert!(preview.settle(maximized_size, maximized_size, 11, false));
    set_window_maximized(&mut windows, &mut scheduler, surface, false, work, &config).unwrap();
    let preview = &mut windows.get_mut(&surface).unwrap().resize_preview;
    assert!(preview.active());
    preview.submitted(saved.1, 11, false);
    assert!(!preview.settle(saved.1, maximized_size, 12, false));
    assert!(!preview.settle(saved.1, saved.1, 11, false));
    assert!(preview.settle(saved.1, saved.1, 12, false));
    assert_eq!(
        (windows[&surface].position, windows[&surface].requested_size),
        saved
    );
    // Restoring through a titlebar drag uses the same completion gate.
    set_window_maximized(&mut windows, &mut scheduler, surface, true, work, &config).unwrap();
    windows.get_mut(&surface).unwrap().resize_preview = ResizePreview::Idle;
    let mut drag =
        WindowInteraction::begin_move(&windows, surface, PointF { x: 100.0, y: 10.0 }).unwrap();
    apply_window_interaction(
        &mut windows,
        &mut drag,
        &mut scheduler,
        PointF { x: 140.0, y: 50.0 },
        SizeI {
            width: 1920,
            height: 1080,
        },
        &config,
    )
    .unwrap();
    assert!(!windows[&surface].maximized);
    assert_eq!(windows[&surface].requested_size, saved.1);
    assert!(windows[&surface].resize_preview.active());
    assert!(windows[&surface].native_configure.resize_final.is_none());
    windows.get_mut(&surface).unwrap().minimized = true;
    adapter.attach(
        id,
        surface,
        original,
        false,
        windows.get_mut(&surface).unwrap(),
        &config,
    );
    assert!(windows[&surface].minimized);
}

#[test]
fn unmanaged_popups_remain_unframed() {
    let mut adapter = X11Windows::default();
    let mut image = test_window(
        SizeI {
            width: 100,
            height: 100,
        },
        PointI::default(),
    );
    image.role = SurfaceRole::Xwayland;
    adapter.attach(
        id(),
        WaylandSurfaceId::from_raw(20).unwrap(),
        Geometry {
            x: 40,
            y: 50,
            width: 100,
            height: 100,
            border: 0,
        },
        true,
        &mut image,
        &LinuxShellConfig::default(),
    );
    assert_eq!(image.backend, None);
    assert!(!window_has_frame(&image));
    assert_eq!(image.position, PointI { x: 40, y: 50 });
    let native = WaylandSurfaceId::from_raw(21).unwrap();
    let popup = WaylandSurfaceId::from_raw(20).unwrap();
    let mut windows = BTreeMap::from([
        (popup, image),
        (
            native,
            test_window(
                SizeI {
                    width: 640,
                    height: 480,
                },
                PointI { x: 40, y: 50 },
            ),
        ),
    ]);
    let point = PointF { x: 100.0, y: 60.0 };
    assert!(
        hit_test_decoration(&super::super::input::test_input_world(&windows.keys().copied().collect::<Vec<_>>()),
            &windows,
            &[native, popup],
            point,
            &LinuxShellConfig::default(),
            &[]
        )
        .is_none()
    );
    windows.remove(&popup);
    assert!(matches!(
        hit_test_decoration(&super::super::input::test_input_world(&windows.keys().copied().collect::<Vec<_>>()),
            &windows,
            &[native],
            point,
            &LinuxShellConfig::default(),
            &[]
        ),
        Some((_, DecorationHit::Titlebar))
    ));
}

#[test]
fn size_constraints_project_to_client_grid_and_aspect() {
    use crate::integrations::x11::normal_hints::{AspectRatio, NormalHints};
    let hints = NormalHints {
        minimum: Some(SizeI {
            width: 80,
            height: 40,
        }),
        maximum: Some(SizeI {
            width: 800,
            height: 600,
        }),
        increment: Some(SizeI {
            width: 8,
            height: 16,
        }),
        ..Default::default()
    };
    let size = constrained_size(
        hints,
        SizeI {
            width: 333,
            height: 217,
        },
    )
    .unwrap();
    assert!(hints.accepts_size(size));
    assert!((size.width - 333).abs() <= 8 && (size.height - 217).abs() <= 16);
    let square = NormalHints {
        aspect: Some((
            AspectRatio {
                numerator: 1,
                denominator: 1,
            },
            AspectRatio {
                numerator: 1,
                denominator: 1,
            },
        )),
        ..Default::default()
    };
    let size = constrained_size(
        square,
        SizeI {
            width: 300,
            height: 150,
        },
    )
    .unwrap();
    assert_eq!(size.width, size.height);
    assert!(square.accepts_size(size));
}
