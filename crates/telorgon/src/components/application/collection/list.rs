//! Named stable-key ordinary list rows.
//!
//! `ListView` owns row descriptors and ordered semantics only. Row contents may create their own
//! independent controls; selection, composite focus, virtualization, and collection data remain
//! with their dedicated owners.

use std::fmt;

use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, MountWriter, Property, SemanticCollection,
    SemanticName, SemanticNode, SemanticRelationship, SemanticRelationshipKind, SemanticRole,
    SizeRule, SizeRule2D, UiNodeId,
};

use crate::components::application::{DensityClass, DensityMetrics};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListViewItem<K> {
    key: K,
    label: String,
}

impl<K> ListViewItem<K> {
    pub fn new(key: K, label: impl Into<String>) -> Result<Self, ListViewItemError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ListViewItemError::MissingAccessibleName);
        }
        Ok(Self { key, label })
    }

    pub const fn key(&self) -> &K {
        &self.key
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListViewItemError {
    MissingAccessibleName,
}

impl fmt::Display for ListViewItemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("list-view item accessible name is empty")
    }
}

impl std::error::Error for ListViewItemError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListViewMove<K> {
    key: K,
    from: usize,
    to: usize,
}

impl<K> ListViewMove<K> {
    pub const fn key(&self) -> &K {
        &self.key
    }

    pub const fn from(&self) -> usize {
        self.from
    }

    pub const fn to(&self) -> usize {
        self.to
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListViewUpdate<K> {
    inserted: Vec<K>,
    removed: Vec<K>,
    moved: Vec<ListViewMove<K>>,
    updated: Vec<K>,
    reused: usize,
    changed: bool,
    revision: u64,
}

impl<K> ListViewUpdate<K> {
    pub fn inserted(&self) -> &[K] {
        &self.inserted
    }

    pub fn removed(&self) -> &[K] {
        &self.removed
    }

    pub fn moved(&self) -> &[ListViewMove<K>] {
        &self.moved
    }

    /// Stable rows whose accessible label changed.
    pub fn updated(&self) -> &[K] {
        &self.updated
    }

    pub const fn reused(&self) -> usize {
        self.reused
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListViewDiagnostics {
    pub updates: u64,
    pub unchanged_updates: u64,
    pub inserted: u64,
    pub removed: u64,
    pub moved: u64,
    pub label_updates: u64,
    pub failures: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ListViewStyle {
    pub container: BoxStyle,
    pub row: BoxStyle,
    pub gap: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListView<K> {
    label: String,
    items: Vec<ListViewItem<K>>,
    density: DensityMetrics,
    style: ListViewStyle,
    revision: u64,
    diagnostics: ListViewDiagnostics,
}

impl<K> ListView<K>
where
    K: Clone + Eq,
{
    pub fn new(
        label: impl Into<String>,
        items: impl IntoIterator<Item = ListViewItem<K>>,
    ) -> Result<Self, ListViewError<K>> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ListViewError::MissingAccessibleName);
        }
        let items: Vec<_> = items.into_iter().collect();
        validate_unique(&items)?;
        Ok(Self {
            label,
            items,
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: ListViewStyle::default(),
            revision: 1,
            diagnostics: ListViewDiagnostics::default(),
        })
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: ListViewStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn items(&self) -> &[ListViewItem<K>] {
        &self.items
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn diagnostics(&self) -> ListViewDiagnostics {
        self.diagnostics
    }

    /// Atomically replaces the controlled row-descriptor snapshot and reports stable-key work.
    pub fn update_items(
        &mut self,
        items: impl IntoIterator<Item = ListViewItem<K>>,
    ) -> Result<ListViewUpdate<K>, ListViewError<K>> {
        let items: Vec<_> = items.into_iter().collect();
        validate_unique(&items).inspect_err(|_| {
            self.diagnostics.failures += 1;
        })?;
        if items == self.items {
            self.diagnostics.unchanged_updates += 1;
            return Ok(ListViewUpdate {
                inserted: Vec::new(),
                removed: Vec::new(),
                moved: Vec::new(),
                updated: Vec::new(),
                reused: items.len(),
                changed: false,
                revision: self.revision,
            });
        }

        let inserted: Vec<_> = items
            .iter()
            .filter(|item| self.index_of(&item.key).is_none())
            .map(|item| item.key.clone())
            .collect();
        let removed: Vec<_> = self
            .items
            .iter()
            .filter(|item| !items.iter().any(|next| next.key == item.key))
            .map(|item| item.key.clone())
            .collect();
        let mut moved = Vec::new();
        let mut updated = Vec::new();
        let mut reused = 0;
        for (to, item) in items.iter().enumerate() {
            if let Some(from) = self.index_of(&item.key) {
                reused += 1;
                if from != to {
                    moved.push(ListViewMove {
                        key: item.key.clone(),
                        from,
                        to,
                    });
                }
                if self.items[from].label != item.label {
                    updated.push(item.key.clone());
                }
            }
        }
        let revision = self.next_revision()?;
        self.items = items;
        self.revision = revision;
        self.diagnostics.updates += 1;
        self.diagnostics.inserted += inserted.len() as u64;
        self.diagnostics.removed += removed.len() as u64;
        self.diagnostics.moved += moved.len() as u64;
        self.diagnostics.label_updates += updated.len() as u64;
        Ok(ListViewUpdate {
            inserted,
            removed,
            moved,
            updated,
            reused,
            changed: true,
            revision,
        })
    }

    pub fn mount<'storage, Action, Content>(
        &self,
        ui: &mut Ui<'_, 'storage, Action>,
        host: UiNodeId,
        mut content: Content,
    ) -> RuntimeResult<ListViewRef<K>>
    where
        Action: 'static,
        Content: FnMut(&ListViewItem<K>, &mut MountWriter<'storage, Action>),
    {
        let item_count = u32::try_from(self.items.len())
            .map_err(|_| RuntimeError::new("list view exceeds semantic item capacity"))?;
        let minimum = self.density.effective_minimum();
        let mut mounted = Vec::with_capacity(self.items.len());
        let root = ui
            .foundation()
            .container_node_under(
                host,
                self.style.container,
                LayoutStyle {
                    flow: Flow::Vertical,
                    gap: self.style.gap,
                    ..LayoutStyle::default()
                },
                |writer| {
                    for item in &self.items {
                        let mut style = self.style.row;
                        style.min_size = SizeRule2D {
                            width: SizeRule::Logical(minimum.width()),
                            height: SizeRule::Logical(minimum.height()),
                        };
                        let control = writer.layer(true, style, LayoutStyle::default(), |writer| {
                            content(item, writer)
                        });
                        mounted.push((item.clone(), control));
                    }
                },
            )
            .ok_or_else(|| RuntimeError::new("application list-view host is stale"))?;

        let mut rows = Vec::with_capacity(mounted.len());
        for (index, (item, control)) in mounted.into_iter().enumerate() {
            let name = ui.foundation().intern(&item.label);
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        role: SemanticRole::ListItem,
                        name: SemanticName::Text(name),
                        collection: Some(SemanticCollection {
                            item_index: u32::try_from(index).ok(),
                            item_count: Some(item_count),
                            position_in_set: u32::try_from(index + 1).ok(),
                            set_size: Some(item_count),
                            ..SemanticCollection::default()
                        }),
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid list-view row semantics: {error:?}"))
                })?;
            rows.push(ListViewRowRef {
                key: item.key,
                control,
                index,
            });
        }

        let name = ui.foundation().intern(&self.label);
        let relationships = rows
            .iter()
            .map(|row| SemanticRelationship {
                kind: SemanticRelationshipKind::Owns,
                target: row.control.node,
            })
            .collect();
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::List,
                    name: SemanticName::Text(name),
                    relationships,
                    collection: Some(SemanticCollection {
                        item_count: Some(item_count),
                        set_size: (item_count > 0).then_some(item_count),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid list-view semantics: {error:?}"))
            })?;

        Ok(ListViewRef { root, rows })
    }

    fn index_of(&self, key: &K) -> Option<usize> {
        self.items.iter().position(|item| &item.key == key)
    }

    fn next_revision(&mut self) -> Result<u64, ListViewError<K>> {
        self.revision.checked_add(1).ok_or_else(|| {
            self.diagnostics.failures += 1;
            ListViewError::RevisionExhausted
        })
    }
}

#[derive(Clone, Debug)]
pub struct ListViewRef<K> {
    root: ControlHandle,
    rows: Vec<ListViewRowRef<K>>,
}

impl<K> ListViewRef<K> {
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub fn rows(&self) -> &[ListViewRowRef<K>] {
        &self.rows
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Debug)]
pub struct ListViewRowRef<K> {
    key: K,
    control: ControlHandle,
    index: usize,
}

impl<K> ListViewRowRef<K> {
    pub const fn key(&self) -> &K {
        &self.key
    }

    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn index(&self) -> usize {
        self.index
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.control.style
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListViewError<K> {
    MissingAccessibleName,
    DuplicateKey(K),
    RevisionExhausted,
}

impl<K: fmt::Debug> fmt::Display for ListViewError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "list-view operation failed: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for ListViewError<K> {}

fn validate_unique<K>(items: &[ListViewItem<K>]) -> Result<(), ListViewError<K>>
where
    K: Clone + Eq,
{
    for (index, item) in items.iter().enumerate() {
        if items[..index].iter().any(|other| other.key == item.key) {
            return Err(ListViewError::DuplicateKey(item.key.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::UiRoot;

    use super::*;

    fn item(key: u8, label: &str) -> ListViewItem<u8> {
        ListViewItem::new(key, label).unwrap()
    }

    #[test]
    fn construction_requires_names_and_unique_keys_but_allows_empty_lists() {
        assert_eq!(
            ListViewItem::new(1_u8, " ").unwrap_err(),
            ListViewItemError::MissingAccessibleName
        );
        assert_eq!(
            ListView::<u8>::new(" ", []).unwrap_err(),
            ListViewError::MissingAccessibleName
        );
        assert_eq!(
            ListView::new("Items", [item(1, "One"), item(1, "Again")]).unwrap_err(),
            ListViewError::DuplicateKey(1)
        );
        assert!(
            ListView::<u8>::new("Empty items", [])
                .unwrap()
                .items()
                .is_empty()
        );
    }

    #[test]
    fn controlled_snapshot_reports_insert_remove_move_update_and_reuse_atomically() {
        let mut list =
            ListView::new("Items", [item(1, "One"), item(2, "Two"), item(3, "Three")]).unwrap();
        let update = list
            .update_items([item(3, "Three updated"), item(2, "Two"), item(4, "Four")])
            .unwrap();
        assert_eq!(update.inserted(), &[4]);
        assert_eq!(update.removed(), &[1]);
        assert_eq!(update.reused(), 2);
        assert_eq!(update.updated(), &[3]);
        assert_eq!(
            update
                .moved()
                .iter()
                .map(|movement| (*movement.key(), movement.from(), movement.to()))
                .collect::<Vec<_>>(),
            vec![(3, 2, 0)]
        );
        let revision = list.revision();
        assert_eq!(
            list.update_items([item(3, "Three"), item(3, "Again")]),
            Err(ListViewError::DuplicateKey(3))
        );
        assert_eq!(list.revision(), revision);
        assert_eq!(list.items()[0].label(), "Three updated");
        assert_eq!(list.diagnostics().failures, 1);
    }

    #[derive(Clone, Debug)]
    enum MountedAction {
        RowControl(u8),
    }

    struct MountedList {
        mounted: Rc<RefCell<Option<ListViewRef<u8>>>>,
        child_nodes: Rc<RefCell<Vec<UiNodeId>>>,
    }

    impl Component for MountedList {
        type State = ListView<u8>;
        type Action = MountedAction;

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
            ListView::new("Recent documents", [item(7, "Seven"), item(9, "Nine")])
                .unwrap()
                .density(DensityMetrics::baseline(DensityClass::Touch))
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.mounted.replace(Some(
                state
                    .mount(ui, root.0, |item, writer| {
                        let control = writer.button(
                            MountedAction::RowControl(*item.key()),
                            BoxStyle::default(),
                            |_| {},
                        );
                        self.child_nodes.borrow_mut().push(control.node);
                    })
                    .unwrap(),
            ));
            root
        }

        fn action(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            let MountedAction::RowControl(key) = action;
            assert!(matches!(key, 7 | 9));
        }
    }

    #[test]
    fn mounted_list_has_ordered_semantics_touch_rows_and_independent_child_controls() {
        let mounted = Rc::new(RefCell::new(None));
        let child_nodes = Rc::new(RefCell::new(Vec::new()));
        let runtime = ViewRuntime::from_component(MountedList {
            mounted: mounted.clone(),
            child_nodes: child_nodes.clone(),
        })
        .unwrap();
        let mounted = mounted.borrow();
        let mounted = mounted.as_ref().unwrap();
        let root = runtime.ui().semantics.get(mounted.node()).unwrap();
        assert_eq!(root.role, SemanticRole::List);
        assert_eq!(root.collection.unwrap().item_count, Some(2));
        assert_eq!(root.relationships.len(), 2);
        for (index, row) in mounted.rows().iter().enumerate() {
            assert!(
                !runtime
                    .ui()
                    .interactions
                    .get(row.node())
                    .is_some_and(|interaction| interaction.focusable)
            );
            let semantic = runtime.ui().semantics.get(row.node()).unwrap();
            assert_eq!(semantic.role, SemanticRole::ListItem);
            assert_eq!(
                semantic.collection.unwrap().position_in_set,
                u32::try_from(index + 1).ok()
            );
            assert_eq!(
                runtime.ui().box_styles.get(row.node()).unwrap().min_size,
                SizeRule2D {
                    width: SizeRule::Logical(44.0),
                    height: SizeRule::Logical(44.0),
                }
            );
        }
        assert_eq!(child_nodes.borrow().len(), 2);
        assert!(child_nodes.borrow().iter().all(|node| {
            runtime
                .ui()
                .interactions
                .get(*node)
                .is_some_and(|interaction| interaction.focusable)
        }));
    }

    #[test]
    fn mounted_empty_list_reports_a_known_zero_count_without_rows() {
        struct Empty;
        impl Component for Empty {
            type State = ListView<u8>;
            type Action = ();

            fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
                ListView::new("Empty", []).unwrap()
            }

            fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
                let root =
                    ui.foundation()
                        .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
                state.mount(ui, root.0, |_, _| {}).unwrap();
                root
            }

            fn action(
                &self,
                _state: &mut Self::State,
                _action: Self::Action,
                _context: &mut UpdateContext<'_, Self>,
            ) {
            }
        }
        let runtime = ViewRuntime::from_component(Empty).unwrap();
        let list = runtime
            .ui()
            .semantics
            .iter()
            .find_map(|(_, semantic)| (semantic.role == SemanticRole::List).then_some(semantic))
            .unwrap();
        assert_eq!(list.collection.unwrap().item_count, Some(0));
        assert!(list.relationships.is_empty());
    }
}
