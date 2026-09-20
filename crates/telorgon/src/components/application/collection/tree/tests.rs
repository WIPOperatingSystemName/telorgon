use std::cell::RefCell;
use std::rc::Rc;

use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
use crate::ui::{SemanticAction, UiRoot};

use super::*;
use crate::components::application::{SelectionFollowsFocus, SelectionMode};

fn hierarchy(expanded: impl IntoIterator<Item = u8>) -> TreeHierarchy<u8> {
    TreeHierarchy::new(
        [
            TreeItem::new(1, "Projects", None).unwrap(),
            TreeItem::new(2, "Telorgon", Some(1)).unwrap(),
            TreeItem::new(3, "Sources", Some(2)).unwrap(),
            TreeItem::new(4, "Tests", Some(2)).unwrap(),
            TreeItem::new(5, "Archive", None).unwrap(),
        ],
        expanded,
    )
    .unwrap()
}

fn tree(expanded: impl IntoIterator<Item = u8>) -> TreeView<u8> {
    let hierarchy = hierarchy(expanded);
    TreeView::new(
        "Files",
        hierarchy,
        SelectionModel::new(
            SelectionMode::Multiple,
            SelectionFollowsFocus::Enabled,
            [1, 2, 3, 4, 5],
            [1],
            Some(1),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn hierarchy_rejects_invalid_parent_order_preorder_and_expansion() {
    assert!(matches!(
        TreeHierarchy::new(
            [
                TreeItem::new(1, "child", Some(2)).unwrap(),
                TreeItem::new(2, "parent", None).unwrap()
            ],
            []
        ),
        Err(TreeHierarchyError::ParentMustPrecede { .. })
    ));
    assert!(matches!(
        TreeHierarchy::new(
            [
                TreeItem::new(1, "root", None).unwrap(),
                TreeItem::new(2, "other", None).unwrap(),
                TreeItem::new(3, "late child", Some(1)).unwrap()
            ],
            []
        ),
        Err(TreeHierarchyError::NonContiguousSubtree(3))
    ));
    assert_eq!(
        TreeHierarchy::new([TreeItem::new(1, "leaf", None).unwrap()], [1]),
        Err(TreeHierarchyError::LeafExpansion(1))
    );
}

#[test]
fn visible_preorder_and_hierarchy_metadata_are_stable() {
    let hierarchy = hierarchy([1, 2]);
    assert_eq!(hierarchy.visible_keys(), [1, 2, 3, 4, 5]);
    assert_eq!(hierarchy.level(&3), Some(3));
    assert_eq!(hierarchy.sibling_position(&4), Some((2, 2)));
    assert_eq!(hierarchy.parent(&3).unwrap().key(), &2);
}

#[test]
fn directional_open_descend_close_ascend_is_rtl_aware_and_controlled() {
    let mut tree = tree([]);
    let open = tree
        .navigate(
            CompositeNavigationCommand::Right,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    assert_eq!(open.expansion().unwrap().key(), &1);
    assert!(!tree.hierarchy().is_expanded(&1));
    tree.apply_expansion(open.into_expansion().unwrap())
        .unwrap();
    let descend = tree
        .navigate(
            CompositeNavigationCommand::Right,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    assert!(matches!(
        descend.change(),
        CompositeChange::Highlighted { current: 2, .. }
    ));
    assert_eq!(descend.selection().unwrap().selected(), &[1, 2]);
    let ascend = tree
        .navigate(
            CompositeNavigationCommand::Right,
            WritingDirection::RightToLeft,
        )
        .unwrap();
    assert!(matches!(
        ascend.change(),
        CompositeChange::Highlighted { current: 1, .. }
    ));
}

#[test]
fn expansion_and_selection_proposals_are_revision_checked_and_source_preserving() {
    let mut tree = tree([]);
    let expansion = tree
        .propose_expansion(1, true, ChangeSource::Accessibility)
        .unwrap();
    let stale = expansion.clone();
    let applied = tree.apply_expansion(expansion).unwrap();
    assert_eq!(applied.expansion().source(), ChangeSource::Accessibility);
    assert!(matches!(
        tree.apply_expansion(stale),
        Err(TreeViewError::Hierarchy(
            TreeHierarchyError::StaleExpansionProposal { .. }
        ))
    ));
    let activation = tree
        .propose_item_activation(2, ChangeSource::Pointer)
        .unwrap();
    assert_eq!(activation.source(), ChangeSource::Pointer);
    assert_eq!(tree.selection().selected(), &[1]);
    tree.apply_selection(activation.into_selection().unwrap())
        .unwrap();
    assert_eq!(tree.selection().selected(), &[1, 2]);
}

struct MountedTree {
    mounted: Rc<RefCell<Option<TreeViewRef<u8>>>>,
    actions: Rc<RefCell<Vec<TreeViewActivation<u8>>>>,
}

impl Component for MountedTree {
    type State = TreeView<u8>;
    type Action = TreeViewActivation<u8>;

    fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
        tree([1]).density(DensityMetrics::baseline(DensityClass::Touch))
    }

    fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let root = ui
            .foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
        self.mounted
            .replace(Some(state.mount(ui, root.0, |intent| intent).unwrap()));
        root
    }

    fn action(
        &self,
        _state: &mut Self::State,
        action: Self::Action,
        _context: &mut UpdateContext<'_, Self>,
    ) {
        self.actions.borrow_mut().push(action);
    }
}

#[test]
fn mount_has_one_focus_entry_visible_tree_semantics_metadata_and_routes() {
    let mounted = Rc::new(RefCell::new(None));
    let actions = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = ViewRuntime::from_component(MountedTree {
        mounted: mounted.clone(),
        actions: actions.clone(),
    })
    .unwrap();
    let mounted = mounted.borrow();
    let mounted = mounted.as_ref().unwrap();
    assert_eq!(mounted.items().len(), 3);
    let root = runtime.ui().semantics.get(mounted.node()).unwrap();
    assert_eq!(root.role, SemanticRole::Tree);
    assert_eq!(root.collection.unwrap().item_count, Some(5));
    assert!(root.actions.contains(SemanticAction::Focus));
    assert_eq!(
        runtime
            .ui()
            .interactions
            .iter()
            .filter(|(_, interaction)| interaction.focusable)
            .count(),
        1
    );
    let branch = runtime
        .ui()
        .semantics
        .get(mounted.items()[0].node())
        .unwrap();
    assert_eq!(branch.role, SemanticRole::TreeItem);
    assert_eq!(branch.state.expanded, Some(true));
    assert_eq!(branch.collection.unwrap().level, Some(1));
    assert!(branch.actions.contains(SemanticAction::Collapse));
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(mounted.items()[0].node())
            .unwrap()
            .min_size,
        SizeRule2D {
            width: SizeRule::Logical(44.0),
            height: SizeRule::Logical(44.0),
        }
    );
    let item_node = mounted.items()[1].node();
    assert!(runtime.dispatch_activation(item_node, ChangeSource::Accessibility));
    assert_eq!(actions.borrow()[0].source(), ChangeSource::Accessibility);
}
