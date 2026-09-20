use super::*;
use crate::test_alloc;
#[derive(Clone, Debug)]
enum Action {
    Save,
}
#[test]
fn mounts_once_and_coalesces_property_writes() {
    let mut ui = MountedUi::default();
    let root;
    let button;
    {
        let mut builder = MountWriter::<Action>::new(&mut ui);
        let mut saved = None;
        root = builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
            saved = Some(
                builder.button(Action::Save, BoxStyle::default(), |builder| {
                    builder.text("Save", ColorRgba8::rgba(255, 255, 255, 255), 14.0);
                }),
            );
        });
        button = saved.unwrap();
    }
    assert_eq!(ui.root(), Some(root));
    let (_, result) = ui.transaction(|tx| {
        tx.set(button.enabled, false);
        tx.set(button.enabled, true);
        tx.set(button.opacity, 0.5);
    });
    assert_eq!(result.property_patches, 1);
    assert!(ui.interactions.get(button.node).unwrap().enabled);
    assert_eq!(ui.box_styles.get(button.node).unwrap().opacity, 0.5);
}
#[test]
fn state_only_changes_do_not_dirty_layout() {
    let mut ui = MountedUi::default();
    let button;
    {
        let mut builder = MountWriter::<Action>::new(&mut ui);
        let mut saved = None;
        builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
            saved = Some(
                builder.button(Action::Save, BoxStyle::default(), |builder| {
                    builder.text("Save", ColorRgba8::rgba(255, 255, 255, 255), 14.0);
                }),
            );
        });
        button = saved.unwrap();
    }
    ui.nodes.clear_dirty(button.node, DirtyFlags::ALL);
    ui.transaction(|tx| tx.set(button.background, ColorRgba8::rgba(1, 2, 3, 255)));
    let dirty = ui.nodes.core(button.node).unwrap().dirty;
    assert!(dirty.contains(DirtyFlags::PAINT));
    assert!(!dirty.intersects(DirtyFlags::LAYOUT));
}

#[test]
fn warmed_property_transaction_allocates_nothing() {
    let mut ui = MountedUi::default();
    let button;
    {
        let mut builder = MountWriter::<Action>::new(&mut ui);
        let mut saved = None;
        builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
            saved = Some(
                builder.button(Action::Save, BoxStyle::default(), |builder| {
                    builder.text("Save", ColorRgba8::rgba(255, 255, 255, 255), 14.0);
                }),
            );
        });
        button = saved.unwrap();
    }
    ui.transaction(|transaction| transaction.set(button.opacity, 0.75));
    test_alloc::begin();
    ui.transaction(|transaction| {
        transaction.set(button.opacity, 0.5);
        transaction.set(button.opacity, 0.25);
    });
    assert_eq!(test_alloc::finish(), 0);
}

#[test]
fn ten_thousand_simple_nodes_stay_within_low_single_digit_megabytes() {
    let mut ui = MountedUi::default();
    {
        let mut builder = MountWriter::<()>::new(&mut ui);
        builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
            for _ in 0..10_000 {
                builder.container(BoxStyle::default(), LayoutStyle::default(), |_| {});
            }
        });
    }
    let report = ui.memory_report();
    assert_eq!(report.mounted_nodes, 10_001);
    assert!(
        report.total_bytes() < 5 * 1024 * 1024,
        "{report:?} ({} bytes)",
        report.total_bytes()
    );
}

#[test]
fn every_mounted_visual_node_has_a_generation_safe_style_binding() {
    let mut ui = MountedUi::default();
    {
        let mut writer = MountWriter::<Action>::new(&mut ui);
        writer.root(BoxStyle::default(), LayoutStyle::default(), |writer| {
            writer.container(BoxStyle::default(), LayoutStyle::default(), |writer| {
                writer.text("label", ColorRgba8::rgba(1, 2, 3, 255), 12.0);
                writer.image(ImageId(7), BoxStyle::default());
            });
            writer.button(Action::Save, BoxStyle::default(), |_| {});
            writer.scroll(BoxStyle::default(), LayoutStyle::default(), |_| {});
        });
    }
    for node in ui.nodes.alive() {
        assert!(
            ui.style_bindings().iter().any(|binding| {
                ui.nodes.contains(binding.state_root)
                    && binding.slots.iter().any(|slot| slot.node == *node)
            }),
            "visual node {node:?} has no style binding"
        );
    }
}

#[test]
fn keyed_reconcile_preserves_identity_order_and_updates_values() {
    let mut ui = MountedUi::default();
    let root = {
        let mut builder = MountWriter::<()>::new(&mut ui);
        builder.root(BoxStyle::default(), LayoutStyle::default(), |_| {})
    };
    ui.transaction(|transaction| {
        transaction.reconcile_keyed(
            root.0,
            vec![
                NodeFixture::container(Some(1), BoxStyle::default(), Vec::new()),
                NodeFixture::container(Some(2), BoxStyle::default(), Vec::new()),
            ],
        );
    });
    let original: Vec<_> = ui.nodes.children(root.0).collect();
    let changed = BoxStyle {
        opacity: 0.5,
        ..BoxStyle::default()
    };
    ui.transaction(|transaction| {
        transaction.reconcile_keyed(
            root.0,
            vec![
                NodeFixture::container(Some(2), changed, Vec::new()),
                NodeFixture::container(Some(1), BoxStyle::default(), Vec::new()),
            ],
        );
    });
    let reordered: Vec<_> = ui.nodes.children(root.0).collect();
    assert_eq!(reordered, vec![original[1], original[0]]);
    assert_eq!(ui.box_styles.get(original[1]), Some(&changed));
}

#[test]
fn unkeyed_reconcile_replaces_old_nodes_instead_of_accumulating_them() {
    let mut ui = MountedUi::default();
    let root = {
        let mut builder = MountWriter::<()>::new(&mut ui);
        builder.root(BoxStyle::default(), LayoutStyle::default(), |_| {})
    };
    let child = || NodeFixture::container(None, BoxStyle::default(), Vec::new());
    ui.transaction(|transaction| transaction.reconcile_keyed(root.0, vec![child()]));
    let old = ui.nodes.children(root.0).next().unwrap();
    ui.transaction(|transaction| transaction.reconcile_keyed(root.0, vec![child()]));
    let children: Vec<_> = ui.nodes.children(root.0).collect();
    assert_eq!(children.len(), 1);
    assert_ne!(children[0], old);
    assert!(!ui.nodes.contains(old));
}
