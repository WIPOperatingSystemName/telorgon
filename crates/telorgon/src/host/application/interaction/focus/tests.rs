use super::*;
use crate::ui::{BoxStyle, LayoutStyle, MountWriter};

struct Fixture {
    ui: MountedUi,
    root: NodeId,
    trigger: NodeId,
    first_scope: NodeId,
    first_editor: NodeId,
    second_scope: NodeId,
    second_editor: NodeId,
}

fn fixture(nested: bool) -> Fixture {
    let mut ui = MountedUi::default();
    let mut trigger = None;
    let mut first_scope = None;
    let mut first_editor = None;
    let mut second_scope = None;
    let mut second_editor = None;
    let root = MountWriter::<()>::new(&mut ui)
        .root(BoxStyle::default(), LayoutStyle::default(), |writer| {
            trigger = Some(writer.button_node(BoxStyle::default(), |_| {}).node);
            first_scope =
                Some(
                    writer.container(BoxStyle::default(), LayoutStyle::default(), |writer| {
                        first_editor = Some(writer.button_node(BoxStyle::default(), |_| {}).node);
                        if nested {
                            second_scope = Some(writer.container(
                                BoxStyle::default(),
                                LayoutStyle::default(),
                                |writer| {
                                    second_editor =
                                        Some(writer.button_node(BoxStyle::default(), |_| {}).node);
                                },
                            ));
                        }
                    }),
                );
            if !nested {
                second_scope = Some(writer.container(
                    BoxStyle::default(),
                    LayoutStyle::default(),
                    |writer| {
                        second_editor = Some(writer.button_node(BoxStyle::default(), |_| {}).node);
                    },
                ));
            }
        })
        .0;
    Fixture {
        ui,
        root,
        trigger: trigger.unwrap(),
        first_scope: first_scope.unwrap(),
        first_editor: first_editor.unwrap(),
        second_scope: second_scope.unwrap(),
        second_editor: second_editor.unwrap(),
    }
}

fn synchronize(router: &mut InteractionRouter, ui: &mut MountedUi) {
    router.sync(ui);
    router.sync_focus_requests(ui);
}

fn open_both(router: &mut InteractionRouter, fixture: &mut Fixture) {
    router.set_focus(&mut fixture.ui, Some(fixture.trigger), true);
    fixture.ui.set_focus_scope(fixture.first_scope, true);
    synchronize(router, &mut fixture.ui);
    assert_eq!(router.focused(), Some(fixture.first_editor));
    fixture.ui.set_focus_scope(fixture.second_scope, true);
    synchronize(router, &mut fixture.ui);
    assert_eq!(router.focused(), Some(fixture.second_editor));
}

#[test]
fn sibling_scopes_restore_each_other_then_the_original_trigger() {
    for remove in [false, true] {
        let mut fixture = fixture(false);
        let mut router = InteractionRouter::default();
        open_both(&mut router, &mut fixture);
        assert_eq!(router.scope_restore.len(), 2);
        if remove {
            fixture.ui.remove(fixture.second_scope);
        } else {
            fixture.ui.set_focus_scope(fixture.second_scope, false);
        }
        synchronize(&mut router, &mut fixture.ui);
        assert_eq!(router.focused(), Some(fixture.first_editor));
        if remove {
            fixture.ui.remove(fixture.first_scope);
        } else {
            fixture.ui.set_focus_scope(fixture.first_scope, false);
        }
        synchronize(&mut router, &mut fixture.ui);
        assert_eq!(router.focused(), Some(fixture.trigger));
        assert!(router.scope_restore.is_empty());
    }
}

#[test]
fn closing_a_lower_sibling_scope_preserves_the_outer_return_link() {
    for remove in [false, true] {
        let mut fixture = fixture(false);
        let mut router = InteractionRouter::default();
        open_both(&mut router, &mut fixture);
        if remove {
            fixture.ui.remove(fixture.first_scope);
        } else {
            fixture.ui.set_focus_scope(fixture.first_scope, false);
        }
        synchronize(&mut router, &mut fixture.ui);
        assert_eq!(router.focused(), Some(fixture.second_editor));
        assert_eq!(
            router.scope_restore,
            vec![(fixture.second_scope, Some(fixture.trigger))]
        );
        fixture.ui.remove(fixture.second_scope);
        synchronize(&mut router, &mut fixture.ui);
        assert_eq!(router.focused(), Some(fixture.trigger));
    }
}

#[test]
fn nested_scope_removal_restores_the_live_parent_or_outer_trigger() {
    for remove_both in [false, true] {
        let mut fixture = fixture(true);
        let mut router = InteractionRouter::default();
        open_both(&mut router, &mut fixture);
        if remove_both {
            fixture.ui.remove(fixture.first_scope);
        } else {
            fixture.ui.remove(fixture.second_scope);
        }
        synchronize(&mut router, &mut fixture.ui);
        if !remove_both {
            assert_eq!(router.focused(), Some(fixture.first_editor));
            fixture.ui.remove(fixture.first_scope);
            synchronize(&mut router, &mut fixture.ui);
        }
        assert_eq!(router.focused(), Some(fixture.trigger));
        assert!(router.scope_restore.is_empty());
    }
}

#[test]
fn a_newly_revealed_scope_inherits_the_closed_scopes_return_owner() {
    let mut fixture = fixture(false);
    let mut router = InteractionRouter::default();
    router.set_focus(&mut fixture.ui, Some(fixture.trigger), true);
    fixture.ui.set_focus_scope(fixture.first_scope, true);
    fixture.ui.set_focus_scope(fixture.second_scope, true);
    synchronize(&mut router, &mut fixture.ui);
    assert_eq!(router.focused(), Some(fixture.second_editor));
    fixture.ui.remove(fixture.second_scope);
    synchronize(&mut router, &mut fixture.ui);
    assert_eq!(router.focused(), Some(fixture.first_editor));
    assert_eq!(
        router.scope_restore,
        vec![(fixture.first_scope, Some(fixture.trigger))]
    );
    fixture.ui.remove(fixture.first_scope);
    synchronize(&mut router, &mut fixture.ui);
    assert_eq!(router.focused(), Some(fixture.trigger));
}

#[test]
fn a_removed_return_owner_cannot_focus_a_replacement_generation() {
    let mut fixture = fixture(false);
    let mut router = InteractionRouter::default();
    router.set_focus(&mut fixture.ui, Some(fixture.trigger), true);
    fixture.ui.set_focus_scope(fixture.first_scope, true);
    synchronize(&mut router, &mut fixture.ui);
    fixture.ui.remove(fixture.trigger);
    let replacement = MountWriter::<()>::under(&mut fixture.ui, fixture.root)
        .unwrap()
        .button_node(BoxStyle::default(), |_| {})
        .node;
    assert_eq!(replacement.index(), fixture.trigger.index());
    assert_ne!(replacement, fixture.trigger);
    fixture.ui.remove(fixture.first_scope);
    synchronize(&mut router, &mut fixture.ui);
    assert_eq!(router.focused(), None);
    assert!(router.scope_restore.is_empty());
}
