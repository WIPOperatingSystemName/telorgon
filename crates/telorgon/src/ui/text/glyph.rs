use crate::foundation::ColorRgba8;
use cosmic_text::{SwashContent, SwashImage};

use crate::ui::text::{TextError, TextResult};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AtlasGlyph {
    pub dst_x: i32,
    pub dst_y: i32,
    pub width_px: i32,
    pub height_px: i32,
    pub atlas_x: i32,
    pub atlas_y: i32,
    pub color: ColorRgba8,
}

pub(crate) fn glyph_image_alpha(image: &SwashImage) -> TextResult<Vec<u8>> {
    let width = image.placement.width as usize;
    let height = image.placement.height as usize;
    let pixel_count = width * height;

    match image.content {
        SwashContent::Mask => {
            if image.data.len() < pixel_count {
                return Err(TextError::new("glyph mask is shorter than its placement"));
            }
            Ok(image.data[..pixel_count].to_vec())
        }
        SwashContent::SubpixelMask | SwashContent::Color => {
            if image.data.len() < pixel_count * 4 {
                return Err(TextError::new(
                    "glyph RGBA data is shorter than its placement",
                ));
            }
            let mut alpha = Vec::with_capacity(pixel_count);
            for pixel in image.data.chunks_exact(4).take(pixel_count) {
                alpha.push(pixel[3].max(pixel[0]).max(pixel[1]).max(pixel[2]));
            }
            Ok(alpha)
        }
    }
}
