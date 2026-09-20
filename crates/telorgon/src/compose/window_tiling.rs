//! Desktop snap tiling, registered as an ordinary shell widget.
use super::*;
use crate::ui::Border;
use crate::{ColorRgba8, Easing, Fill, WindowTween, tween_ms};

/// Logical destination within the selected output's work area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileTarget {
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}
impl TileTarget {
    pub(crate) fn left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }
    pub(crate) fn row(self) -> Option<bool> {
        match self {
            Self::Left | Self::Right => None,
            Self::TopLeft | Self::TopRight => Some(false),
            _ => Some(true),
        }
    }
    pub(crate) fn conflicts(self, other: Self) -> bool {
        self.left() == other.left()
            && (self.row().is_none() || other.row().is_none() || self.row() == other.row())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TilePreviewMotion {
    pub(crate) appear: WindowTween,
    pub(crate) relocate: crate::GeometryMotion,
    pub(crate) disappear: WindowTween,
}
impl TilePreviewMotion {
    pub const fn smooth() -> Self {
        Self {
            appear: tween_ms(120, Easing::EaseOut),
            relocate: crate::GeometryMotion::Tween(tween_ms(180, Easing::EaseOut)),
            disappear: tween_ms(90, Easing::EaseOut),
        }
    }
    pub const fn none() -> Self {
        Self {
            appear: tween_ms(0, Easing::Linear),
            relocate: crate::GeometryMotion::Tween(tween_ms(0, Easing::Linear)),
            disappear: tween_ms(0, Easing::Linear),
        }
    }
    pub const fn appear(mut self, value: WindowTween) -> Self {
        self.appear = value;
        self
    }
    pub const fn relocate(mut self, value: WindowTween) -> Self {
        self.relocate = crate::GeometryMotion::Tween(value);
        self
    }
    pub const fn movement(mut self, value: crate::GeometryMotion) -> Self {
        self.relocate = value;
        self
    }
    pub const fn disappear(mut self, value: WindowTween) -> Self {
        self.disappear = value;
        self
    }
}
impl Default for TilePreviewMotion {
    fn default() -> Self {
        Self::smooth()
    }
}

/// Uses the same color/glass material as window resize placeholders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TilePreviewDesign {
    pub fill: Fill,
    pub border: Border,
    pub corner_radius: f32,
    /// Preview-only inset at outer work-area edges, never at shared tile boundaries.
    /// Logical units; finite and nonnegative. Defaults to zero.
    pub padding: Insets,
    pub motion: TilePreviewMotion,
}
impl Default for TilePreviewDesign {
    fn default() -> Self {
        Self {
            fill: Fill::Color(ColorRgba8::rgba(100, 160, 240, 80)),
            border: Border::all(1.0, ColorRgba8::rgba(150, 195, 255, 220)),
            corner_radius: 12.0,
            padding: Insets::ZERO,
            motion: TilePreviewMotion::smooth(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowTiling {
    pub(crate) halves: bool,
    pub(crate) quadrants: bool,
    pub(crate) shared_resize: bool,
    pub(crate) edge_threshold: f32,
    pub(crate) corner_threshold: f32,
    pub(crate) divider_hit_width: f32,
    pub(crate) preview: TilePreviewDesign,
}
impl Default for WindowTiling {
    fn default() -> Self {
        Self::snap()
    }
}
impl WindowTiling {
    pub fn snap() -> Self {
        Self {
            halves: true,
            quadrants: true,
            shared_resize: true,
            edge_threshold: 16.0,
            corner_threshold: 48.0,
            divider_hit_width: 8.0,
            preview: TilePreviewDesign::default(),
        }
    }
    pub const fn halves(mut self, value: bool) -> Self {
        self.halves = value;
        self
    }
    pub const fn quadrants(mut self, value: bool) -> Self {
        self.quadrants = value;
        self
    }
    pub const fn shared_resize(mut self, value: bool) -> Self {
        self.shared_resize = value;
        self
    }
    pub fn edge_threshold(mut self, value: f32) -> Self {
        assert!(value.is_finite() && value >= 0.0);
        self.edge_threshold = value;
        self
    }
    pub fn corner_threshold(mut self, value: f32) -> Self {
        assert!(value.is_finite() && value >= 0.0);
        self.corner_threshold = value;
        self
    }
    pub fn divider_hit_width(mut self, value: f32) -> Self {
        assert!(value.is_finite() && value > 0.0);
        self.divider_hit_width = value;
        self
    }
    pub fn preview(mut self, value: TilePreviewDesign) -> Self {
        assert!(value.corner_radius.is_finite() && value.corner_radius >= 0.0);
        for side in [
            value.border.top,
            value.border.right,
            value.border.bottom,
            value.border.left,
        ] {
            assert!(side.width.is_finite() && side.width >= 0.0);
        }
        let padding = value.padding.0;
        for distance in [padding.top, padding.right, padding.bottom, padding.left] {
            assert!(
                distance.is_finite() && distance >= 0.0,
                "tile preview padding must be finite and nonnegative"
            );
        }
        self.preview = value;
        self
    }
    pub(crate) fn target(self, p: crate::PointF, output: crate::RectI) -> Option<TileTarget> {
        if !p.x.is_finite() || !p.y.is_finite() || output.width < 2 || output.height < 2 {
            return None;
        }
        let x = p.x - output.x as f32;
        let y = p.y - output.y as f32;
        let w = output.width as f32;
        let h = output.height as f32;
        if x < 0.0 || y < 0.0 || x > w || y > h {
            return None;
        }
        let c = self.corner_threshold.min(w / 2.0).min(h / 2.0);
        if self.quadrants {
            if x <= c && y <= c {
                return Some(TileTarget::TopLeft);
            }
            if x >= w - c && y <= c {
                return Some(TileTarget::TopRight);
            }
            if x <= c && y >= h - c {
                return Some(TileTarget::BottomLeft);
            }
            if x >= w - c && y >= h - c {
                return Some(TileTarget::BottomRight);
            }
        }
        let e = self.edge_threshold.min(w / 2.0);
        if self.halves && x <= e {
            Some(TileTarget::Left)
        } else if self.halves && x >= w - e {
            Some(TileTarget::Right)
        } else {
            None
        }
    }
}
impl ComponentFields for WindowTiling {
    type InputSnapshot = Self;
    fn capture_inputs(&self) -> Self {
        *self
    }
    fn update_inputs(&mut self, incoming: Self) -> bool {
        let changed = *self != incoming;
        *self = incoming;
        changed
    }
    fn restore_inputs(&mut self, value: Self) -> bool {
        self.update_inputs(value)
    }
}
impl Component for WindowTiling {
    fn view(&self) -> impl View {
        stack()
            .border_sides(self.preview.border)
            .corner_radius(self.preview.corner_radius)
    }
}
impl ShellWidget for WindowTiling {
    fn surface(&self) -> ShellSurfaceSpec {
        let mut spec = ShellSurfaceSpec::new()
            .pointer(ShellPointer::PassThrough)
            .visible(false);
        spec.tiling = Some(*self);
        spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corners_precede_halves_and_disabled_targets_are_absent() {
        let area = crate::RectI {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
        };
        let p = |x, y| crate::PointF { x, y };
        assert_eq!(
            WindowTiling::snap().target(p(1., 1.), area),
            Some(TileTarget::TopLeft)
        );
        assert_eq!(
            WindowTiling::snap().target(p(999., 799.), area),
            Some(TileTarget::BottomRight)
        );
        assert_eq!(
            WindowTiling::snap().target(p(1., 400.), area),
            Some(TileTarget::Left)
        );
        assert_eq!(
            WindowTiling::snap().halves(false).target(p(1., 400.), area),
            None
        );
        assert_eq!(
            WindowTiling::snap()
                .quadrants(false)
                .target(p(1., 1.), area),
            Some(TileTarget::Left)
        );
        assert_eq!(WindowTiling::snap().target(p(500., 400.), area), None);
        assert_eq!(WindowTiling::snap().target(p(f32::NAN, 0.), area), None);
    }
}
