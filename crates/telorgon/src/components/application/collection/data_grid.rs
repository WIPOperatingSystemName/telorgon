//! Stable-key two-dimensional data-grid navigation and controlled intents.
//!
//! `Table` remains the rectangular presentation-descriptor validator, `SelectionModel` remains the
//! selected-cell owner, and `CompositeStateMachine` remains the active-cell owner. This adapter
//! adds only two-dimensional navigation, grid semantics, and activation intent routing.

use std::cell::RefCell;
use std::fmt;
use std::hash::Hash;
use std::rc::Rc;

use crate::input::{
    ChangeSource, CompositeError, CompositeItem, CompositeNavigationCommand,
    CompositeNavigationPolicy, CompositeStateMachine, WritingDirection,
};
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, Property, SemanticActions, SemanticCollection, SemanticName,
    SemanticNode, SemanticParticipation, SemanticRelationship, SemanticRelationshipKind,
    SemanticRole, SemanticState, UiNodeId,
};

use super::{
    SelectionError, SelectionMode, SelectionModel, SelectionProposal, SelectionTransition, Table,
    TableCellRef, TableRef, TableStyle,
};
use crate::components::application::{DensityClass, DensityMetrics};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DataGridCell<R, C> {
    row: R,
    column: C,
}

pub(crate) type DataGridFocusSelection<R, C> =
    Result<Option<SelectionProposal<DataGridCell<R, C>>>, DataGridError<R, C>>;

impl<R, C> DataGridCell<R, C> {
    pub const fn new(row: R, column: C) -> Self {
        Self { row, column }
    }

    pub const fn row(&self) -> &R {
        &self.row
    }

    pub const fn column(&self) -> &C {
        &self.column
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DataGridStyle {
    pub root: BoxStyle,
    pub table: TableStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataGridNavigation<R, C> {
    command: CompositeNavigationCommand,
    previous: Option<DataGridCell<R, C>>,
    current: Option<DataGridCell<R, C>>,
    changed: bool,
    boundary: bool,
    selection: Option<SelectionProposal<DataGridCell<R, C>>>,
}

impl<R, C> DataGridNavigation<R, C> {
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

    pub const fn selection(&self) -> Option<&SelectionProposal<DataGridCell<R, C>>> {
        self.selection.as_ref()
    }

    pub fn into_selection(self) -> Option<SelectionProposal<DataGridCell<R, C>>> {
        self.selection
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataGridActivation<R, C> {
    cell: DataGridCell<R, C>,
    source: ChangeSource,
    selection: Option<SelectionProposal<DataGridCell<R, C>>>,
}

impl<R, C> DataGridActivation<R, C> {
    pub const fn cell(&self) -> &DataGridCell<R, C> {
        &self.cell
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub const fn selection(&self) -> Option<&SelectionProposal<DataGridCell<R, C>>> {
        self.selection.as_ref()
    }

    pub fn into_selection(self) -> Option<SelectionProposal<DataGridCell<R, C>>> {
        self.selection
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DataGridDiagnostics {
    pub navigation_requests: u64,
    pub boundaries: u64,
    pub activation_requests: u64,
    pub selection_requests: u64,
    pub selection_applies: u64,
    pub failures: u64,
}

#[derive(Clone, Debug)]
pub struct DataGrid<R, C> {
    table: Table<R, C>,
    selection: SelectionModel<DataGridCell<R, C>>,
    composite: CompositeStateMachine<DataGridCell<R, C>>,
    density: DensityMetrics,
    style: DataGridStyle,
    diagnostics: DataGridDiagnostics,
}

impl<R, C> DataGrid<R, C>
where
    R: Copy + Eq + Hash,
    C: Copy + Eq + Hash,
{
    pub fn cells(table: &Table<R, C>) -> Vec<DataGridCell<R, C>> {
        table
            .rows()
            .iter()
            .flat_map(|row| {
                table
                    .columns()
                    .iter()
                    .map(move |column| DataGridCell::new(*row.key(), *column.key()))
            })
            .collect()
    }

    pub fn new(
        table: Table<R, C>,
        selection: SelectionModel<DataGridCell<R, C>>,
    ) -> Result<Self, DataGridError<R, C>> {
        let cells = Self::cells(&table);
        if selection.items() != cells {
            return Err(DataGridError::SelectionItemsMismatch);
        }
        let mut composite = CompositeStateMachine::new(CompositeNavigationPolicy::default());
        composite
            .update_items(
                cells
                    .iter()
                    .copied()
                    .map(|key| CompositeItem { key, enabled: true }),
            )
            .map_err(DataGridError::Composite)?;
        composite
            .enter(selection.selected().first().copied())
            .map_err(DataGridError::Composite)?;
        Ok(Self {
            table,
            selection,
            composite,
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: DataGridStyle::default(),
            diagnostics: DataGridDiagnostics::default(),
        })
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: DataGridStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn table(&self) -> &Table<R, C> {
        &self.table
    }

    pub const fn selection(&self) -> &SelectionModel<DataGridCell<R, C>> {
        &self.selection
    }

    pub fn active_cell(&self) -> Option<DataGridCell<R, C>> {
        self.composite.active_descendant()
    }

    pub const fn diagnostics(&self) -> DataGridDiagnostics {
        self.diagnostics
    }

    pub fn navigate(
        &mut self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<DataGridNavigation<R, C>, DataGridError<R, C>> {
        self.diagnostics.navigation_requests += 1;
        let previous = self.active_cell();
        let Some(previous_cell) = previous else {
            self.diagnostics.boundaries += 1;
            return Ok(DataGridNavigation {
                command,
                previous,
                current: previous,
                changed: false,
                boundary: true,
                selection: None,
            });
        };
        let row_index = self
            .table
            .rows()
            .iter()
            .position(|row| *row.key() == previous_cell.row)
            .expect("active grid rows remain in the validated table");
        let column_index = self
            .table
            .columns()
            .iter()
            .position(|column| *column.key() == previous_cell.column)
            .expect("active grid columns remain in the validated table");
        let row_count = self.table.rows().len();
        let column_count = self.table.columns().len();
        let logical_command = match (command, direction) {
            (CompositeNavigationCommand::Left, WritingDirection::RightToLeft) => {
                CompositeNavigationCommand::Right
            }
            (CompositeNavigationCommand::Right, WritingDirection::RightToLeft) => {
                CompositeNavigationCommand::Left
            }
            _ => command,
        };
        let target_indices = match logical_command {
            CompositeNavigationCommand::Up => {
                row_index.checked_sub(1).map(|row| (row, column_index))
            }
            CompositeNavigationCommand::Down => {
                (row_index + 1 < row_count).then_some((row_index + 1, column_index))
            }
            CompositeNavigationCommand::Left => column_index
                .checked_sub(1)
                .map(|column| (row_index, column)),
            CompositeNavigationCommand::Right => {
                (column_index + 1 < column_count).then_some((row_index, column_index + 1))
            }
            CompositeNavigationCommand::Home => Some((row_index, 0)),
            CompositeNavigationCommand::End => Some((row_index, column_count - 1)),
            CompositeNavigationCommand::Previous => {
                let flat = row_index * column_count + column_index;
                flat.checked_sub(1)
                    .map(|index| (index / column_count, index % column_count))
            }
            CompositeNavigationCommand::Next => {
                let flat = row_index * column_count + column_index;
                (flat + 1 < row_count * column_count)
                    .then_some(((flat + 1) / column_count, (flat + 1) % column_count))
            }
        };
        let Some((target_row, target_column)) = target_indices else {
            self.diagnostics.boundaries += 1;
            return Ok(DataGridNavigation {
                command,
                previous,
                current: previous,
                changed: false,
                boundary: true,
                selection: None,
            });
        };
        let current = DataGridCell::new(
            *self.table.rows()[target_row].key(),
            *self.table.columns()[target_column].key(),
        );
        if current == previous_cell {
            self.diagnostics.boundaries += 1;
            return Ok(DataGridNavigation {
                command,
                previous,
                current: previous,
                changed: false,
                boundary: true,
                selection: None,
            });
        }
        self.composite
            .set_active_descendant(current)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(DataGridError::Composite)?;
        let selection = self
            .selection
            .propose_focus(&current, ChangeSource::Directional)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(DataGridError::Selection)?;
        if selection.is_some() {
            self.diagnostics.selection_requests += 1;
        }
        Ok(DataGridNavigation {
            command,
            previous,
            current: Some(current),
            changed: true,
            boundary: false,
            selection,
        })
    }

    pub fn propose_cell_activation(
        &mut self,
        cell: DataGridCell<R, C>,
        source: ChangeSource,
    ) -> Result<DataGridActivation<R, C>, DataGridError<R, C>> {
        self.composite
            .set_active_descendant(cell)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(DataGridError::Composite)?;
        self.activation(cell, source)
    }

    pub fn propose_active_activation(
        &mut self,
        source: ChangeSource,
    ) -> Result<DataGridActivation<R, C>, DataGridError<R, C>> {
        let cell = self
            .composite
            .request_active_selection(source)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(DataGridError::Composite)?
            .key;
        self.activation(cell, source)
    }

    pub fn apply_selection(
        &mut self,
        proposal: SelectionProposal<DataGridCell<R, C>>,
    ) -> Result<SelectionTransition<DataGridCell<R, C>>, DataGridError<R, C>> {
        let transition = self
            .selection
            .apply(proposal)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(DataGridError::Selection)?;
        self.diagnostics.selection_applies += 1;
        Ok(transition)
    }

    /// Moves the existing grid composite for a higher-level collection adapter. The returned
    /// selection remains a controlled proposal owned by this grid's selection model.
    pub(crate) fn focus_cell_for_composition(
        &mut self,
        cell: DataGridCell<R, C>,
        source: ChangeSource,
    ) -> DataGridFocusSelection<R, C> {
        self.composite
            .set_active_descendant(cell)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(DataGridError::Composite)?;
        let selection = self
            .selection
            .propose_focus(&cell, source)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(DataGridError::Selection)?;
        if selection.is_some() {
            self.diagnostics.selection_requests += 1;
        }
        Ok(selection)
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<DataGridRef<R, C>>
    where
        R: 'static,
        C: 'static,
        Action: 'static,
        Map: Fn(DataGridActivation<R, C>) -> Action + 'static,
    {
        let behavior = Rc::new(RefCell::new(self.clone()));
        let map = Rc::new(map);
        let has_cells = !self.selection.items().is_empty();
        let root = ui
            .foundation()
            .button_node_under(host, self.style.root, |_| {})
            .ok_or_else(|| RuntimeError::new("application data-grid host is stale"))?;
        if !has_cells {
            ui.foundation().disabled(root.node, true);
        }
        let mut table_style = self.style.table;
        table_style.table = BoxStyle::default();
        let table_ref = self
            .table
            .clone()
            .density(self.density)
            .style(table_style)
            .mount(ui, root.node)?;
        ui.foundation()
            .semantic_node(
                table_ref.node(),
                SemanticNode {
                    participation: SemanticParticipation::Exclude,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid data-grid structural semantics: {error:?}"))
            })?;

        let row_count = u32::try_from(self.table.rows().len())
            .map_err(|_| RuntimeError::new("data grid exceeds semantic row capacity"))?;
        let column_count = u32::try_from(self.table.columns().len())
            .map_err(|_| RuntimeError::new("data grid exceeds semantic column capacity"))?;
        let selectable = self.selection.mode() != SelectionMode::None;
        for row in table_ref.rows() {
            for cell_ref in row.cells() {
                let cell = DataGridCell::new(*cell_ref.row(), *cell_ref.column());
                let descriptor = self
                    .table
                    .cell(cell.row(), cell.column())
                    .expect("mounted table cells preserve validated coordinates");
                let name = if descriptor.text().trim().is_empty() {
                    SemanticName::Unspecified
                } else {
                    SemanticName::Text(ui.foundation().intern(descriptor.text()))
                };
                let column_header = table_ref.columns()[cell_ref.column_index()].node();
                ui.foundation()
                    .semantic_node(
                        cell_ref.node(),
                        SemanticNode {
                            role: SemanticRole::Cell,
                            name,
                            state: SemanticState {
                                selected: Some(self.selection.is_selected(&cell)),
                                ..SemanticState::default()
                            },
                            actions: SemanticActions::ACTIVATE
                                | if selectable {
                                    SemanticActions::SELECT
                                } else {
                                    SemanticActions::NONE
                                },
                            relationships: vec![
                                SemanticRelationship {
                                    kind: SemanticRelationshipKind::LabelledBy,
                                    target: row.header_node(),
                                },
                                SemanticRelationship {
                                    kind: SemanticRelationshipKind::LabelledBy,
                                    target: column_header,
                                },
                            ],
                            collection: Some(collection_position(
                                cell_ref.column_index(),
                                column_count,
                            )),
                            ..SemanticNode::default()
                        },
                    )
                    .map_err(|error| {
                        RuntimeError::new(format!("invalid data-grid cell semantics: {error:?}"))
                    })?;
                let behavior = behavior.clone();
                let map = map.clone();
                ui.route_activation_fallible(cell_ref.node(), move |activation| {
                    let intent = behavior
                        .borrow_mut()
                        .propose_cell_activation(cell, activation.source)
                        .map_err(|_| RuntimeError::new("data-grid cell activation failed"))?;
                    Ok(map(intent))
                })?;
            }
        }

        let active_node = self.active_cell().and_then(|active| {
            table_ref
                .rows()
                .iter()
                .flat_map(|row| row.cells())
                .find(|cell| *cell.row() == active.row && *cell.column() == active.column)
                .map(TableCellRef::node)
        });
        let mut relationships = Vec::with_capacity(table_ref.rows().len() + 2);
        relationships.push(owns(table_ref.header_row_node()));
        relationships.extend(table_ref.rows().iter().map(|row| owns(row.node())));
        if let Some(active_node) = active_node {
            relationships.push(SemanticRelationship {
                kind: SemanticRelationshipKind::ActiveDescendant,
                target: active_node,
            });
        }
        let name = ui.foundation().intern(self.table.label());
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Grid,
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
                        item_count: Some(row_count),
                        set_size: (row_count > 0).then_some(row_count),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid data-grid semantics: {error:?}"))
            })?;
        if has_cells {
            let behavior = behavior.clone();
            let map = map.clone();
            ui.route_activation_fallible(root.node, move |activation| {
                let intent = behavior
                    .borrow_mut()
                    .propose_active_activation(activation.source)
                    .map_err(|_| RuntimeError::new("data-grid active activation failed"))?;
                Ok(map(intent))
            })?;
        }
        Ok(DataGridRef {
            root,
            table: table_ref,
            behavior,
        })
    }

    fn activation(
        &mut self,
        cell: DataGridCell<R, C>,
        source: ChangeSource,
    ) -> Result<DataGridActivation<R, C>, DataGridError<R, C>> {
        self.diagnostics.activation_requests += 1;
        let selection = match self.selection.mode() {
            SelectionMode::None => None,
            SelectionMode::Single => Some(self.selection.propose_select(&cell, source)),
            SelectionMode::Multiple => Some(self.selection.propose_toggle(&cell, source)),
        }
        .transpose()
        .inspect_err(|_| self.diagnostics.failures += 1)
        .map_err(DataGridError::Selection)?;
        if selection.is_some() {
            self.diagnostics.selection_requests += 1;
        }
        Ok(DataGridActivation {
            cell,
            source,
            selection,
        })
    }
}

#[derive(Clone, Debug)]
pub struct DataGridRef<R: 'static, C: 'static> {
    root: ControlHandle,
    table: TableRef<R, C>,
    behavior: Rc<RefCell<DataGrid<R, C>>>,
}

impl<R, C> DataGridRef<R, C>
where
    R: Copy + Eq + Hash + 'static,
    C: Copy + Eq + Hash + 'static,
{
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub const fn table(&self) -> &TableRef<R, C> {
        &self.table
    }

    pub fn active_cell(&self) -> Option<DataGridCell<R, C>> {
        self.behavior.borrow().active_cell()
    }

    pub fn navigate(
        &self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<DataGridNavigation<R, C>, DataGridError<R, C>> {
        self.behavior.borrow_mut().navigate(command, direction)
    }

    pub fn propose_active_activation(
        &self,
        source: ChangeSource,
    ) -> Result<DataGridActivation<R, C>, DataGridError<R, C>> {
        self.behavior.borrow_mut().propose_active_activation(source)
    }

    pub(crate) fn focus_cell_for_composition(
        &self,
        cell: DataGridCell<R, C>,
        source: ChangeSource,
    ) -> DataGridFocusSelection<R, C> {
        self.behavior
            .borrow_mut()
            .focus_cell_for_composition(cell, source)
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataGridError<R, C> {
    SelectionItemsMismatch,
    Selection(SelectionError<DataGridCell<R, C>>),
    Composite(CompositeError<DataGridCell<R, C>>),
}

impl<R: fmt::Debug, C: fmt::Debug> fmt::Display for DataGridError<R, C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "data-grid operation failed: {self:?}")
    }
}

impl<R: fmt::Debug, C: fmt::Debug> std::error::Error for DataGridError<R, C> {}

fn collection_position(index: usize, count: u32) -> SemanticCollection {
    SemanticCollection {
        item_index: u32::try_from(index).ok(),
        item_count: Some(count),
        position_in_set: u32::try_from(index + 1).ok(),
        set_size: Some(count),
        ..SemanticCollection::default()
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
    use crate::ui::{SemanticAction, UiRoot};

    use super::*;
    use crate::components::application::{SelectionFollowsFocus, TableCell, TableColumn, TableRow};

    type RecordedActivations = Rc<RefCell<Vec<(DataGridCell<u8, u8>, ChangeSource)>>>;

    fn table() -> Table<u8, u8> {
        Table::new(
            "Grid",
            [
                TableColumn::new(10, "Name").unwrap(),
                TableColumn::new(20, "Status").unwrap(),
            ],
            [
                TableRow::new(
                    1,
                    "API",
                    [TableCell::new(10, "Gateway"), TableCell::new(20, "Ready")],
                )
                .unwrap(),
                TableRow::new(
                    2,
                    "Jobs",
                    [TableCell::new(10, "Worker"), TableCell::new(20, "Paused")],
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn grid(
        mode: SelectionMode,
        follows: SelectionFollowsFocus,
        selected: impl IntoIterator<Item = DataGridCell<u8, u8>>,
    ) -> DataGrid<u8, u8> {
        let table = table();
        let cells = DataGrid::cells(&table);
        DataGrid::new(
            table,
            SelectionModel::new(mode, follows, cells, selected, None).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn construction_requires_selection_items_to_match_the_rectangular_table() {
        let table = table();
        assert!(matches!(
            DataGrid::new(
                table,
                SelectionModel::new(
                    SelectionMode::Single,
                    SelectionFollowsFocus::Disabled,
                    [DataGridCell::new(1, 10)],
                    [],
                    None
                )
                .unwrap()
            ),
            Err(DataGridError::SelectionItemsMismatch)
        ));
    }

    #[test]
    fn navigation_is_two_dimensional_rtl_aware_and_selection_remains_controlled() {
        let first = DataGridCell::new(1, 10);
        let mut grid = grid(
            SelectionMode::Multiple,
            SelectionFollowsFocus::Enabled,
            [first],
        );
        let down = grid
            .navigate(
                CompositeNavigationCommand::Down,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(down.current(), Some(DataGridCell::new(2, 10)));
        assert_eq!(
            down.selection().unwrap().selected(),
            &[first, DataGridCell::new(2, 10)]
        );
        assert_eq!(grid.selection().selected(), &[first]);
        let rtl_left = grid
            .navigate(
                CompositeNavigationCommand::Left,
                WritingDirection::RightToLeft,
            )
            .unwrap();
        assert_eq!(rtl_left.current(), Some(DataGridCell::new(2, 20)));
        let end = grid
            .navigate(
                CompositeNavigationCommand::End,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert!(end.boundary());
    }

    #[test]
    fn activation_preserves_source_and_keeps_active_cell_distinct_until_apply() {
        let first = DataGridCell::new(1, 10);
        let target = DataGridCell::new(2, 20);
        let mut grid = grid(
            SelectionMode::Single,
            SelectionFollowsFocus::Disabled,
            [first],
        );
        let intent = grid
            .propose_cell_activation(target, ChangeSource::Accessibility)
            .unwrap();
        assert_eq!(intent.cell(), &target);
        assert_eq!(intent.source(), ChangeSource::Accessibility);
        assert_eq!(grid.active_cell(), Some(target));
        assert_eq!(grid.selection().selected(), &[first]);
        grid.apply_selection(intent.into_selection().unwrap())
            .unwrap();
        assert_eq!(grid.selection().selected(), &[target]);
    }

    #[derive(Clone, Debug)]
    enum Action {
        Activated(DataGridActivation<u8, u8>),
    }

    struct MountedGrid {
        mounted: Rc<RefCell<Option<DataGridRef<u8, u8>>>>,
        activations: RecordedActivations,
    }

    impl Component for MountedGrid {
        type State = DataGrid<u8, u8>;
        type Action = Action;

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
            grid(
                SelectionMode::Single,
                SelectionFollowsFocus::Disabled,
                [DataGridCell::new(1, 10)],
            )
            .density(DensityMetrics::baseline(DensityClass::Touch))
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui.foundation().root(
                BoxStyle::default(),
                crate::ui::LayoutStyle::default(),
                |_| {},
            );
            self.mounted
                .replace(Some(state.mount(ui, root.0, Action::Activated).unwrap()));
            root
        }

        fn action(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            let Action::Activated(intent) = action;
            self.activations
                .borrow_mut()
                .push((*intent.cell(), intent.source()));
        }
    }

    #[test]
    fn mount_has_one_focus_entry_grid_semantics_and_source_preserving_cell_routes() {
        let mounted = Rc::new(RefCell::new(None));
        let activations = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(MountedGrid {
            mounted: mounted.clone(),
            activations: activations.clone(),
        })
        .unwrap();
        let mounted = mounted.borrow();
        let mounted = mounted.as_ref().unwrap();
        let root = runtime.ui().semantics.get(mounted.node()).unwrap();
        assert_eq!(root.role, SemanticRole::Grid);
        assert!(root.actions.contains(SemanticAction::Activate));
        assert!(root
            .relationships
            .iter()
            .any(|relationship| relationship.kind == SemanticRelationshipKind::ActiveDescendant));
        assert!(
            runtime
                .ui()
                .interactions
                .get(mounted.node())
                .unwrap()
                .focusable
        );
        let first = &mounted.table().rows()[0].cells()[0];
        let target = &mounted.table().rows()[1].cells()[1];
        let first_semantics = runtime.ui().semantics.get(first.node()).unwrap();
        assert_eq!(first_semantics.state.selected, Some(true));
        assert!(first_semantics.actions.contains(SemanticAction::Select));
        assert!(
            !runtime
                .ui()
                .interactions
                .get(first.node())
                .is_some_and(|interaction| interaction.focusable)
        );
        assert!(runtime.dispatch_activation(target.node(), ChangeSource::Pointer));
        assert!(runtime.dispatch_action(mounted.node()));
        assert_eq!(
            &*activations.borrow(),
            &[
                (DataGridCell::new(2, 20), ChangeSource::Pointer),
                (DataGridCell::new(2, 20), ChangeSource::Programmatic),
            ]
        );
    }
}
