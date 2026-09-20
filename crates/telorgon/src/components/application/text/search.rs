//! Search-field composition over the basic single-line field owner.

use std::fmt;

use crate::runtime::{MonotonicInstant, RuntimeResult, Ui};
use crate::ui::text::{
    TextAffinity, TextEdit, TextEditBatch, TextInputConfiguration, TextInputPurpose, TextOffset,
    TextRange, TextReturnKeyAction, TextRevision, TextSelection,
};
use crate::ui::{SemanticNode, SemanticRole, StringId, UiNodeId};

use crate::components::application::DensityClass;

use super::{
    EditHistoryCommand, EditHistoryKind, Submitted, TextController, TextControllerUpdate,
    TextField, TextFieldCommand, TextFieldCommandAvailability, TextFieldError, TextFieldMode,
    TextFieldOutput, TextFieldRef, TextFieldStyle,
};

/// One renderer- and platform-neutral request routed through a search field's controller.
pub enum SearchFieldCommand {
    Edit {
        batch: TextEditBatch,
        kind: EditHistoryKind,
        recorded_at: MonotonicInstant,
    },
    SetSelection {
        base_revision: TextRevision,
        selection: TextSelection,
    },
    Clear {
        base_revision: TextRevision,
        recorded_at: MonotonicInstant,
    },
    Submit,
    History(EditHistoryCommand),
}

impl fmt::Debug for SearchFieldCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Edit {
                batch,
                kind,
                recorded_at,
            } => formatter
                .debug_struct("Edit")
                .field("base_revision", &batch.base_revision)
                .field("edit_count", &batch.edits.len())
                .field("kind", kind)
                .field("recorded_at", recorded_at)
                .finish_non_exhaustive(),
            Self::SetSelection {
                base_revision,
                selection,
            } => formatter
                .debug_struct("SetSelection")
                .field("base_revision", base_revision)
                .field("selection", selection)
                .finish(),
            Self::Clear {
                base_revision,
                recorded_at,
            } => formatter
                .debug_struct("Clear")
                .field("base_revision", base_revision)
                .field("recorded_at", recorded_at)
                .finish(),
            Self::Submit => formatter.write_str("Submit"),
            Self::History(command) => formatter.debug_tuple("History").field(command).finish(),
        }
    }
}

/// Typed result distinguishing ordinary controller updates, clear, and submission.
#[derive(Clone, Debug)]
pub enum SearchFieldOutput {
    Updated(TextControllerUpdate),
    Cleared(TextControllerUpdate),
    Submitted(Submitted),
}

/// Mode-, content-, and history-derived search command availability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchFieldCommandAvailability {
    pub can_edit: bool,
    pub can_select: bool,
    pub can_clear: bool,
    pub can_submit: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

impl From<TextFieldCommandAvailability> for SearchFieldCommandAvailability {
    fn from(field: TextFieldCommandAvailability) -> Self {
        Self {
            can_edit: field.can_edit,
            can_select: field.can_select,
            can_clear: false,
            can_submit: field.can_submit,
            can_undo: field.can_undo,
            can_redo: field.can_redo,
        }
    }
}

/// Search-specific clear and submit behavior composed over one basic text field.
pub struct SearchField {
    field: TextField,
}

impl fmt::Debug for SearchField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchField")
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl SearchField {
    pub fn new(
        controller: TextController,
        label: impl Into<String>,
        mode: TextFieldMode,
    ) -> Result<Self, SearchFieldError> {
        Ok(Self {
            field: TextField::new(controller, label, mode)
                .map_err(SearchFieldError::Field)?
                .return_action(TextReturnKeyAction::Search)
                .map_err(SearchFieldError::Field)?,
        })
    }

    pub fn required(mut self, required: bool) -> Self {
        self.field = self.field.required(required);
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.field = self.field.invalid(invalid);
        self
    }

    pub fn density(mut self, density: DensityClass) -> Self {
        self.field = self.field.density(density);
        self
    }

    pub fn style(mut self, style: TextFieldStyle) -> Self {
        self.field = self.field.style(style);
        self
    }

    pub const fn controller(&self) -> &TextController {
        self.field.controller()
    }

    pub fn into_controller(self) -> TextController {
        self.field.into_controller()
    }

    pub const fn mode(&self) -> TextFieldMode {
        self.field.mode()
    }

    pub fn input_configuration(&self) -> TextInputConfiguration {
        let mut configuration = self.field.input_configuration();
        configuration.purpose = TextInputPurpose::Search;
        configuration
    }

    pub fn command_availability(&self) -> SearchFieldCommandAvailability {
        let mut availability =
            SearchFieldCommandAvailability::from(self.field.command_availability());
        availability.can_clear =
            availability.can_edit && !self.field.controller().snapshot().is_empty();
        availability
    }

    pub fn route(
        &mut self,
        command: SearchFieldCommand,
    ) -> Result<SearchFieldOutput, SearchFieldError> {
        match command {
            SearchFieldCommand::Edit {
                batch,
                kind,
                recorded_at,
            } => self.route_field(TextFieldCommand::Edit {
                batch,
                kind,
                recorded_at,
            }),
            SearchFieldCommand::SetSelection {
                base_revision,
                selection,
            } => self.route_field(TextFieldCommand::SetSelection {
                base_revision,
                selection,
            }),
            SearchFieldCommand::Submit => self.route_field(TextFieldCommand::Submit),
            SearchFieldCommand::History(command) => {
                self.route_field(TextFieldCommand::History(command))
            }
            SearchFieldCommand::Clear {
                base_revision,
                recorded_at,
            } => self.clear(base_revision, recorded_at),
        }
    }

    pub fn semantic_node(&self, name: StringId, value: Option<StringId>) -> SemanticNode {
        self.field
            .semantic_node_internal(name, value, false, SemanticRole::SearchBox)
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<SearchFieldRef> {
        Ok(SearchFieldRef {
            field: self
                .field
                .mount_internal(ui, host, false, 1, SemanticRole::SearchBox)?,
            availability: self.command_availability(),
        })
    }

    fn route_field(
        &mut self,
        command: TextFieldCommand,
    ) -> Result<SearchFieldOutput, SearchFieldError> {
        match self.field.route(command).map_err(SearchFieldError::Field)? {
            TextFieldOutput::Updated(update) => Ok(SearchFieldOutput::Updated(update)),
            TextFieldOutput::Submitted(submitted) => Ok(SearchFieldOutput::Submitted(submitted)),
        }
    }

    fn clear(
        &mut self,
        base_revision: TextRevision,
        recorded_at: MonotonicInstant,
    ) -> Result<SearchFieldOutput, SearchFieldError> {
        self.field
            .ensure_editable()
            .map_err(SearchFieldError::Field)?;
        let snapshot = self.field.controller().snapshot();
        if base_revision != snapshot.revision() {
            return Err(SearchFieldError::StaleClear {
                base: base_revision,
                current: snapshot.revision(),
            });
        }
        if snapshot.is_empty() {
            return Err(SearchFieldError::AlreadyEmpty);
        }
        let batch = TextEditBatch {
            base_revision,
            edits: vec![TextEdit {
                range: TextRange::new(TextOffset::ZERO, snapshot.end())
                    .expect("ordered snapshot extent"),
                replacement: String::new(),
            }],
            selection: TextSelection::collapsed(TextOffset::ZERO, TextAffinity::Downstream),
            composition: None,
        };
        let output = self
            .field
            .route(TextFieldCommand::Edit {
                batch,
                kind: EditHistoryKind::Replacement,
                recorded_at,
            })
            .map_err(SearchFieldError::Field)?;
        let TextFieldOutput::Updated(update) = output else {
            unreachable!("a clear edit cannot produce submission")
        };
        Ok(SearchFieldOutput::Cleared(update))
    }
}

/// Focused mount-time reference; editing remains owned by [`SearchField`] and its controller.
#[derive(Clone, Copy, Debug)]
pub struct SearchFieldRef {
    field: TextFieldRef,
    availability: SearchFieldCommandAvailability,
}

impl SearchFieldRef {
    pub const fn field(self) -> TextFieldRef {
        self.field
    }

    pub const fn node(self) -> UiNodeId {
        self.field.node()
    }

    pub const fn revision(self) -> TextRevision {
        self.field.revision()
    }

    pub const fn mode(self) -> TextFieldMode {
        self.field.mode()
    }

    pub const fn availability(self) -> SearchFieldCommandAvailability {
        self.availability
    }
}

#[derive(Debug)]
pub enum SearchFieldError {
    AlreadyEmpty,
    StaleClear {
        base: TextRevision,
        current: TextRevision,
    },
    Field(TextFieldError),
}

impl fmt::Display for SearchFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyEmpty => formatter.write_str("search field is already empty"),
            Self::StaleClear { base, current } => write!(
                formatter,
                "search clear cites stale revision {base:?}; current revision is {current:?}"
            ),
            Self::Field(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SearchFieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Field(error) => Some(error),
            Self::AlreadyEmpty | Self::StaleClear { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::text::{TextEdit, TextMultiline};
    use crate::ui::{BoxStyle, LayoutStyle, NodeKind, SemanticValue, SizeRule, UiRoot};

    use super::*;
    use crate::components::application::EditHistoryPolicy;

    fn selection(offset: u32) -> TextSelection {
        TextSelection::collapsed(TextOffset(offset), TextAffinity::Downstream)
    }

    fn replacement(revision: TextRevision, end: u32, text: &str) -> TextEditBatch {
        TextEditBatch {
            base_revision: revision,
            edits: vec![TextEdit {
                range: TextRange::new(TextOffset::ZERO, TextOffset(end)).unwrap(),
                replacement: text.to_owned(),
            }],
            selection: selection(u32::try_from(text.len()).unwrap()),
            composition: None,
        }
    }

    #[test]
    fn configuration_and_semantics_are_search_specific() {
        let empty =
            SearchField::new(TextController::new(), "Search", TextFieldMode::Editable).unwrap();
        let configuration = empty.input_configuration();
        assert_eq!(configuration.purpose, TextInputPurpose::Search);
        assert_eq!(configuration.multiline, TextMultiline::SingleLine);
        assert_eq!(configuration.return_key, TextReturnKeyAction::Search);
        assert!(!empty.command_availability().can_clear);
        assert!(empty.command_availability().can_submit);
        assert_eq!(
            empty.semantic_node(StringId(1), Some(StringId(2))).role,
            SemanticRole::SearchBox
        );
    }

    #[test]
    fn clear_is_revision_checked_typed_and_undoable() {
        let mut controller = TextController::from_text("query").unwrap();
        controller
            .enable_edit_history(EditHistoryPolicy::default())
            .unwrap();
        let mut search = SearchField::new(controller, "Search", TextFieldMode::Editable).unwrap();
        assert!(search.command_availability().can_clear);
        let output = search
            .route(SearchFieldCommand::Clear {
                base_revision: TextRevision::INITIAL,
                recorded_at: MonotonicInstant::from_nanos(1),
            })
            .unwrap();
        let SearchFieldOutput::Cleared(update) = output else {
            panic!("clear must remain distinguishable from an ordinary edit")
        };
        assert!(update.changed_text());
        assert!(update.snapshot.is_empty());
        assert!(!search.command_availability().can_clear);
        assert!(search.command_availability().can_undo);
        let undone = search
            .route(SearchFieldCommand::History(EditHistoryCommand::Undo))
            .unwrap();
        let SearchFieldOutput::Updated(undone) = undone else {
            panic!("undo must publish an ordinary controller update")
        };
        assert_eq!(
            undone
                .snapshot
                .chunks()
                .map(|chunk| chunk.text)
                .collect::<String>(),
            "query"
        );
    }

    #[test]
    fn invalid_clear_and_single_line_edit_reject_without_mutation() {
        let mut search = SearchField::new(
            TextController::from_text("query").unwrap(),
            "Search",
            TextFieldMode::Editable,
        )
        .unwrap();
        assert!(matches!(
            search.route(SearchFieldCommand::Clear {
                base_revision: TextRevision(5),
                recorded_at: MonotonicInstant::from_nanos(1),
            }),
            Err(SearchFieldError::StaleClear { .. })
        ));
        assert!(matches!(
            search.route(SearchFieldCommand::Edit {
                batch: replacement(TextRevision::INITIAL, 5, "query\nmore"),
                kind: EditHistoryKind::Typing,
                recorded_at: MonotonicInstant::from_nanos(1),
            }),
            Err(SearchFieldError::Field(TextFieldError::MultilineEdit))
        ));
        assert_eq!(search.controller().revision(), TextRevision::INITIAL);

        let mut empty =
            SearchField::new(TextController::new(), "Search", TextFieldMode::Editable).unwrap();
        assert!(matches!(
            empty.route(SearchFieldCommand::Clear {
                base_revision: TextRevision::INITIAL,
                recorded_at: MonotonicInstant::from_nanos(1),
            }),
            Err(SearchFieldError::AlreadyEmpty)
        ));
        assert_eq!(empty.controller().revision(), TextRevision::INITIAL);
    }

    #[test]
    fn submit_is_search_typed_and_does_not_mutate_text() {
        let mut search = SearchField::new(
            TextController::from_text("query").unwrap(),
            "Search",
            TextFieldMode::Editable,
        )
        .unwrap();
        let revision = search.controller().revision();
        let output = search.route(SearchFieldCommand::Submit).unwrap();
        let SearchFieldOutput::Submitted(submitted) = output else {
            panic!("search submit must publish a submission")
        };
        assert_eq!(submitted.action, TextReturnKeyAction::Search);
        assert_eq!(submitted.revision, revision);
        assert_eq!(search.controller().revision(), revision);
        assert!(!format!("{search:?}").contains("query"));
        assert!(
            !format!(
                "{:?}",
                SearchFieldCommand::Edit {
                    batch: replacement(revision, 5, "private query"),
                    kind: EditHistoryKind::Typing,
                    recorded_at: MonotonicInstant::from_nanos(1),
                }
            )
            .contains("private query")
        );
    }

    struct MountedSearch {
        search: SearchField,
        reference: Rc<Cell<Option<SearchFieldRef>>>,
    }

    impl Component for MountedSearch {
        type State = ();
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.reference
                .set(Some(self.search.mount(ui, root.0).unwrap()));
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
    fn mounted_search_has_searchbox_semantics_touch_floor_and_secure_redaction() {
        let reference = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(MountedSearch {
            search: SearchField::new(
                TextController::from_text("secret").unwrap(),
                "Search",
                TextFieldMode::Secure,
            )
            .unwrap()
            .density(DensityClass::Touch),
            reference: reference.clone(),
        })
        .unwrap();
        let reference = reference.get().unwrap();
        assert_eq!(
            runtime.ui().kinds.get(reference.node()),
            Some(&NodeKind::TextInput)
        );
        let semantic = runtime.ui().semantics.get(reference.node()).unwrap();
        assert_eq!(semantic.role, SemanticRole::SearchBox);
        assert_eq!(semantic.value, SemanticValue::None);
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(reference.node())
                .unwrap()
                .min_size
                .height,
            SizeRule::Logical(44.0)
        );
        assert!(reference.availability().can_clear);
        assert!(!format!("{:?}", runtime.ui()).contains("secret"));
    }
}
