//! Dedicated secure-field policy over the basic field's redacted mode.

use std::fmt;

use crate::runtime::{MonotonicInstant, RuntimeResult, Ui};
use crate::ui::text::{
    TextEditBatch, TextInputConfiguration, TextInputPolicy, TextReturnKeyAction, TextRevision,
    TextSelection,
};
use crate::ui::{SemanticNode, SemanticRole, StringId, UiNodeId};

use crate::components::application::DensityClass;

use super::{
    EditHistoryKind, Submitted, TextController, TextControllerUpdate, TextField, TextFieldCommand,
    TextFieldError, TextFieldMode, TextFieldOutput, TextFieldRef, TextFieldStyle,
};

/// How secure editing content may appear in one neutral output channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SecureContentExposure {
    Omitted,
    RedactedDisplay,
}

/// Fixed privacy guarantees enforced by the current secure-field package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecureFieldPrivacyPolicy {
    diagnostics: SecureContentExposure,
    semantics: SecureContentExposure,
    visual_capture: SecureContentExposure,
    retain_plaintext_history: bool,
    allow_copy: bool,
    allow_cut: bool,
}

impl SecureFieldPrivacyPolicy {
    pub const BASELINE: Self = Self {
        diagnostics: SecureContentExposure::Omitted,
        semantics: SecureContentExposure::Omitted,
        visual_capture: SecureContentExposure::RedactedDisplay,
        retain_plaintext_history: false,
        allow_copy: false,
        allow_cut: false,
    };

    pub const fn diagnostics(self) -> SecureContentExposure {
        self.diagnostics
    }

    pub const fn semantics(self) -> SecureContentExposure {
        self.semantics
    }

    /// Describes Telorgon's mounted content only; it does not claim OS screenshot prevention.
    pub const fn visual_capture(self) -> SecureContentExposure {
        self.visual_capture
    }

    pub const fn retains_plaintext_history(self) -> bool {
        self.retain_plaintext_history
    }

    pub const fn allows_copy(self) -> bool {
        self.allow_copy
    }

    pub const fn allows_cut(self) -> bool {
        self.allow_cut
    }
}

/// Clipboard capability facts supplied by a later platform-service owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecureContextCapabilities {
    pub can_read_plain_text: bool,
}

/// Secure context-command availability computed without invoking a platform service.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecureContextCommandAvailability {
    pub can_select_all: bool,
    pub can_paste: bool,
    pub can_copy: bool,
    pub can_cut: bool,
}

/// Mode- and fixed-policy-derived direct command availability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecureFieldCommandAvailability {
    pub can_edit: bool,
    pub can_select: bool,
    pub can_submit: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// One secure editing request. History traversal is intentionally absent.
pub enum SecureFieldCommand {
    Edit {
        batch: TextEditBatch,
    },
    SetSelection {
        base_revision: TextRevision,
        selection: TextSelection,
    },
    Submit,
}

impl fmt::Debug for SecureFieldCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Edit { batch } => formatter
                .debug_struct("Edit")
                .field("base_revision", &batch.base_revision)
                .field("edit_count", &batch.edits.len())
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
        }
    }
}

/// Redacted update metadata without a plaintext-bearing snapshot or changed ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecureFieldUpdate {
    pub revision: TextRevision,
    pub text_changed: bool,
    pub selection_changed: bool,
    pub composition_changed: bool,
}

impl From<&TextControllerUpdate> for SecureFieldUpdate {
    fn from(update: &TextControllerUpdate) -> Self {
        Self {
            revision: update.snapshot.revision(),
            text_changed: update.changed_text(),
            selection_changed: update.changed_selection(),
            composition_changed: update.changed_composition(),
        }
    }
}

/// Secure outputs never contain controller snapshots or plaintext.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecureFieldOutput {
    Updated(SecureFieldUpdate),
    Submitted(Submitted),
}

/// Dedicated secure-field wrapper with one fixed, inspectable privacy policy.
pub struct SecureField {
    field: TextField,
}

impl fmt::Debug for SecureField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureField")
            .field("revision", &self.field.controller().revision())
            .field("privacy", &SecureFieldPrivacyPolicy::BASELINE)
            .finish_non_exhaustive()
    }
}

impl SecureField {
    pub fn new(
        controller: TextController,
        label: impl Into<String>,
    ) -> Result<Self, SecureFieldError> {
        Ok(Self {
            field: TextField::new(controller, label, TextFieldMode::Secure)
                .map_err(SecureFieldError::Field)?,
        })
    }

    pub fn return_action(mut self, action: TextReturnKeyAction) -> Result<Self, SecureFieldError> {
        self.field = self
            .field
            .return_action(action)
            .map_err(SecureFieldError::Field)?;
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

    pub const fn privacy_policy(&self) -> SecureFieldPrivacyPolicy {
        SecureFieldPrivacyPolicy::BASELINE
    }

    pub const fn revision(&self) -> TextRevision {
        self.field.controller().revision()
    }

    /// Explicit ownership recovery; ordinary diagnostics and outputs expose no content accessor.
    pub fn into_controller(self) -> TextController {
        self.field.into_controller()
    }

    pub fn input_configuration(&self) -> TextInputConfiguration {
        let mut configuration = self.field.input_configuration();
        configuration.correction = TextInputPolicy::Disabled;
        configuration.spelling = TextInputPolicy::Disabled;
        configuration
    }

    pub fn command_availability(&self) -> SecureFieldCommandAvailability {
        let availability = self.field.command_availability();
        SecureFieldCommandAvailability {
            can_edit: availability.can_edit,
            can_select: availability.can_select,
            can_submit: availability.can_submit,
            can_undo: false,
            can_redo: false,
        }
    }

    pub fn context_command_availability(
        &self,
        capabilities: SecureContextCapabilities,
    ) -> SecureContextCommandAvailability {
        let availability = self.command_availability();
        SecureContextCommandAvailability {
            can_select_all: availability.can_select,
            can_paste: availability.can_edit && capabilities.can_read_plain_text,
            can_copy: false,
            can_cut: false,
        }
    }

    pub fn route(
        &mut self,
        command: SecureFieldCommand,
    ) -> Result<SecureFieldOutput, SecureFieldError> {
        let command = match command {
            SecureFieldCommand::Edit { batch } => TextFieldCommand::Edit {
                batch,
                kind: EditHistoryKind::HistoryReset,
                recorded_at: MonotonicInstant::ZERO,
            },
            SecureFieldCommand::SetSelection {
                base_revision,
                selection,
            } => TextFieldCommand::SetSelection {
                base_revision,
                selection,
            },
            SecureFieldCommand::Submit => TextFieldCommand::Submit,
        };
        match self.field.route(command).map_err(SecureFieldError::Field)? {
            TextFieldOutput::Updated(update) => {
                Ok(SecureFieldOutput::Updated(SecureFieldUpdate::from(&update)))
            }
            TextFieldOutput::Submitted(submitted) => Ok(SecureFieldOutput::Submitted(submitted)),
        }
    }

    pub fn semantic_node(&self, name: StringId) -> SemanticNode {
        self.field
            .semantic_node_internal(name, None, false, SemanticRole::TextInput)
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<SecureFieldRef> {
        Ok(SecureFieldRef {
            field: self
                .field
                .mount_internal(ui, host, false, 1, SemanticRole::TextInput)?,
            availability: self.command_availability(),
            privacy: SecureFieldPrivacyPolicy::BASELINE,
        })
    }
}

/// Focused mount-time secure reference containing no plaintext value.
#[derive(Clone, Copy, Debug)]
pub struct SecureFieldRef {
    field: TextFieldRef,
    availability: SecureFieldCommandAvailability,
    privacy: SecureFieldPrivacyPolicy,
}

impl SecureFieldRef {
    pub const fn field(self) -> TextFieldRef {
        self.field
    }

    pub const fn node(self) -> UiNodeId {
        self.field.node()
    }

    pub const fn revision(self) -> TextRevision {
        self.field.revision()
    }

    pub const fn availability(self) -> SecureFieldCommandAvailability {
        self.availability
    }

    pub const fn privacy_policy(self) -> SecureFieldPrivacyPolicy {
        self.privacy
    }
}

#[derive(Debug)]
pub enum SecureFieldError {
    Field(TextFieldError),
}

impl fmt::Display for SecureFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SecureFieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Field(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::text::{TextAffinity, TextEdit, TextOffset, TextRange};
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
    fn construction_enforces_fixed_privacy_input_and_history_policy() {
        let mut controller = TextController::from_text("credential").unwrap();
        controller
            .enable_edit_history(EditHistoryPolicy::default())
            .unwrap();
        let secure = SecureField::new(controller, "Password").unwrap();
        let policy = secure.privacy_policy();
        assert_eq!(policy.diagnostics(), SecureContentExposure::Omitted);
        assert_eq!(policy.semantics(), SecureContentExposure::Omitted);
        assert_eq!(
            policy.visual_capture(),
            SecureContentExposure::RedactedDisplay
        );
        assert!(!policy.retains_plaintext_history());
        assert!(!policy.allows_copy());
        assert!(!policy.allows_cut());
        assert!(!secure.command_availability().can_undo);
        assert!(!secure.command_availability().can_redo);
        assert!(secure.input_configuration().secure_entry);
        assert_eq!(
            secure.input_configuration().correction,
            TextInputPolicy::Disabled
        );
        assert_eq!(
            secure.input_configuration().spelling,
            TextInputPolicy::Disabled
        );
        assert!(!format!("{secure:?}").contains("credential"));
    }

    #[test]
    fn outputs_and_command_debug_never_return_plaintext_snapshots() {
        let mut secure =
            SecureField::new(TextController::from_text("old secret").unwrap(), "Password").unwrap();
        let command = SecureFieldCommand::Edit {
            batch: replacement(TextRevision::INITIAL, 10, "new secret"),
        };
        assert!(!format!("{command:?}").contains("new secret"));
        let output = secure.route(command).unwrap();
        let SecureFieldOutput::Updated(update) = output else {
            panic!("secure edit must publish redacted update metadata")
        };
        assert!(update.text_changed);
        assert_eq!(update.revision, TextRevision(1));
        assert!(!format!("{output:?}").contains("new secret"));

        let controller = secure.into_controller();
        assert_eq!(
            controller
                .snapshot()
                .chunks()
                .map(|chunk| chunk.text)
                .collect::<String>(),
            "new secret"
        );
        assert!(!controller.edit_history_availability().enabled);
    }

    #[test]
    fn context_availability_forbids_copy_cut_and_capability_gates_paste() {
        let secure = SecureField::new(TextController::new(), "Password").unwrap();
        let unavailable = secure.context_command_availability(SecureContextCapabilities::default());
        assert!(unavailable.can_select_all);
        assert!(!unavailable.can_paste);
        assert!(!unavailable.can_copy);
        assert!(!unavailable.can_cut);

        let available = secure.context_command_availability(SecureContextCapabilities {
            can_read_plain_text: true,
        });
        assert!(available.can_paste);
        assert!(!available.can_copy);
        assert!(!available.can_cut);
    }

    #[test]
    fn submission_and_semantics_expose_no_content() {
        let mut secure =
            SecureField::new(TextController::from_text("credential").unwrap(), "Password")
                .unwrap()
                .return_action(TextReturnKeyAction::Done)
                .unwrap();
        let semantic = secure.semantic_node(StringId(1));
        assert_eq!(semantic.value, SemanticValue::None);
        let revision = secure.revision();
        let output = secure.route(SecureFieldCommand::Submit).unwrap();
        let SecureFieldOutput::Submitted(submitted) = output else {
            panic!("secure submission must contain only revision and action")
        };
        assert_eq!(submitted.revision, revision);
        assert_eq!(submitted.action, TextReturnKeyAction::Done);
        assert!(!format!("{output:?}").contains("credential"));
    }

    struct MountedSecure {
        secure: SecureField,
        reference: Rc<Cell<Option<SecureFieldRef>>>,
    }

    impl Component for MountedSecure {
        type State = ();
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.reference
                .set(Some(self.secure.mount(ui, root.0).unwrap()));
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
    fn mounted_secure_field_contains_only_bullets_and_omitted_semantic_value() {
        let reference = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(MountedSecure {
            secure: SecureField::new(TextController::from_text("sëcret").unwrap(), "Password")
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
        assert_eq!(
            runtime.ui().string(
                runtime
                    .ui()
                    .texts
                    .get(reference.field().value_node())
                    .unwrap()
                    .content
            ),
            Some("••••••")
        );
        assert_eq!(
            runtime.ui().semantics.get(reference.node()).unwrap().value,
            SemanticValue::None
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
        assert_eq!(
            reference.privacy_policy().visual_capture(),
            SecureContentExposure::RedactedDisplay
        );
        assert!(!format!("{:?}", runtime.ui()).contains("sëcret"));
    }
}
