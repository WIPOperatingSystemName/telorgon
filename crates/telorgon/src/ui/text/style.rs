use crate::foundation::ColorRgba8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTextStyle {
    pub color: ColorRgba8,
    pub font_size_px: i32,
    pub line_height_px: i32,
    pub font_family: String,
    pub font_weight: u16,
}

impl ResolvedTextStyle {
    pub fn new(color: ColorRgba8, font_size_px: i32) -> Self {
        Self {
            color,
            font_size_px,
            line_height_px: (font_size_px as f32 * 1.25).ceil() as i32,
            font_family: "sans-serif".to_string(),
            font_weight: 400,
        }
    }

    pub fn typography(
        mut self,
        family: impl Into<String>,
        weight: u16,
        line_height_px: i32,
    ) -> Self {
        self.font_family = family.into();
        self.font_weight = weight;
        self.line_height_px = line_height_px.max(self.font_size_px);
        self
    }
}
