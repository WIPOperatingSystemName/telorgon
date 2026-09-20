//! Mounted validation summary derived from one canonical [`Form`].

use std::fmt;
use std::rc::Rc;

use crate::foundation::ColorRgba8;
use crate::input::ChangeSource;
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, Property, SemanticActions, SemanticCollection,
    SemanticName, SemanticNode, SemanticRelationship, SemanticRelationshipKind, SemanticRole,
    SizeRule, SizeRule2D, UiNodeId,
};

use super::{Form, FormFocusIntent, FormRevealIntent, ValidationKind};
use crate::components::application::{DensityClass, DensityMetrics};

/// One non-valid field projected from a canonical form snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationSummaryEntry<K> {
    field: K,
    label: String,
    kind: ValidationKind,
    message: String,
    canonical_index: usize,
}

impl<K> ValidationSummaryEntry<K> {
    pub const fn field(&self) -> &K {
        &self.field
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn kind(&self) -> ValidationKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub const fn canonical_index(&self) -> usize {
        self.canonical_index
    }
}

/// Source-preserving request to focus and reveal one summary entry's field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationSummaryAction<K> {
    form_revision: u64,
    kind: ValidationKind,
    source: ChangeSource,
    focus: FormFocusIntent<K>,
    reveal: FormRevealIntent<K>,
}

impl<K> ValidationSummaryAction<K> {
    pub const fn form_revision(&self) -> u64 {
        self.form_revision
    }

    pub const fn kind(&self) -> ValidationKind {
        self.kind
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub const fn focus(&self) -> &FormFocusIntent<K> {
        &self.focus
    }

    pub const fn reveal(&self) -> &FormRevealIntent<K> {
        &self.reveal
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidationSummaryStyle {
    pub container: BoxStyle,
    pub entry: BoxStyle,
    pub label_color: ColorRgba8,
    pub message_color: ColorRgba8,
    pub label_size: f32,
    pub message_size: f32,
    pub gap: f32,
}

impl Default for ValidationSummaryStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            entry: BoxStyle::default(),
            label_color: ColorRgba8::rgba(248, 249, 252, 255),
            message_color: ColorRgba8::rgba(222, 225, 232, 255),
            label_size: 14.0,
            message_size: 13.0,
            gap: 4.0,
        }
    }
}

/// Ordered, non-owning projection of non-valid fields from one form revision.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidationSummary<K> {
    label: String,
    form_revision: u64,
    entries: Vec<ValidationSummaryEntry<K>>,
    density: DensityClass,
    style: ValidationSummaryStyle,
}

impl<K> ValidationSummary<K>
where
    K: Clone + Eq,
{
    pub fn new(
        label: impl Into<String>,
        form: &Form<K>,
    ) -> Result<Self, ValidationSummaryError<K>> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ValidationSummaryError::MissingAccessibleName);
        }
        let entries = form
            .fields()
            .iter()
            .zip(form.validations())
            .enumerate()
            .filter_map(|(canonical_index, (field, validation))| {
                validation
                    .result()
                    .message()
                    .map(|message| ValidationSummaryEntry {
                        field: field.key().clone(),
                        label: field.label().to_owned(),
                        kind: validation.result().kind(),
                        message: message.to_owned(),
                        canonical_index,
                    })
            })
            .collect();
        Ok(Self {
            label,
            form_revision: form.revision(),
            entries,
            density: DensityClass::Standard,
            style: ValidationSummaryStyle::default(),
        })
    }

    pub const fn density(mut self, density: DensityClass) -> Self {
        self.density = density;
        self
    }

    pub const fn style(mut self, style: ValidationSummaryStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn form_revision(&self) -> u64 {
        self.form_revision
    }

    pub fn entries(&self) -> &[ValidationSummaryEntry<K>] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn activate(
        &self,
        field: &K,
        source: ChangeSource,
    ) -> Result<ValidationSummaryAction<K>, ValidationSummaryError<K>> {
        let entry = self
            .entries
            .iter()
            .find(|entry| &entry.field == field)
            .ok_or_else(|| ValidationSummaryError::UnknownEntry(field.clone()))?;
        Ok(entry.action(self.form_revision, source))
    }

    pub fn semantic_role(&self) -> SemanticRole {
        if self
            .entries
            .iter()
            .any(|entry| entry.kind == ValidationKind::Invalid)
        {
            SemanticRole::Alert
        } else {
            SemanticRole::Status
        }
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<ValidationSummaryRef<K>>
    where
        K: 'static,
        Action: 'static,
        Map: Fn(ValidationSummaryAction<K>) -> Action + 'static,
    {
        let minimum = DensityMetrics::baseline(self.density).effective_minimum();
        let mut entry_style = self.style.entry;
        entry_style.min_size = SizeRule2D {
            width: SizeRule::Logical(minimum.width()),
            height: SizeRule::Logical(minimum.height()),
        };
        let mut mounted = Vec::with_capacity(self.entries.len());
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
                    writer.text(&self.label, self.style.label_color, self.style.label_size);
                    for (summary_index, entry) in self.entries.iter().enumerate() {
                        let visible = format!("{}: {}", entry.label, entry.message);
                        let control = writer.action_node(entry_style, false, |writer| {
                            writer.text(
                                &visible,
                                self.style.message_color,
                                self.style.message_size,
                            );
                        });
                        mounted.push((summary_index, entry.clone(), control));
                    }
                },
            )
            .ok_or_else(|| RuntimeError::new("validation-summary host is stale"))?;

        let map = Rc::new(map);
        let mut entry_refs = Vec::with_capacity(mounted.len());
        for (summary_index, entry, control) in mounted {
            let name = ui.foundation().intern(&entry.label);
            let description = ui.foundation().intern(&entry.message);
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        role: SemanticRole::Link,
                        name: SemanticName::Text(name),
                        description: Some(description),
                        actions: SemanticActions::ACTIVATE,
                        collection: Some(SemanticCollection {
                            item_index: u32::try_from(summary_index).ok(),
                            item_count: u32::try_from(self.entries.len()).ok(),
                            ..SemanticCollection::default()
                        }),
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid validation-summary entry: {error:?}"))
                })?;
            if entry.kind == ValidationKind::Invalid {
                ui.foundation().invalid(control.node, true);
            }
            let routed_entry = entry.clone();
            let map = map.clone();
            let revision = self.form_revision;
            ui.route_activation(control.node, move |activation| {
                map(routed_entry.action(revision, activation.source))
            })?;
            entry_refs.push(ValidationSummaryEntryRef {
                field: entry.field,
                kind: entry.kind,
                canonical_index: entry.canonical_index,
                control,
            });
        }

        let name = ui.foundation().intern(&self.label);
        let relationships = entry_refs
            .iter()
            .map(|entry| SemanticRelationship {
                kind: SemanticRelationshipKind::Owns,
                target: entry.control.node,
            })
            .collect();
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: self.semantic_role(),
                    name: SemanticName::Text(name),
                    relationships,
                    collection: Some(SemanticCollection {
                        item_count: u32::try_from(entry_refs.len()).ok(),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid validation-summary semantics: {error:?}"))
            })?;
        Ok(ValidationSummaryRef {
            control: root,
            form_revision: self.form_revision,
            entries: entry_refs,
        })
    }
}

impl<K> ValidationSummaryEntry<K>
where
    K: Clone,
{
    fn action(&self, form_revision: u64, source: ChangeSource) -> ValidationSummaryAction<K> {
        ValidationSummaryAction {
            form_revision,
            kind: self.kind,
            source,
            focus: FormFocusIntent::new(self.field.clone()),
            reveal: FormRevealIntent::new(self.field.clone()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ValidationSummaryRef<K> {
    control: ControlHandle,
    form_revision: u64,
    entries: Vec<ValidationSummaryEntryRef<K>>,
}

impl<K> ValidationSummaryRef<K> {
    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn form_revision(&self) -> u64 {
        self.form_revision
    }

    pub fn entries(&self) -> &[ValidationSummaryEntryRef<K>] {
        &self.entries
    }
}

#[derive(Clone, Debug)]
pub struct ValidationSummaryEntryRef<K> {
    field: K,
    kind: ValidationKind,
    canonical_index: usize,
    control: ControlHandle,
}

impl<K> ValidationSummaryEntryRef<K> {
    pub const fn field(&self) -> &K {
        &self.field
    }

    pub const fn kind(&self) -> ValidationKind {
        self.kind
    }

    pub const fn canonical_index(&self) -> usize {
        self.canonical_index
    }

    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationSummaryError<K> {
    MissingAccessibleName,
    UnknownEntry(K),
}

impl<K> fmt::Display for ValidationSummaryError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingAccessibleName => "validation summary requires an accessible name",
            Self::UnknownEntry(_) => "validation summary entry is not in the derived form snapshot",
        })
    }
}

impl<K> std::error::Error for ValidationSummaryError<K> where K: fmt::Debug {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::ui::layout::RevealAlignment;
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{NodeKind, UiRoot};

    use super::*;
    use crate::components::application::{FieldMetadata, FieldValidation, ValidationResult};

    fn form() -> Form<&'static str> {
        Form::new(
            [
                FieldMetadata::new("name", "Name").unwrap(),
                FieldMetadata::new("email", "Email").unwrap(),
                FieldMetadata::new("region", "Region").unwrap(),
                FieldMetadata::new("code", "Code").unwrap(),
            ],
            [
                FieldValidation::new("code", ValidationResult::Valid),
                FieldValidation::new(
                    "region",
                    ValidationResult::pending("Checking region").unwrap(),
                ),
                FieldValidation::new(
                    "email",
                    ValidationResult::invalid("Enter an email").unwrap(),
                ),
                FieldValidation::new("name", ValidationResult::warning("Unusual name").unwrap()),
            ],
        )
        .unwrap()
    }

    #[test]
    fn summary_derives_only_non_valid_entries_in_canonical_form_order() {
        let summary = ValidationSummary::new("Review fields", &form()).unwrap();
        assert_eq!(summary.entries().len(), 3);
        assert_eq!(summary.entries()[0].field(), &"name");
        assert_eq!(summary.entries()[0].kind(), ValidationKind::Warning);
        assert_eq!(summary.entries()[1].field(), &"email");
        assert_eq!(summary.entries()[1].canonical_index(), 1);
        assert_eq!(summary.entries()[2].field(), &"region");
        assert_eq!(summary.semantic_role(), SemanticRole::Alert);
    }

    #[test]
    fn entry_action_reuses_form_focus_and_reveal_intents_without_executing_them() {
        let summary = ValidationSummary::new("Review fields", &form()).unwrap();
        let action = summary
            .activate(&"email", ChangeSource::Accessibility)
            .unwrap();
        assert_eq!(action.form_revision(), 1);
        assert_eq!(action.kind(), ValidationKind::Invalid);
        assert_eq!(action.source(), ChangeSource::Accessibility);
        assert_eq!(action.focus().field(), &"email");
        assert_eq!(action.reveal().field(), &"email");
        assert_eq!(action.reveal().alignment(), RevealAlignment::Nearest);
        assert!(matches!(
            summary.activate(&"missing", ChangeSource::Programmatic),
            Err(ValidationSummaryError::UnknownEntry("missing"))
        ));
    }

    struct MountedSummary {
        reference: Rc<RefCell<Option<ValidationSummaryRef<&'static str>>>>,
    }

    impl Component for MountedSummary {
        type State = ();
        type Action = ValidationSummaryAction<&'static str>;

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let summary = ValidationSummary::new("Review fields", &form())
                .unwrap()
                .density(DensityClass::Touch);
            *self.reference.borrow_mut() =
                Some(summary.mount(ui, root.0, |action| action).unwrap());
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
    fn mounted_summary_exposes_visible_status_text_semantics_and_touch_targets() {
        let reference = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(MountedSummary {
            reference: reference.clone(),
        })
        .unwrap();
        let reference = reference.borrow();
        let reference = reference.as_ref().unwrap();
        assert_eq!(
            runtime.ui().kinds.get(reference.node()),
            Some(&NodeKind::Box)
        );
        let root = runtime.ui().semantics.get(reference.node()).unwrap();
        assert_eq!(root.role, SemanticRole::Alert);
        assert_eq!(root.relationships.len(), 3);
        for entry in reference.entries() {
            let semantic = runtime.ui().semantics.get(entry.node()).unwrap();
            assert_eq!(semantic.role, SemanticRole::Link);
            assert!(semantic.description.is_some());
            assert!(
                semantic
                    .actions
                    .contains(crate::ui::SemanticAction::Activate)
            );
            assert_eq!(
                runtime
                    .ui()
                    .box_styles
                    .get(entry.node())
                    .unwrap()
                    .min_size
                    .height,
                SizeRule::Logical(44.0)
            );
        }
    }
}
