use super::super::client::maximize_preview_tests::test_window;
use super::*;

#[test]
fn raising_existing_focus_moves_its_family_once_and_preserves_child_order() {
    let ids = [1, 2, 3, 4].map(|raw| WaylandSurfaceId::from_raw(raw).unwrap());
    let mut windows = BTreeMap::new();
    for id in ids {
        windows.insert(
            id,
            test_window(
                SizeI {
                    width: 10,
                    height: 10,
                },
                PointI::default(),
            ),
        );
    }
    windows.get_mut(&ids[0]).unwrap().role = SurfaceRole::Xwayland;
    windows.get_mut(&ids[1]).unwrap().parent = Some(ids[0]);
    windows.get_mut(&ids[2]).unwrap().parent = Some(ids[1]);
    let mut order = ids.to_vec();
    assert!(raise_family(ids[0], &windows, &mut order));
    assert_eq!(order, [ids[3], ids[0], ids[1], ids[2]]);
    assert!(!raise_family(ids[0], &windows, &mut order));
}

#[test]
fn restoring_minimized_owner_reinserts_it_below_retained_children() {
    let ids = [10, 11, 12].map(|raw| WaylandSurfaceId::from_raw(raw).unwrap());
    let mut windows = BTreeMap::new();
    for id in ids {
        windows.insert(
            id,
            test_window(
                SizeI {
                    width: 100,
                    height: 100,
                },
                PointI::default(),
            ),
        );
    }
    windows.get_mut(&ids[0]).unwrap().role = SurfaceRole::Xwayland;
    windows.get_mut(&ids[1]).unwrap().parent = Some(ids[0]);
    for has_child in [false, true] {
        let mut order = if has_child {
            vec![ids[1], ids[2]]
        } else {
            vec![ids[2]]
        };
        windows.get_mut(&ids[0]).unwrap().minimized = true;
        // Taskbar activation clears minimized before the checked X11 focus completes.
        windows.get_mut(&ids[0]).unwrap().minimized = false;
        assert!(raise_family(ids[0], &windows, &mut order));
        let expected = if has_child {
            vec![ids[2], ids[0], ids[1]]
        } else {
            vec![ids[2], ids[0]]
        };
        assert_eq!(order, expected);
        assert!(!raise_family(ids[0], &windows, &mut order));
        assert_eq!(order, expected);
    }
}
