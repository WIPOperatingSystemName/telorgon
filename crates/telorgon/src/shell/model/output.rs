//! Validated, immutable output snapshots supplied by a shell policy host.

use std::fmt;
use std::num::NonZeroU64;

use crate::foundation::{EdgeInsets, RectF, SizeI};

use crate::shell::OutputId;

/// Monotonic host revision of one output snapshot.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputRevision(NonZeroU64);

impl OutputRevision {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub const fn from_raw(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Host-reported orientation and reflection of output content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OutputTransform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl OutputTransform {
    pub const fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Rotate90 | Self::Rotate270 | Self::Flipped90 | Self::Flipped270
        )
    }
}

/// Color presentations the host reports as available for this output.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OutputColorCapabilities(u8);

impl OutputColorCapabilities {
    pub const NONE: Self = Self(0);
    pub const SRGB: Self = Self(1 << 0);
    pub const DISPLAY_P3: Self = Self(1 << 1);
    pub const REC2020: Self = Self(1 << 2);
    pub const HDR_STATIC_METADATA: Self = Self(1 << 3);
    const ALL_BITS: u8 = (1 << 4) - 1;

    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL_BITS == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl fmt::Debug for OutputColorCapabilities {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputColorCapabilities")
            .field("bits", &format_args!("{:#06b}", self.bits()))
            .finish()
    }
}

impl std::ops::BitOr for OutputColorCapabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for OutputColorCapabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Validated geometry and presentation facts for one output revision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputGeometry {
    logical_bounds: RectF,
    usable_bounds: RectF,
    physical_size: SizeI,
    scale: f32,
    transform: OutputTransform,
    safe_insets: EdgeInsets,
    color_capabilities: OutputColorCapabilities,
}

impl OutputGeometry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        logical_bounds: RectF,
        usable_bounds: RectF,
        physical_size: SizeI,
        scale: f32,
        transform: OutputTransform,
        safe_insets: EdgeInsets,
        color_capabilities: OutputColorCapabilities,
    ) -> Result<Self, OutputGeometryError> {
        if !valid_positive_rect(logical_bounds) {
            return Err(OutputGeometryError::InvalidLogicalBounds);
        }
        if !valid_positive_rect(usable_bounds) {
            return Err(OutputGeometryError::InvalidUsableBounds);
        }
        if !contains_rect(logical_bounds, usable_bounds) {
            return Err(OutputGeometryError::UsableBoundsOutsideLogical);
        }
        if physical_size.width <= 0 || physical_size.height <= 0 {
            return Err(OutputGeometryError::InvalidPhysicalSize);
        }
        if !scale.is_finite() || scale <= 0.0 {
            return Err(OutputGeometryError::InvalidScale);
        }
        if !valid_insets(safe_insets) {
            return Err(OutputGeometryError::InvalidSafeInsets);
        }
        if safe_insets.horizontal() >= logical_bounds.width
            || safe_insets.vertical() >= logical_bounds.height
        {
            return Err(OutputGeometryError::SafeInsetsConsumeLogicalBounds);
        }

        Ok(Self {
            logical_bounds,
            usable_bounds,
            physical_size,
            scale,
            transform,
            safe_insets,
            color_capabilities,
        })
    }

    pub const fn logical_bounds(self) -> RectF {
        self.logical_bounds
    }

    pub const fn usable_bounds(self) -> RectF {
        self.usable_bounds
    }

    pub const fn physical_size(self) -> SizeI {
        self.physical_size
    }

    pub const fn scale(self) -> f32 {
        self.scale
    }

    pub const fn transform(self) -> OutputTransform {
        self.transform
    }

    pub const fn safe_insets(self) -> EdgeInsets {
        self.safe_insets
    }

    pub const fn color_capabilities(self) -> OutputColorCapabilities {
        self.color_capabilities
    }

    /// Logical drawing bounds remaining after applying the host's safe insets.
    pub fn safe_bounds(self) -> RectF {
        RectF {
            x: self.logical_bounds.x + self.safe_insets.left,
            y: self.logical_bounds.y + self.safe_insets.top,
            width: self.logical_bounds.width - self.safe_insets.horizontal(),
            height: self.logical_bounds.height - self.safe_insets.vertical(),
        }
    }
}

/// Complete immutable host truth for one output revision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputSnapshot {
    id: OutputId,
    revision: OutputRevision,
    geometry: OutputGeometry,
}

impl OutputSnapshot {
    pub const fn new(id: OutputId, revision: OutputRevision, geometry: OutputGeometry) -> Self {
        Self {
            id,
            revision,
            geometry,
        }
    }

    pub const fn id(self) -> OutputId {
        self.id
    }

    pub const fn revision(self) -> OutputRevision {
        self.revision
    }

    pub const fn geometry(self) -> OutputGeometry {
        self.geometry
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputGeometryError {
    InvalidLogicalBounds,
    InvalidUsableBounds,
    UsableBoundsOutsideLogical,
    InvalidPhysicalSize,
    InvalidScale,
    InvalidSafeInsets,
    SafeInsetsConsumeLogicalBounds,
}

impl fmt::Display for OutputGeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidLogicalBounds => "logical output bounds must be finite and positive",
            Self::InvalidUsableBounds => "usable output bounds must be finite and positive",
            Self::UsableBoundsOutsideLogical => {
                "usable output bounds must remain inside logical bounds"
            }
            Self::InvalidPhysicalSize => "physical output size must be positive",
            Self::InvalidScale => "output scale must be finite and positive",
            Self::InvalidSafeInsets => "safe output insets must be finite and nonnegative",
            Self::SafeInsetsConsumeLogicalBounds => {
                "safe output insets must leave positive logical bounds"
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for OutputGeometryError {}

fn valid_positive_rect(rect: RectF) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.width > 0.0
        && rect.height.is_finite()
        && rect.height > 0.0
        && rect.right().is_finite()
        && rect.bottom().is_finite()
}

fn contains_rect(outer: RectF, inner: RectF) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

fn valid_insets(insets: EdgeInsets) -> bool {
    [insets.top, insets.right, insets.bottom, insets.left]
        .into_iter()
        .all(|value| value.is_finite() && value >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logical() -> RectF {
        RectF {
            x: -1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        }
    }

    fn geometry() -> OutputGeometry {
        OutputGeometry::new(
            logical(),
            RectF {
                x: -1920.0,
                y: 32.0,
                width: 1920.0,
                height: 1048.0,
            },
            SizeI {
                width: 3840,
                height: 2160,
            },
            2.0,
            OutputTransform::Normal,
            EdgeInsets {
                top: 8.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            },
            OutputColorCapabilities::SRGB | OutputColorCapabilities::DISPLAY_P3,
        )
        .unwrap()
    }

    #[test]
    fn snapshot_preserves_host_identity_revision_and_output_facts() {
        let output = OutputSnapshot::new(
            OutputId::from_raw(9).unwrap(),
            OutputRevision::from_raw(4).unwrap(),
            geometry(),
        );

        assert_eq!(output.id().get(), 9);
        assert_eq!(output.revision().get(), 4);
        assert_eq!(output.geometry().physical_size().width, 3840);
        assert!(
            output
                .geometry()
                .color_capabilities()
                .contains(OutputColorCapabilities::DISPLAY_P3)
        );
        assert_eq!(output.geometry().safe_bounds().y, 8.0);
    }

    #[test]
    fn invalid_geometry_is_rejected_before_publication() {
        assert_eq!(
            OutputGeometry::new(
                logical(),
                RectF {
                    x: -2000.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1080.0,
                },
                SizeI {
                    width: 1920,
                    height: 1080,
                },
                1.0,
                OutputTransform::Normal,
                EdgeInsets::ZERO,
                OutputColorCapabilities::SRGB,
            ),
            Err(OutputGeometryError::UsableBoundsOutsideLogical)
        );

        assert_eq!(
            OutputGeometry::new(
                logical(),
                logical(),
                SizeI {
                    width: 1920,
                    height: 1080,
                },
                f32::NAN,
                OutputTransform::Normal,
                EdgeInsets::ZERO,
                OutputColorCapabilities::SRGB,
            ),
            Err(OutputGeometryError::InvalidScale)
        );
    }

    #[test]
    fn transform_reports_axis_swaps_without_reinterpreting_host_geometry() {
        assert!(OutputTransform::Rotate90.swaps_axes());
        assert!(OutputTransform::Flipped270.swaps_axes());
        assert!(!OutputTransform::Normal.swaps_axes());
        assert!(!OutputTransform::Flipped180.swaps_axes());
    }
}
