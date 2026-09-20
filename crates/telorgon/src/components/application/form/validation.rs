//! Typed, field-associated validation results.

use std::fmt;

/// The kind of one application field validation result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ValidationKind {
    #[default]
    Valid,
    Warning,
    Invalid,
    Pending,
}

/// Validated visible and assistive text for a non-valid field state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ValidationMessage(String);

impl ValidationMessage {
    pub fn new(message: impl Into<String>) -> Result<Self, ValidationResultError> {
        let message = message.into();
        if message.trim().is_empty() {
            Err(ValidationResultError::MissingMessage)
        } else {
            Ok(Self(message))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A typed validation result with visible, assistive text for every non-valid state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ValidationResult {
    #[default]
    Valid,
    Warning(ValidationMessage),
    Invalid(ValidationMessage),
    Pending(ValidationMessage),
}

impl ValidationResult {
    pub fn warning(message: impl Into<String>) -> Result<Self, ValidationResultError> {
        Ok(Self::Warning(ValidationMessage::new(message)?))
    }

    pub fn invalid(message: impl Into<String>) -> Result<Self, ValidationResultError> {
        Ok(Self::Invalid(ValidationMessage::new(message)?))
    }

    pub fn pending(message: impl Into<String>) -> Result<Self, ValidationResultError> {
        Ok(Self::Pending(ValidationMessage::new(message)?))
    }

    pub const fn kind(&self) -> ValidationKind {
        match self {
            Self::Valid => ValidationKind::Valid,
            Self::Warning(_) => ValidationKind::Warning,
            Self::Invalid(_) => ValidationKind::Invalid,
            Self::Pending(_) => ValidationKind::Pending,
        }
    }

    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Valid => None,
            Self::Warning(message) | Self::Invalid(message) | Self::Pending(message) => {
                Some(message.as_str())
            }
        }
    }

    pub const fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid(_))
    }

    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending(_))
    }
}

/// One validation input explicitly associated with a stable field key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldValidation<K> {
    field: K,
    result: ValidationResult,
}

impl<K> FieldValidation<K> {
    pub const fn new(field: K, result: ValidationResult) -> Self {
        Self { field, result }
    }

    pub const fn field(&self) -> &K {
        &self.field
    }

    pub const fn result(&self) -> &ValidationResult {
        &self.result
    }

    pub fn into_result(self) -> ValidationResult {
        self.result
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationResultError {
    MissingMessage,
}

impl fmt::Display for ValidationResultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("non-valid field validation requires a visible message")
    }
}

impl std::error::Error for ValidationResultError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_non_valid_result_requires_text_and_reports_its_kind() {
        assert_eq!(ValidationResult::Valid.kind(), ValidationKind::Valid);
        assert!(matches!(
            ValidationResult::warning(" "),
            Err(ValidationResultError::MissingMessage)
        ));

        let warning = ValidationResult::warning("Check this value").unwrap();
        let invalid = ValidationResult::invalid("A value is required").unwrap();
        let pending = ValidationResult::pending("Checking availability").unwrap();
        assert_eq!(warning.kind(), ValidationKind::Warning);
        assert_eq!(warning.message(), Some("Check this value"));
        assert!(invalid.is_invalid());
        assert!(pending.is_pending());
    }

    #[test]
    fn validation_retains_the_stable_field_association() {
        let validation = FieldValidation::new(
            "account-name",
            ValidationResult::invalid("Already in use").unwrap(),
        );
        assert_eq!(validation.field(), &"account-name");
        assert_eq!(validation.result().kind(), ValidationKind::Invalid);
    }
}
