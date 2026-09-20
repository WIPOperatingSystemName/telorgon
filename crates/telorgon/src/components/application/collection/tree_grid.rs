//! Hierarchical rows adapted through the existing data-grid cell owner.
//!
//! `TreeHierarchy` remains the sole hierarchy/expansion owner and `DataGrid` remains the sole
//! table, cell-selection, active-cell, and two-dimensional navigation owner.

use std::cell::RefCell;
use std::fmt;
use std::hash::Hash;
use std::rc::Rc;

use crate::input::{ChangeSource, CompositeNavigationCommand, WritingDirection};
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    SemanticActions, SemanticCollection, SemanticName, SemanticNode, SemanticRelationship,
    SemanticRelationshipKind, SemanticRole, SemanticState, UiNodeId,
};

use super::{
    DataGrid, DataGridActivation, DataGridCell, DataGridError, DataGridNavigation, DataGridRef,
    SelectionProposal, TableCellRef, TreeExpansionProposal, TreeExpansionTransition, TreeHierarchy,
    TreeHierarchyError,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeGridNavigation<R, C> {
    command: CompositeNavigationCommand,
    previous: Option<DataGridCell<R, C>>,
    current: Option<DataGridCell<R, C>>,
    changed: bool,
    boundary: bool,
    expansion: Option<TreeExpansionProposal<R>>,
    selection: Option<SelectionProposal<DataGridCell<R, C>>>,
}

impl<R, C> TreeGridNavigation<R, C> {
    pub const fn command(&self) -> CompositeNavigationCommand {
        self.command
    }

    pub const fn previous(&self) -> Option<DataGridCell<R, C>>
    where
        R: Copy,
        C: Copy,
    {
        self.previous
    }

    pub const fn current(&self) -> Option<DataGridCell<R, C>>
    where
        R: Copy,
        C: Copy,
    {
        self.current
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }

    pub const fn boundary(&self) -> bool {
        self.boundary
    }

    pub const fn expansion(&self) -> Option<&TreeExpansionProposal<R>> {
        self.expansion.as_ref()
    }

    pub const fn selection(&self) -> Option<&SelectionProposal<DataGridCell<R, C>>> {
        self.selection.as_ref()
    }

    pub fn into_expansion(self) -> Option<TreeExpansionProposal<R>> {
        self.expansion
    }

    pub fn into_selection(self) -> Option<SelectionProposal<DataGridCell<R, C>>> {
        self.selection
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeGridActivation<R, C> {
    cell: DataGridActivation<R, C>,
}

impl<R, C> TreeGridActivation<R, C> {
    pub const fn cell(&self) -> &DataGridActivation<R, C> {
        &self.cell
    }

    pub fn into_cell(self) -> DataGridActivation<R, C> {
        self.cell
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeGridDiagnostics {
    pub navigation_requests: u64,
    pub hierarchy_requests: u64,
    pub boundaries: u64,
    pub expansion_applies: u64,
    pub failures: u64,
}

#[derive(Clone, Debug)]
pub struct TreeGrid<R, C> {
    hierarchy: TreeHierarchy<R>,
    grid: DataGrid<R, C>,
    disclosure_column: C,
    diagnostics: TreeGridDiagnostics,
}

impl<R, C> TreeGrid<R, C>
where
    R: Copy + Eq + Hash,
    C: Copy + Eq + Hash,
{
    pub fn new(
        hierarchy: TreeHierarchy<R>,
        grid: DataGrid<R, C>,
        disclosure_column: C,
    ) -> Result<Self, TreeGridError<R, C>> {
        validate_grid(&hierarchy, &grid, disclosure_column)?;
        Ok(Self {
            hierarchy,
            grid,
            disclosure_column,
            diagnostics: TreeGridDiagnostics::default(),
        })
    }

    pub const fn hierarchy(&self) -> &TreeHierarchy<R> {
        &self.hierarchy
    }

    pub const fn grid(&self) -> &DataGrid<R, C> {
        &self.grid
    }

    pub const fn disclosure_column(&self) -> &C {
        &self.disclosure_column
    }

    pub fn active_cell(&self) -> Option<DataGridCell<R, C>> {
        self.grid.active_cell()
    }

    pub const fn diagnostics(&self) -> TreeGridDiagnostics {
        self.diagnostics
    }

    pub fn navigate(
        &mut self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<TreeGridNavigation<R, C>, TreeGridError<R, C>> {
        self.diagnostics.navigation_requests += 1;
        let previous = self.active_cell();
        let decision = tree_decision(
            &mut self.hierarchy,
            previous,
            self.disclosure_column,
            command,
            direction,
        )
        .inspect_err(|_| self.diagnostics.failures += 1)?;
        self.finish_navigation(previous, command, direction, decision)
    }

    /// Applies a controlled expansion only together with a replacement grid whose rows exactly
    /// match the resulting visible hierarchy. Validation occurs before either owner is replaced.
    pub fn apply_expansion(
        &mut self,
        proposal: TreeExpansionProposal<R>,
        replacement: DataGrid<R, C>,
    ) -> Result<TreeExpansionTransition<R>, TreeGridError<R, C>> {
        let mut hierarchy = self.hierarchy.clone();
        let transition = hierarchy
            .apply_expansion(proposal)
            .map_err(TreeGridError::Hierarchy)?;
        validate_grid(&hierarchy, &replacement, self.disclosure_column)?;
        self.hierarchy = hierarchy;
        self.grid = replacement;
        if transition.changed() {
            self.diagnostics.expansion_applies += 1;
        }
        Ok(transition)
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<TreeGridRef<R, C>>
    where
        R: 'static,
        C: 'static,
        Action: 'static,
        Map: Fn(TreeGridActivation<R, C>) -> Action + 'static,
    {
        let map = Rc::new(map);
        let grid_ref = self.grid.mount(ui, host, {
            let map = map.clone();
            move |cell| map(TreeGridActivation { cell })
        })?;
        let total_count = u32::try_from(self.hierarchy.items().len())
            .map_err(|_| RuntimeError::new("tree grid exceeds semantic row capacity"))?;
        let root_count = u32::try_from(
            self.hierarchy
                .items()
                .iter()
                .filter(|item| item.parent().is_none())
                .count(),
        )
        .map_err(|_| RuntimeError::new("tree grid exceeds semantic root capacity"))?;
        for row in grid_ref.table().rows() {
            let index = self
                .hierarchy
                .items()
                .iter()
                .position(|candidate| candidate.key() == row.key())
                .expect("validated tree-grid rows have canonical indexes");
            let (position, set_size) = self
                .hierarchy
                .sibling_position(row.key())
                .expect("validated tree-grid rows have sibling metadata");
            let branch = self.hierarchy.is_branch(row.key());
            let expanded = branch.then(|| self.hierarchy.is_expanded(row.key()));
            let mut relationships = Vec::with_capacity(row.cells().len() + 1);
            relationships.push(owns(row.header_node()));
            relationships.extend(row.cells().iter().map(|cell| owns(cell.node())));
            ui.foundation()
                .semantic_node(
                    row.node(),
                    SemanticNode {
                        role: SemanticRole::Row,
                        state: SemanticState {
                            expanded,
                            ..SemanticState::default()
                        },
                        actions: if branch {
                            if expanded == Some(true) {
                                SemanticActions::COLLAPSE
                            } else {
                                SemanticActions::EXPAND
                            }
                        } else {
                            SemanticActions::NONE
                        },
                        relationships,
                        collection: Some(SemanticCollection {
                            item_index: u32::try_from(index).ok(),
                            item_count: Some(total_count),
                            level: self.hierarchy.level(row.key()),
                            position_in_set: Some(position),
                            set_size: Some(set_size),
                        }),
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid tree-grid row semantics: {error:?}"))
                })?;
        }

        let active_node = self.active_cell().and_then(|active| {
            grid_ref
                .table()
                .rows()
                .iter()
                .flat_map(|row| row.cells())
                .find(|cell| cell.row() == active.row() && cell.column() == active.column())
                .map(TableCellRef::node)
        });
        let mut relationships = Vec::with_capacity(grid_ref.table().rows().len() + 2);
        relationships.push(owns(grid_ref.table().header_row_node()));
        relationships.extend(grid_ref.table().rows().iter().map(|row| owns(row.node())));
        if let Some(active_node) = active_node {
            relationships.push(SemanticRelationship {
                kind: SemanticRelationshipKind::ActiveDescendant,
                target: active_node,
            });
        }
        let name = ui.foundation().intern(self.grid.table().label());
        let has_cells = self.active_cell().is_some();
        ui.foundation()
            .semantic_node(
                grid_ref.node(),
                SemanticNode {
                    role: SemanticRole::TreeGrid,
                    name: SemanticName::Text(name),
                    state: SemanticState {
                        disabled: !has_cells,
                        focusable: has_cells,
                        ..SemanticState::default()
                    },
                    actions: if has_cells {
                        SemanticActions::FOCUS | SemanticActions::ACTIVATE
                    } else {
                        SemanticActions::NONE
                    },
                    relationships,
                    collection: Some(SemanticCollection {
                        item_count: Some(total_count),
                        set_size: (root_count > 0).then_some(root_count),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid tree-grid semantics: {error:?}"))
            })?;
        Ok(TreeGridRef {
            grid: grid_ref,
            hierarchy: Rc::new(RefCell::new(self.hierarchy.clone())),
            disclosure_column: self.disclosure_column,
        })
    }

    fn finish_navigation(
        &mut self,
        previous: Option<DataGridCell<R, C>>,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
        decision: TreeGridDecision<R>,
    ) -> Result<TreeGridNavigation<R, C>, TreeGridError<R, C>> {
        match decision {
            TreeGridDecision::Delegate => {
                let navigation = self
                    .grid
                    .navigate(command, direction)
                    .map_err(TreeGridError::Grid)?;
                Ok(from_grid_navigation(navigation))
            }
            TreeGridDecision::Boundary => {
                self.diagnostics.boundaries += 1;
                Ok(boundary_navigation(command, previous))
            }
            TreeGridDecision::Expansion(expansion) => {
                self.diagnostics.hierarchy_requests += 1;
                Ok(expansion_navigation(command, previous, expansion))
            }
            TreeGridDecision::FocusRow(row) => {
                let previous = previous.expect("tree-row focus requires an active cell");
                let current = DataGridCell::new(row, *previous.column());
                let selection = self
                    .grid
                    .focus_cell_for_composition(current, ChangeSource::Directional)
                    .map_err(TreeGridError::Grid)?;
                Ok(focus_navigation(command, previous, current, selection))
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct TreeGridRef<R: 'static, C: 'static> {
    grid: DataGridRef<R, C>,
    hierarchy: Rc<RefCell<TreeHierarchy<R>>>,
    disclosure_column: C,
}

impl<R, C> TreeGridRef<R, C>
where
    R: Copy + Eq + Hash + 'static,
    C: Copy + Eq + Hash + 'static,
{
    pub const fn node(&self) -> UiNodeId {
        self.grid.node()
    }

    pub const fn grid(&self) -> &DataGridRef<R, C> {
        &self.grid
    }

    pub fn active_cell(&self) -> Option<DataGridCell<R, C>> {
        self.grid.active_cell()
    }

    pub fn navigate(
        &self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<TreeGridNavigation<R, C>, TreeGridError<R, C>> {
        let previous = self.active_cell();
        let decision = tree_decision(
            &mut self.hierarchy.borrow_mut(),
            previous,
            self.disclosure_column,
            command,
            direction,
        )?;
        match decision {
            TreeGridDecision::Delegate => self
                .grid
                .navigate(command, direction)
                .map(from_grid_navigation)
                .map_err(TreeGridError::Grid),
            TreeGridDecision::Boundary => Ok(boundary_navigation(command, previous)),
            TreeGridDecision::Expansion(expansion) => {
                Ok(expansion_navigation(command, previous, expansion))
            }
            TreeGridDecision::FocusRow(row) => {
                let previous = previous.expect("tree-row focus requires an active cell");
                let current = DataGridCell::new(row, *previous.column());
                let selection = self
                    .grid
                    .focus_cell_for_composition(current, ChangeSource::Directional)
                    .map_err(TreeGridError::Grid)?;
                Ok(focus_navigation(command, previous, current, selection))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeGridError<R, C> {
    UnknownDisclosureColumn(C),
    DisabledHierarchyItem(R),
    VisibleRowsMismatch,
    RowLabelMismatch(R),
    Hierarchy(TreeHierarchyError<R>),
    Grid(DataGridError<R, C>),
}

impl<R: fmt::Debug, C: fmt::Debug> fmt::Display for TreeGridError<R, C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "tree-grid operation failed: {self:?}")
    }
}

impl<R: fmt::Debug, C: fmt::Debug> std::error::Error for TreeGridError<R, C> {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TreeGridDecision<R> {
    Delegate,
    Boundary,
    Expansion(TreeExpansionProposal<R>),
    FocusRow(R),
}

fn tree_decision<R, C>(
    hierarchy: &mut TreeHierarchy<R>,
    active: Option<DataGridCell<R, C>>,
    disclosure_column: C,
    command: CompositeNavigationCommand,
    direction: WritingDirection,
) -> Result<TreeGridDecision<R>, TreeGridError<R, C>>
where
    R: Copy + Eq + Hash,
    C: Copy + Eq + Hash,
{
    if !matches!(
        command,
        CompositeNavigationCommand::Left | CompositeNavigationCommand::Right
    ) {
        return Ok(TreeGridDecision::Delegate);
    }
    let Some(active) = active else {
        return Ok(TreeGridDecision::Boundary);
    };
    if *active.column() != disclosure_column {
        return Ok(TreeGridDecision::Delegate);
    }
    let opens = matches!(
        (command, direction),
        (
            CompositeNavigationCommand::Right,
            WritingDirection::LeftToRight
        ) | (
            CompositeNavigationCommand::Left,
            WritingDirection::RightToLeft
        )
    );
    let row = *active.row();
    if opens {
        if !hierarchy.is_branch(&row) {
            return Ok(TreeGridDecision::Boundary);
        }
        if !hierarchy.is_expanded(&row) {
            return hierarchy
                .propose_expansion(row, true, ChangeSource::Directional)
                .map(TreeGridDecision::Expansion)
                .map_err(TreeGridError::Hierarchy);
        }
        return Ok(hierarchy
            .children(&row)
            .into_iter()
            .find(|item| item.is_enabled() && hierarchy.is_visible(item.key()))
            .map_or(TreeGridDecision::Boundary, |item| {
                TreeGridDecision::FocusRow(*item.key())
            }));
    }
    if hierarchy.is_branch(&row) && hierarchy.is_expanded(&row) {
        return hierarchy
            .propose_expansion(row, false, ChangeSource::Directional)
            .map(TreeGridDecision::Expansion)
            .map_err(TreeGridError::Hierarchy);
    }
    Ok(hierarchy
        .parent(&row)
        .filter(|item| item.is_enabled() && hierarchy.is_visible(item.key()))
        .map_or(TreeGridDecision::Boundary, |item| {
            TreeGridDecision::FocusRow(*item.key())
        }))
}

fn validate_grid<R, C>(
    hierarchy: &TreeHierarchy<R>,
    grid: &DataGrid<R, C>,
    disclosure_column: C,
) -> Result<(), TreeGridError<R, C>>
where
    R: Copy + Eq + Hash,
    C: Copy + Eq + Hash,
{
    if !grid
        .table()
        .columns()
        .iter()
        .any(|column| *column.key() == disclosure_column)
    {
        return Err(TreeGridError::UnknownDisclosureColumn(disclosure_column));
    }
    if let Some(item) = hierarchy.items().iter().find(|item| !item.is_enabled()) {
        return Err(TreeGridError::DisabledHierarchyItem(*item.key()));
    }
    let visible = hierarchy.visible_keys();
    if grid
        .table()
        .rows()
        .iter()
        .map(|row| *row.key())
        .ne(visible.iter().copied())
    {
        return Err(TreeGridError::VisibleRowsMismatch);
    }
    for row in grid.table().rows() {
        let item = hierarchy
            .item(row.key())
            .expect("visible row identity was validated above");
        if row.label() != item.label() {
            return Err(TreeGridError::RowLabelMismatch(*row.key()));
        }
    }
    Ok(())
}

fn from_grid_navigation<R, C>(navigation: DataGridNavigation<R, C>) -> TreeGridNavigation<R, C>
where
    R: Copy,
    C: Copy,
{
    TreeGridNavigation {
        command: navigation.command(),
        previous: navigation.previous(),
        current: navigation.current(),
        changed: navigation.changed(),
        boundary: navigation.boundary(),
        expansion: None,
        selection: navigation.into_selection(),
    }
}

fn boundary_navigation<R, C>(
    command: CompositeNavigationCommand,
    current: Option<DataGridCell<R, C>>,
) -> TreeGridNavigation<R, C>
where
    R: Copy,
    C: Copy,
{
    TreeGridNavigation {
        command,
        previous: current,
        current,
        changed: false,
        boundary: true,
        expansion: None,
        selection: None,
    }
}

fn expansion_navigation<R, C>(
    command: CompositeNavigationCommand,
    current: Option<DataGridCell<R, C>>,
    expansion: TreeExpansionProposal<R>,
) -> TreeGridNavigation<R, C>
where
    R: Copy,
    C: Copy,
{
    TreeGridNavigation {
        command,
        previous: current,
        current,
        changed: false,
        boundary: false,
        expansion: Some(expansion),
        selection: None,
    }
}

fn focus_navigation<R, C>(
    command: CompositeNavigationCommand,
    previous: DataGridCell<R, C>,
    current: DataGridCell<R, C>,
    selection: Option<SelectionProposal<DataGridCell<R, C>>>,
) -> TreeGridNavigation<R, C>
where
    R: Copy + Eq,
    C: Copy + Eq,
{
    TreeGridNavigation {
        command,
        previous: Some(previous),
        current: Some(current),
        changed: current != previous,
        boundary: false,
        expansion: None,
        selection,
    }
}

fn owns(target: UiNodeId) -> SemanticRelationship {
    SemanticRelationship {
        kind: SemanticRelationshipKind::Owns,
        target,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::UiRoot;

    use super::*;
    use crate::components::application::{
        DensityClass, DensityMetrics, SelectionFollowsFocus, SelectionMode, SelectionModel, Table,
        TableCell, TableColumn, TableRow, TreeItem,
    };

    fn hierarchy(expanded: impl IntoIterator<Item = u8>) -> TreeHierarchy<u8> {
        TreeHierarchy::new(
            [
                TreeItem::new(1, "Projects", None).unwrap(),
                TreeItem::new(2, "Telorgon", Some(1)).unwrap(),
                TreeItem::new(3, "Tests", Some(1)).unwrap(),
                TreeItem::new(4, "Archive", None).unwrap(),
            ],
            expanded,
        )
        .unwrap()
    }

    fn data_grid(hierarchy: &TreeHierarchy<u8>) -> DataGrid<u8, u8> {
        let rows: Vec<_> = hierarchy
            .visible_keys()
            .into_iter()
            .map(|key| {
                let item = hierarchy.item(&key).unwrap();
                TableRow::new(
                    key,
                    item.label(),
                    [
                        TableCell::new(10, item.label()),
                        TableCell::new(20, format!("row-{key}")),
                    ],
                )
                .unwrap()
            })
            .collect();
        let table = Table::new(
            "Project tree grid",
            [
                TableColumn::new(10, "Name").unwrap(),
                TableColumn::new(20, "Value").unwrap(),
            ],
            rows,
        )
        .unwrap();
        let cells = DataGrid::cells(&table);
        let selected = cells.first().copied();
        DataGrid::new(
            table,
            SelectionModel::new(
                SelectionMode::Multiple,
                SelectionFollowsFocus::Enabled,
                cells,
                selected,
                selected,
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn tree_grid(expanded: impl IntoIterator<Item = u8>) -> TreeGrid<u8, u8> {
        let hierarchy = hierarchy(expanded);
        let grid = data_grid(&hierarchy);
        TreeGrid::new(hierarchy, grid, 10).unwrap()
    }

    #[test]
    fn constructor_rejects_disclosure_and_visible_row_mismatches() {
        let collapsed = hierarchy([]);
        assert!(matches!(
            TreeGrid::new(collapsed.clone(), data_grid(&collapsed), 99),
            Err(TreeGridError::UnknownDisclosureColumn(99))
        ));
        let expanded = hierarchy([1]);
        assert!(matches!(
            TreeGrid::new(expanded, data_grid(&collapsed), 10),
            Err(TreeGridError::VisibleRowsMismatch)
        ));
    }

    #[test]
    fn expansion_is_controlled_atomic_then_descends_and_ascends_in_rtl() {
        let mut tree_grid = tree_grid([]);
        let open = tree_grid
            .navigate(
                CompositeNavigationCommand::Right,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        let proposal = open.into_expansion().unwrap();
        assert!(!tree_grid.hierarchy().is_expanded(&1));
        let mut replacement_hierarchy = tree_grid.hierarchy().clone();
        replacement_hierarchy
            .apply_expansion(proposal.clone())
            .unwrap();
        let replacement = data_grid(&replacement_hierarchy);
        tree_grid.apply_expansion(proposal, replacement).unwrap();
        let descend = tree_grid
            .navigate(
                CompositeNavigationCommand::Right,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(descend.current(), Some(DataGridCell::new(2, 10)));
        assert_eq!(descend.selection().unwrap().selected().len(), 2);
        assert_eq!(tree_grid.grid().selection().selected().len(), 1);
        let ascend = tree_grid
            .navigate(
                CompositeNavigationCommand::Right,
                WritingDirection::RightToLeft,
            )
            .unwrap();
        assert_eq!(ascend.current(), Some(DataGridCell::new(1, 10)));
    }

    #[test]
    fn ordinary_grid_navigation_remains_owned_by_data_grid() {
        let mut tree_grid = tree_grid([1]);
        let descend = tree_grid
            .navigate(
                CompositeNavigationCommand::Right,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(descend.current(), Some(DataGridCell::new(2, 10)));
        let across = tree_grid
            .navigate(
                CompositeNavigationCommand::End,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(across.current(), Some(DataGridCell::new(2, 20)));
        let down = tree_grid
            .navigate(
                CompositeNavigationCommand::Down,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(down.current(), Some(DataGridCell::new(3, 20)));
    }

    struct MountedTreeGrid {
        mounted: Rc<RefCell<Option<TreeGridRef<u8, u8>>>>,
    }

    impl Component for MountedTreeGrid {
        type State = TreeGrid<u8, u8>;
        type Action = TreeGridActivation<u8, u8>;

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
            let hierarchy = hierarchy([1]);
            let grid = data_grid(&hierarchy).density(DensityMetrics::baseline(DensityClass::Touch));
            TreeGrid::new(hierarchy, grid, 10).unwrap()
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui.foundation().root(
                crate::ui::BoxStyle::default(),
                crate::ui::LayoutStyle::default(),
                |_| {},
            );
            self.mounted
                .replace(Some(state.mount(ui, root.0, |intent| intent).unwrap()));
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

    #[test]
    fn mount_reuses_one_grid_focus_entry_and_adds_tree_row_metadata() {
        let mounted = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(MountedTreeGrid {
            mounted: mounted.clone(),
        })
        .unwrap();
        let mounted = mounted.borrow();
        let mounted = mounted.as_ref().unwrap();
        let root = runtime.ui().semantics.get(mounted.node()).unwrap();
        assert_eq!(root.role, SemanticRole::TreeGrid);
        assert_eq!(root.collection.unwrap().item_count, Some(4));
        assert_eq!(
            runtime
                .ui()
                .interactions
                .iter()
                .filter(|(_, interaction)| interaction.focusable)
                .count(),
            1
        );
        let first_row = &mounted.grid().table().rows()[0];
        let row = runtime.ui().semantics.get(first_row.node()).unwrap();
        assert_eq!(row.collection.unwrap().level, Some(1));
        assert_eq!(row.state.expanded, Some(true));
        assert!(row.actions.contains(crate::ui::SemanticAction::Collapse));
        assert!(runtime.ui().semantics.iter().all(|(_, semantic)| {
            !matches!(semantic.role, SemanticRole::Grid | SemanticRole::Table)
        }));
    }
}
