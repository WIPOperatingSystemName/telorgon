//! Programmer-facing composition style values.

mod text;

use crate::foundation::EdgeInsets;
use crate::ui::SizeRule;

pub use text::TextStyle;

/// Shared start, center, or end alignment for composition builders.
///
/// Use this with [`crate::authoring::compose::Container::justify_content`] to position children along a
/// container's flow direction, [`crate::authoring::compose::Container::align_items`] to position them across it, and
/// [`crate::authoring::compose::Text::text_align`] to position glyph lines within a text box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    #[default]
    Start,
    Center,
    End,
}

impl From<Alignment> for crate::ui::MainAxisAlignment {
    fn from(value: Alignment) -> Self {
        match value {
            Alignment::Start => Self::Start,
            Alignment::Center => Self::Center,
            Alignment::End => Self::End,
        }
    }
}

impl From<Alignment> for crate::ui::CrossAxisAlignment {
    fn from(value: Alignment) -> Self {
        match value {
            Alignment::Start => Self::Start,
            Alignment::Center => Self::Center,
            Alignment::End => Self::End,
        }
    }
}

impl From<Alignment> for crate::ui::TextAlign {
    fn from(value: Alignment) -> Self {
        match value {
            Alignment::Start => Self::Start,
            Alignment::Center => Self::Center,
            Alignment::End => Self::End,
        }
    }
}

/// Concise dimension accepted by composition builders.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Dimension {
    /// Fixed size in logical layout units; the host applies output scaling when rendering.
    Logical(f32),
    Percent(f32),
    Fill(f32),
    #[default]
    Shrink,
}

impl Dimension {
    pub const FILL: Self = Self::Fill(1.0);
}

impl From<f32> for Dimension {
    fn from(value: f32) -> Self {
        Self::Logical(value)
    }
}

impl From<SizeRule> for Dimension {
    fn from(value: SizeRule) -> Self {
        match value {
            SizeRule::Logical(value) => Self::Logical(value),
            SizeRule::Percent(value) => Self::Percent(value),
            SizeRule::Fill(value) => Self::Fill(value),
            SizeRule::Shrink => Self::Shrink,
        }
    }
}

impl From<Dimension> for SizeRule {
    fn from(value: Dimension) -> Self {
        match value {
            Dimension::Logical(value) => Self::Logical(value),
            Dimension::Percent(value) => Self::Percent(value),
            Dimension::Fill(value) => Self::Fill(value),
            Dimension::Shrink => Self::Shrink,
        }
    }
}

/// Compact padding/margin input.
///
/// Builders accept `12.0` for every edge, `(vertical, horizontal)`, or
/// `(top, right, bottom, left)`. Named constructors are available when tuple ordering would be
/// unclear at the call site.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets(pub(crate) EdgeInsets);

impl Insets {
    pub const ZERO: Self = Self(EdgeInsets::ZERO);

    /// Creates explicit top, right, bottom, and left insets, in CSS clockwise order.
    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self(EdgeInsets {
            top,
            right,
            bottom,
            left,
        })
    }

    /// Applies the same inset to every edge.
    pub const fn all(value: f32) -> Self {
        Self::new(value, value, value, value)
    }

    /// Applies one inset to top/bottom and another to left/right.
    pub const fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Self::new(vertical, horizontal, vertical, horizontal)
    }
}

impl From<f32> for Insets {
    fn from(value: f32) -> Self {
        Self::all(value)
    }
}

impl From<(f32, f32)> for Insets {
    fn from((vertical, horizontal): (f32, f32)) -> Self {
        Self::symmetric(vertical, horizontal)
    }
}

impl From<(f32, f32, f32, f32)> for Insets {
    fn from((top, right, bottom, left): (f32, f32, f32, f32)) -> Self {
        Self::new(top, right, bottom, left)
    }
}

impl From<EdgeInsets> for Insets {
    fn from(value: EdgeInsets) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insets_support_named_and_compact_construction() {
        let explicit = Insets::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(explicit.0.top, 1.0);
        assert_eq!(explicit.0.right, 2.0);
        assert_eq!(explicit.0.bottom, 3.0);
        assert_eq!(explicit.0.left, 4.0);

        assert_eq!(Insets::all(8.0), Insets::from(8.0));
        assert_eq!(Insets::symmetric(6.0, 12.0), Insets::from((6.0, 12.0)));
        assert_eq!(explicit, Insets::from((1.0, 2.0, 3.0, 4.0)));
        assert_eq!(Insets::ZERO, Insets::default());
    }
}
