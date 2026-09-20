use std::sync::Arc;

use crate::foundation::RectI;

use crate::ui::text::AtlasGlyph;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextRunId(pub u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextRunKey {
    pub text_revision: u64,
    pub text_hash: u64,
    pub font_revision: u64,
    pub family_hash: u64,
    pub size_bits: u32,
    pub weight: u16,
    pub line_height_bits: u32,
    pub width_constraint_bits: Option<u32>,
    pub height_constraint_bits: Option<u32>,
    pub scale_bits: u32,
    pub shaping: u16,
}

impl TextRunKey {
    pub fn new(
        text_revision: u64,
        font_revision: u64,
        family: &str,
        size: f32,
        weight: u16,
        line_height: f32,
        width_constraint: Option<f32>,
        height_constraint: Option<f32>,
        scale: f32,
    ) -> Self {
        Self {
            text_revision,
            text_hash: 0,
            font_revision,
            family_hash: stable_string_hash(family),
            size_bits: size.to_bits(),
            weight,
            line_height_bits: line_height.to_bits(),
            width_constraint_bits: width_constraint.map(f32::to_bits),
            height_constraint_bits: height_constraint.map(f32::to_bits),
            scale_bits: scale.to_bits(),
            shaping: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RetainedTextRun {
    pub glyphs: Arc<[AtlasGlyph]>,
    pub clusters: Arc<[u32]>,
    pub bounds: RectI,
    pub advance_width_px: f32,
    pub height_px: f32,
    pub baseline: f32,
    pub line_count: u32,
    pub text_revision: u64,
    pub style_revision: u64,
    pub atlas_version: u64,
    pub atlas_generation: u64,
}

#[derive(Clone, Debug)]
pub struct RetainedTextRequest<'a> {
    pub key: TextRunKey,
    pub text: &'a str,
    pub family: &'a str,
    pub font_size_px: i32,
    pub line_height_px: i32,
    pub max_width_px: Option<f32>,
    pub max_height_px: Option<f32>,
}

pub(crate) fn stable_string_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ byte as u64).wrapping_mul(0x100000001b3)
    })
}
