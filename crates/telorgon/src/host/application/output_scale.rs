//! Boot policy for the Linux desktop. UI geometry is authored in logical units.
use super::{AppError, AppResult};
use crate::foundation::SizeI;
use crate::platform::contracts::ScaleFactor;

/// Output density policy. Fixed values are factors: 1.0 = 100%, 2.0 = 200%.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum OutputScale {
    /// Estimate density from KMS dimensions, targeting 110 DPI on displays at least 20 inches
    /// and 135 DPI on smaller panels. Uses 25% steps within 100–400%, limited to preserve
    /// a 960-by-720 logical workspace (in either orientation) where possible.
    /// Missing or implausible dimensions use 100%.
    #[default]
    Auto,
    /// Explicit 100–400% scale, quantized to Wayland's 1/120 increments.
    Fixed(f32),
}

impl OutputScale {
    pub(crate) fn validate(self) -> AppResult<()> {
        if let Self::Fixed(value) = self
            && (!value.is_finite() || !(1.0..=4.0).contains(&value))
        {
            return Err(AppError::new(
                "output scale must be finite and between 1.0 and 4.0",
            ));
        }
        Ok(())
    }

    pub(crate) fn resolve(self, pixels: SizeI, millimeters: SizeI) -> AppResult<ScaleFactor> {
        self.validate()?;
        let value = match self {
            Self::Fixed(value) => (value * 120.0).round() / 120.0,
            Self::Auto => automatic_scale(pixels, millimeters),
        };
        ScaleFactor::new(value).map_err(|error| AppError::new(error.to_string()))
    }
}

fn automatic_scale(pixels: SizeI, millimeters: SizeI) -> f32 {
    // Reject missing sizes and EDID aspect-ratio placeholders before interpreting them as DPI.
    let short_mm = millimeters.width.min(millimeters.height);
    let long_mm = millimeters.width.max(millimeters.height);
    if !(50..=3000).contains(&short_mm)
        || !(50..=3000).contains(&long_mm)
        || matches!((long_mm, short_mm), (160, 90 | 100) | (1600, 900 | 1000))
    {
        return 1.0;
    }
    let x_dpi = pixels.width as f64 * 25.4 / millimeters.width as f64;
    let y_dpi = pixels.height as f64 * 25.4 / millimeters.height as f64;
    if !(50.0..=500.0).contains(&x_dpi)
        || !(50.0..=500.0).contains(&y_dpi)
        || x_dpi.max(y_dpi) / x_dpi.min(y_dpi) > 1.1
    {
        return 1.0;
    }

    let diagonal_inches = (millimeters.width as f64).hypot(millimeters.height as f64) / 25.4;
    let dpi = (pixels.width as f64).hypot(pixels.height as f64) / diagonal_inches;
    // Larger displays are generally viewed from farther away; use a larger physical UI.
    let target_dpi = if diagonal_inches >= 20.0 {
        110.0
    } else {
        135.0
    };
    let preferred_steps = (dpi / target_dpi * 4.0).round();

    // A density estimate must not make an otherwise usable mode too cramped. Sorting the
    // axes makes the policy invariant under rotation. Fixed preferences bypass this limit.
    let workspace_limit = (pixels.width.min(pixels.height) as f64 / 720.0)
        .min(pixels.width.max(pixels.height) as f64 / 960.0);
    let maximum_steps = (workspace_limit * 4.0).floor().clamp(4.0, 16.0);
    (preferred_steps.clamp(4.0, maximum_steps) / 4.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto(width: i32, height: i32, width_mm: i32, height_mm: i32) -> f32 {
        OutputScale::Auto
            .resolve(
                SizeI { width, height },
                SizeI {
                    width: width_mm,
                    height: height_mm,
                },
            )
            .unwrap()
            .get()
    }

    #[test]
    fn display_matrix_balances_physical_ui_size_and_workspace() {
        for (width, height, width_mm, height_mm, expected) in [
            (1920, 1080, 531, 299, 1.0),  // 24-inch FHD
            (2560, 1440, 597, 336, 1.0),  // 27-inch QHD
            (3840, 2160, 531, 299, 1.75), // 24-inch UHD
            (3840, 2160, 597, 336, 1.5),  // 27-inch UHD
            (3840, 2160, 708, 398, 1.25), // 32-inch UHD
            (3840, 2160, 941, 529, 1.0),  // 43-inch UHD
            (3840, 2160, 344, 194, 2.0),  // 15.6-inch UHD laptop
            (3840, 2160, 294, 165, 2.5),  // 13.3-inch UHD laptop
            (5120, 2880, 597, 336, 2.0),  // 27-inch 5K
            (7680, 4320, 708, 398, 2.5),  // 32-inch 8K
        ] {
            assert_eq!(
                auto(width, height, width_mm, height_mm),
                expected,
                "{width}x{height}, {width_mm}x{height_mm} mm"
            );
            assert_eq!(auto(height, width, height_mm, width_mm), expected);
        }
    }

    #[test]
    fn automatic_scale_preserves_space_on_small_dense_panels() {
        // Density alone would choose 250%, leaving only 768x432 logical units.
        assert_eq!(auto(1920, 1080, 145, 82), 1.5);
        assert_eq!(auto(1080, 1920, 82, 145), 1.5);
        // A physically small mode must not shrink below its already limited 1x workspace.
        assert_eq!(auto(800, 600, 100, 75), 1.0);
        assert_eq!(
            OutputScale::Fixed(2.5)
                .resolve(
                    SizeI {
                        width: 1920,
                        height: 1080
                    },
                    SizeI {
                        width: 145,
                        height: 82
                    },
                )
                .unwrap()
                .get(),
            2.5
        );
    }

    #[test]
    fn increasing_resolution_preserves_logical_workspace_at_matching_density() {
        for multiplier in [1, 2, 3, 4] {
            let width = 1920 * multiplier;
            let height = 1080 * multiplier;
            let scale = auto(width, height, 443, 249); // approximately 110 DPI at 1080p
            assert_eq!(scale, multiplier as f32);
            assert_eq!(width as f32 / scale, 1920.0);
            assert_eq!(height as f32 / scale, 1080.0);
            assert_eq!((scale * 120.0) % 1.0, 0.0);
        }
    }

    #[test]
    fn bogus_aspect_ratio_metadata_and_invalid_modes_are_not_density_evidence() {
        for (width_mm, height_mm) in [(160, 90), (160, 100), (1600, 900), (1600, 1000)] {
            assert_eq!(auto(3840, 2160, width_mm, height_mm), 1.0);
            assert_eq!(auto(2160, 3840, height_mm, width_mm), 1.0);
        }
        for (width, height) in [(0, 2160), (-3840, 2160), (i32::MAX, i32::MAX)] {
            assert_eq!(auto(width, height, 597, 336), 1.0);
        }
    }

    #[test]
    fn boot_scale_uses_density_instead_of_resolution_alone() {
        let full_hd = SizeI {
            width: 1920,
            height: 1080,
        };
        let uhd = SizeI {
            width: 3840,
            height: 2160,
        };
        let desktop = SizeI {
            width: 531,
            height: 299,
        }; // 24 inch
        assert_eq!(
            OutputScale::Auto.resolve(full_hd, desktop).unwrap().get(),
            1.0
        );
        assert_eq!(OutputScale::Auto.resolve(uhd, desktop).unwrap().get(), 1.75);
        assert_eq!(
            OutputScale::Auto
                .resolve(
                    uhd,
                    SizeI {
                        width: 941,
                        height: 529
                    }
                )
                .unwrap()
                .get(),
            1.0
        );
        assert_eq!(
            OutputScale::Auto
                .resolve(
                    uhd,
                    SizeI {
                        width: 597,
                        height: 336
                    }
                )
                .unwrap()
                .get(),
            1.5
        );
    }
    #[test]
    fn fixed_scale_is_quantized_to_the_announced_protocol_value() {
        let scale = OutputScale::Fixed(1.333)
            .resolve(SizeI::default(), SizeI::default())
            .unwrap();
        assert_eq!((scale.get() * 120.0).round() as u32, 160);
        assert_eq!(scale.get(), 160.0 / 120.0);
        assert_eq!(scale.get().ceil() as i32, 2);
    }

    #[test]
    fn invalid_edid_falls_back_and_explicit_preference_wins() {
        let pixels = SizeI {
            width: 3840,
            height: 2160,
        };
        for size in [
            SizeI::default(),
            SizeI {
                width: 1,
                height: 1,
            },
            SizeI {
                width: 500,
                height: 500,
            },
            SizeI {
                width: -1,
                height: 300,
            },
        ] {
            assert_eq!(OutputScale::Auto.resolve(pixels, size).unwrap().get(), 1.0);
            assert_eq!(
                OutputScale::Fixed(1.5).resolve(pixels, size).unwrap().get(),
                1.5
            );
        }
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 4.1] {
            assert!(OutputScale::Fixed(scale).validate().is_err());
        }
    }
}
