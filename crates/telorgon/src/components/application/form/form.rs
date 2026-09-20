//! Canonical application form ordering and declarative submission outcomes.
//!
//! `Form` owns field order only. Values, validation execution, focus, scrolling, summaries, and
//! platform behavior remain with their existing or later owners.

use std::fmt;

use crate::ui::layout::RevealAlignment;

use super::{FieldMetadata, FieldValidation, ValidationKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormDiagnostics {
    pub updates: u64,
    pub unchanged_updates: u64,
    pub submissions: u64,
    pub accepted_submissions: u64,
    pub invalid_submissions: u64,
    pub failures: u64,
}

/// Atomic controlled field/validation snapshot update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormUpdate<K> {
    previous_order: Vec<K>,
    order: Vec<K>,
    changed: bool,
    revision: u64,
}

impl<K> FormUpdate<K> {
    pub fn previous_order(&self) -> &[K] {
        &self.previous_order
    }

    pub fn order(&self) -> &[K] {
        &self.order
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

/// Caller-applied focus request for one invalid field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormFocusIntent<K> {
    field: K,
}

impl<K> FormFocusIntent<K> {
    pub(crate) const fn new(field: K) -> Self {
        Self { field }
    }

    pub const fn field(&self) -> &K {
        &self.field
    }
}

/// Caller-applied nearest-edge reveal request for one invalid field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormRevealIntent<K> {
    field: K,
}

impl<K> FormRevealIntent<K> {
    pub(crate) const fn new(field: K) -> Self {
        Self { field }
    }

    pub const fn field(&self) -> &K {
        &self.field
    }

    pub const fn alignment(&self) -> RevealAlignment {
        RevealAlignment::Nearest
    }
}

/// Submission rejected by the first invalid field in canonical form order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormInvalidSubmission<K> {
    revision: u64,
    canonical_index: usize,
    field: K,
    focus: FormFocusIntent<K>,
    reveal: FormRevealIntent<K>,
}

impl<K> FormInvalidSubmission<K> {
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn canonical_index(&self) -> usize {
        self.canonical_index
    }

    pub const fn field(&self) -> &K {
        &self.field
    }

    pub const fn focus(&self) -> &FormFocusIntent<K> {
        &self.focus
    }

    pub const fn reveal(&self) -> &FormRevealIntent<K> {
        &self.reveal
    }
}

/// Submission with no invalid fields; warning and pending keys remain explicit for caller policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormAcceptedSubmission<K> {
    revision: u64,
    warning_fields: Vec<K>,
    pending_fields: Vec<K>,
}

impl<K> FormAcceptedSubmission<K> {
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn warning_fields(&self) -> &[K] {
        &self.warning_fields
    }

    pub fn pending_fields(&self) -> &[K] {
        &self.pending_fields
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormSubmission<K> {
    Accepted(FormAcceptedSubmission<K>),
    Invalid(FormInvalidSubmission<K>),
}

/// One canonical stable-field order over an exact controlled validation snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form<K> {
    fields: Vec<FieldMetadata<K>>,
    validations: Vec<FieldValidation<K>>,
    revision: u64,
    diagnostics: FormDiagnostics,
}

impl<K> Form<K>
where
    K: Clone + Eq,
{
    pub fn new(
        fields: impl IntoIterator<Item = FieldMetadata<K>>,
        validations: impl IntoIterator<Item = FieldValidation<K>>,
    ) -> Result<Self, FormError<K>> {
        let fields: Vec<_> = fields.into_iter().collect();
        let validations = validate_snapshot(&fields, validations.into_iter().collect())?;
        Ok(Self {
            fields,
            validations,
            revision: 1,
            diagnostics: FormDiagnostics::default(),
        })
    }

    pub fn fields(&self) -> &[FieldMetadata<K>] {
        &self.fields
    }

    pub fn order(&self) -> Vec<K> {
        self.fields
            .iter()
            .map(|field| field.key().clone())
            .collect()
    }

    pub fn validations(&self) -> &[FieldValidation<K>] {
        &self.validations
    }

    pub fn validation(&self, key: &K) -> Option<&FieldValidation<K>> {
        self.validations
            .iter()
            .find(|validation| validation.field() == key)
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn diagnostics(&self) -> FormDiagnostics {
        self.diagnostics
    }

    /// Replaces metadata order and controlled validation together after complete validation.
    pub fn update(
        &mut self,
        fields: impl IntoIterator<Item = FieldMetadata<K>>,
        validations: impl IntoIterator<Item = FieldValidation<K>>,
    ) -> Result<FormUpdate<K>, FormError<K>> {
        let fields: Vec<_> = fields.into_iter().collect();
        let validations = validate_snapshot(&fields, validations.into_iter().collect())
            .inspect_err(|_| self.diagnostics.failures += 1)?;
        let previous_order = self.order();
        let order: Vec<_> = fields.iter().map(|field| field.key().clone()).collect();
        let changed = fields != self.fields || validations != self.validations;
        let revision = if changed {
            self.revision
                .checked_add(1)
                .ok_or(FormError::RevisionOverflow)?
        } else {
            self.revision
        };

        self.diagnostics.updates += 1;
        if changed {
            self.fields = fields;
            self.validations = validations;
            self.revision = revision;
        } else {
            self.diagnostics.unchanged_updates += 1;
        }
        Ok(FormUpdate {
            previous_order,
            order,
            changed,
            revision,
        })
    }

    /// Inspects the controlled snapshot without executing validation, focus, or scrolling.
    pub fn submit(&mut self) -> FormSubmission<K> {
        self.diagnostics.submissions += 1;
        if let Some((canonical_index, validation)) = self
            .validations
            .iter()
            .enumerate()
            .find(|(_, validation)| validation.result().kind() == ValidationKind::Invalid)
        {
            self.diagnostics.invalid_submissions += 1;
            let field = validation.field().clone();
            return FormSubmission::Invalid(FormInvalidSubmission {
                revision: self.revision,
                canonical_index,
                field: field.clone(),
                focus: FormFocusIntent::new(field.clone()),
                reveal: FormRevealIntent::new(field),
            });
        }

        self.diagnostics.accepted_submissions += 1;
        let warning_fields = self
            .validations
            .iter()
            .filter(|validation| validation.result().kind() == ValidationKind::Warning)
            .map(|validation| validation.field().clone())
            .collect();
        let pending_fields = self
            .validations
            .iter()
            .filter(|validation| validation.result().kind() == ValidationKind::Pending)
            .map(|validation| validation.field().clone())
            .collect();
        FormSubmission::Accepted(FormAcceptedSubmission {
            revision: self.revision,
            warning_fields,
            pending_fields,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormError<K> {
    DuplicateField(K),
    DuplicateValidation(K),
    UnknownValidation(K),
    MissingValidation(K),
    RevisionOverflow,
}

impl<K> fmt::Display for FormError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DuplicateField(_) => "form field keys must be unique",
            Self::DuplicateValidation(_) => "form validation keys must be unique",
            Self::UnknownValidation(_) => "form validation references an unknown field",
            Self::MissingValidation(_) => "every form field requires a controlled validation input",
            Self::RevisionOverflow => "form revision overflow",
        })
    }
}

impl<K> std::error::Error for FormError<K> where K: fmt::Debug {}

fn validate_snapshot<K>(
    fields: &[FieldMetadata<K>],
    validations: Vec<FieldValidation<K>>,
) -> Result<Vec<FieldValidation<K>>, FormError<K>>
where
    K: Clone + Eq,
{
    for (index, field) in fields.iter().enumerate() {
        if fields[..index]
            .iter()
            .any(|candidate| candidate.key() == field.key())
        {
            return Err(FormError::DuplicateField(field.key().clone()));
        }
    }
    for (index, validation) in validations.iter().enumerate() {
        if validations[..index]
            .iter()
            .any(|candidate| candidate.field() == validation.field())
        {
            return Err(FormError::DuplicateValidation(validation.field().clone()));
        }
        if !fields.iter().any(|field| field.key() == validation.field()) {
            return Err(FormError::UnknownValidation(validation.field().clone()));
        }
    }

    let mut canonical = Vec::with_capacity(fields.len());
    for field in fields {
        let validation = validations
            .iter()
            .find(|validation| validation.field() == field.key())
            .ok_or_else(|| FormError::MissingValidation(field.key().clone()))?;
        canonical.push(validation.clone());
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::application::ValidationResult;

    fn field(key: &'static str) -> FieldMetadata<&'static str> {
        FieldMetadata::new(key, key).unwrap()
    }

    fn validation(key: &'static str, result: ValidationResult) -> FieldValidation<&'static str> {
        FieldValidation::new(key, result)
    }

    #[test]
    fn construction_requires_unique_fields_and_one_exact_validation_per_field() {
        assert!(matches!(
            Form::new(
                [field("name"), field("name")],
                [validation("name", ValidationResult::Valid)]
            ),
            Err(FormError::DuplicateField("name"))
        ));
        assert!(matches!(
            Form::new(
                [field("name")],
                [
                    validation("name", ValidationResult::Valid),
                    validation("name", ValidationResult::Valid)
                ]
            ),
            Err(FormError::DuplicateValidation("name"))
        ));
        assert!(matches!(
            Form::new(
                [field("name")],
                [validation("other", ValidationResult::Valid)]
            ),
            Err(FormError::UnknownValidation("other"))
        ));
        assert!(matches!(
            Form::new(
                [field("name"), field("email")],
                [validation("name", ValidationResult::Valid)]
            ),
            Err(FormError::MissingValidation("email"))
        ));
    }

    #[test]
    fn validation_inputs_are_canonicalized_to_stable_field_order() {
        let form = Form::new(
            [field("name"), field("email")],
            [
                validation("email", ValidationResult::Valid),
                validation(
                    "name",
                    ValidationResult::warning("Choose a recognizable name").unwrap(),
                ),
            ],
        )
        .unwrap();
        assert_eq!(form.order(), ["name", "email"]);
        assert_eq!(form.validations()[0].field(), &"name");
        assert_eq!(form.validations()[1].field(), &"email");
    }

    #[test]
    fn updates_are_atomic_revisioned_and_report_unchanged_snapshots() {
        let mut form = Form::new(
            [field("name")],
            [validation("name", ValidationResult::Valid)],
        )
        .unwrap();
        let unchanged = form
            .update(
                [field("name")],
                [validation("name", ValidationResult::Valid)],
            )
            .unwrap();
        assert!(!unchanged.changed());
        assert_eq!(unchanged.revision(), 1);

        let before = form.clone();
        assert!(matches!(
            form.update(
                [field("name")],
                [validation("other", ValidationResult::Valid)]
            ),
            Err(FormError::UnknownValidation("other"))
        ));
        assert_eq!(form.fields(), before.fields());
        assert_eq!(form.validations(), before.validations());
        assert_eq!(form.revision(), before.revision());

        let changed = form
            .update(
                [field("email"), field("name")],
                [
                    validation("name", ValidationResult::Valid),
                    validation("email", ValidationResult::Valid),
                ],
            )
            .unwrap();
        assert!(changed.changed());
        assert_eq!(changed.previous_order(), ["name"]);
        assert_eq!(changed.order(), ["email", "name"]);
        assert_eq!(changed.revision(), 2);
    }

    #[test]
    fn submission_targets_the_first_invalid_field_in_canonical_order() {
        let mut form = Form::new(
            [field("name"), field("email"), field("region")],
            [
                validation(
                    "region",
                    ValidationResult::invalid("Choose a region").unwrap(),
                ),
                validation(
                    "email",
                    ValidationResult::invalid("Enter an email").unwrap(),
                ),
                validation("name", ValidationResult::Valid),
            ],
        )
        .unwrap();
        let FormSubmission::Invalid(invalid) = form.submit() else {
            panic!("the first canonical invalid field must reject submission");
        };
        assert_eq!(invalid.field(), &"email");
        assert_eq!(invalid.canonical_index(), 1);
        assert_eq!(invalid.focus().field(), &"email");
        assert_eq!(invalid.reveal().field(), &"email");
        assert_eq!(invalid.reveal().alignment(), RevealAlignment::Nearest);
        assert_eq!(form.diagnostics().invalid_submissions, 1);
    }

    #[test]
    fn accepted_submission_preserves_warning_and_pending_keys_for_caller_policy() {
        let mut form = Form::new(
            [field("name"), field("email"), field("region")],
            [
                validation("name", ValidationResult::warning("Unusual name").unwrap()),
                validation(
                    "email",
                    ValidationResult::pending("Checking email").unwrap(),
                ),
                validation("region", ValidationResult::Valid),
            ],
        )
        .unwrap();
        let FormSubmission::Accepted(accepted) = form.submit() else {
            panic!("warning and pending policy remains with the caller");
        };
        assert_eq!(accepted.warning_fields(), ["name"]);
        assert_eq!(accepted.pending_fields(), ["email"]);
        assert_eq!(accepted.revision(), form.revision());
        assert_eq!(form.diagnostics().accepted_submissions, 1);
    }
}
