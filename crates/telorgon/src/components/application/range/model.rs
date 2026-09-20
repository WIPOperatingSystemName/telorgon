//! Validated finite range values shared by application range controls.

use std::fmt::Debug;

/// Scalar conversion required by range math.
///
/// Implementations must preserve finite values through `to_f64`/`from_f64` closely enough for
/// comparison and step normalization. Telorgon supplies implementations for `f32` and `f64`.
pub trait RangeScalar: Copy + Debug + PartialEq + 'static {
    fn to_f64(self) -> f64;
    fn from_f64(value: f64) -> Option<Self>;
}

impl RangeScalar for f32 {
    fn to_f64(self) -> f64 {
        f64::from(self)
    }

    fn from_f64(value: f64) -> Option<Self> {
        let value = value as f32;
        value.is_finite().then_some(value)
    }
}

impl RangeScalar for f64 {
    fn to_f64(self) -> f64 {
        self
    }

    fn from_f64(value: f64) -> Option<Self> {
        value.is_finite().then_some(value)
    }
}

/// Deterministic text formatting for range values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RangeFormat {
    fraction_digits: u8,
    prefix: String,
    suffix: String,
}

impl RangeFormat {
    pub const MAX_FRACTION_DIGITS: u8 = 15;

    pub fn new(fraction_digits: u8) -> Result<Self, RangeModelError> {
        if fraction_digits > Self::MAX_FRACTION_DIGITS {
            return Err(RangeModelError::InvalidFormatPrecision {
                requested: fraction_digits,
                maximum: Self::MAX_FRACTION_DIGITS,
            });
        }
        Ok(Self {
            fraction_digits,
            prefix: String::new(),
            suffix: String::new(),
        })
    }

    pub fn prefix(mut self, prefix: impl Into<String>) -> Result<Self, RangeModelError> {
        let prefix = prefix.into();
        validate_affix(&prefix, RangeAffix::Prefix)?;
        self.prefix = prefix;
        Ok(self)
    }

    pub fn suffix(mut self, suffix: impl Into<String>) -> Result<Self, RangeModelError> {
        let suffix = suffix.into();
        validate_affix(&suffix, RangeAffix::Suffix)?;
        self.suffix = suffix;
        Ok(self)
    }

    pub const fn fraction_digits(&self) -> u8 {
        self.fraction_digits
    }

    pub fn prefix_text(&self) -> &str {
        &self.prefix
    }

    pub fn suffix_text(&self) -> &str {
        &self.suffix
    }

    fn format(&self, value: f64) -> String {
        let value = if value == 0.0 { 0.0 } else { value };
        format!(
            "{}{:.*}{}",
            self.prefix,
            usize::from(self.fraction_digits),
            value,
            self.suffix
        )
    }
}

/// Optional labelled position authored into a range model.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeMark<T> {
    value: T,
    label: Option<String>,
}

impl<T> RangeMark<T> {
    pub const fn new(value: T) -> Self {
        Self { value, label: None }
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub const fn value(&self) -> &T {
        &self.value
    }

    pub fn label_text(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

/// Finite ordered bounds, step policy, formatting, and marks for application range controls.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeModel<T> {
    minimum: T,
    maximum: T,
    step: T,
    page_step: T,
    format: RangeFormat,
    marks: Vec<RangeMark<T>>,
}

impl<T> RangeModel<T>
where
    T: RangeScalar,
{
    pub fn new(minimum: T, maximum: T, step: T, page_step: T) -> Result<Self, RangeModelError> {
        let minimum_number = finite(minimum, RangeNumber::Minimum)?;
        let maximum_number = finite(maximum, RangeNumber::Maximum)?;
        let step_number = finite(step, RangeNumber::Step)?;
        let page_step_number = finite(page_step, RangeNumber::PageStep)?;
        if minimum_number >= maximum_number {
            return Err(RangeModelError::UnorderedBounds);
        }
        if step_number <= 0.0 {
            return Err(RangeModelError::NonPositiveStep);
        }
        if page_step_number <= 0.0 {
            return Err(RangeModelError::NonPositivePageStep);
        }
        Ok(Self {
            minimum,
            maximum,
            step,
            page_step,
            format: RangeFormat::default(),
            marks: Vec::new(),
        })
    }

    pub fn with_format(mut self, format: RangeFormat) -> Self {
        self.format = format;
        self
    }

    pub fn with_marks(
        mut self,
        marks: impl IntoIterator<Item = RangeMark<T>>,
    ) -> Result<Self, RangeModelError> {
        let marks: Vec<_> = marks.into_iter().collect();
        validate_marks(&marks, self.minimum.to_f64(), self.maximum.to_f64())?;
        self.marks = marks;
        Ok(self)
    }

    pub const fn minimum(&self) -> T {
        self.minimum
    }

    pub const fn maximum(&self) -> T {
        self.maximum
    }

    pub const fn step(&self) -> T {
        self.step
    }

    pub const fn page_step(&self) -> T {
        self.page_step
    }

    pub const fn format(&self) -> &RangeFormat {
        &self.format
    }

    pub fn marks(&self) -> &[RangeMark<T>] {
        &self.marks
    }

    /// Clamps a finite value to the authored bounds without applying step normalization.
    pub fn clamp(&self, value: T) -> Result<T, RangeModelError> {
        let value_number = finite(value, RangeNumber::Value)?;
        if value_number <= self.minimum.to_f64() {
            Ok(self.minimum)
        } else if value_number >= self.maximum.to_f64() {
            Ok(self.maximum)
        } else {
            Ok(value)
        }
    }

    /// Returns the nearest valid step position, treating both explicit bounds as reachable values.
    pub fn normalize(&self, value: T) -> Result<T, RangeModelError> {
        let value = self.clamp(value)?;
        if value == self.minimum || value == self.maximum {
            return Ok(value);
        }

        let minimum = self.minimum.to_f64();
        let maximum = self.maximum.to_f64();
        let value = value.to_f64();
        let step = self.step.to_f64();
        let units = ((value - minimum) / step).round();
        let stepped = (minimum + units * step).clamp(minimum, maximum);
        let normalized = nearest(value, [minimum, stepped, maximum]);
        T::from_f64(normalized).ok_or(RangeModelError::UnrepresentableValue)
    }

    /// Moves across the discrete step positions while keeping an unaligned explicit maximum
    /// reachable as its own final position.
    pub fn step_by(&self, value: T, steps: i64) -> Result<T, RangeModelError> {
        let current = self.normalize(value)?.to_f64();
        if steps == 0 {
            return T::from_f64(current).ok_or(RangeModelError::UnrepresentableValue);
        }
        let minimum = self.minimum.to_f64();
        let maximum = self.maximum.to_f64();
        let step = self.step.to_f64();
        let quotient = (maximum - minimum) / step;
        let aligned_maximum =
            (quotient - quotient.round()).abs() <= f64::EPSILON * 16.0 * quotient.abs().max(1.0);
        let current_index = if current == maximum && !aligned_maximum {
            quotient.floor() + 1.0
        } else {
            ((current - minimum) / step).round()
        };
        let target_index = current_index + steps as f64;
        let candidate = if target_index <= 0.0 {
            minimum
        } else if target_index >= quotient.ceil() {
            maximum
        } else {
            minimum + target_index * step
        };
        T::from_f64(candidate.clamp(minimum, maximum)).ok_or(RangeModelError::UnrepresentableValue)
    }

    /// Applies page-sized movement, then normalizes the result to a valid step or endpoint.
    pub fn page_by(&self, value: T, pages: i64) -> Result<T, RangeModelError> {
        let current = self.normalize(value)?.to_f64();
        let minimum = self.minimum.to_f64();
        let maximum = self.maximum.to_f64();
        let candidate = current + pages as f64 * self.page_step.to_f64();
        let candidate = T::from_f64(candidate.clamp(minimum, maximum))
            .ok_or(RangeModelError::UnrepresentableValue)?;
        self.normalize(candidate)
    }

    /// Formats a finite in-range value without silently clamping or normalizing it.
    pub fn format_value(&self, value: T) -> Result<String, RangeModelError> {
        let value = finite(value, RangeNumber::Value)?;
        if value < self.minimum.to_f64() || value > self.maximum.to_f64() {
            return Err(RangeModelError::ValueOutOfBounds);
        }
        Ok(self.format.format(value))
    }
}

fn finite<T: RangeScalar>(value: T, number: RangeNumber) -> Result<f64, RangeModelError> {
    let value = value.to_f64();
    if value.is_finite() {
        Ok(value)
    } else {
        Err(RangeModelError::NonFinite(number))
    }
}

fn nearest(value: f64, candidates: [f64; 3]) -> f64 {
    let mut result = candidates[0];
    let mut distance = (value - result).abs();
    for candidate in candidates.into_iter().skip(1) {
        let candidate_distance = (value - candidate).abs();
        if candidate_distance < distance {
            result = candidate;
            distance = candidate_distance;
        }
    }
    result
}

fn validate_marks<T: RangeScalar>(
    marks: &[RangeMark<T>],
    minimum: f64,
    maximum: f64,
) -> Result<(), RangeModelError> {
    let mut previous = None;
    for (index, mark) in marks.iter().enumerate() {
        let value = mark.value.to_f64();
        if !value.is_finite() {
            return Err(RangeModelError::NonFiniteMark { index });
        }
        if value < minimum || value > maximum {
            return Err(RangeModelError::MarkOutOfBounds { index });
        }
        if previous.is_some_and(|previous| value <= previous) {
            return Err(RangeModelError::MarksNotStrictlyIncreasing { index });
        }
        if let Some(label) = &mark.label {
            if label.trim().is_empty() {
                return Err(RangeModelError::EmptyMarkLabel { index });
            }
            if label.chars().any(char::is_control) {
                return Err(RangeModelError::InvalidMarkLabel { index });
            }
        }
        previous = Some(value);
    }
    Ok(())
}

fn validate_affix(value: &str, affix: RangeAffix) -> Result<(), RangeModelError> {
    if value.chars().any(char::is_control) {
        Err(RangeModelError::InvalidFormatAffix(affix))
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeNumber {
    Minimum,
    Maximum,
    Step,
    PageStep,
    Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeAffix {
    Prefix,
    Suffix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeModelError {
    NonFinite(RangeNumber),
    UnorderedBounds,
    NonPositiveStep,
    NonPositivePageStep,
    UnrepresentableValue,
    ValueOutOfBounds,
    InvalidFormatPrecision { requested: u8, maximum: u8 },
    InvalidFormatAffix(RangeAffix),
    NonFiniteMark { index: usize },
    MarkOutOfBounds { index: usize },
    MarksNotStrictlyIncreasing { index: usize },
    EmptyMarkLabel { index: usize },
    InvalidMarkLabel { index: usize },
}

impl std::fmt::Display for RangeModelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFinite(number) => write!(formatter, "range {number:?} must be finite"),
            Self::UnorderedBounds => formatter.write_str("range minimum must be less than maximum"),
            Self::NonPositiveStep => formatter.write_str("range step must be positive"),
            Self::NonPositivePageStep => formatter.write_str("range page step must be positive"),
            Self::UnrepresentableValue => {
                formatter.write_str("normalized range value is not representable")
            }
            Self::ValueOutOfBounds => formatter.write_str("range value is outside the bounds"),
            Self::InvalidFormatPrecision { requested, maximum } => write!(
                formatter,
                "range format precision {requested} exceeds maximum {maximum}"
            ),
            Self::InvalidFormatAffix(affix) => {
                write!(
                    formatter,
                    "range format {affix:?} contains a control character"
                )
            }
            Self::NonFiniteMark { index } => write!(formatter, "range mark {index} is not finite"),
            Self::MarkOutOfBounds { index } => {
                write!(formatter, "range mark {index} is outside the bounds")
            }
            Self::MarksNotStrictlyIncreasing { index } => write!(
                formatter,
                "range mark {index} is not strictly greater than its predecessor"
            ),
            Self::EmptyMarkLabel { index } => {
                write!(formatter, "range mark {index} label is empty")
            }
            Self::InvalidMarkLabel { index } => {
                write!(
                    formatter,
                    "range mark {index} label contains a control character"
                )
            }
        }
    }
}

impl std::error::Error for RangeModelError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_rejects_nonfinite_unordered_and_nonpositive_inputs() {
        assert_eq!(
            RangeModel::new(f64::NAN, 10.0, 1.0, 5.0),
            Err(RangeModelError::NonFinite(RangeNumber::Minimum))
        );
        assert_eq!(
            RangeModel::new(0.0, f64::INFINITY, 1.0, 5.0),
            Err(RangeModelError::NonFinite(RangeNumber::Maximum))
        );
        assert_eq!(
            RangeModel::new(10.0, 10.0, 1.0, 5.0),
            Err(RangeModelError::UnorderedBounds)
        );
        assert_eq!(
            RangeModel::new(0.0, 10.0, 0.0, 5.0),
            Err(RangeModelError::NonPositiveStep)
        );
        assert_eq!(
            RangeModel::new(0.0, 10.0, 1.0, -1.0),
            Err(RangeModelError::NonPositivePageStep)
        );
    }

    #[test]
    fn clamp_and_normalize_preserve_bounds_and_choose_nearest_step() {
        let model = RangeModel::new(1.0_f64, 10.0, 3.0, 6.0).unwrap();
        assert_eq!(model.clamp(-4.0), Ok(1.0));
        assert_eq!(model.clamp(14.0), Ok(10.0));
        assert_eq!(model.normalize(2.2), Ok(1.0));
        assert_eq!(model.normalize(3.0), Ok(4.0));
        assert_eq!(model.normalize(8.2), Ok(7.0));
        assert_eq!(model.normalize(9.6), Ok(10.0));
        assert_eq!(model.normalize(10.0), Ok(10.0));
    }

    #[test]
    fn f32_models_use_the_same_finite_and_normalization_contract() {
        let model = RangeModel::new(-1.0_f32, 1.0, 0.25, 0.5).unwrap();
        assert_eq!(model.normalize(-0.63), Ok(-0.75));
        assert_eq!(model.normalize(0.62), Ok(0.5));
        assert_eq!(
            model.normalize(f32::NAN),
            Err(RangeModelError::NonFinite(RangeNumber::Value))
        );
    }

    #[test]
    fn step_and_page_movement_keep_unaligned_endpoints_reachable() {
        let model = RangeModel::new(0.0_f64, 10.0, 3.0, 5.0).unwrap();
        assert_eq!(model.step_by(6.0, 1), Ok(9.0));
        assert_eq!(model.step_by(9.0, 1), Ok(10.0));
        assert_eq!(model.step_by(10.0, -1), Ok(9.0));
        assert_eq!(model.step_by(0.0, -100), Ok(0.0));
        assert_eq!(model.page_by(3.0, 1), Ok(9.0));
        assert_eq!(model.page_by(9.0, 1), Ok(10.0));
    }

    #[test]
    fn formatting_is_bounded_deterministic_and_does_not_hide_invalid_values() {
        let format = RangeFormat::new(2)
            .unwrap()
            .prefix("$")
            .unwrap()
            .suffix(" USD")
            .unwrap();
        let model = RangeModel::new(-10.0_f64, 10.0, 0.25, 1.0)
            .unwrap()
            .with_format(format);
        assert_eq!(model.format_value(1.5), Ok("$1.50 USD".to_owned()));
        assert_eq!(model.format_value(-0.0), Ok("$0.00 USD".to_owned()));
        assert_eq!(
            model.format_value(11.0),
            Err(RangeModelError::ValueOutOfBounds)
        );
        assert_eq!(
            RangeFormat::new(16),
            Err(RangeModelError::InvalidFormatPrecision {
                requested: 16,
                maximum: 15,
            })
        );
        assert_eq!(
            RangeFormat::new(0).unwrap().suffix("bad\nlabel"),
            Err(RangeModelError::InvalidFormatAffix(RangeAffix::Suffix))
        );
    }

    #[test]
    fn marks_are_finite_ordered_bounded_and_optionally_labelled() {
        let model = RangeModel::new(0.0_f64, 10.0, 1.0, 5.0)
            .unwrap()
            .with_marks([
                RangeMark::new(0.0).label("Low"),
                RangeMark::new(5.0),
                RangeMark::new(10.0).label("High"),
            ])
            .unwrap();
        assert_eq!(model.marks().len(), 3);
        assert_eq!(model.marks()[0].label_text(), Some("Low"));
        assert_eq!(model.marks()[1].label_text(), None);

        for (marks, expected) in [
            (
                vec![RangeMark::new(f64::NAN)],
                RangeModelError::NonFiniteMark { index: 0 },
            ),
            (
                vec![RangeMark::new(11.0)],
                RangeModelError::MarkOutOfBounds { index: 0 },
            ),
            (
                vec![RangeMark::new(5.0), RangeMark::new(5.0)],
                RangeModelError::MarksNotStrictlyIncreasing { index: 1 },
            ),
            (
                vec![RangeMark::new(5.0), RangeMark::new(4.0)],
                RangeModelError::MarksNotStrictlyIncreasing { index: 1 },
            ),
            (
                vec![RangeMark::new(5.0).label(" ")],
                RangeModelError::EmptyMarkLabel { index: 0 },
            ),
        ] {
            assert_eq!(
                RangeModel::new(0.0_f64, 10.0, 1.0, 5.0)
                    .unwrap()
                    .with_marks(marks),
                Err(expected)
            );
        }
    }
}
