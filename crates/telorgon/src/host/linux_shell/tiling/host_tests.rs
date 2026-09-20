use super::super::client::maximize_preview_tests::test_window;
use super::*;
fn id(n: u32) -> WaylandSurfaceId {
    WaylandSurfaceId::from_raw(n).unwrap()
}
fn setup() -> (
    TilingController,
    BTreeMap<WaylandSurfaceId, ClientWindow>,
    ConfigureScheduler,
    LinuxShellConfig,
) {
    let mut windows = BTreeMap::new();
    for n in 1..=4 {
        let mut w = test_window(
            SizeI {
                width: 600,
                height: 400,
            },
            PointI { x: 80, y: 90 },
        );
        w.server_decorated = false;
        windows.insert(id(n), w);
    }
    (
        TilingController {
            policy: Some(WindowTiling::snap()),
            area: RectI {
                x: 0,
                y: 40,
                width: 1200,
                height: 760,
            },
            ..Default::default()
        },
        windows,
        ConfigureScheduler::default(),
        LinuxShellConfig {
            preferred_window_minimum: SizeI {
                width: 100,
                height: 100,
            },
            ..Default::default()
        },
    )
}
#[test]
fn quadrant_dividers_veil_every_affected_member() {
    for divider in [Divider::Vertical, Divider::Left, Divider::Right] {
        let (mut t, mut w, mut q, c) = setup();
        for (n, target) in [
            (1, TileTarget::TopLeft),
            (2, TileTarget::BottomLeft),
            (3, TileTarget::TopRight),
            (4, TileTarget::BottomRight),
        ] {
            assert!(t.snap(&mut w, &mut q, id(n), target, &c));
            w.get_mut(&id(n)).unwrap().native_configure.resize_final = None;
        }
        t.begin(divider, PointF { x: 600., y: 420. }, &mut w, &mut q);
        for window in w.values() {
            assert_eq!(
                window.resize_veil_active(),
                affects(divider, window.tile.unwrap().target)
            );
        }
    }
}

#[test]
fn shared_resize_reveals_together_after_the_slowest_client() {
    let (mut t, mut w, mut q, c) = setup();
    for (n, target) in [(1, TileTarget::Left), (2, TileTarget::Right)] {
        assert!(t.snap(&mut w, &mut q, id(n), target, &c));
    }
    t.begin(
        Divider::Vertical,
        PointF { x: 600., y: 400. },
        &mut w,
        &mut q,
    );
    t.finish(&mut w, &mut q);
    let ready = w.get_mut(&id(1)).unwrap();
    ready.native_configure.resize_final = None;
    ready.native_configure.resize_anchor = None;
    t.release_ready_group(&mut w);
    assert!(w[&id(1)].resize_veil_active());
    assert!(w[&id(2)].resize_veil_active());
    assert!(!w[&id(3)].tile_resize_hold);
    let ready = w.get_mut(&id(2)).unwrap();
    ready.native_configure.resize_final = None;
    ready.native_configure.resize_anchor = None;
    t.release_ready_group(&mut w);
    assert!(!w[&id(1)].resize_veil_active());
    assert!(!w[&id(2)].resize_veil_active());
}

#[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
#[test]
fn x11_publication_holds_the_shared_reveal_until_ready() {
    let (mut t, mut w, mut q, c) = setup();
    for (n, target) in [(1, TileTarget::Left), (2, TileTarget::Right)] {
        w.get_mut(&id(n)).unwrap().backend =
            Some(WindowBackend::X11(crate::integrations::x11::association::XWindow {
                generation: 1,
                xid: n,
                incarnation: 1,
            }));
        assert!(t.snap(&mut w, &mut q, id(n), target, &c));
    }
    t.begin(
        Divider::Vertical,
        PointF { x: 600., y: 400. },
        &mut w,
        &mut q,
    );
    t.finish(&mut w, &mut q);
    w.get_mut(&id(1)).unwrap().resize_preview = Default::default();
    t.release_ready_group(&mut w);
    assert!(w[&id(1)].tile_resize_hold);
    w.get_mut(&id(2)).unwrap().resize_preview = Default::default();
    t.release_ready_group(&mut w);
    assert!(!w[&id(1)].resize_veil_active());
    assert!(!w[&id(2)].resize_veil_active());
}

#[test]
fn removed_resize_member_does_not_block_the_group() {
    let (mut t, mut w, mut q, c) = setup();
    for (n, target) in [(1, TileTarget::Left), (2, TileTarget::Right)] {
        assert!(t.snap(&mut w, &mut q, id(n), target, &c));
    }
    t.begin(
        Divider::Vertical,
        PointF { x: 600., y: 400. },
        &mut w,
        &mut q,
    );
    t.finish(&mut w, &mut q);
    w.remove(&id(2));
    let ready = w.get_mut(&id(1)).unwrap();
    ready.native_configure.resize_final = None;
    ready.native_configure.resize_anchor = None;
    t.release_ready_group(&mut w);
    assert!(!w[&id(1)].resize_veil_active());
}

#[test]
fn snap_float_and_displacement_request_motion_handoff_but_dividers_do_not() {
    let (mut t, mut windows, mut scheduler, config) = setup();
    for window in windows.values_mut() {
        window.motion_style = crate::WindowMotion::smooth();
    }
    assert!(t.snap(
        &mut windows,
        &mut scheduler,
        id(1),
        TileTarget::Left,
        &config
    ));
    let window = windows.get_mut(&id(1)).unwrap();
    // Even an immediately-ready client must retain one placeholder frame for entry.
    window.native_configure.resize_final = None;
    assert!(window.resize_veil_active());
    assert!(std::mem::take(&mut window.motion_veil_pending));
    assert!(t.snap(
        &mut windows,
        &mut scheduler,
        id(2),
        TileTarget::Left,
        &config
    ));
    assert!(windows[&id(1)].tile.is_none());
    assert!(
        windows[&id(1)].motion_veil_pending,
        "displaced occupant restores with motion"
    );
    assert!(windows[&id(2)].motion_veil_pending);
    windows.get_mut(&id(2)).unwrap().motion_veil_pending = false;
    t.begin(
        Divider::Vertical,
        PointF { x: 600.0, y: 300.0 },
        &mut windows,
        &mut scheduler,
    );
    t.update(
        PointF { x: 620.0, y: 300.0 },
        &mut windows,
        &mut scheduler,
        &config,
    );
    assert!(
        !windows[&id(2)].motion_veil_pending,
        "divider updates stay direct"
    );
    t.finish(&mut windows, &mut scheduler);
    assert!(!windows[&id(2)].motion_veil_pending);
    float_window(windows.get_mut(&id(2)).unwrap(), id(2), &mut scheduler);
    assert!(windows[&id(2)].motion_veil_pending);
    windows.get_mut(&id(3)).unwrap().motion_style = crate::WindowMotion::none();
    assert!(t.snap(
        &mut windows,
        &mut scheduler,
        id(3),
        TileTarget::Right,
        &config
    ));
    assert!(!windows[&id(3)].motion_veil_pending);
}

#[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
#[test]
fn x11_tile_transitions_keep_the_existing_content_readiness_gate() {
    let (mut t, mut windows, mut scheduler, config) = setup();
    let window = windows.get_mut(&id(1)).unwrap();
    window.backend = Some(WindowBackend::X11(crate::integrations::x11::association::XWindow {
        generation: 1,
        xid: 10,
        incarnation: 1,
    }));
    window.motion_style = crate::WindowMotion::fluid();
    assert!(t.snap(
        &mut windows,
        &mut scheduler,
        id(1),
        TileTarget::Left,
        &config
    ));
    let window = windows.get_mut(&id(1)).unwrap();
    assert!(std::mem::take(&mut window.motion_veil_pending));
    assert!(window.resize_preview.active());
    assert!(!window.resize_preview.dragging());
    assert!(
        window.resize_veil_active(),
        "X11 must still wait for configured-size content"
    );
    float_window(window, id(1), &mut scheduler);
    assert!(window.motion_veil_pending);
    assert!(window.resize_preview.active());
}

#[test]
fn preview_padding_does_not_change_committed_window_geometry() {
    let (mut controller, mut windows, mut scheduler, config) = setup();
    controller.policy = Some(WindowTiling::snap().preview(crate::TilePreviewDesign {
        padding: crate::authoring::compose::Insets::all(20.0),
        ..Default::default()
    }));
    let expected = tile_rect(controller.area, controller.splits, TileTarget::TopLeft);
    assert!(controller.snap(
        &mut windows,
        &mut scheduler,
        id(1),
        TileTarget::TopLeft,
        &config
    ));
    assert_eq!(windows[&id(1)].tile.unwrap().rect, expected);
    assert_eq!(
        windows[&id(1)].position,
        PointI {
            x: expected.x,
            y: expected.y
        }
    );
    assert_eq!(
        windows[&id(1)].requested_size,
        SizeI {
            width: expected.width,
            height: expected.height
        }
    );
}

#[test]
fn snap_saves_floating_geometry_and_displaces_conflicting_slots() {
    let (mut t, mut w, mut q, c) = setup();
    assert!(t.snap(&mut w, &mut q, id(1), TileTarget::TopLeft, &c));
    assert_eq!(w[&id(1)].position, PointI { x: 0, y: 40 });
    assert_eq!(
        w[&id(1)].requested_size,
        SizeI {
            width: 600,
            height: 380
        }
    );
    assert!(w[&id(1)].native_configure.resize_final.is_some());
    let states = window_toplevel_states(&w[&id(1)], false, false);
    assert!(states.tiled_left && states.tiled_right && states.tiled_top && states.tiled_bottom);
    assert!(!states.maximized && !states.resizing);
    assert!(t.snap(&mut w, &mut q, id(2), TileTarget::BottomLeft, &c));
    assert!(w[&id(1)].tile.is_some());
    assert!(t.snap(&mut w, &mut q, id(3), TileTarget::Left, &c));
    for n in 1..=2 {
        assert!(w[&id(n)].tile.is_none());
        assert_eq!(w[&id(n)].position, PointI { x: 80, y: 90 });
        assert_eq!(
            w[&id(n)].requested_size,
            SizeI {
                width: 600,
                height: 400
            }
        );
    }
}
#[test]
fn empty_layout_forgets_dividers_before_preview_and_direct_snap() {
    for through_preview in [false, true] {
        let (mut t, mut w, mut q, c) = setup();
        assert!(t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c));
        assert!(t.snap(&mut w, &mut q, id(2), TileTarget::Right, &c));
        t.begin(
            Divider::Vertical,
            PointF { x: 600.0, y: 400.0 },
            &mut w,
            &mut q,
        );
        t.update(PointF { x: 800.0, y: 400.0 }, &mut w, &mut q, &c);
        t.finish(&mut w, &mut q);
        assert_ne!(t.splits[0], 0.5);
        float_window(w.get_mut(&id(1)).unwrap(), id(1), &mut q);
        let existing = w[&id(2)].tile;
        // An active layout keeps its divider, so newly filled slots stay adjacent.
        assert!(t.snap(&mut w, &mut q, id(3), TileTarget::Left, &c));
        assert_eq!(w[&id(2)].tile, existing);
        assert_eq!(
            w[&id(3)].tile.unwrap().rect.right(),
            existing.unwrap().rect.x
        );
        for n in [2, 3] {
            float_window(w.get_mut(&id(n)).unwrap(), id(n), &mut q);
        }
        t.splits[1] = 0.3;
        t.splits[2] = 0.7;
        if through_preview {
            t.preview(
                &mut [],
                &w,
                None,
                PointF::default(),
                SizeI {
                    width: 1200,
                    height: 800,
                },
                &c,
                false,
            );
            assert_eq!(t.splits, [0.5; 3]);
        }
        assert!(t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c));
        assert_eq!(t.splits, [0.5; 3]);
        assert_eq!(w[&id(1)].tile.unwrap().rect.width, 600);
        assert!(t.snap(&mut w, &mut q, id(2), TileTarget::TopLeft, &c));
        assert_eq!(w[&id(2)].tile.unwrap().rect.height, 380);
        assert_eq!(w[&id(1)].tile.unwrap().rect.height, 380);
    }
}

#[test]
fn quadrant_snap_splits_existing_half_and_preserves_restore_and_motion() {
    for (half, incoming, remaining, opposite) in [
        (
            TileTarget::Left,
            TileTarget::TopLeft,
            TileTarget::BottomLeft,
            TileTarget::Right,
        ),
        (
            TileTarget::Left,
            TileTarget::BottomLeft,
            TileTarget::TopLeft,
            TileTarget::Right,
        ),
        (
            TileTarget::Right,
            TileTarget::TopRight,
            TileTarget::BottomRight,
            TileTarget::Left,
        ),
        (
            TileTarget::Right,
            TileTarget::BottomRight,
            TileTarget::TopRight,
            TileTarget::Left,
        ),
    ] {
        let (mut t, mut w, mut q, c) = setup();
        for n in [1, 2] {
            w.get_mut(&id(n)).unwrap().motion_style = crate::WindowMotion::smooth();
        }
        assert!(t.snap(&mut w, &mut q, id(1), half, &c));
        assert!(t.snap(&mut w, &mut q, id(3), opposite, &c));
        let restore = w[&id(1)].restore_geometry;
        let untouched = w[&id(3)].tile;
        w.get_mut(&id(1)).unwrap().motion_veil_pending = false;
        assert!(t.snap(&mut w, &mut q, id(2), incoming, &c));
        for (n, target) in [(1, remaining), (2, incoming)] {
            let window = &w[&id(n)];
            assert_eq!(window.tile.unwrap().target, target);
            assert_eq!(
                window.tile.unwrap().rect,
                tile_rect(t.area, t.splits, target)
            );
            assert!(window.motion_veil_pending);
            assert!(window.native_configure.resize_final.is_some());
        }
        assert_eq!(w[&id(1)].restore_geometry, restore);
        assert_eq!(w[&id(3)].tile, untouched);
        float_window(w.get_mut(&id(1)).unwrap(), id(1), &mut q);
        assert_eq!(
            Some((w[&id(1)].position, w[&id(1)].requested_size)),
            restore
        );
    }
}

#[test]
fn quadrant_split_respects_existing_half_minimum_size() {
    let (mut t, mut w, mut q, c) = setup();
    assert!(t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c));
    w.get_mut(&id(1)).unwrap().size_policy.minimum = Some(SizeI {
        width: 100,
        height: 500,
    });
    assert!(t.snap(&mut w, &mut q, id(2), TileTarget::TopLeft, &c));
    assert!(w[&id(1)].tile.is_none());
    assert_eq!(w[&id(2)].tile.unwrap().target, TileTarget::TopLeft);
}

#[test]
fn shared_divider_clamps_all_members_and_horizontal_splits_are_independent() {
    let (mut t, mut w, mut q, c) = setup();
    for (n, target) in [
        (1, TileTarget::TopLeft),
        (2, TileTarget::BottomLeft),
        (3, TileTarget::TopRight),
        (4, TileTarget::BottomRight),
    ] {
        assert!(t.snap(&mut w, &mut q, id(n), target, &c));
    }
    w.get_mut(&id(3)).unwrap().size_policy.minimum = Some(SizeI {
        width: 450,
        height: 200,
    });
    t.begin(
        Divider::Vertical,
        PointF { x: 600., y: 400. },
        &mut w,
        &mut q,
    );
    t.update(PointF { x: 1150., y: 400. }, &mut w, &mut q, &c);
    assert_eq!(w[&id(1)].tile.unwrap().rect.right(), 750);
    assert_eq!(w[&id(3)].requested_size.width, 450);
    assert!(w[&id(1)].native_configure.resize_anchor.is_some());
    assert!(w[&id(1)].native_configure.resize_final.is_none());
    t.finish(&mut w, &mut q);
    assert!(w[&id(3)].native_configure.resize_final.is_some());
    let right = w[&id(3)].tile;
    t.begin(Divider::Left, PointF { x: 100., y: 420. }, &mut w, &mut q);
    t.update(PointF { x: 100., y: 520. }, &mut w, &mut q, &c);
    assert_eq!(w[&id(1)].tile.unwrap().rect.bottom(), 520);
    assert_eq!(w[&id(2)].position.y, 520);
    assert_eq!(w[&id(3)].tile, right);
}
#[test]
fn restored_tile_can_snap_on_the_same_pointer_event() {
    let (mut t, mut w, mut q, c) = setup();
    assert!(t.snap(&mut w, &mut q, id(1), TileTarget::TopLeft, &c));
    let mut grab =
        WindowInteraction::begin_move(&w, id(1), PointF { x: 300., y: 45. }).unwrap();
    let pointer = PointF { x: 1., y: 1. };
    let output = SizeI {
        width: 1200,
        height: 800,
    };
    apply_window_interaction(&mut w, &mut grab, &mut q, pointer, output, &c).unwrap();
    assert!(w[&id(1)].tile.is_none());
    t.preview(&mut [], &w, Some(grab), pointer, output, &c, false);
    t.commit(&mut w, &mut q, grab, &c);
    assert_eq!(w[&id(1)].tile.unwrap().target, TileTarget::TopLeft);
}

#[test]
fn impossible_snap_does_not_displace_existing_window() {
    let (mut t, mut w, mut q, c) = setup();
    assert!(t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c));
    w.get_mut(&id(2)).unwrap().size_policy.minimum = Some(SizeI {
        width: 800,
        height: 400,
    });
    assert!(!t.snap(&mut w, &mut q, id(2), TileTarget::Left, &c));
    assert!(w[&id(1)].tile.is_some());
    assert!(w[&id(2)].tile.is_none());
}
#[test]
fn tiled_title_drag_restores_and_rebases_grab() {
    let (mut t, mut w, mut q, c) = setup();
    assert!(t.snap(&mut w, &mut q, id(1), TileTarget::TopLeft, &c));
    let mut grab =
        WindowInteraction::begin_move(&w, id(1), PointF { x: 300., y: 45. }).unwrap();
    apply_window_interaction(
        &mut w,
        &mut grab,
        &mut q,
        PointF { x: 302., y: 45. },
        SizeI {
            width: 1200,
            height: 800,
        },
        &c,
    )
    .unwrap();
    assert!(w[&id(1)].tile.is_some());
    apply_window_interaction(
        &mut w,
        &mut grab,
        &mut q,
        PointF { x: 500., y: 100. },
        SizeI {
            width: 1200,
            height: 800,
        },
        &c,
    )
    .unwrap();
    assert!(w[&id(1)].tile.is_none());
    assert_eq!(
        w[&id(1)].requested_size,
        SizeI {
            width: 600,
            height: 400
        }
    );
    let position = w[&id(1)].position;
    apply_window_interaction(
        &mut w,
        &mut grab,
        &mut q,
        PointF { x: 510., y: 110. },
        SizeI {
            width: 1200,
            height: 800,
        },
        &c,
    )
    .unwrap();
    assert_eq!(
        w[&id(1)].position,
        PointI {
            x: position.x + 10,
            y: position.y + 10
        }
    );
}
#[test]
fn overlaying_floating_window_blocks_divider_and_disabling_widget_restores_members() {
    let (mut t, mut w, mut q, c) = setup();
    t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c);
    t.snap(&mut w, &mut q, id(2), TileTarget::Right, &c);
    let p = PointF { x: 600., y: 400. };
    assert_eq!(t.hover(&w, &[id(1), id(2)], p, &c), Some(Divider::Vertical));
    assert_eq!(t.hover(&w, &[id(1), id(2), id(3)], p, &c), None);
    t.sync(
        &mut [],
        &mut w,
        &mut q,
        RectI {
            x: 0,
            y: 40,
            width: 1200,
            height: 760,
        },
        &c,
        false,
    )
    .unwrap();
    assert!(w.values().all(|w| w.tile.is_none()));
}
#[test]
fn maximize_from_tile_preserves_original_floating_restore() {
    let (mut t, mut w, mut q, c) = setup();
    t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c);
    set_window_maximized(&mut w, &mut q, id(1), true, t.area, &c).unwrap();
    assert!(w[&id(1)].tile.is_none());
    set_window_maximized(&mut w, &mut q, id(1), false, t.area, &c).unwrap();
    assert_eq!(w[&id(1)].position, PointI { x: 80, y: 90 });
    assert_eq!(
        w[&id(1)].requested_size,
        SizeI {
            width: 600,
            height: 400
        }
    );
}
