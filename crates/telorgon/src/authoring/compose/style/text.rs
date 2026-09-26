use crate::foundation::ColorRgba8;
use crate::ui::TextStyle as RetainedTextStyle;

use super::Alignment;

/// Sparse, reusable text styling for composition code.
///
/// Unspecified fields inherit Telorgon's default text values. The runtime resolves this
/// authoring value into its retained text representation before mounting or patching a node.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextStyle {
    pub font_family: Option<&'static str>,
    pub color: Option<ColorRgba8>,
    pub size: Option<f32>,
    pub line_height: Option<f32>,
    pub weight: Option<u16>,
    pub text_align: Option<Alignment>,
    pub vertical_align: Option<Alignment>,
    pub fit_height: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_family_is_interned_without_changing_other_text_properties() {
        let style = TextStyle::new()
            .font_family("Inter 18pt")
            .size(19.0)
            .weight(600)
            .resolve_with(|family| {
                assert_eq!(family, "Inter 18pt");
                crate::ui::StringId(42)
            });
        assert_eq!(style.family, crate::ui::StringId(42));
        assert_eq!(style.size, 19.0);
        assert_eq!(style.weight, 600);
        assert_eq!(
            TextStyle::new()
                .resolve_with(|_| panic!("default family needs no interning"))
                .family,
            crate::ui::StringId(1)
        );
    }
}

impl TextStyle {
    pub const fn new() -> Self {
        Self {
            font_family: None,
            color: None,
            size: None,
            line_height: None,
            weight: None,
            text_align: None,
            vertical_align: None,
            fit_height: None,
        }
    }

    pub const fn color(mut self, color: ColorRgba8) -> Self {
        self.color = Some(color);
        self
    }
    pub const fn font_family(mut self, family: &'static str) -> Self {
        self.font_family = Some(family);
        self
    }
    pub(crate) fn resolve_with(
        self,
        intern: impl FnOnce(&str) -> crate::ui::StringId,
    ) -> RetainedTextStyle {
        let mut style = self.resolve();
        if let Some(family) = self.font_family {
            style.family = intern(family);
        }
        style
    }

    pub const fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    pub const fn line_height(mut self, line_height: f32) -> Self {
        self.line_height = Some(line_height);
        self
    }

    pub const fn weight(mut self, weight: u16) -> Self {
        self.weight = Some(weight);
        self
    }

    /// Aligns glyph lines within the text element's content box.
    pub const fn text_align(mut self, alignment: Alignment) -> Self {
        self.text_align = Some(alignment);
        self
    }

    pub const fn vertical_align(mut self, alignment: Alignment) -> Self {
        self.vertical_align = Some(alignment);
        self
    }

    pub const fn fit_height(mut self, enabled: bool) -> Self {
        self.fit_height = Some(enabled);
        self
    }

    #[doc(hidden)]
    pub fn resolve(self) -> RetainedTextStyle {
        let size = self.size.unwrap_or(14.0);
        RetainedTextStyle {
            color: self.color.unwrap_or(ColorRgba8::rgba(27, 31, 40, 255)),
            size,
            line_height: self.line_height.unwrap_or(size * 1.25),
            family: crate::ui::StringId(1),
            weight: self.weight.unwrap_or(400),
            align: self.text_align.unwrap_or_default().into(),
            vertical_align: self.vertical_align.unwrap_or_default().into(),
            fit_height: self.fit_height.unwrap_or(false),
        }
    }
}
