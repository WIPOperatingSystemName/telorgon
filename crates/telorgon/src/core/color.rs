#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ColorRgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ColorRgba8 {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Blends the stored RGB channels toward white, preserving alpha.
    ///
    /// `0.0` leaves the color unchanged; `1.0` produces white. Amounts outside
    /// this range are clamped, and NaN leaves the color unchanged. Channels are
    /// rounded to the nearest integer. This is a tint in encoded RGB space,
    /// not a perceptual lightness or linear-light adjustment.
    pub const fn lighten(self, amount: f32) -> Self {
        self.shade_toward(255, amount)
    }

    /// Blends the stored RGB channels toward black, preserving alpha.
    ///
    /// `0.0` leaves the color unchanged; `1.0` produces black. Clamping,
    /// rounding, NaN handling, and color-space behavior match [`Self::lighten`].
    pub const fn darken(self, amount: f32) -> Self {
        self.shade_toward(0, amount)
    }

    const fn shade_toward(self, target: u8, amount: f32) -> Self {
        // This comparison also makes NaN a no-op without relying on const clamp.
        if !(amount > 0.0) {
            return self;
        }
        if amount >= 1.0 {
            return Self::rgba(target, target, target, self.a);
        }
        const fn channel(value: u8, target: u8, amount: f32) -> u8 {
            (value as f32 + (target as f32 - value as f32) * amount + 0.5) as u8
        }
        Self::rgba(
            channel(self.r, target, amount),
            channel(self.g, target, amount),
            channel(self.b, target, amount),
            self.a,
        )
    }

    pub const fn to_ne_u32(self) -> u32 {
        u32::from_ne_bytes([self.r, self.g, self.b, self.a])
    }

    pub fn with_alpha_scale(self, opacity: f32) -> Self {
        let alpha = (self.a as f32 * opacity.clamp(0.0, 1.0)).round() as u8;
        Self { a: alpha, ..self }
    }
}

#[cfg(test)]
mod tests {
    use super::ColorRgba8;

    #[test]
    fn shades_are_const_compatible_and_round_channels() {
        const BASE: ColorRgba8 = ColorRgba8::rgba(20, 100, 200, 73);
        const LIGHT: ColorRgba8 = BASE.lighten(0.5);
        const DARK: ColorRgba8 = BASE.darken(0.25);
        assert_eq!(LIGHT, ColorRgba8::rgba(138, 178, 228, 73));
        assert_eq!(DARK, ColorRgba8::rgba(15, 75, 150, 73));
    }

    #[test]
    fn shade_boundaries_preserve_alpha_and_handle_nonfinite_amounts() {
        for alpha in [0, 73, 255] {
            let base = ColorRgba8::rgba(20, 100, 200, alpha);
            for amount in [f32::NEG_INFINITY, -1.0, 0.0, f32::NAN] {
                assert_eq!(base.lighten(amount), base);
                assert_eq!(base.darken(amount), base);
            }
            for amount in [1.0, 2.0, f32::INFINITY] {
                assert_eq!(base.lighten(amount), ColorRgba8::rgba(255, 255, 255, alpha));
                assert_eq!(base.darken(amount), ColorRgba8::rgba(0, 0, 0, alpha));
            }
        }
    }

    #[test]
    fn shades_are_monotonic_across_all_channel_values() {
        for value in 0..=255 {
            let base = ColorRgba8::rgba(value, value, value, 123);
            let mut previous_light = value;
            let mut previous_dark = value;
            for step in 0..=100 {
                let amount = step as f32 / 100.0;
                let light = base.lighten(amount);
                let dark = base.darken(amount);
                assert!(light.r >= previous_light);
                assert!(dark.r <= previous_dark);
                assert_eq!(light, ColorRgba8::rgba(light.r, light.r, light.r, 123));
                assert_eq!(dark, ColorRgba8::rgba(dark.r, dark.r, dark.r, 123));
                previous_light = light.r;
                previous_dark = dark.r;
            }
        }
    }
}
