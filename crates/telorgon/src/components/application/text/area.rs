//! Multiline application text area composed over the basic field owner.

use std::fmt;

use crate::runtime::{MonotonicInstant, RuntimeResult, Ui};
use crate::ui::text::{
    TextEditBatch, TextInputConfiguration, TextMultiline, TextReturnKeyAction, TextRevision,
    TextSelection,
};
use crate::ui::{SemanticNode, SemanticRole, StringId, UiNodeId};

use crate::components::application::DensityClass;

use super::{
    EditHistoryCommand, EditHistoryKind, TextController, TextField, TextFieldCommand,
    TextFieldCommandAvailability, TextFieldError, TextFieldMode, TextFieldOutput, TextFieldRef,
    TextFieldStyle,
};

/// What pressing the platform return key requests from a text area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAreaReturnPolicy {
    /// Apply the supplied revision-checked newline edit.
    #[default]
    InsertNewline,
    /// Submit without applying the supplied newline edit.
    Submit(TextReturnKeyAction),
}

/// One renderer- and platform-neutral request routed through a text area's controller.
pub enum TextAreaCommand {
    Edit {
        batch: TextEditBatch,
        kind: EditHistoryKind,
        recorded_at: MonotonicInstant,
    },
    SetSelection {
        base_revision: TextRevision,
        selection: TextSelection,
    },
    Return {
        newline: TextEditBatch,
        recorded_at: MonotonicInstant,
    },
    History(EditHistoryCommand),
}

impl fmt::Debug for TextAreaCommand {
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
            Self::Return {
                newline,
                recorded_at,
            } => formatter
                .debug_struct("Return")
                .field("base_revision", &newline.base_revision)
                .field("edit_count", &newline.edits.len())
                .field("recorded_at", recorded_at)
                .finish_non_exhaustive(),
            Self::History(command) => formatter.debug_tuple("History").field(command).finish(),
        }
    }
}

/// Text areas publish the same controller update or submission values as text fields.
pub type TextAreaOutput = TextFieldOutput;

/// One multiline field companion retaining the basic field as the sole shared-policy owner.
pub struct TextArea {
    field: TextField,
    return_policy: TextAreaReturnPolicy,
    minimum_lines: u32,
}

impl fmt::Debug for TextArea {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextArea")
            .field("field", &self.field)
            .field("return_policy", &self.return_policy)
            .field("minimum_lines", &self.minimum_lines)
            .finish_non_exhaustive()
    }
}

impl TextArea {
    pub fn new(
        controller: TextController,
        label: impl Into<String>,
        mode: TextFieldMode,
    ) -> Result<Self, TextAreaError> {
        Ok(Self {
            field: TextField::new(controller, label, mode).map_err(TextAreaError::Field)?,
            return_policy: TextAreaReturnPolicy::InsertNewline,
            minimum_lines: 3,
        })
    }

    pub fn return_policy(mut self, policy: TextAreaReturnPolicy) -> Result<Self, TextAreaError> {
        if let TextAreaReturnPolicy::Submit(action) = policy {
            if action == TextReturnKeyAction::Newline {
                return Err(TextAreaError::NewlineSubmitAction);
            }
            self.field = self
                .field
                .return_action(action)
                .map_err(TextAreaError::Field)?;
        }
        self.return_policy = policy;
        Ok(self)
    }

    pub fn minimum_lines(mut self, minimum_lines: u32) -> Result<Self, TextAreaError> {
        if minimum_lines < 2 {
            return Err(TextAreaError::InvalidMinimumLines);
        }
        self.minimum_lines = minimum_lines;
        Ok(self)
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

    pub const fn configured_return_policy(&self) -> TextAreaReturnPolicy {
        self.return_policy
    }

    pub const fn visible_line_floor(&self) -> u32 {
        self.minimum_lines
    }

    pub fn input_configuration(&self) -> TextInputConfiguration {
        let mut configuration = self.field.input_configuration();
        configuration.multiline = TextMultiline::MultiLine;
        configuration.return_key = match self.return_policy {
            TextAreaReturnPolicy::InsertNewline => TextReturnKeyAction::Newline,
            TextAreaReturnPolicy::Submit(action) => action,
        };
        configuration
    }

    pub fn command_availability(&self) -> TextFieldCommandAvailability {
        let mut availability = self.field.command_availability();
        availability.can_submit = availability.can_submit
            && matches!(self.return_policy, TextAreaReturnPolicy::Submit(_));
        availability
    }

    pub fn route(&mut self, command: TextAreaCommand) -> Result<TextAreaOutput, TextAreaError> {
        let command = match command {
            TextAreaCommand::Edit {
                batch,
                kind,
                recorded_at,
            } => TextFieldCommand::Edit {
                batch,
                kind,
                recorded_at,
            },
            TextAreaCommand::SetSelection {
                base_revision,
                selection,
            } => TextFieldCommand::SetSelection {
                base_revision,
                selection,
            },
            TextAreaCommand::History(command) => TextFieldCommand::History(command),
            TextAreaCommand::Return {
                newline,
                recorded_at,
            } => match self.return_policy {
                TextAreaReturnPolicy::InsertNewline => {
                    if !newline.edits.iter().any(|edit| {
                        edit.replacement
                            .chars()
                            .any(|character| matches!(character, '\n' | '\r'))
                    }) {
                        return Err(TextAreaError::MissingNewlineEdit);
                    }
                    TextFieldCommand::Edit {
                        batch: newline,
                        kind: EditHistoryKind::Typing,
                        recorded_at,
                    }
                }
                TextAreaReturnPolicy::Submit(_) => TextFieldCommand::Submit,
            },
        };
        self.field
            .route_internal(command, true)
            .map_err(TextAreaError::Field)
    }

    pub fn semantic_node(&self, name: StringId, value: Option<StringId>) -> SemanticNode {
        self.field
            .semantic_node_internal(name, value, true, SemanticRole::TextInput)
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<TextAreaRef> {
        Ok(TextAreaRef {
            field: self
                .field
                .mount_internal(ui, host, true, self.minimum_lines, SemanticRole::TextInput)?
                .with_availability(self.command_availability()),
            availability: self.command_availability(),
            return_policy: self.return_policy,
            minimum_lines: self.minimum_lines,
        })
    }
}

/// Focused mount-time reference; editing remains owned by [`TextArea`] and its controller.
#[derive(Clone, Copy, Debug)]
pub struct TextAreaRef {
    field: TextFieldRef,
    availability: TextFieldCommandAvailability,
    return_policy: TextAreaReturnPolicy,
    minimum_lines: u32,
}

impl TextAreaRef {
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

    pub const fn availability(self) -> TextFieldCommandAvailability {
        self.availability
    }

    pub const fn return_policy(self) -> TextAreaReturnPolicy {
        self.return_policy
    }

    pub const fn minimum_lines(self) -> u32 {
        self.minimum_lines
    }
}

#[derive(Debug)]
pub enum TextAreaError {
    NewlineSubmitAction,
    InvalidMinimumLines,
    MissingNewlineEdit,
    Field(TextFieldError),
}

impl fmt::Display for TextAreaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NewlineSubmitAction => {
                formatter.write_str("text area submit policy requires a non-newline action")
            }
            Self::InvalidMinimumLines => {
                formatter.write_str("text area requires at least two visible lines")
            }
            Self::MissingNewlineEdit => {
                formatter.write_str("text area newline policy requires a newline edit")
            }
            Self::Field(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TextAreaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Field(error) => Some(error),
            Self::NewlineSubmitAction | Self::InvalidMinimumLines | Self::MissingNewlineEdit => {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::text::{TextAffinity, TextEdit, TextOffset, TextRange};
    use crate::ui::{BoxStyle, LayoutStyle, NodeKind, SemanticRole, SizeRule, UiRoot};

    use super::*;

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
    fn configuration_validates_policy_and_exposes_multiline_input() {
        assert!(matches!(
            TextArea::new(TextController::new(), "Notes", TextFieldMode::Editable)
                .unwrap()
                .minimum_lines(1),
            Err(TextAreaError::InvalidMinimumLines)
        ));
        assert!(matches!(
            TextArea::new(TextController::new(), "Notes", TextFieldMode::Editable)
                .unwrap()
                .return_policy(TextAreaReturnPolicy::Submit(TextReturnKeyAction::Newline)),
            Err(TextAreaError::NewlineSubmitAction)
        ));
        let area = TextArea::new(TextController::new(), "Notes", TextFieldMode::Editable).unwrap();
        assert_eq!(
            area.input_configuration().multiline,
            TextMultiline::MultiLine
        );
        assert_eq!(
            area.input_configuration().return_key,
            TextReturnKeyAction::Newline
        );
        assert!(!area.command_availability().can_submit);
    }

    #[test]
    fn edit_and_return_insert_multiline_text_through_the_shared_controller() {
        let mut area =
            TextArea::new(TextController::new(), "Notes", TextFieldMode::Editable).unwrap();
        let edited = area
            .route(TextAreaCommand::Edit {
                batch: replacement(TextRevision::INITIAL, 0, "one\ntwo"),
                kind: EditHistoryKind::Typing,
                recorded_at: MonotonicInstant::from_nanos(1),
            })
            .unwrap();
        assert!(matches!(edited, TextAreaOutput::Updated(_)));
        let revision = area.controller().revision();
        area.route(TextAreaCommand::Return {
            newline: replacement(revision, 7, "one\ntwo\n"),
            recorded_at: MonotonicInstant::from_nanos(2),
        })
        .unwrap();
        let text: String = area
            .controller()
            .snapshot()
            .chunks()
            .map(|chunk| chunk.text)
            .collect();
        assert_eq!(text, "one\ntwo\n");
    }

    #[test]
    fn submit_return_does_not_apply_the_newline_batch() {
        let mut area = TextArea::new(
            TextController::from_text("query").unwrap(),
            "Query",
            TextFieldMode::Editable,
        )
        .unwrap()
        .return_policy(TextAreaReturnPolicy::Submit(TextReturnKeyAction::Search))
        .unwrap();
        let revision = area.controller().revision();
        let output = area
            .route(TextAreaCommand::Return {
                newline: replacement(revision, 5, "query\n"),
                recorded_at: MonotonicInstant::from_nanos(1),
            })
            .unwrap();
        let TextAreaOutput::Submitted(submitted) = output else {
            panic!("submit return must publish submission")
        };
        assert_eq!(submitted.action, TextReturnKeyAction::Search);
        assert_eq!(submitted.revision, revision);
        assert_eq!(area.controller().revision(), revision);
        assert!(area.command_availability().can_submit);
    }

    #[test]
    fn return_requires_a_newline_batch_and_debug_redacts_replacement() {
        let mut area =
            TextArea::new(TextController::new(), "Notes", TextFieldMode::Editable).unwrap();
        assert!(matches!(
            area.route(TextAreaCommand::Return {
                newline: replacement(TextRevision::INITIAL, 0, "private text"),
                recorded_at: MonotonicInstant::from_nanos(1),
            }),
            Err(TextAreaError::MissingNewlineEdit)
        ));
        let command = TextAreaCommand::Edit {
            batch: replacement(TextRevision::INITIAL, 0, "private text"),
            kind: EditHistoryKind::Typing,
            recorded_at: MonotonicInstant::from_nanos(1),
        };
        assert!(!format!("{command:?}").contains("private text"));
    }

    struct MountedArea {
        area: TextArea,
        reference: Rc<Cell<Option<TextAreaRef>>>,
    }

    impl Component for MountedArea {
        type State = ();
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.reference
                .set(Some(self.area.mount(ui, root.0).unwrap()));
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
    fn mounted_area_uses_multiline_semantics_and_a_three_line_floor() {
        let reference = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(MountedArea {
            area: TextArea::new(
                TextController::from_text("one\ntwo").unwrap(),
                "Notes",
                TextFieldMode::Editable,
            )
            .unwrap(),
            reference: reference.clone(),
        })
        .unwrap();
        let reference = reference.get().unwrap();
        assert_eq!(
            runtime.ui().kinds.get(reference.node()),
            Some(&NodeKind::TextInput)
        );
        let semantics = runtime.ui().semantics.get(reference.node()).unwrap();
        assert_eq!(semantics.role, SemanticRole::TextInput);
        assert!(semantics.state.multiline);
        let SizeRule::Logical(height) = runtime
            .ui()
            .box_styles
            .get(reference.node())
            .unwrap()
            .min_size
            .height
        else {
            panic!("text area must use a fixed minimum height")
        };
        assert!(height > 44.0);
        assert_eq!(reference.minimum_lines(), 3);
    }
}
