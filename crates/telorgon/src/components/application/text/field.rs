//! Basic single-line application text field policy, semantics, commands, and mounting.

use std::fmt;
#[path = "clipboard.rs"]
mod clipboard;
pub use clipboard::{TextClipboardTarget, TextClipboardTransfer};

use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::runtime::{MonotonicInstant, RuntimeError, RuntimeResult, Ui};
use crate::ui::text::{
    TextEditBatch, TextInputConfiguration, TextMultiline, TextReturnKeyAction, TextRevision,
    TextSelection, TextSnapshot,
};
use crate::ui::{
    Background, Border, BoxStyle, ControlHandle, CornerRadii, LayoutStyle, Property,
    SemanticActions, SemanticName, SemanticNode, SemanticRole, SemanticState, SemanticValue,
    SizeRule, SizeRule2D, StringId, TextHandle, UiNodeId,
};

use crate::components::application::{DensityClass, DensityMetrics};

use super::{
    EditHistoryCommand, EditHistoryKind, EditRejected, Submitted, TextController,
    TextControllerHistoryError, TextControllerUpdate,
};

/// Mutually exclusive baseline interaction and privacy modes for a single-line field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextFieldMode {
    #[default]
    Editable,
    ReadOnly,
    Disabled,
    Secure,
}

impl TextFieldMode {
    pub const fn is_editable(self) -> bool {
        matches!(self, Self::Editable | Self::Secure)
    }

    pub const fn is_secure(self) -> bool {
        matches!(self, Self::Secure)
    }

    pub const fn is_disabled(self) -> bool {
        matches!(self, Self::Disabled)
    }
}

/// Named visual slots for one application field density.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextFieldVisualStyle {
    pub container: BoxStyle,
    pub label_color: ColorRgba8,
    pub value_color: ColorRgba8,
    pub label_size: f32,
    pub value_size: f32,
    pub gap: f32,
    pub minimum_width: f32,
}

/// Explicit Compact/Standard/Touch field visuals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextFieldStyle {
    pub compact: TextFieldVisualStyle,
    pub standard: TextFieldVisualStyle,
    pub touch: TextFieldVisualStyle,
}

impl TextFieldStyle {
    pub const fn resolve(self, density: DensityClass) -> TextFieldVisualStyle {
        match density {
            DensityClass::Compact => self.compact,
            DensityClass::Standard => self.standard,
            DensityClass::Touch => self.touch,
        }
    }
}

impl Default for TextFieldStyle {
    fn default() -> Self {
        fn visual(
            padding: f32,
            label_size: f32,
            value_size: f32,
            gap: f32,
            minimum_width: f32,
        ) -> TextFieldVisualStyle {
            TextFieldVisualStyle {
                container: BoxStyle {
                    padding: EdgeInsets::all(padding),
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(35, 39, 49, 255)),
                        border: Border::all(1.0, ColorRgba8::rgba(92, 101, 123, 255)),
                        corner_radii: CornerRadii::all(6.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                label_color: ColorRgba8::rgba(189, 195, 208, 255),
                value_color: ColorRgba8::rgba(248, 249, 252, 255),
                label_size,
                value_size,
                gap,
                minimum_width,
            }
        }

        Self {
            compact: visual(4.0, 11.0, 12.0, 2.0, 120.0),
            standard: visual(6.0, 12.0, 14.0, 3.0, 160.0),
            touch: visual(8.0, 14.0, 16.0, 4.0, 200.0),
        }
    }
}

/// One renderer- and platform-neutral request routed through the field's controller.
pub enum TextFieldCommand {
    Edit {
        batch: TextEditBatch,
        kind: EditHistoryKind,
        recorded_at: MonotonicInstant,
    },
    SetSelection {
        base_revision: TextRevision,
        selection: TextSelection,
    },
    Submit,
    History(EditHistoryCommand),
}

impl fmt::Debug for TextFieldCommand {
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
            Self::Submit => formatter.write_str("Submit"),
            Self::History(command) => formatter.debug_tuple("History").field(command).finish(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum TextFieldOutput {
    Updated(TextControllerUpdate),
    Submitted(Submitted),
}

/// Current mode- and controller-derived command availability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextFieldCommandAvailability {
    pub can_edit: bool,
    pub can_select: bool,
    pub can_submit: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// One basic single-line field owning exactly one application text controller.
pub struct TextField {
    clipboard_identity: std::sync::Arc<()>,
    clipboard_epoch: u64,
    controller: TextController,
    label: String,
    mode: TextFieldMode,
    return_action: TextReturnKeyAction,
    required: bool,
    invalid: bool,
    density: DensityClass,
    style: TextFieldStyle,
}

impl fmt::Debug for TextField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextField")
            .field("label", &self.label)
            .field("mode", &self.mode)
            .field("return_action", &self.return_action)
            .field("required", &self.required)
            .field("invalid", &self.invalid)
            .field("density", &self.density)
            .field("revision", &self.controller.revision())
            .field("selection", &self.controller.selection())
            .finish_non_exhaustive()
    }
}

impl TextField {
    pub fn new(
        mut controller: TextController,
        label: impl Into<String>,
        mode: TextFieldMode,
    ) -> Result<Self, TextFieldError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TextFieldError::MissingAccessibleName);
        }
        if mode.is_secure() {
            controller.disable_edit_history();
        }
        Ok(Self {
            clipboard_identity: std::sync::Arc::new(()),
            clipboard_epoch: 0,
            controller,
            label,
            mode,
            return_action: TextReturnKeyAction::Done,
            required: false,
            invalid: false,
            density: DensityClass::Standard,
            style: TextFieldStyle::default(),
        })
    }

    pub fn return_action(mut self, action: TextReturnKeyAction) -> Result<Self, TextFieldError> {
        if action == TextReturnKeyAction::Newline {
            return Err(TextFieldError::NewlineReturnAction);
        }
        self.return_action = action;
        Ok(self)
    }

    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn density(mut self, density: DensityClass) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: TextFieldStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn controller(&self) -> &TextController {
        &self.controller
    }

    pub fn into_controller(self) -> TextController {
        self.controller
    }

    pub const fn mode(&self) -> TextFieldMode {
        self.mode
    }

    pub fn input_configuration(&self) -> TextInputConfiguration {
        TextInputConfiguration {
            secure_entry: self.mode.is_secure(),
            multiline: TextMultiline::SingleLine,
            return_key: self.return_action,
            ..TextInputConfiguration::default()
        }
    }

    pub fn command_availability(&self) -> TextFieldCommandAvailability {
        let history = self.controller.edit_history_availability();
        let editable = self.mode.is_editable();
        let secure = self.mode.is_secure();
        TextFieldCommandAvailability {
            can_edit: editable,
            can_select: !self.mode.is_disabled(),
            can_submit: editable,
            can_undo: editable && !secure && history.can_undo,
            can_redo: editable && !secure && history.can_redo,
        }
    }

    pub fn route(&mut self, command: TextFieldCommand) -> Result<TextFieldOutput, TextFieldError> {
        self.route_internal(command, false)
    }

    pub(crate) fn route_internal(
        &mut self,
        command: TextFieldCommand,
        multiline: bool,
    ) -> Result<TextFieldOutput, TextFieldError> {
        match command {
            TextFieldCommand::Edit {
                batch,
                kind,
                recorded_at,
            } => {
                self.ensure_editable()?;
                if !multiline
                    && batch.edits.iter().any(|edit| {
                        edit.replacement
                            .chars()
                            .any(|character| matches!(character, '\n' | '\r'))
                    })
                {
                    return Err(TextFieldError::MultilineEdit);
                }
                let update = if self.mode.is_secure()
                    || !self.controller.edit_history_availability().enabled
                {
                    self.controller
                        .apply_edits(batch)
                        .map_err(TextFieldError::Edit)?
                } else {
                    self.controller
                        .apply_edits_recorded(batch, kind, recorded_at)
                        .map_err(TextFieldError::History)?
                };
                Ok(TextFieldOutput::Updated(update))
            }
            TextFieldCommand::SetSelection {
                base_revision,
                selection,
            } => {
                self.ensure_selectable()?;
                let update = self
                    .controller
                    .set_selection(base_revision, selection)
                    .map_err(TextFieldError::Edit)?;
                Ok(TextFieldOutput::Updated(update))
            }
            TextFieldCommand::Submit => {
                self.ensure_editable()?;
                Ok(TextFieldOutput::Submitted(Submitted {
                    revision: self.controller.revision(),
                    action: self.return_action,
                }))
            }
            TextFieldCommand::History(command) => {
                self.ensure_editable()?;
                if self.mode.is_secure() {
                    return Err(TextFieldError::SecureHistoryUnavailable);
                }
                let update = self
                    .controller
                    .apply_edit_history_command(command)
                    .map_err(TextFieldError::History)?;
                Ok(TextFieldOutput::Updated(update))
            }
        }
    }

    pub fn semantic_node(&self, name: StringId, value: Option<StringId>) -> SemanticNode {
        self.semantic_node_internal(name, value, false, SemanticRole::TextInput)
    }

    pub(crate) fn semantic_node_internal(
        &self,
        name: StringId,
        value: Option<StringId>,
        multiline: bool,
        role: SemanticRole,
    ) -> SemanticNode {
        let mut actions = SemanticActions::NONE;
        if !self.mode.is_disabled() {
            actions |= SemanticActions::FOCUS | SemanticActions::SET_SELECTION;
        }
        if self.mode.is_editable() {
            actions |= SemanticActions::SET_TEXT;
        }
        SemanticNode {
            role,
            name: SemanticName::Text(name),
            state: SemanticState {
                disabled: self.mode.is_disabled(),
                read_only: self.mode == TextFieldMode::ReadOnly,
                multiline,
                required: self.required,
                invalid: self.invalid,
                focusable: !self.mode.is_disabled(),
                ..SemanticState::default()
            },
            value: if self.mode.is_secure() {
                SemanticValue::None
            } else {
                value.map_or(SemanticValue::None, SemanticValue::Text)
            },
            actions,
            ..SemanticNode::default()
        }
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<TextFieldRef> {
        self.mount_internal(ui, host, false, 1, SemanticRole::TextInput)
    }

    pub(crate) fn mount_internal<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        multiline: bool,
        minimum_lines: u32,
        semantic_role: SemanticRole,
    ) -> RuntimeResult<TextFieldRef> {
        self.mount_internal_with_semantics(
            ui,
            host,
            multiline,
            minimum_lines,
            semantic_role,
            |semantic| semantic,
        )
    }

    pub(crate) fn mount_internal_with_semantics<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        multiline: bool,
        minimum_lines: u32,
        semantic_role: SemanticRole,
        transform_semantics: impl FnOnce(SemanticNode) -> SemanticNode,
    ) -> RuntimeResult<TextFieldRef> {
        let plain = snapshot_text(&self.controller.snapshot());
        let display = if self.mode.is_secure() {
            "•".repeat(plain.chars().count())
        } else {
            plain.clone()
        };
        let visual = self.style.resolve(self.density);
        let minimum = DensityMetrics::baseline(self.density).effective_minimum();
        let mut container = visual.container;
        container.min_size = SizeRule2D {
            width: SizeRule::Logical(visual.minimum_width.max(minimum.width())),
            height: SizeRule::Logical(if multiline {
                minimum.height().max(
                    visual.container.padding.vertical()
                        + visual.label_size * 1.25
                        + visual.gap
                        + visual.value_size * 1.25 * minimum_lines as f32,
                )
            } else {
                minimum.height()
            }),
        };
        if self.mode.is_disabled() {
            container.opacity *= 0.55;
        }

        let label = self.label.clone();
        let mut value_handle = None;
        let control = ui
            .foundation()
            .text_input_node_under(
                host,
                container,
                LayoutStyle {
                    gap: visual.gap,
                    ..LayoutStyle::default()
                },
                !self.mode.is_disabled(),
                |writer| {
                    writer.text(&label, visual.label_color, visual.label_size);
                    value_handle =
                        Some(writer.text(&display, visual.value_color, visual.value_size));
                },
            )
            .ok_or_else(|| RuntimeError::new("application text-field host is stale"))?;
        let name = ui.foundation().intern(&self.label);
        let semantic_value = (!self.mode.is_secure()).then(|| ui.foundation().intern(&plain));
        let semantics = transform_semantics(self.semantic_node_internal(
            name,
            semantic_value,
            multiline,
            semantic_role,
        ));
        ui.foundation()
            .semantic_node(control.node, semantics)
            .map_err(|error| {
                RuntimeError::new(format!("invalid text-field semantics: {error:?}"))
            })?;
        if self.mode.is_disabled() {
            ui.foundation().disabled(control.node, true);
        }
        if self.invalid {
            ui.foundation().invalid(control.node, true);
        }
        Ok(TextFieldRef {
            control,
            value: value_handle.expect("text field always mounts a value node"),
            revision: self.controller.revision(),
            mode: self.mode,
            availability: self.command_availability(),
        })
    }

    pub(crate) fn ensure_editable(&self) -> Result<(), TextFieldError> {
        match self.mode {
            TextFieldMode::Editable | TextFieldMode::Secure => Ok(()),
            TextFieldMode::ReadOnly => Err(TextFieldError::ReadOnly),
            TextFieldMode::Disabled => Err(TextFieldError::Disabled),
        }
    }

    fn ensure_selectable(&self) -> Result<(), TextFieldError> {
        if self.mode.is_disabled() {
            Err(TextFieldError::Disabled)
        } else {
            Ok(())
        }
    }
}

/// Focused mount-time reference; editing remains owned by [`TextField`] and its controller.
#[derive(Clone, Copy, Debug)]
pub struct TextFieldRef {
    control: ControlHandle,
    value: TextHandle,
    revision: TextRevision,
    mode: TextFieldMode,
    availability: TextFieldCommandAvailability,
}

impl TextFieldRef {
    pub(crate) const fn with_availability(
        mut self,
        availability: TextFieldCommandAvailability,
    ) -> Self {
        self.availability = availability;
        self
    }

    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn value_node(self) -> UiNodeId {
        self.value.node
    }

    pub const fn value_text(self) -> Property<StringId> {
        self.value.text
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn revision(self) -> TextRevision {
        self.revision
    }

    pub const fn mode(self) -> TextFieldMode {
        self.mode
    }

    pub const fn availability(self) -> TextFieldCommandAvailability {
        self.availability
    }
}

#[derive(Debug)]
pub enum TextFieldError {
    MissingAccessibleName,
    NewlineReturnAction,
    MultilineEdit,
    ReadOnly,
    Disabled,
    SecureHistoryUnavailable,
    Edit(EditRejected),
    History(TextControllerHistoryError),
}

impl fmt::Display for TextFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAccessibleName => {
                formatter.write_str("text field requires an accessible label")
            }
            Self::NewlineReturnAction => {
                formatter.write_str("single-line text field cannot use the newline return action")
            }
            Self::MultilineEdit => {
                formatter.write_str("single-line text field rejected a newline edit")
            }
            Self::ReadOnly => formatter.write_str("text field is read-only"),
            Self::Disabled => formatter.write_str("text field is disabled"),
            Self::SecureHistoryUnavailable => {
                formatter.write_str("secure text field does not retain plaintext edit history")
            }
            Self::Edit(error) => error.fmt(formatter),
            Self::History(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TextFieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Edit(error) => Some(error),
            Self::History(error) => Some(error),
            Self::MissingAccessibleName
            | Self::NewlineReturnAction
            | Self::MultilineEdit
            | Self::ReadOnly
            | Self::Disabled
            | Self::SecureHistoryUnavailable => None,
        }
    }
}

fn snapshot_text(snapshot: &TextSnapshot) -> String {
    snapshot.chunks().map(|chunk| chunk.text).collect()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::text::{TextAffinity, TextEdit, TextOffset, TextRange};
    use crate::ui::{NodeKind, UiRoot};

    use super::*;
    use crate::components::application::EditHistoryPolicy;

    fn selection(offset: u32) -> TextSelection {
        TextSelection::collapsed(TextOffset(offset), TextAffinity::Downstream)
    }

    fn replace_command(
        revision: TextRevision,
        end: u32,
        replacement: &str,
        at: u64,
    ) -> TextFieldCommand {
        let replacement_end = u32::try_from(replacement.len()).unwrap();
        TextFieldCommand::Edit {
            batch: TextEditBatch {
                base_revision: revision,
                edits: vec![TextEdit {
                    range: TextRange::new(TextOffset::ZERO, TextOffset(end)).unwrap(),
                    replacement: replacement.to_owned(),
                }],
                selection: selection(replacement_end),
                composition: None,
            },
            kind: EditHistoryKind::Typing,
            recorded_at: MonotonicInstant::from_nanos(at),
        }
    }

    #[test]
    fn construction_validates_label_return_action_and_secure_history_policy() {
        assert!(matches!(
            TextField::new(TextController::new(), " ", TextFieldMode::Editable),
            Err(TextFieldError::MissingAccessibleName)
        ));
        let field = TextField::new(TextController::new(), "Name", TextFieldMode::Editable).unwrap();
        assert!(matches!(
            field.return_action(TextReturnKeyAction::Newline),
            Err(TextFieldError::NewlineReturnAction)
        ));

        let mut controller = TextController::from_text("secret").unwrap();
        controller
            .enable_edit_history(EditHistoryPolicy::default())
            .unwrap();
        let secure = TextField::new(controller, "Password", TextFieldMode::Secure).unwrap();
        assert!(!secure.controller().edit_history_availability().enabled);
        assert!(secure.input_configuration().secure_entry);
        assert_eq!(
            secure.input_configuration().multiline,
            TextMultiline::SingleLine
        );
        assert!(!format!("{secure:?}").contains("secret"));
        assert!(
            !format!(
                "{:?}",
                replace_command(TextRevision::INITIAL, 0, "private command text", 1)
            )
            .contains("private command text")
        );
    }

    #[test]
    fn mode_semantics_distinguish_editable_read_only_disabled_and_secure() {
        for (mode, editable, selectable, focusable, value_visible) in [
            (TextFieldMode::Editable, true, true, true, true),
            (TextFieldMode::ReadOnly, false, true, true, true),
            (TextFieldMode::Disabled, false, false, false, true),
            (TextFieldMode::Secure, true, true, true, false),
        ] {
            let field = TextField::new(TextController::new(), "Field", mode).unwrap();
            let semantic = field.semantic_node(StringId(1), Some(StringId(2)));
            assert_eq!(semantic.role, SemanticRole::TextInput);
            assert_eq!(semantic.state.disabled, mode == TextFieldMode::Disabled);
            assert_eq!(semantic.state.read_only, mode == TextFieldMode::ReadOnly);
            assert_eq!(semantic.state.focusable, focusable);
            assert_eq!(
                semantic
                    .actions
                    .contains(crate::ui::SemanticAction::SetText),
                editable
            );
            assert_eq!(
                semantic
                    .actions
                    .contains(crate::ui::SemanticAction::SetSelection),
                selectable
            );
            assert_eq!(
                matches!(semantic.value, SemanticValue::Text(_)),
                value_visible
            );
        }
    }

    #[test]
    fn command_boundary_rejects_multiline_and_mode_incompatible_mutation_atomically() {
        let mut editable = TextField::new(
            TextController::from_text("one").unwrap(),
            "Name",
            TextFieldMode::Editable,
        )
        .unwrap();
        let revision = editable.controller().revision();
        assert!(matches!(
            editable.route(replace_command(revision, 3, "one\ntwo", 1)),
            Err(TextFieldError::MultilineEdit)
        ));
        assert_eq!(snapshot_text(&editable.controller().snapshot()), "one");
        assert_eq!(editable.controller().revision(), revision);

        let mut read_only = TextField::new(
            TextController::from_text("read").unwrap(),
            "Read",
            TextFieldMode::ReadOnly,
        )
        .unwrap();
        assert!(matches!(
            read_only.route(replace_command(TextRevision::INITIAL, 4, "write", 1)),
            Err(TextFieldError::ReadOnly)
        ));
        assert!(matches!(
            read_only.route(TextFieldCommand::SetSelection {
                base_revision: TextRevision::INITIAL,
                selection: selection(2),
            }),
            Ok(TextFieldOutput::Updated(_))
        ));

        let mut disabled = TextField::new(
            TextController::from_text("off").unwrap(),
            "Disabled",
            TextFieldMode::Disabled,
        )
        .unwrap();
        assert!(matches!(
            disabled.route(TextFieldCommand::SetSelection {
                base_revision: TextRevision::INITIAL,
                selection: selection(1),
            }),
            Err(TextFieldError::Disabled)
        ));
    }

    #[test]
    fn editable_field_routes_history_and_submission_without_platform_services() {
        let mut controller = TextController::new();
        controller
            .enable_edit_history(EditHistoryPolicy::default())
            .unwrap();
        let mut field = TextField::new(controller, "Query", TextFieldMode::Editable)
            .unwrap()
            .return_action(TextReturnKeyAction::Search)
            .unwrap();
        field
            .route(replace_command(TextRevision::INITIAL, 0, "query", 1))
            .unwrap();
        assert!(field.command_availability().can_undo);
        let undone = field
            .route(TextFieldCommand::History(EditHistoryCommand::Undo))
            .unwrap();
        let TextFieldOutput::Updated(undone) = undone else {
            panic!("undo must publish a controller update");
        };
        assert_eq!(snapshot_text(&undone.snapshot), "");
        assert!(field.command_availability().can_redo);

        let submitted = field.route(TextFieldCommand::Submit).unwrap();
        let TextFieldOutput::Submitted(submitted) = submitted else {
            panic!("submit must remain distinct from text mutation");
        };
        assert_eq!(submitted.revision, field.controller().revision());
        assert_eq!(submitted.action, TextReturnKeyAction::Search);
    }

    struct MountedField {
        field: TextField,
        reference: Rc<Cell<Option<TextFieldRef>>>,
    }

    impl Component for MountedField {
        type State = ();
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.reference
                .set(Some(self.field.mount(ui, root.0).unwrap()));
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
        field: TextField,
    ) -> (
        ViewRuntime<crate::runtime::ComponentRuntimeDriver<MountedField>>,
        TextFieldRef,
    ) {
        let reference = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(MountedField {
            field,
            reference: reference.clone(),
        })
        .unwrap();
        let reference = reference.get().unwrap();
        (runtime, reference)
    }

    #[test]
    fn mounted_field_uses_text_input_identity_semantics_and_touch_floor() {
        let field = TextField::new(
            TextController::from_text("value").unwrap(),
            "Account",
            TextFieldMode::Editable,
        )
        .unwrap()
        .required(true)
        .invalid(true)
        .density(DensityClass::Touch);
        let (runtime, reference) = mounted(field);
        assert_eq!(
            runtime.ui().kinds.get(reference.node()),
            Some(&NodeKind::TextInput)
        );
        let semantic = runtime.ui().semantics.get(reference.node()).unwrap();
        assert_eq!(semantic.role, SemanticRole::TextInput);
        assert!(semantic.state.required);
        assert!(semantic.state.invalid);
        assert!(
            semantic
                .actions
                .contains(crate::ui::SemanticAction::SetText)
        );
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
        assert!(reference.availability().can_edit);
    }

    #[test]
    fn mounted_secure_field_exposes_only_redacted_display_and_no_semantic_value() {
        let field = TextField::new(
            TextController::from_text("sëcret").unwrap(),
            "Password",
            TextFieldMode::Secure,
        )
        .unwrap();
        let (runtime, reference) = mounted(field);
        let visual = runtime.ui().texts.get(reference.value_node()).unwrap();
        assert_eq!(runtime.ui().string(visual.content), Some("••••••"));
        assert_eq!(
            runtime.ui().semantics.get(reference.node()).unwrap().value,
            SemanticValue::None
        );
        assert!(!format!("{:?}", runtime.ui()).contains("sëcret"));
    }
}
