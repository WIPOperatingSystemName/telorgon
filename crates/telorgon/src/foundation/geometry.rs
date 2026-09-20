#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PointI {
    pub x: i32,
    pub y: i32,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SizeI {
    pub width: i32,
    pub height: i32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RectI {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl RectI {
    pub fn right(self) -> i32 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(self) -> i32 {
        self.y.saturating_add(self.height)
    }

    pub fn contains(self, point: PointI) -> bool {
        point.x >= self.x && point.y >= self.y && point.x < self.right() && point.y < self.bottom()
    }
}
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct PointF {
    pub x: f32,
    pub y: f32,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct SizeF {
    pub width: f32,
    pub height: f32,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl RectF {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };
    pub fn right(self) -> f32 {
        self.x + self.width
    }
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }
    pub fn area(self) -> f32 {
        self.width.max(0.0) * self.height.max(0.0)
    }
    pub fn contains(self, point: PointF) -> bool {
        point.x >= self.x && point.y >= self.y && point.x < self.right() && point.y < self.bottom()
    }
    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > x && bottom > y).then_some(Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
    pub fn union(self, other: Self) -> Self {
        if self.area() == 0.0 {
            return other;
        }
        if other.area() == 0.0 {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }
    pub fn inflate(self, amount: f32) -> Self {
        Self {
            x: self.x - amount,
            y: self.y - amount,
            width: self.width + amount * 2.0,
            height: self.height + amount * 2.0,
        }
    }
    pub fn translate(self, offset: PointF) -> Self {
        Self {
            x: self.x + offset.x,
            y: self.y + offset.y,
            ..self
        }
    }
    pub fn to_i32(self) -> RectI {
        RectI {
            x: self.x.floor() as i32,
            y: self.y.floor() as i32,
            width: self.width.ceil().max(0.0) as i32,
            height: self.height.ceil().max(0.0) as i32,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct EdgeInsets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}
impl EdgeInsets {
    pub const ZERO: Self = Self {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };
    pub const fn all(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
    pub fn horizontal(self) -> f32 {
        self.left + self.right
    }
    pub fn vertical(self) -> f32 {
        self.top + self.bottom
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Transform2D {
    pub translation: PointF,
    pub scale: PointF,
    /// Clockwise rotation in radians.
    pub rotation: f32,
    /// Unit-space pivot within the local border rectangle (0..1 on each axis).
    pub origin: PointF,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Affine2D {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Default for Affine2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine2D {
    pub const IDENTITY: Self = Self {
        m11: 1.0,
        m12: 0.0,
        m21: 0.0,
        m22: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    pub const fn translation(x: f32, y: f32) -> Self {
        Self {
            tx: x,
            ty: y,
            ..Self::IDENTITY
        }
    }

    /// Returns `self * local`, applying `local` before `self`.
    pub fn then(self, local: Self) -> Self {
        Self {
            m11: self.m11 * local.m11 + self.m21 * local.m12,
            m12: self.m12 * local.m11 + self.m22 * local.m12,
            m21: self.m11 * local.m21 + self.m21 * local.m22,
            m22: self.m12 * local.m21 + self.m22 * local.m22,
            tx: self.m11 * local.tx + self.m21 * local.ty + self.tx,
            ty: self.m12 * local.tx + self.m22 * local.ty + self.ty,
        }
    }

    pub fn transform_point(self, point: PointF) -> PointF {
        PointF {
            x: self.m11 * point.x + self.m21 * point.y + self.tx,
            y: self.m12 * point.x + self.m22 * point.y + self.ty,
        }
    }

    pub fn transform_rect(self, rect: RectF) -> RectF {
        let points = [
            self.transform_point(PointF {
                x: rect.x,
                y: rect.y,
            }),
            self.transform_point(PointF {
                x: rect.right(),
                y: rect.y,
            }),
            self.transform_point(PointF {
                x: rect.right(),
                y: rect.bottom(),
            }),
            self.transform_point(PointF {
                x: rect.x,
                y: rect.bottom(),
            }),
        ];
        let left = points
            .iter()
            .map(|point| point.x)
            .fold(f32::INFINITY, f32::min);
        let top = points
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min);
        let right = points
            .iter()
            .map(|point| point.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let bottom = points
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max);
        RectF {
            x: left,
            y: top,
            width: (right - left).max(0.0),
            height: (bottom - top).max(0.0),
        }
    }

    pub fn inverse(self) -> Option<Self> {
        let determinant = self.m11 * self.m22 - self.m21 * self.m12;
        if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
            return None;
        }
        let inverse = 1.0 / determinant;
        let m11 = self.m22 * inverse;
        let m12 = -self.m12 * inverse;
        let m21 = -self.m21 * inverse;
        let m22 = self.m11 * inverse;
        Some(Self {
            m11,
            m12,
            m21,
            m22,
            tx: -(m11 * self.tx + m21 * self.ty),
            ty: -(m12 * self.tx + m22 * self.ty),
        })
    }
}

impl Transform2D {
    pub fn affine_for_rect(self, rect: RectF) -> Affine2D {
        let pivot = PointF {
            x: rect.x + rect.width * self.origin.x,
            y: rect.y + rect.height * self.origin.y,
        };
        let (sin, cos) = self.rotation.sin_cos();
        Affine2D::translation(self.translation.x, self.translation.y)
            .then(Affine2D::translation(pivot.x, pivot.y))
            .then(Affine2D {
                m11: cos * self.scale.x,
                m12: sin * self.scale.x,
                m21: -sin * self.scale.y,
                m22: cos * self.scale.y,
                tx: 0.0,
                ty: 0.0,
            })
            .then(Affine2D::translation(-pivot.x, -pivot.y))
    }
}
impl Default for Transform2D {
    fn default() -> Self {
        Self {
            translation: PointF::default(),
            scale: PointF { x: 1.0, y: 1.0 },
            rotation: 0.0,
            origin: PointF::default(),
        }
    }
}
