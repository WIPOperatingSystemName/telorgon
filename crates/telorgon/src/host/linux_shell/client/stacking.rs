use super::*;

pub(super) fn insert_new_surface(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    order: &mut Vec<WaylandSurfaceId>,
    surface: WaylandSurfaceId,
) {
    // DMA-BUF children can be ready before a parent's asynchronous SHM copy.
    // Import completion order must not place the parent over existing children.
    let index = order
        .iter()
        .position(|candidate| {
            let mut parent = windows.get(candidate).and_then(|window| window.parent);
            for _ in 0..windows.len() {
                let Some(id) = parent else { break };
                if id == surface {
                    return true;
                }
                parent = windows.get(&id).and_then(|window| window.parent);
            }
            false
        })
        .unwrap_or(order.len());
    order.insert(index, surface);
}

// A child can finish DMA-BUF import before its parent's SHM copy. It must not
// become an independent window while that parent is absent from the host scene.
pub(in crate::host::linux_shell) fn surface_tree_visible(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    surface: WaylandSurfaceId,
) -> bool {
    let mut candidate = surface;
    for _ in 0..windows.len() {
        let Some(window) = windows.get(&candidate) else { return false };
        if window.hidden_on_primary() {
            return false;
        }
        if !matches!(window.role, SurfaceRole::Subsurface | SurfaceRole::XdgPopup) {
            return true;
        }
        let Some(parent) = window.parent else { return false };
        candidate = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::super::maximize_preview_tests::test_window;
    use super::*;

    #[test]
    fn children_wait_for_the_entire_parent_chain_and_hide_with_it() {
        let root = WaylandSurfaceId::from_raw(1).unwrap();
        let child = WaylandSurfaceId::from_raw(2).unwrap();
        let nested = WaylandSurfaceId::from_raw(3).unwrap();
        let make = || test_window(SizeI { width: 100, height: 80 }, PointI::default());
        let mut windows = BTreeMap::new();
        let mut descendant = make();
        descendant.role = SurfaceRole::Subsurface;
        descendant.parent = Some(child);
        windows.insert(nested, descendant);
        assert!(!surface_tree_visible(&windows, nested));
        let mut parent = make();
        parent.role = SurfaceRole::Subsurface;
        parent.parent = Some(root);
        windows.insert(child, parent);
        assert!(!surface_tree_visible(&windows, child));
        assert!(!surface_tree_visible(&windows, nested));
        windows.insert(root, make());
        assert!(surface_tree_visible(&windows, root));
        assert!(surface_tree_visible(&windows, nested));
        windows.get_mut(&root).unwrap().minimized = true;
        assert!(!surface_tree_visible(&windows, nested));
        windows.get_mut(&root).unwrap().minimized = false;
        assert!(surface_tree_visible(&windows, nested));
        windows.remove(&root);
        assert!(!surface_tree_visible(&windows, nested));
        windows.get_mut(&child).unwrap().parent = Some(nested);
        assert!(!surface_tree_visible(&windows, nested)); // malformed cycle fails closed
    }

    #[test]
    fn late_parent_stays_below_prepared_descendants() {
        let ids: Vec<_> = (1..=4)
            .map(|id| WaylandSurfaceId::from_raw(id).unwrap())
            .collect();
        let mut windows = BTreeMap::new();
        for &id in &ids {
            windows.insert(
                id,
                test_window(
                    SizeI {
                        width: 100,
                        height: 80,
                    },
                    PointI::default(),
                ),
            );
        }
        for (child, parent) in [(ids[1], ids[0]), (ids[2], ids[1])] {
            let window = windows.get_mut(&child).unwrap();
            window.role = SurfaceRole::Subsurface;
            window.parent = Some(parent);
        }
        let mut order = vec![ids[3]];
        for &id in [ids[2], ids[0], ids[1]].iter() {
            insert_new_surface(&windows, &mut order, id);
        }
        assert_eq!(order, [ids[3], ids[0], ids[1], ids[2]]);
        crate::host::linux_shell::input::raise_toplevel(&windows, &mut order, ids[0]);
        assert_eq!(order, [ids[3], ids[0], ids[1], ids[2]]);
    }
}
