use crate::foundation::{EdgeInsets, PointF, RectF};
use crate::ui::{Border, CornerRadii};

/// An analytic rounded clip in output-pixel coordinates, independent of a retained scene.
/// Composite placements can intersect two such bounds without rewriting client pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedClip {
    pub rect: RectF,
    pub radii: CornerRadii,
    /// Keep the outside of this contour (for a transparent content aperture).
    pub inverted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::ColorRgba8;

    #[test]
    fn resize_outset_and_containment_share_the_border_curve() {
        let outer = RoundedClip::new(
            RectF {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 80.0,
            },
            CornerRadii::all(14.0),
        );
        let inner = outer.inset(Border::all(4.0, ColorRgba8::rgba(0, 0, 0, 0)));
        let expanded = outer.outset(EdgeInsets::all(6.0));
        assert_eq!(
            expanded.rect.x + expanded.radii.top_left,
            outer.rect.x + outer.radii.top_left
        );
        assert_eq!(expanded.radii, CornerRadii::all(20.0));
        let border_point = PointF { x: 15.5, y: 25.5 };
        assert!(outer.contains(border_point));
        assert!(!inner.contains(border_point));
        assert!(inner.inverse().contains(border_point));
        assert!(expanded.contains(PointF { x: 5.0, y: 60.0 }));
        assert!(!expanded.contains(PointF { x: 3.0, y: 60.0 }));
        assert!(
            !expanded.contains(PointF { x: 4.0, y: 14.0 }),
            "outside square corner is not in the rounded band"
        );
        assert_eq!(
            outer.inverse().outset(EdgeInsets::all(6.0)),
            expanded.inverse()
        );
    }

    #[test]
    fn inverse_coverage_is_complementary_including_empty_bounds_and_antialiasing() {
        let clip = RoundedClip::new(
            RectF {
                x: 3.0,
                y: 5.0,
                width: 20.0,
                height: 16.0,
            },
            CornerRadii::all(6.0),
        );
        for y in 0..28 {
            for x in 0..28 {
                let point = PointF {
                    x: x as f32 + 0.5,
                    y: y as f32 + 0.5,
                };
                assert_eq!(clip.coverage(point) + clip.inverse().coverage(point), 1.0);
            }
        }
        assert_eq!(clip.inverse().inverse(), clip);
        let inset = Border::all(2.0, ColorRgba8::rgba(0, 0, 0, 0));
        assert_eq!(clip.inverse().inset(inset), clip.inset(inset).inverse());
        let empty = RoundedClip::new(
            RectF {
                width: 0.0,
                ..clip.rect
            },
            clip.radii,
        );
        assert_eq!(empty.inverse().coverage(PointF { x: 0.0, y: 0.0 }), 1.0);
    }

    #[test]
    fn inner_curve_is_concentric_with_uniform_border_and_clamps_when_thick() {
        let outer = RoundedClip::new(
            RectF {
                x: 20.0,
                y: 30.0,
                width: 100.0,
                height: 80.0,
            },
            CornerRadii::all(12.0),
        );
        for width in [0.0, 2.0, 12.0, 20.0] {
            let inner = outer.inset(Border::all(width, ColorRgba8::rgba(0, 0, 0, 255)));
            assert_eq!(inner.rect.x, 20.0 + width);
            assert_eq!(inner.rect.y, 30.0 + width);
            assert_eq!(inner.rect.width, 100.0 - width * 2.0);
            assert_eq!(inner.radii, CornerRadii::all((12.0 - width).max(0.0)));
            if width <= 12.0 {
                assert_eq!(
                    inner.rect.x + inner.radii.top_left,
                    outer.rect.x + outer.radii.top_left
                );
            }
        }
    }

    #[test]
    fn clip_coverage_handles_square_empty_oversized_and_translated_bounds() {
        let rect = RectF {
            x: -20.0,
            y: 10.0,
            width: 20.0,
            height: 10.0,
        };
        let square = RoundedClip::new(rect, CornerRadii::default());
        assert_eq!(square.coverage(PointF { x: -19.5, y: 10.5 }), 1.0);
        let rounded = RoundedClip::new(rect, CornerRadii::all(1000.0));
        assert_eq!(rounded.radii, CornerRadii::all(5.0));
        assert_eq!(rounded.coverage(PointF { x: -19.5, y: 10.5 }), 0.0);
        assert_eq!(rounded.coverage(PointF { x: -10.0, y: 15.0 }), 1.0);
        let empty = RoundedClip::new(RectF { width: 0.0, ..rect }, CornerRadii::default());
        assert!(empty.is_valid());
        assert_eq!(empty.coverage(PointF { x: -20.0, y: 15.0 }), 0.0);
        assert!(
            !RoundedClip {
                rect: RectF {
                    x: f32::NAN,
                    ..rect
                },
                ..square
            }
            .is_valid()
        );
    }
}

impl RoundedClip {
    pub fn new(rect: RectF, radii: CornerRadii) -> Self {
        let limit = (rect.width.min(rect.height) * 0.5).max(0.0);
        let radius = |r: f32| {
            if r.is_finite() {
                r.clamp(0.0, limit)
            } else {
                0.0
            }
        };
        Self {
            rect,
            inverted: false,
            radii: CornerRadii {
                top_left: radius(radii.top_left),
                top_right: radius(radii.top_right),
                bottom_right: radius(radii.bottom_right),
                bottom_left: radius(radii.bottom_left),
            },
        }
    }

    pub fn inverse(mut self) -> Self {
        self.inverted = !self.inverted;
        self
    }

    /// Expands a contour without changing the source geometry (for outside input tolerance).
    pub fn outset(self, insets: EdgeInsets) -> Self {
        let top = insets.top.max(0.0);
        let right = insets.right.max(0.0);
        let bottom = insets.bottom.max(0.0);
        let left = insets.left.max(0.0);
        let mut outer = Self::new(
            RectF {
                x: self.rect.x - left,
                y: self.rect.y - top,
                width: self.rect.width + left + right,
                height: self.rect.height + top + bottom,
            },
            CornerRadii {
                top_left: self.radii.top_left + top.max(left),
                top_right: self.radii.top_right + top.max(right),
                bottom_right: self.radii.bottom_right + bottom.max(right),
                bottom_left: self.radii.bottom_left + bottom.max(left),
            },
        );
        outer.inverted = self.inverted;
        outer
    }

    /// Geometric input containment; antialiasing does not enlarge the hit target.
    pub fn contains(self, point: PointF) -> bool {
        let inside = self.rect.contains(point) && self.inner_coverage(point) >= 0.5;
        if self.inverted { !inside } else { inside }
    }

    /// Matches the analytic box renderer's inner border contour, including zero-width borders.
    pub fn inset(self, border: Border) -> Self {
        let top = border.top.width.max(0.0);
        let right = border.right.width.max(0.0);
        let bottom = border.bottom.width.max(0.0);
        let left = border.left.width.max(0.0);
        let mut inner = Self::new(
            RectF {
                x: self.rect.x + left,
                y: self.rect.y + top,
                width: (self.rect.width - left - right).max(0.0),
                height: (self.rect.height - top - bottom).max(0.0),
            },
            CornerRadii {
                top_left: (self.radii.top_left - top.max(left)).max(0.0),
                top_right: (self.radii.top_right - top.max(right)).max(0.0),
                bottom_right: (self.radii.bottom_right - bottom.max(right)).max(0.0),
                bottom_left: (self.radii.bottom_left - bottom.max(left)).max(0.0),
            },
        );
        inner.inverted = self.inverted;
        inner
    }

    pub fn is_valid(self) -> bool {
        [
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.radii.top_left,
            self.radii.top_right,
            self.radii.bottom_right,
            self.radii.bottom_left,
        ]
        .into_iter()
        .all(f32::is_finite)
            && self.rect.width >= 0.0
            && self.rect.height >= 0.0
            && [
                self.radii.top_left,
                self.radii.top_right,
                self.radii.bottom_right,
                self.radii.bottom_left,
            ]
            .into_iter()
            .all(|r| r >= 0.0)
    }

    /// One-output-pixel analytic antialiasing, shared with the composite fragment shaders.
    pub fn coverage(self, point: PointF) -> f32 {
        let coverage = self.inner_coverage(point);
        if self.inverted {
            1.0 - coverage
        } else {
            coverage
        }
    }

    fn inner_coverage(self, point: PointF) -> f32 {
        if self.rect.width <= 0.0 || self.rect.height <= 0.0 {
            return 0.0;
        }
        let half_x = self.rect.width * 0.5;
        let half_y = self.rect.height * 0.5;
        let x = point.x - self.rect.x;
        let y = point.y - self.rect.y;
        let radius = if x < half_x {
            if y < half_y {
                self.radii.top_left
            } else {
                self.radii.bottom_left
            }
        } else if y < half_y {
            self.radii.top_right
        } else {
            self.radii.bottom_right
        };
        let radius = radius.min(half_x.min(half_y));
        let qx = (x - half_x).abs() - (half_x - radius);
        let qy = (y - half_y).abs() - (half_y - radius);
        let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
        (0.5 - distance).clamp(0.0, 1.0)
    }
}
