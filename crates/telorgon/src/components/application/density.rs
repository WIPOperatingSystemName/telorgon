//! Component target metrics resolved from neutral application density.

use std::fmt;

pub use crate::components::application::primitives::LogicalDensityClass as DensityClass;
use crate::foundation::SizeF;

/// Validated minimum logical hit size for an interactive component.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractiveTargetSize {
    width: f32,
    height: f32,
}

impl InteractiveTargetSize {
    pub const COMPACT: Self = Self {
        width: 24.0,
        height: 24.0,
    };
    pub const STANDARD: Self = Self {
        width: 32.0,
        height: 32.0,
    };
    pub const TOUCH: Self = Self {
        width: 44.0,
        height: 44.0,
    };

    pub fn new(width: f32, height: f32) -> Result<Self, DensityError> {
        if !is_positive_finite(width) || !is_positive_finite(height) {
            return Err(DensityError::InvalidInteractiveTarget);
        }
        Ok(Self { width, height })
    }

    pub const fn width(self) -> f32 {
        self.width
    }

    pub const fn height(self) -> f32 {
        self.height
    }

    pub const fn logical_size(self) -> SizeF {
        SizeF {
            width: self.width,
            height: self.height,
        }
    }

    pub const fn max(self, other: Self) -> Self {
        Self {
            width: if self.width >= other.width {
                self.width
            } else {
                other.width
            },
            height: if self.height >= other.height {
                self.height
            } else {
                other.height
            },
        }
    }

    pub const fn contains(self, size: SizeF) -> bool {
        size.width >= self.width && size.height >= self.height
    }
}

/// Resolved baseline, theme, and accessibility/platform target floors for one density class.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DensityMetrics {
    class: DensityClass,
    baseline_minimum: InteractiveTargetSize,
    theme_minimum: Option<InteractiveTargetSize>,
    required_minimum: Option<InteractiveTargetSize>,
    effective_minimum: InteractiveTargetSize,
}

impl DensityMetrics {
    /// Resolves an effective target without allowing either optional policy to lower the baseline.
    pub const fn resolve(
        class: DensityClass,
        theme_minimum: Option<InteractiveTargetSize>,
        required_minimum: Option<InteractiveTargetSize>,
    ) -> Self {
        let baseline_minimum = baseline_for(class);
        let with_theme = match theme_minimum {
            Some(theme) => baseline_minimum.max(theme),
            None => baseline_minimum,
        };
        let effective_minimum = match required_minimum {
            Some(required) => with_theme.max(required),
            None => with_theme,
        };
        Self {
            class,
            baseline_minimum,
            theme_minimum,
            required_minimum,
            effective_minimum,
        }
    }

    pub const fn baseline(class: DensityClass) -> Self {
        Self::resolve(class, None, None)
    }

    pub const fn class(self) -> DensityClass {
        self.class
    }

    pub const fn baseline_minimum(self) -> InteractiveTargetSize {
        self.baseline_minimum
    }

    pub const fn theme_minimum(self) -> Option<InteractiveTargetSize> {
        self.theme_minimum
    }

    pub const fn required_minimum(self) -> Option<InteractiveTargetSize> {
        self.required_minimum
    }

    pub const fn effective_minimum(self) -> InteractiveTargetSize {
        self.effective_minimum
    }

    /// Assesses component hit geometry. Visible artwork may be smaller than this hit geometry.
    pub fn assess(self, actual: SizeF) -> Result<TargetAssessment, DensityError> {
        if !is_nonnegative_finite(actual.width) || !is_nonnegative_finite(actual.height) {
            return Err(DensityError::InvalidActualTarget);
        }
        Ok(TargetAssessment {
            actual,
            required: self.effective_minimum,
            meets_minimum: self.effective_minimum.contains(actual),
        })
    }
}

/// Result used by component diagnostics and acceptance fixtures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetAssessment {
    pub actual: SizeF,
    pub required: InteractiveTargetSize,
    pub meets_minimum: bool,
}

/// Invalid target geometry rejected before component policy is resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DensityError {
    InvalidInteractiveTarget,
    InvalidActualTarget,
}

impl fmt::Display for DensityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid component density metric: {self:?}")
    }
}

impl std::error::Error for DensityError {}

const fn baseline_for(class: DensityClass) -> InteractiveTargetSize {
    match class {
        DensityClass::Compact => InteractiveTargetSize::COMPACT,
        DensityClass::Standard => InteractiveTargetSize::STANDARD,
        DensityClass::Touch => InteractiveTargetSize::TOUCH,
    }
}

fn is_positive_finite(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

fn is_nonnegative_finite(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_profiles_have_exact_contract_targets() {
        assert_eq!(
            DensityMetrics::baseline(DensityClass::Compact)
                .effective_minimum()
                .logical_size(),
            SizeF {
                width: 24.0,
                height: 24.0
            }
        );
        assert_eq!(
            DensityMetrics::baseline(DensityClass::Standard)
                .effective_minimum()
                .logical_size(),
            SizeF {
                width: 32.0,
                height: 32.0
            }
        );
        assert_eq!(
            DensityMetrics::baseline(DensityClass::Touch)
                .effective_minimum()
                .logical_size(),
            SizeF {
                width: 44.0,
                height: 44.0
            }
        );
    }

    #[test]
    fn theme_and_required_floors_resolve_per_axis() {
        let theme = InteractiveTargetSize::new(50.0, 36.0).unwrap();
        let required = InteractiveTargetSize::new(40.0, 48.0).unwrap();
        let metrics = DensityMetrics::resolve(DensityClass::Standard, Some(theme), Some(required));
        assert_eq!(
            metrics.effective_minimum().logical_size(),
            SizeF {
                width: 50.0,
                height: 48.0
            }
        );
    }

    #[test]
    fn policy_cannot_reduce_the_density_baseline() {
        let smaller = InteractiveTargetSize::new(8.0, 12.0).unwrap();
        let metrics = DensityMetrics::resolve(DensityClass::Touch, Some(smaller), Some(smaller));
        assert_eq!(metrics.effective_minimum(), InteractiveTargetSize::TOUCH);
    }

    #[test]
    fn assessment_reports_violations_without_rejecting_small_artwork() {
        let metrics = DensityMetrics::baseline(DensityClass::Standard);
        let too_small = metrics
            .assess(SizeF {
                width: 24.0,
                height: 40.0,
            })
            .unwrap();
        assert!(!too_small.meets_minimum);

        let exact = metrics
            .assess(SizeF {
                width: 32.0,
                height: 32.0,
            })
            .unwrap();
        assert!(exact.meets_minimum);
    }

    #[test]
    fn nonfinite_or_negative_geometry_is_rejected() {
        assert_eq!(
            InteractiveTargetSize::new(f32::NAN, 20.0),
            Err(DensityError::InvalidInteractiveTarget)
        );
        assert_eq!(
            DensityMetrics::baseline(DensityClass::Compact).assess(SizeF {
                width: -1.0,
                height: 24.0,
            }),
            Err(DensityError::InvalidActualTarget)
        );
    }
}
