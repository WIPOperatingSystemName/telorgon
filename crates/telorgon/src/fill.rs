//! Reusable shell-preview fill materials; ordinary widget backgrounds remain separate.
use crate::core::ColorRgba8;

/// Reusable interior appearance for shell previews. Borders and geometry belong to the design.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Fill {
    /// No interior paint; the preview may still have a border.
    #[default]
    None,
    /// Straight RGBA; alpha reveals the sharp desktop underneath.
    Color(ColorRgba8),
    /// Cached, filtered desktop backdrop with an opaque tinted result.
    Glass(GlassStyle),
}

impl Fill {
    /// Flat color, or the glass tint (also the software fallback).
    pub const fn color(self) -> ColorRgba8 {
        match self {
            Self::None => ColorRgba8::rgba(0, 0, 0, 0),
            Self::Color(color) => color,
            Self::Glass(style) => style.tint,
        }
    }
}

/// Rounded liquid-glass lens over a cached desktop backdrop. Software uses the flat tint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassStyle {
    /// RGB tint and tint strength in alpha; zero alpha still produces opaque glass.
    pub tint: ColorRgba8,
    /// Gaussian blur diameter in logical pixels (sigma = half this value).
    /// Filtering preserves the full backdrop resolution; the refracted rim uses a sharp source.
    /// Zero keeps the sharp backdrop. Clamped to 0..=64; non-finite values use 4.
    pub blur_radius: f32,
    /// Inward glass band width from the rounded surface outline in logical pixels
    /// (1..=128), limited to the smaller window half-size. Corner bending blends smoothly.
    pub bevel_width: f32,
    /// Additional gentle inner fade in logical pixels (0..=128). Zero keeps the edge-only profile.
    pub blend_softness: f32,
    /// Refraction strength in logical pixels (0..=64); zero removes lens displacement.
    pub refraction: f32,
    /// RGB separation near the rim in logical pixels (0..=4). Zero uses one texture sample.
    pub dispersion: f32,
    /// Grazing-angle reflection strength (0..=1).
    pub fresnel: f32,
}

impl GlassStyle {
    /// A mildly blurred, mostly clear lens with a restrained reflective rim.
    pub const fn liquid() -> Self {
        Self {
            tint: ColorRgba8::rgba(23, 27, 37, 32),
            blur_radius: 4.0,
            bevel_width: 24.0,
            blend_softness: 0.0,
            refraction: 18.0,
            dispersion: 0.65,
            fresnel: 0.45,
        }
    }

    pub(crate) fn normalized(mut self) -> Self {
        fn finite(value: f32, default: f32, min: f32, max: f32) -> f32 {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                default
            }
        }
        self.blur_radius = finite(self.blur_radius, 4.0, 0.0, 64.0);
        self.bevel_width = finite(self.bevel_width, 24.0, 1.0, 128.0);
        self.blend_softness = finite(self.blend_softness, 0.0, 0.0, 128.0);
        self.refraction = finite(self.refraction, 18.0, 0.0, 64.0);
        self.dispersion = finite(self.dispersion, 0.65, 0.0, 4.0);

        self.fresnel = finite(self.fresnel, 0.45, 0.0, 1.0);
        self
    }
}

impl Default for GlassStyle {
    fn default() -> Self {
        Self::liquid()
    }
}
