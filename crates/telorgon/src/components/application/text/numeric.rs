//! Numeric editing state and commit policy composed over the basic text field.

use std::fmt;

use crate::runtime::{MonotonicInstant, RuntimeResult, Ui};
use crate::ui::text::{
    TextEditBatch, TextInputConfiguration, TextInputPurpose, TextRevision, TextSelection,
};
use crate::ui::{SemanticNode, SemanticRole, SemanticValue, StringId, UiNodeId};

use crate::components::application::{DensityClass, RangeModel, RangeModelError, RangeScalar};

use super::{
    EditHistoryCommand, EditHistoryKind, TextController, TextControllerUpdate, TextField,
    TextFieldCommand, TextFieldCommandAvailability, TextFieldError, TextFieldMode, TextFieldOutput,
    TextFieldRef, TextFieldStyle,
};

/// Locale-neutral decimal conversion used after the editing grammar is complete.
pub trait NumericFieldScalar: RangeScalar {
    fn parse_decimal(text: &str) -> Result<Self, NumericScalarParseError>;
}

impl NumericFieldScalar for f32 {
    fn parse_decimal(text: &str) -> Result<Self, NumericScalarParseError> {
        let value = text
            .parse::<f32>()
            .map_err(|_| NumericScalarParseError::Unrepresentable)?;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(NumericScalarParseError::NonFinite)
        }
    }
}

impl NumericFieldScalar for f64 {
    fn parse_decimal(text: &str) -> Result<Self, NumericScalarParseError> {
        let value = text
            .parse::<f64>()
            .map_err(|_| NumericScalarParseError::Unrepresentable)?;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(NumericScalarParseError::NonFinite)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericScalarParseError {
    NonFinite,
    Unrepresentable,
}

/// Incomplete decimal forms preserved as editable text rather than coerced values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericIntermediate {
    Empty,
    Sign,
    DecimalPoint,
    TrailingDecimalPoint,
    ExponentMarker,
    ExponentSign,
}

/// Why complete-looking numeric text cannot currently commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericInvalid {
    Syntax,
    NonFinite,
    Unrepresentable,
    Constraint(RangeModelError),
}

/// Current transient parse/constraint state of the controller text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumericFieldState<T> {
    Intermediate(NumericIntermediate),
    Valid(T),
    Invalid(NumericInvalid),
}

impl<T> NumericFieldState<T> {
    pub const fn is_valid(&self) -> bool {
        matches!(self, Self::Valid(_))
    }

    pub const fn is_intermediate(&self) -> bool {
        matches!(self, Self::Intermediate(_))
    }

    pub const fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid(_))
    }
}

/// Accepted numeric value and deterministic formatted representation.
#[derive(Clone, Debug, PartialEq)]
pub struct NumericCommit<T> {
    pub revision: TextRevision,
    pub value: T,
    pub formatted: String,
}

/// One renderer- and platform-neutral request routed through a numeric field.
pub enum NumericFieldCommand {
    Edit {
        batch: TextEditBatch,
        kind: EditHistoryKind,
        recorded_at: MonotonicInstant,
    },
    SetSelection {
        base_revision: TextRevision,
        selection: TextSelection,
    },
    Commit,
    History(EditHistoryCommand),
}

impl fmt::Debug for NumericFieldCommand {
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
            Self::Commit => formatter.write_str("Commit"),
            Self::History(command) => formatter.debug_tuple("History").field(command).finish(),
        }
    }
}

/// Typed result preserving transient state separately from accepted numeric commits.
#[derive(Clone, Debug)]
pub enum NumericFieldOutput<T> {
    Updated {
        update: TextControllerUpdate,
        state: NumericFieldState<T>,
    },
    Committed(NumericCommit<T>),
    CommitRejected {
        revision: TextRevision,
        state: NumericFieldState<T>,
    },
}

/// Mode-, history-, and parse-derived numeric command availability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NumericFieldCommandAvailability {
    pub can_edit: bool,
    pub can_select: bool,
    pub can_commit: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Numeric parsing/constraint behavior composed over one basic text field.
pub struct NumericField<T> {
    field: TextField,
    model: RangeModel<T>,
}

impl<T> fmt::Debug for NumericField<T>
where
    T: NumericFieldScalar,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NumericField")
            .field("field", &self.field)
            .field("model", &self.model)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl<T> NumericField<T>
where
    T: NumericFieldScalar,
{
    pub fn new(
        controller: TextController,
        label: impl Into<String>,
        mode: TextFieldMode,
        model: RangeModel<T>,
    ) -> Result<Self, NumericFieldError> {
        if mode.is_secure() {
            return Err(NumericFieldError::SecureModeUnsupported);
        }
        Ok(Self {
            field: TextField::new(controller, label, mode).map_err(NumericFieldError::Field)?,
            model,
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

    pub const fn model(&self) -> &RangeModel<T> {
        &self.model
    }

    pub fn input_configuration(&self) -> TextInputConfiguration {
        let mut configuration = self.field.input_configuration();
        configuration.purpose = TextInputPurpose::Decimal;
        configuration
    }

    pub fn state(&self) -> NumericFieldState<T> {
        parse_numeric_state(&snapshot_text(self.field.controller()), &self.model)
    }

    pub fn command_availability(&self) -> NumericFieldCommandAvailability {
        let TextFieldCommandAvailability {
            can_edit,
            can_select,
            can_undo,
            can_redo,
            ..
        } = self.field.command_availability();
        NumericFieldCommandAvailability {
            can_edit,
            can_select,
            can_commit: can_edit && self.state().is_valid(),
            can_undo,
            can_redo,
        }
    }

    pub fn route(
        &mut self,
        command: NumericFieldCommand,
    ) -> Result<NumericFieldOutput<T>, NumericFieldError> {
        match command {
            NumericFieldCommand::Edit {
                batch,
                kind,
                recorded_at,
            } => self.route_field(TextFieldCommand::Edit {
                batch,
                kind,
                recorded_at,
            }),
            NumericFieldCommand::SetSelection {
                base_revision,
                selection,
            } => self.route_field(TextFieldCommand::SetSelection {
                base_revision,
                selection,
            }),
            NumericFieldCommand::History(command) => {
                self.route_field(TextFieldCommand::History(command))
            }
            NumericFieldCommand::Commit => self.commit(),
        }
    }

    pub fn semantic_node(
        &self,
        name: StringId,
        editing_text: Option<StringId>,
        formatted_value: Option<StringId>,
    ) -> SemanticNode {
        let semantic =
            self.field
                .semantic_node_internal(name, editing_text, false, SemanticRole::TextInput);
        numeric_semantics(semantic, self.state(), formatted_value, &self.model)
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<NumericFieldRef<T>> {
        let state = self.state();
        let formatted_value = match state {
            NumericFieldState::Valid(value) => self
                .model
                .format_value(value)
                .ok()
                .map(|formatted| ui.foundation().intern(&formatted)),
            NumericFieldState::Intermediate(_) | NumericFieldState::Invalid(_) => None,
        };
        let minimum = self.model.minimum().to_f64();
        let maximum = self.model.maximum().to_f64();
        let step = self.model.step().to_f64();
        let field = self.field.mount_internal_with_semantics(
            ui,
            host,
            false,
            1,
            SemanticRole::TextInput,
            |mut semantic| {
                semantic.state.invalid |= state.is_invalid();
                if let NumericFieldState::Valid(value) = state {
                    semantic.value = SemanticValue::Number {
                        current: value.to_f64(),
                        minimum,
                        maximum,
                        step: Some(step),
                        value_text: formatted_value,
                    };
                }
                semantic
            },
        )?;
        if state.is_invalid() {
            ui.foundation().invalid(field.node(), true);
        }
        Ok(NumericFieldRef {
            field,
            state,
            availability: self.command_availability(),
        })
    }

    fn route_field(
        &mut self,
        command: TextFieldCommand,
    ) -> Result<NumericFieldOutput<T>, NumericFieldError> {
        let output = self
            .field
            .route(command)
            .map_err(NumericFieldError::Field)?;
        let TextFieldOutput::Updated(update) = output else {
            unreachable!("numeric edit/selection/history commands cannot submit")
        };
        Ok(NumericFieldOutput::Updated {
            update,
            state: self.state(),
        })
    }

    fn commit(&self) -> Result<NumericFieldOutput<T>, NumericFieldError> {
        self.field
            .ensure_editable()
            .map_err(NumericFieldError::Field)?;
        let revision = self.field.controller().revision();
        let state = self.state();
        match state {
            NumericFieldState::Valid(value) => Ok(NumericFieldOutput::Committed(NumericCommit {
                revision,
                value,
                formatted: self
                    .model
                    .format_value(value)
                    .expect("valid numeric state passed model constraints"),
            })),
            NumericFieldState::Intermediate(_) | NumericFieldState::Invalid(_) => {
                Ok(NumericFieldOutput::CommitRejected { revision, state })
            }
        }
    }
}

/// Focused mount-time reference; editing remains owned by [`NumericField`] and its controller.
#[derive(Clone, Copy, Debug)]
pub struct NumericFieldRef<T> {
    field: TextFieldRef,
    state: NumericFieldState<T>,
    availability: NumericFieldCommandAvailability,
}

impl<T> NumericFieldRef<T>
where
    T: Copy,
{
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

    pub const fn state(self) -> NumericFieldState<T> {
        self.state
    }

    pub const fn availability(self) -> NumericFieldCommandAvailability {
        self.availability
    }
}

#[derive(Debug)]
pub enum NumericFieldError {
    SecureModeUnsupported,
    Field(TextFieldError),
}

impl fmt::Display for NumericFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SecureModeUnsupported => {
                formatter.write_str("numeric field does not expose parsed values in secure mode")
            }
            Self::Field(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for NumericFieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Field(error) => Some(error),
            Self::SecureModeUnsupported => None,
        }
    }
}

fn numeric_semantics<T: NumericFieldScalar>(
    mut semantic: SemanticNode,
    state: NumericFieldState<T>,
    formatted_value: Option<StringId>,
    model: &RangeModel<T>,
) -> SemanticNode {
    semantic.state.invalid |= state.is_invalid();
    if let NumericFieldState::Valid(value) = state {
        semantic.value = SemanticValue::Number {
            current: value.to_f64(),
            minimum: model.minimum().to_f64(),
            maximum: model.maximum().to_f64(),
            step: Some(model.step().to_f64()),
            value_text: formatted_value,
        };
    }
    semantic
}

fn snapshot_text(controller: &TextController) -> String {
    controller
        .snapshot()
        .chunks()
        .map(|chunk| chunk.text)
        .collect()
}

fn parse_numeric_state<T: NumericFieldScalar>(
    text: &str,
    model: &RangeModel<T>,
) -> NumericFieldState<T> {
    match decimal_syntax(text) {
        DecimalSyntax::Intermediate(intermediate) => NumericFieldState::Intermediate(intermediate),
        DecimalSyntax::Invalid => NumericFieldState::Invalid(NumericInvalid::Syntax),
        DecimalSyntax::Complete => match T::parse_decimal(text) {
            Ok(value) => match model.format_value(value) {
                Ok(_) => NumericFieldState::Valid(value),
                Err(error) => NumericFieldState::Invalid(NumericInvalid::Constraint(error)),
            },
            Err(NumericScalarParseError::NonFinite) => {
                NumericFieldState::Invalid(NumericInvalid::NonFinite)
            }
            Err(NumericScalarParseError::Unrepresentable) => {
                NumericFieldState::Invalid(NumericInvalid::Unrepresentable)
            }
        },
    }
}

enum DecimalSyntax {
    Intermediate(NumericIntermediate),
    Complete,
    Invalid,
}

fn decimal_syntax(text: &str) -> DecimalSyntax {
    if text.is_empty() {
        return DecimalSyntax::Intermediate(NumericIntermediate::Empty);
    }
    if !text.is_ascii() {
        return DecimalSyntax::Invalid;
    }
    let bytes = text.as_bytes();
    let mut index = 0;
    if matches!(bytes[index], b'+' | b'-') {
        index += 1;
        if index == bytes.len() {
            return DecimalSyntax::Intermediate(NumericIntermediate::Sign);
        }
    }

    let integer_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    let integer_digits = index - integer_start;
    let mut fraction_digits = 0;
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        let fraction_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        fraction_digits = index - fraction_start;
        if index == bytes.len() && fraction_digits == 0 {
            return if integer_digits == 0 {
                DecimalSyntax::Intermediate(NumericIntermediate::DecimalPoint)
            } else {
                DecimalSyntax::Intermediate(NumericIntermediate::TrailingDecimalPoint)
            };
        }
    }
    if integer_digits == 0 && fraction_digits == 0 {
        return DecimalSyntax::Invalid;
    }

    if index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
        index += 1;
        if index == bytes.len() {
            return DecimalSyntax::Intermediate(NumericIntermediate::ExponentMarker);
        }
        if matches!(bytes[index], b'+' | b'-') {
            index += 1;
            if index == bytes.len() {
                return DecimalSyntax::Intermediate(NumericIntermediate::ExponentSign);
            }
        }
        let exponent_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if exponent_start == index {
            return DecimalSyntax::Invalid;
        }
    }

    if index == bytes.len() {
        DecimalSyntax::Complete
    } else {
        DecimalSyntax::Invalid
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::text::{TextAffinity, TextEdit, TextOffset, TextRange, TextSelection};
    use crate::ui::{BoxStyle, LayoutStyle, NodeKind, SizeRule, UiRoot};

    use super::*;
    use crate::components::application::{EditHistoryPolicy, RangeFormat};

    fn model() -> RangeModel<f64> {
        RangeModel::new(-10.0, 10.0, 0.5, 2.0)
            .unwrap()
            .with_format(RangeFormat::new(2).unwrap())
    }

    fn field(text: &str) -> NumericField<f64> {
        NumericField::new(
            TextController::from_text(text).unwrap(),
            "Amount",
            TextFieldMode::Editable,
            model(),
        )
        .unwrap()
    }

    fn replacement(revision: TextRevision, end: u32, text: &str) -> TextEditBatch {
        TextEditBatch {
            base_revision: revision,
            edits: vec![TextEdit {
                range: TextRange::new(TextOffset::ZERO, TextOffset(end)).unwrap(),
                replacement: text.to_owned(),
            }],
            selection: TextSelection::collapsed(
                TextOffset(u32::try_from(text.len()).unwrap()),
                TextAffinity::Downstream,
            ),
            composition: None,
        }
    }

    #[test]
    fn decimal_grammar_preserves_intermediate_forms_and_rejects_invalid_syntax() {
        for (text, intermediate) in [
            ("", NumericIntermediate::Empty),
            ("-", NumericIntermediate::Sign),
            (".", NumericIntermediate::DecimalPoint),
            ("-.", NumericIntermediate::DecimalPoint),
            ("1.", NumericIntermediate::TrailingDecimalPoint),
            ("1e", NumericIntermediate::ExponentMarker),
            ("1e-", NumericIntermediate::ExponentSign),
        ] {
            assert_eq!(
                field(text).state(),
                NumericFieldState::Intermediate(intermediate)
            );
        }
        assert_eq!(field(".5").state(), NumericFieldState::Valid(0.5));
        let f32_field = NumericField::new(
            TextController::from_text(".5").unwrap(),
            "Amount",
            TextFieldMode::Editable,
            RangeModel::new(-1.0_f32, 1.0, 0.1, 0.5).unwrap(),
        )
        .unwrap();
        assert_eq!(f32_field.state(), NumericFieldState::Valid(0.5_f32));
        assert_eq!(
            field("1..2").state(),
            NumericFieldState::Invalid(NumericInvalid::Syntax)
        );
    }

    #[test]
    fn parsing_distinguishes_constraints_and_nonfinite_values() {
        assert_eq!(field("10").state(), NumericFieldState::Valid(10.0));
        assert_eq!(
            field("10.1").state(),
            NumericFieldState::Invalid(NumericInvalid::Constraint(
                RangeModelError::ValueOutOfBounds
            ))
        );
        assert_eq!(
            field("1e999").state(),
            NumericFieldState::Invalid(NumericInvalid::NonFinite)
        );
    }

    #[test]
    fn edits_publish_transient_state_without_rewriting_text() {
        let mut numeric = field("1");
        let output = numeric
            .route(NumericFieldCommand::Edit {
                batch: replacement(TextRevision::INITIAL, 1, "1."),
                kind: EditHistoryKind::Typing,
                recorded_at: MonotonicInstant::from_nanos(1),
            })
            .unwrap();
        let NumericFieldOutput::Updated { update, state } = output else {
            panic!("edit must publish its transient numeric state")
        };
        assert_eq!(
            state,
            NumericFieldState::Intermediate(NumericIntermediate::TrailingDecimalPoint)
        );
        assert_eq!(
            update
                .snapshot
                .chunks()
                .map(|chunk| chunk.text)
                .collect::<String>(),
            "1."
        );
    }

    #[test]
    fn commit_requires_valid_state_and_never_mutates_controller_text() {
        let numeric = &mut field("1.5");
        let revision = numeric.controller().revision();
        let output = numeric.route(NumericFieldCommand::Commit).unwrap();
        let NumericFieldOutput::Committed(commit) = output else {
            panic!("valid numeric text must commit")
        };
        assert_eq!(commit.revision, revision);
        assert_eq!(commit.value, 1.5);
        assert_eq!(commit.formatted, "1.50");
        assert_eq!(numeric.controller().revision(), revision);

        let intermediate = &mut field("1.");
        assert!(matches!(
            intermediate.route(NumericFieldCommand::Commit).unwrap(),
            NumericFieldOutput::CommitRejected {
                state: NumericFieldState::Intermediate(_),
                ..
            }
        ));
        assert_eq!(intermediate.controller().revision(), TextRevision::INITIAL);
    }

    #[test]
    fn history_recomputes_numeric_state_and_secure_mode_is_rejected() {
        let mut controller = TextController::from_text("1").unwrap();
        controller
            .enable_edit_history(EditHistoryPolicy::default())
            .unwrap();
        let mut numeric =
            NumericField::new(controller, "Amount", TextFieldMode::Editable, model()).unwrap();
        numeric
            .route(NumericFieldCommand::Edit {
                batch: replacement(TextRevision::INITIAL, 1, "2"),
                kind: EditHistoryKind::Typing,
                recorded_at: MonotonicInstant::from_nanos(1),
            })
            .unwrap();
        let output = numeric
            .route(NumericFieldCommand::History(EditHistoryCommand::Undo))
            .unwrap();
        assert!(matches!(
            output,
            NumericFieldOutput::Updated {
                state: NumericFieldState::Valid(1.0),
                ..
            }
        ));
        assert!(matches!(
            NumericField::new(
                TextController::from_text("1234").unwrap(),
                "PIN",
                TextFieldMode::Secure,
                model()
            ),
            Err(NumericFieldError::SecureModeUnsupported)
        ));
        assert_eq!(
            numeric.input_configuration().purpose,
            TextInputPurpose::Decimal
        );
    }

    struct MountedNumeric {
        numeric: NumericField<f64>,
        reference: Rc<Cell<Option<NumericFieldRef<f64>>>>,
    }

    impl Component for MountedNumeric {
        type State = ();
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.reference
                .set(Some(self.numeric.mount(ui, root.0).unwrap()));
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

    fn mounted(
        numeric: NumericField<f64>,
    ) -> (
        ViewRuntime<crate::runtime::ComponentRuntimeDriver<MountedNumeric>>,
        NumericFieldRef<f64>,
    ) {
        let reference = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(MountedNumeric {
            numeric,
            reference: reference.clone(),
        })
        .unwrap();
        let reference = reference.get().unwrap();
        (runtime, reference)
    }

    #[test]
    fn mounted_valid_and_invalid_states_publish_numeric_semantics_and_error_state() {
        let (runtime, reference) = mounted(field("1.5").density(DensityClass::Touch));
        assert_eq!(
            runtime.ui().kinds.get(reference.node()),
            Some(&NodeKind::TextInput)
        );
        assert!(matches!(
            runtime.ui().semantics.get(reference.node()).unwrap().value,
            SemanticValue::Number {
                current: 1.5,
                minimum: -10.0,
                maximum: 10.0,
                step: Some(0.5),
                ..
            }
        ));
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
        assert!(reference.availability().can_commit);

        let (runtime, reference) = mounted(field("invalid"));
        let semantic = runtime.ui().semantics.get(reference.node()).unwrap();
        assert!(semantic.state.invalid);
        assert!(
            runtime
                .ui()
                .interactions
                .get(reference.node())
                .unwrap()
                .flags
                .contains(crate::ui::InteractionFlags::INVALID)
        );
        assert!(!reference.availability().can_commit);
    }
}
