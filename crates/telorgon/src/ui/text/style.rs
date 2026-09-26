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

impl crate::ui::TextStyle {
    /// Shared logical metrics for layout and paint. Fitting uses the default 1.25
    /// line-height ratio and overrides authored font size and line spacing.
    pub(crate) fn metrics(
        self,
        height: Option<f32>,
        box_style: &crate::ui::BoxStyle,
    ) -> (f32, f32) {
        if self.fit_height && box_style.height != crate::ui::SizeRule::Shrink {
            if let Some(height) = height.filter(|height| height.is_finite() && *height >= 1.0) {
                let line_height = height.floor();
                return ((line_height / 1.25).floor().max(1.0), line_height);
            }
        }
        let size = self.size.ceil().max(1.0);
        (size, self.line_height.ceil().max(size))
    }

    pub(crate) fn vertical_offset(self, available: f32, measured: f32) -> f32 {
        let remaining = (available - measured).max(0.0);
        match self.vertical_align {
            crate::ui::TextAlign::Start => 0.0,
            crate::ui::TextAlign::Center => remaining * 0.5,
            crate::ui::TextAlign::End => remaining,
        }
    }
}

#[cfg(test)]
mod fitting_tests {
    use crate::authoring::compose::TextStyle;
    use crate::ui::{BoxStyle, SizeRule, TextAlign};

    #[test]
    fn height_fitting_overrides_font_size_and_line_spacing_but_not_shrink_height() {
        let mut style = TextStyle::new()
            .size(60.0)
            .line_height(80.0)
            .fit_height(true)
            .resolve();
        let mut box_style = BoxStyle::default();
        box_style.height = SizeRule::Logical(40.0);
        assert_eq!(style.metrics(Some(32.0), &box_style), (25.0, 32.0));
        assert_eq!(style.metrics(Some(80.0), &box_style), (64.0, 80.0));
        box_style.height = SizeRule::Shrink;
        assert_eq!(style.metrics(Some(32.0), &box_style), (60.0, 80.0));
        box_style.height = SizeRule::Fill(1.0);
        assert_eq!(style.metrics(Some(32.0), &box_style), (25.0, 32.0));
        style.fit_height = false;
        assert_eq!(style.metrics(Some(32.0), &box_style), (60.0, 80.0));
    }

    #[test]
    fn vertical_alignment_uses_measured_block_height() {
        let mut style = TextStyle::new().resolve();
        style.vertical_align = TextAlign::Center;
        assert_eq!(style.vertical_offset(40.0, 20.0), 10.0);
        style.vertical_align = TextAlign::End;
        assert_eq!(style.vertical_offset(40.0, 20.0), 20.0);
        assert_eq!(style.vertical_offset(10.0, 20.0), 0.0);
    }
}
