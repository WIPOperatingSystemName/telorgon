//! Stable-key noninteractive tabular relationships.
//!
//! `Table` owns only validated presentation descriptors and mounted semantics. Selection, focus,
//! editing, sorting, resizing, virtualization, collection data, and platform export remain with
//! their dedicated owners.

use std::fmt;

use crate::foundation::ColorRgba8;
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, Property, SemanticCollection, SemanticName,
    SemanticNode, SemanticRelationship, SemanticRelationshipKind, SemanticRole, SizeRule,
    SizeRule2D, UiNodeId,
};

use crate::components::application::{DensityClass, DensityMetrics};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableColumn<C> {
    key: C,
    label: String,
}

impl<C> TableColumn<C> {
    pub fn new(key: C, label: impl Into<String>) -> Result<Self, TableColumnError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TableColumnError::MissingAccessibleName);
        }
        Ok(Self { key, label })
    }

    pub const fn key(&self) -> &C {
        &self.key
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableColumnError {
    MissingAccessibleName,
}

impl fmt::Display for TableColumnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("table column header accessible name is empty")
    }
}

impl std::error::Error for TableColumnError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableCell<C> {
    column: C,
    text: String,
}

impl<C> TableCell<C> {
    pub fn new(column: C, text: impl Into<String>) -> Self {
        Self {
            column,
            text: text.into(),
        }
    }

    pub const fn column(&self) -> &C {
        &self.column
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRow<R, C> {
    key: R,
    label: String,
    cells: Vec<TableCell<C>>,
}

impl<R, C> TableRow<R, C> {
    pub fn new(
        key: R,
        label: impl Into<String>,
        cells: impl IntoIterator<Item = TableCell<C>>,
    ) -> Result<Self, TableRowError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TableRowError::MissingAccessibleName);
        }
        Ok(Self {
            key,
            label,
            cells: cells.into_iter().collect(),
        })
    }

    pub const fn key(&self) -> &R {
        &self.key
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn cells(&self) -> &[TableCell<C>] {
        &self.cells
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableRowError {
    MissingAccessibleName,
}

impl fmt::Display for TableRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("table row header accessible name is empty")
    }
}

impl std::error::Error for TableRowError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableStyle {
    pub table: BoxStyle,
    pub header_row: BoxStyle,
    pub row: BoxStyle,
    pub corner_header: BoxStyle,
    pub column_header: BoxStyle,
    pub row_header: BoxStyle,
    pub cell: BoxStyle,
    pub row_gap: f32,
    pub column_gap: f32,
    pub header_color: ColorRgba8,
    pub cell_color: ColorRgba8,
    pub header_text_size: f32,
    pub cell_text_size: f32,
}

impl Default for TableStyle {
    fn default() -> Self {
        Self {
            table: BoxStyle::default(),
            header_row: BoxStyle::default(),
            row: BoxStyle::default(),
            corner_header: BoxStyle::default(),
            column_header: BoxStyle::default(),
            row_header: BoxStyle::default(),
            cell: BoxStyle::default(),
            row_gap: 0.0,
            column_gap: 0.0,
            header_color: ColorRgba8::rgba(255, 255, 255, 255),
            cell_color: ColorRgba8::rgba(255, 255, 255, 255),
            header_text_size: 14.0,
            cell_text_size: 14.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table<R, C> {
    label: String,
    columns: Vec<TableColumn<C>>,
    rows: Vec<TableRow<R, C>>,
    density: DensityMetrics,
    style: TableStyle,
}

impl<R, C> Table<R, C>
where
    R: Clone + Eq,
    C: Clone + Eq,
{
    pub fn new(
        label: impl Into<String>,
        columns: impl IntoIterator<Item = TableColumn<C>>,
        rows: impl IntoIterator<Item = TableRow<R, C>>,
    ) -> Result<Self, TableError<R, C>> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TableError::MissingAccessibleName);
        }
        let columns: Vec<_> = columns.into_iter().collect();
        let rows: Vec<_> = rows.into_iter().collect();
        validate_table(&columns, &rows)?;
        Ok(Self {
            label,
            columns,
            rows,
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: TableStyle::default(),
        })
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: TableStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn columns(&self) -> &[TableColumn<C>] {
        &self.columns
    }

    pub fn rows(&self) -> &[TableRow<R, C>] {
        &self.rows
    }

    pub fn column(&self, key: &C) -> Option<&TableColumn<C>> {
        self.columns.iter().find(|column| column.key() == key)
    }

    pub fn row(&self, key: &R) -> Option<&TableRow<R, C>> {
        self.rows.iter().find(|row| row.key() == key)
    }

    pub fn cell(&self, row: &R, column: &C) -> Option<&TableCell<C>> {
        self.row(row)?
            .cells
            .iter()
            .find(|cell| cell.column() == column)
    }

    pub fn mount<Action>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<TableRef<R, C>>
    where
        R: 'static,
        C: 'static,
        Action: 'static,
    {
        let row_count = u32::try_from(self.rows.len())
            .map_err(|_| RuntimeError::new("table exceeds semantic row capacity"))?;
        let column_count = u32::try_from(self.columns.len())
            .map_err(|_| RuntimeError::new("table exceeds semantic column capacity"))?;
        let minimum = self.density.effective_minimum();
        let mut header_row = None;
        let mut mounted_columns = Vec::with_capacity(self.columns.len());
        let mut mounted_rows = Vec::with_capacity(self.rows.len());
        let root = ui
            .foundation()
            .container_node_under(
                host,
                self.style.table,
                LayoutStyle {
                    flow: Flow::Vertical,
                    gap: self.style.row_gap,
                    ..LayoutStyle::default()
                },
                |writer| {
                    let header = writer.layer(
                        true,
                        self.style.header_row,
                        LayoutStyle {
                            flow: Flow::Horizontal,
                            gap: self.style.column_gap,
                            ..LayoutStyle::default()
                        },
                        |writer| {
                            writer.layer(
                                true,
                                density_style(self.style.corner_header, minimum),
                                LayoutStyle::default(),
                                |_| {},
                            );
                            for (index, column) in self.columns.iter().enumerate() {
                                let control = writer.layer(
                                    true,
                                    density_style(self.style.column_header, minimum),
                                    LayoutStyle::default(),
                                    |writer| {
                                        writer.text(
                                            column.label(),
                                            self.style.header_color,
                                            self.style.header_text_size,
                                        );
                                    },
                                );
                                mounted_columns.push((index, column.clone(), control));
                            }
                        },
                    );
                    header_row = Some(header);
                    for (row_index, row) in self.rows.iter().enumerate() {
                        let mut row_header = None;
                        let mut cells = Vec::with_capacity(row.cells.len());
                        let control = writer.layer(
                            true,
                            self.style.row,
                            LayoutStyle {
                                flow: Flow::Horizontal,
                                gap: self.style.column_gap,
                                ..LayoutStyle::default()
                            },
                            |writer| {
                                row_header = Some(writer.layer(
                                    true,
                                    density_style(self.style.row_header, minimum),
                                    LayoutStyle::default(),
                                    |writer| {
                                        writer.text(
                                            row.label(),
                                            self.style.header_color,
                                            self.style.header_text_size,
                                        );
                                    },
                                ));
                                for (column_index, cell) in row.cells.iter().enumerate() {
                                    let cell_control = writer.layer(
                                        true,
                                        density_style(self.style.cell, minimum),
                                        LayoutStyle::default(),
                                        |writer| {
                                            writer.text(
                                                cell.text(),
                                                self.style.cell_color,
                                                self.style.cell_text_size,
                                            );
                                        },
                                    );
                                    cells.push((column_index, cell.clone(), cell_control));
                                }
                            },
                        );
                        mounted_rows.push((
                            row_index,
                            row.clone(),
                            control,
                            row_header.expect("row header mounts with its row"),
                            cells,
                        ));
                    }
                },
            )
            .ok_or_else(|| RuntimeError::new("application table host is stale"))?;
        let header_row = header_row.expect("table header row mounts with the table");

        let mut column_refs = Vec::with_capacity(mounted_columns.len());
        for (index, column, control) in mounted_columns {
            let name = ui.foundation().intern(column.label());
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        role: SemanticRole::ColumnHeader,
                        name: SemanticName::Text(name),
                        collection: Some(collection_position(index, column_count)),
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid table column semantics: {error:?}"))
                })?;
            column_refs.push(TableColumnRef {
                key: column.key,
                control,
                index,
            });
        }
        ui.foundation()
            .semantic_node(
                header_row.node,
                SemanticNode {
                    role: SemanticRole::Row,
                    relationships: column_refs
                        .iter()
                        .map(|column| owns(column.control.node))
                        .collect(),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid table header-row semantics: {error:?}"))
            })?;

        let mut row_refs = Vec::with_capacity(mounted_rows.len());
        for (row_index, row, row_control, row_header, cells) in mounted_rows {
            let row_name = ui.foundation().intern(row.label());
            ui.foundation()
                .semantic_node(
                    row_header.node,
                    SemanticNode {
                        role: SemanticRole::RowHeader,
                        name: SemanticName::Text(row_name),
                        collection: Some(collection_position(row_index, row_count)),
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid table row-header semantics: {error:?}"))
                })?;
            let mut cell_refs = Vec::with_capacity(cells.len());
            for (column_index, cell, control) in cells {
                let column_header = column_refs[column_index].control.node;
                let name = if cell.text().trim().is_empty() {
                    SemanticName::Unspecified
                } else {
                    SemanticName::Text(ui.foundation().intern(cell.text()))
                };
                ui.foundation()
                    .semantic_node(
                        control.node,
                        SemanticNode {
                            role: SemanticRole::Cell,
                            name,
                            relationships: vec![
                                SemanticRelationship {
                                    kind: SemanticRelationshipKind::LabelledBy,
                                    target: row_header.node,
                                },
                                SemanticRelationship {
                                    kind: SemanticRelationshipKind::LabelledBy,
                                    target: column_header,
                                },
                            ],
                            collection: Some(collection_position(column_index, column_count)),
                            ..SemanticNode::default()
                        },
                    )
                    .map_err(|error| {
                        RuntimeError::new(format!("invalid table cell semantics: {error:?}"))
                    })?;
                cell_refs.push(TableCellRef {
                    row: row.key.clone(),
                    column: cell.column,
                    control,
                    column_index,
                });
            }
            let mut relationships = Vec::with_capacity(cell_refs.len() + 1);
            relationships.push(owns(row_header.node));
            relationships.extend(cell_refs.iter().map(|cell| owns(cell.control.node)));
            ui.foundation()
                .semantic_node(
                    row_control.node,
                    SemanticNode {
                        role: SemanticRole::Row,
                        relationships,
                        collection: Some(collection_position(row_index, row_count)),
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid table row semantics: {error:?}"))
                })?;
            row_refs.push(TableRowRef {
                key: row.key,
                control: row_control,
                header: row_header,
                index: row_index,
                cells: cell_refs,
            });
        }

        let name = ui.foundation().intern(&self.label);
        let mut relationships = Vec::with_capacity(row_refs.len() + 1);
        relationships.push(owns(header_row.node));
        relationships.extend(row_refs.iter().map(|row| owns(row.control.node)));
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Table,
                    name: SemanticName::Text(name),
                    relationships,
                    collection: Some(SemanticCollection {
                        item_count: Some(row_count),
                        set_size: (row_count > 0).then_some(row_count),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| RuntimeError::new(format!("invalid table semantics: {error:?}")))?;

        Ok(TableRef {
            root,
            header_row,
            columns: column_refs,
            rows: row_refs,
        })
    }
}

#[derive(Clone, Debug)]
pub struct TableRef<R, C> {
    root: ControlHandle,
    header_row: ControlHandle,
    columns: Vec<TableColumnRef<C>>,
    rows: Vec<TableRowRef<R, C>>,
}

impl<R, C> TableRef<R, C> {
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub const fn header_row_node(&self) -> UiNodeId {
        self.header_row.node
    }

    pub fn columns(&self) -> &[TableColumnRef<C>] {
        &self.columns
    }

    pub fn rows(&self) -> &[TableRowRef<R, C>] {
        &self.rows
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Debug)]
pub struct TableColumnRef<C> {
    key: C,
    control: ControlHandle,
    index: usize,
}

impl<C> TableColumnRef<C> {
    pub const fn key(&self) -> &C {
        &self.key
    }

    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn index(&self) -> usize {
        self.index
    }
}

#[derive(Clone, Debug)]
pub struct TableRowRef<R, C> {
    key: R,
    control: ControlHandle,
    header: ControlHandle,
    index: usize,
    cells: Vec<TableCellRef<R, C>>,
}

impl<R, C> TableRowRef<R, C> {
    pub const fn key(&self) -> &R {
        &self.key
    }

    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn header_node(&self) -> UiNodeId {
        self.header.node
    }

    pub const fn index(&self) -> usize {
        self.index
    }

    pub fn cells(&self) -> &[TableCellRef<R, C>] {
        &self.cells
    }
}

#[derive(Clone, Debug)]
pub struct TableCellRef<R, C> {
    row: R,
    column: C,
    control: ControlHandle,
    column_index: usize,
}

impl<R, C> TableCellRef<R, C> {
    pub const fn row(&self) -> &R {
        &self.row
    }

    pub const fn column(&self) -> &C {
        &self.column
    }

    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn column_index(&self) -> usize {
        self.column_index
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableError<R, C> {
    MissingAccessibleName,
    MissingColumns,
    DuplicateColumnKey(C),
    DuplicateRowKey(R),
    CellCountMismatch {
        row: R,
        expected: usize,
        actual: usize,
    },
    CellColumnMismatch {
        row: R,
        index: usize,
        expected: C,
        actual: C,
    },
}

impl<R: fmt::Debug, C: fmt::Debug> fmt::Display for TableError<R, C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "table operation failed: {self:?}")
    }
}

impl<R: fmt::Debug, C: fmt::Debug> std::error::Error for TableError<R, C> {}

fn validate_table<R, C>(
    columns: &[TableColumn<C>],
    rows: &[TableRow<R, C>],
) -> Result<(), TableError<R, C>>
where
    R: Clone + Eq,
    C: Clone + Eq,
{
    if columns.is_empty() {
        return Err(TableError::MissingColumns);
    }
    for (index, column) in columns.iter().enumerate() {
        if columns[..index].iter().any(|other| other.key == column.key) {
            return Err(TableError::DuplicateColumnKey(column.key.clone()));
        }
    }
    for (row_index, row) in rows.iter().enumerate() {
        if rows[..row_index].iter().any(|other| other.key == row.key) {
            return Err(TableError::DuplicateRowKey(row.key.clone()));
        }
        if row.cells.len() != columns.len() {
            return Err(TableError::CellCountMismatch {
                row: row.key.clone(),
                expected: columns.len(),
                actual: row.cells.len(),
            });
        }
        for (index, (cell, column)) in row.cells.iter().zip(columns).enumerate() {
            if cell.column != column.key {
                return Err(TableError::CellColumnMismatch {
                    row: row.key.clone(),
                    index,
                    expected: column.key.clone(),
                    actual: cell.column.clone(),
                });
            }
        }
    }
    Ok(())
}

fn density_style(
    mut style: BoxStyle,
    minimum: crate::components::application::InteractiveTargetSize,
) -> BoxStyle {
    style.min_size = SizeRule2D {
        width: SizeRule::Logical(minimum.width()),
        height: SizeRule::Logical(minimum.height()),
    };
    style
}

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
    use crate::ui::UiRoot;

    use super::*;

    fn columns() -> [TableColumn<u8>; 2] {
        [
            TableColumn::new(10, "Name").unwrap(),
            TableColumn::new(20, "Status").unwrap(),
        ]
    }

    fn row(key: u8, label: &str, name: &str, status: &str) -> TableRow<u8, u8> {
        TableRow::new(
            key,
            label,
            [TableCell::new(10, name), TableCell::new(20, status)],
        )
        .unwrap()
    }

    #[test]
    fn construction_requires_names_unique_keys_columns_and_rectangular_relationships() {
        assert_eq!(
            TableColumn::new(1_u8, " "),
            Err(TableColumnError::MissingAccessibleName)
        );
        assert_eq!(
            TableRow::<u8, u8>::new(1, " ", []),
            Err(TableRowError::MissingAccessibleName)
        );
        assert_eq!(
            Table::<u8, u8>::new("Empty", [], []),
            Err(TableError::MissingColumns)
        );
        assert!(matches!(
            Table::new(
                "Broken",
                columns(),
                [TableRow::new(1, "First", [TableCell::new(20, "wrong")]).unwrap()]
            ),
            Err(TableError::CellCountMismatch { .. })
        ));
        assert!(matches!(
            Table::new(
                "Broken",
                columns(),
                [TableRow::new(
                    1,
                    "First",
                    [TableCell::new(20, "wrong"), TableCell::new(10, "order")]
                )
                .unwrap()]
            ),
            Err(TableError::CellColumnMismatch { index: 0, .. })
        ));
    }

    #[test]
    fn stable_row_column_pairs_address_presentation_cells() {
        let table = Table::new(
            "Services",
            columns(),
            [
                row(1, "API", "Gateway", "Ready"),
                row(2, "Jobs", "Worker", "Paused"),
            ],
        )
        .unwrap();
        assert_eq!(table.column(&20).unwrap().label(), "Status");
        assert_eq!(table.row(&2).unwrap().label(), "Jobs");
        assert_eq!(table.cell(&2, &20).unwrap().text(), "Paused");
        assert!(table.cell(&9, &20).is_none());
    }

    struct MountedTable {
        mounted: Rc<RefCell<Option<TableRef<u8, u8>>>>,
    }

    impl Component for MountedTable {
        type State = Table<u8, u8>;
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
            Table::new(
                "Services",
                columns(),
                [
                    row(1, "API", "Gateway", "Ready"),
                    row(2, "Jobs", "Worker", ""),
                ],
            )
            .unwrap()
            .density(DensityMetrics::baseline(DensityClass::Touch))
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.mounted.replace(Some(state.mount(ui, root.0).unwrap()));
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
    fn mount_emits_ordered_header_relationships_and_no_interactive_nodes() {
        let mounted = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(MountedTable {
            mounted: mounted.clone(),
        })
        .unwrap();
        let mounted = mounted.borrow();
        let mounted = mounted.as_ref().unwrap();
        let root = runtime.ui().semantics.get(mounted.node()).unwrap();
        assert_eq!(root.role, SemanticRole::Table);
        assert_eq!(root.collection.unwrap().item_count, Some(2));
        assert_eq!(root.relationships.len(), 3);
        assert_eq!(
            runtime
                .ui()
                .semantics
                .get(mounted.header_row_node())
                .unwrap()
                .role,
            SemanticRole::Row
        );
        assert_eq!(mounted.columns().len(), 2);
        assert_eq!(mounted.rows().len(), 2);
        let first_row = &mounted.rows()[0];
        assert_eq!(
            runtime
                .ui()
                .semantics
                .get(first_row.header_node())
                .unwrap()
                .role,
            SemanticRole::RowHeader
        );
        let first_cell = &first_row.cells()[0];
        let cell_semantics = runtime.ui().semantics.get(first_cell.node()).unwrap();
        assert_eq!(cell_semantics.role, SemanticRole::Cell);
        assert_eq!(cell_semantics.relationships.len(), 2);
        assert!(
            cell_semantics
                .relationships
                .iter()
                .all(|relationship| relationship.kind == SemanticRelationshipKind::LabelledBy)
        );
        assert_eq!(first_cell.row(), &1);
        assert_eq!(first_cell.column(), &10);
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(first_cell.node())
                .unwrap()
                .min_size,
            SizeRule2D {
                width: SizeRule::Logical(44.0),
                height: SizeRule::Logical(44.0),
            }
        );
        for node in mounted.columns().iter().map(TableColumnRef::node).chain(
            mounted.rows().iter().flat_map(|row| {
                std::iter::once(row.header_node()).chain(row.cells().iter().map(TableCellRef::node))
            }),
        ) {
            assert!(
                !runtime
                    .ui()
                    .interactions
                    .get(node)
                    .is_some_and(|interaction| interaction.focusable)
            );
        }
    }

    #[test]
    fn empty_body_keeps_named_column_headers_and_known_zero_rows() {
        struct Empty;
        impl Component for Empty {
            type State = Table<u8, u8>;
            type Action = ();

            fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
                Table::new("Empty services", columns(), []).unwrap()
            }

            fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
                let root =
                    ui.foundation()
                        .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
                state.mount(ui, root.0).unwrap();
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
        let table = runtime
            .ui()
            .semantics
            .iter()
            .find_map(|(_, semantic)| (semantic.role == SemanticRole::Table).then_some(semantic))
            .unwrap();
        assert_eq!(table.collection.unwrap().item_count, Some(0));
        assert_eq!(table.relationships.len(), 1);
    }
}
