//! Stable application field metadata and semantic association.

use std::fmt;

use crate::ui::{
    SemanticAction, SemanticName, SemanticNode, SemanticRelationship, SemanticRelationshipKind,
    StringId, UiNodeId,
};

use super::{FieldValidation, ValidationResult};

/// Stable identity and non-value inputs shared by application fields.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldMetadata<K> {
    key: K,
    label: String,
    help: Option<String>,
    required: bool,
    read_only: bool,
    enabled: bool,
}

impl<K> FieldMetadata<K> {
    pub fn new(key: K, label: impl Into<String>) -> Result<Self, FieldMetadataError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(FieldMetadataError::MissingLabel);
        }
        Ok(Self {
            key,
            label,
            help: None,
            required: false,
            read_only: false,
            enabled: true,
        })
    }

    pub fn help(mut self, help: impl Into<String>) -> Result<Self, FieldMetadataError> {
        let help = help.into();
        if help.trim().is_empty() {
            return Err(FieldMetadataError::MissingHelp);
        }
        self.help = Some(help);
        Ok(self)
    }

    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    pub const fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub const fn key(&self) -> &K {
        &self.key
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn help_text(&self) -> Option<&str> {
        self.help.as_deref()
    }

    pub const fn is_required(&self) -> bool {
        self.required
    }

    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl<K> FieldMetadata<K>
where
    K: Eq,
{
    /// Applies field metadata and validation to component-authored semantics.
    ///
    /// Help and non-valid validation text are mounted by the caller so this low-level descriptor
    /// does not take layout ownership. Their generation-checked nodes are associated here.
    pub fn decorate_semantics(
        &self,
        mut semantic: SemanticNode,
        label: StringId,
        validation: &FieldValidation<K>,
        support: FieldSemanticSupport,
    ) -> Result<SemanticNode, FieldMetadataError> {
        if validation.field() != &self.key {
            return Err(FieldMetadataError::ValidationFieldMismatch);
        }
        match (self.help.is_some(), support.help) {
            (true, None) => return Err(FieldMetadataError::MissingHelpNode),
            (false, Some(_)) => return Err(FieldMetadataError::UnexpectedHelpNode),
            _ => {}
        }
        match (validation.result(), support.validation) {
            (ValidationResult::Valid, Some(_)) => {
                return Err(FieldMetadataError::UnexpectedValidationNode);
            }
            (ValidationResult::Valid, None) => {}
            (_, None) => return Err(FieldMetadataError::MissingValidationNode),
            (_, Some(_)) => {}
        }
        if support.help.is_some() && support.help == support.validation {
            return Err(FieldMetadataError::DuplicateSupportNode);
        }

        semantic.name = SemanticName::Text(label);
        semantic.state.required |= self.required;
        semantic.state.read_only |= self.read_only;
        semantic.state.disabled |= !self.enabled;
        semantic.state.invalid |= validation.result().is_invalid();
        semantic.state.busy |= validation.result().is_pending();
        if self.read_only {
            semantic.actions.remove(SemanticAction::SetValue);
            semantic.actions.remove(SemanticAction::SetText);
            semantic.actions.remove(SemanticAction::Increment);
            semantic.actions.remove(SemanticAction::Decrement);
        }

        if let Some(target) = support.help {
            semantic.relationships.push(SemanticRelationship {
                kind: SemanticRelationshipKind::Help,
                target,
            });
        }
        if let Some(target) = support.validation {
            let kind = if validation.result().is_invalid() {
                SemanticRelationshipKind::ErrorMessage
            } else {
                SemanticRelationshipKind::DescribedBy
            };
            semantic
                .relationships
                .push(SemanticRelationship { kind, target });
        }
        Ok(semantic)
    }
}

/// Mounted help and validation message identities associated with one field semantic node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FieldSemanticSupport {
    pub help: Option<UiNodeId>,
    pub validation: Option<UiNodeId>,
}

impl FieldSemanticSupport {
    pub const fn new(help: Option<UiNodeId>, validation: Option<UiNodeId>) -> Self {
        Self { help, validation }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldMetadataError {
    MissingLabel,
    MissingHelp,
    ValidationFieldMismatch,
    MissingHelpNode,
    UnexpectedHelpNode,
    MissingValidationNode,
    UnexpectedValidationNode,
    DuplicateSupportNode,
}

impl fmt::Display for FieldMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingLabel => "field metadata requires a visible accessible label",
            Self::MissingHelp => "field help text is empty",
            Self::ValidationFieldMismatch => "validation belongs to a different field key",
            Self::MissingHelpNode => "field help text requires a mounted semantic node",
            Self::UnexpectedHelpNode => "field without help text cannot associate a help node",
            Self::MissingValidationNode => {
                "non-valid field validation requires a mounted semantic message node"
            }
            Self::UnexpectedValidationNode => {
                "valid field validation cannot associate a validation message node"
            }
            Self::DuplicateSupportNode => {
                "field help and validation must use distinct semantic nodes"
            }
        })
    }
}

impl std::error::Error for FieldMetadataError {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, Ui, UpdateContext, ViewRuntime};
    use crate::ui::{BoxStyle, LayoutStyle, SemanticRole, SemanticState, SizeRule2D, UiRoot};

    use super::*;
    use crate::components::application::ValidationKind;

    #[test]
    fn metadata_validates_visible_text_and_retains_stable_inputs() {
        assert!(matches!(
            FieldMetadata::new("name", " "),
            Err(FieldMetadataError::MissingLabel)
        ));
        assert!(matches!(
            FieldMetadata::new("name", "Name").unwrap().help(""),
            Err(FieldMetadataError::MissingHelp)
        ));

        let field = FieldMetadata::new("name", "Account name")
            .unwrap()
            .help("Shown to collaborators")
            .unwrap()
            .required(true)
            .read_only(true)
            .enabled(false);
        assert_eq!(field.key(), &"name");
        assert_eq!(field.label(), "Account name");
        assert_eq!(field.help_text(), Some("Shown to collaborators"));
        assert!(field.is_required());
        assert!(field.is_read_only());
        assert!(!field.is_enabled());
    }

    #[test]
    fn semantic_association_rejects_mismatched_or_incomplete_inputs_atomically() {
        let field = FieldMetadata::new("name", "Name")
            .unwrap()
            .help("Public display name")
            .unwrap();
        let invalid =
            FieldValidation::new("other", ValidationResult::invalid("Already used").unwrap());
        assert!(matches!(
            field.decorate_semantics(
                SemanticNode::new(SemanticRole::TextInput),
                StringId(1),
                &invalid,
                FieldSemanticSupport::new(Some(UiNodeId::new(2, 1)), Some(UiNodeId::new(3, 1))),
            ),
            Err(FieldMetadataError::ValidationFieldMismatch)
        ));

        let valid = FieldValidation::new("name", ValidationResult::Valid);
        assert!(matches!(
            field.decorate_semantics(
                SemanticNode::new(SemanticRole::TextInput),
                StringId(1),
                &valid,
                FieldSemanticSupport::default(),
            ),
            Err(FieldMetadataError::MissingHelpNode)
        ));

        let read_only = FieldMetadata::new("name", "Name").unwrap().read_only(true);
        let mut editable_semantics = SemanticNode::new(SemanticRole::TextInput);
        editable_semantics.actions = crate::ui::SemanticActions::SET_TEXT
            | crate::ui::SemanticActions::SET_SELECTION
            | crate::ui::SemanticActions::FOCUS;
        let decorated = read_only
            .decorate_semantics(
                editable_semantics,
                StringId(1),
                &valid,
                FieldSemanticSupport::default(),
            )
            .unwrap();
        assert!(decorated.state.read_only);
        assert!(!decorated.actions.contains(SemanticAction::SetText));
        assert!(decorated.actions.contains(SemanticAction::SetSelection));
    }

    #[derive(Clone, Copy)]
    struct MountedRefs {
        field: UiNodeId,
        help: UiNodeId,
        validation: UiNodeId,
    }

    struct MountedField {
        refs: Rc<Cell<Option<MountedRefs>>>,
    }

    impl Component for MountedField {
        type State = ();
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let mut support_nodes = None;
            ui.foundation()
                .container_node_under(
                    root.0,
                    BoxStyle::default(),
                    LayoutStyle::default(),
                    |writer| {
                        support_nodes = Some((
                            writer
                                .text("Shown to collaborators", Default::default(), 12.0)
                                .node,
                            writer.text("Already in use", Default::default(), 12.0).node,
                        ));
                    },
                )
                .unwrap();
            let (help, validation) = support_nodes.unwrap();
            ui.foundation()
                .semantic_node(help, SemanticNode::new(SemanticRole::Text))
                .unwrap();
            ui.foundation()
                .semantic_node(validation, SemanticNode::new(SemanticRole::Alert))
                .unwrap();
            let field = ui
                .foundation()
                .text_input_node_under(
                    root.0,
                    BoxStyle {
                        min_size: SizeRule2D::default(),
                        ..BoxStyle::default()
                    },
                    LayoutStyle::default(),
                    true,
                    |_| {},
                )
                .unwrap()
                .node;
            let metadata = FieldMetadata::new("account", "Account name")
                .unwrap()
                .help("Shown to collaborators")
                .unwrap()
                .required(true);
            let result = FieldValidation::new(
                "account",
                ValidationResult::invalid("Already in use").unwrap(),
            );
            assert_eq!(result.result().kind(), ValidationKind::Invalid);
            let label = ui.foundation().intern(metadata.label());
            let semantic = metadata
                .decorate_semantics(
                    SemanticNode {
                        role: SemanticRole::TextInput,
                        state: SemanticState {
                            focusable: true,
                            ..SemanticState::default()
                        },
                        ..SemanticNode::default()
                    },
                    label,
                    &result,
                    FieldSemanticSupport::new(Some(help), Some(validation)),
                )
                .unwrap();
            ui.foundation().semantic_node(field, semantic).unwrap();
            self.refs.set(Some(MountedRefs {
                field,
                help,
                validation,
            }));
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
    fn mounted_field_associates_help_and_invalid_text_without_color_only_state() {
        let refs = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(MountedField { refs: refs.clone() }).unwrap();
        let refs = refs.get().unwrap();
        let semantic = runtime.ui().semantics.get(refs.field).unwrap();
        assert!(semantic.state.required);
        assert!(semantic.state.invalid);
        let SemanticName::Text(label) = semantic.name else {
            panic!("field metadata must supply the semantic label");
        };
        assert_eq!(runtime.ui().string(label), Some("Account name"));
        assert!(semantic.relationships.contains(&SemanticRelationship {
            kind: SemanticRelationshipKind::Help,
            target: refs.help,
        }));
        assert!(semantic.relationships.contains(&SemanticRelationship {
            kind: SemanticRelationshipKind::ErrorMessage,
            target: refs.validation,
        }));
    }
}
