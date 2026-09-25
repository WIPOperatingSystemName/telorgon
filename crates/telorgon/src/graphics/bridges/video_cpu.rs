//! CPU video-to-image conversion for application, shell and embedded renderers.
//! Runs on an application worker or control thread, never on an audio realtime callback.
use crate::{
    foundation::SizeI,
    graphics::render::{ImageAlphaMode, ImageColorEncoding, ImagePixelFormat, ImageResource},
    integrations::pipewire::MediaError,
    media::video::*,
    ui::ImageId,
};
use std::sync::Arc;
#[path = "video_color.rs"]
mod color_conversion;
pub use color_conversion::HdrToneMap;

/// Convert completed tightly packed RGBA readback in place on the completion worker.
/// X formats flatten the already premultiplied pixels against black and set the unused byte.
pub(crate) fn rgba_to_packed(bytes: &mut [u8], pixel: PixelFormat) -> Result<(), MediaError> {
    if bytes.len() % 4 != 0
        || !matches!(
            pixel,
            PixelFormat::Rgba8 | PixelFormat::Bgra8 | PixelFormat::Rgbx8 | PixelFormat::Bgrx8
        )
    {
        return Err(MediaError::InvalidArgument("packed screen video format"));
    }
    if pixel == PixelFormat::Rgba8 {
        return Ok(());
    }
    for rgba in bytes.chunks_exact_mut(4) {
        if matches!(pixel, PixelFormat::Bgra8 | PixelFormat::Bgrx8) {
            rgba.swap(0, 2);
        }
        if matches!(pixel, PixelFormat::Rgbx8 | PixelFormat::Bgrx8) {
            rgba[3] = 255;
        }
    }
    Ok(())
}

/// Explicit interpretation for formats whose transport does not declare alpha association.
#[derive(Clone, Copy, Debug)]
pub struct VideoImageOptions {
    pub alpha: ImageAlphaMode,
    /// Fills unknown fields only. Known source metadata is always authoritative.
    pub fallback_color: Option<Colorimetry>,
    /// Maximum newly allocated output bytes per conversion (default 64 MiB).
    pub max_bytes: usize,
    /// Required for PQ/HLG input. Output remains SDR sRGB; None rejects HDR input.
    pub hdr_tone_map: Option<HdrToneMap>,
}
impl Default for VideoImageOptions {
    fn default() -> Self {
        Self {
            alpha: ImageAlphaMode::Straight,
            fallback_color: None,
            max_bytes: 64 * 1024 * 1024,
            hdr_tone_map: None,
        }
    }
}
impl VideoImageOptions {
    /// Discover color-conversion support without acquiring a frame or native resource.
    pub fn supports_color(self, format: VideoFormat) -> Result<(), MediaError> {
        format.validate()?;
        self.resolve_color(format).map(|_| ())
    }
    fn resolve_color(self, format: VideoFormat) -> Result<Colorimetry, MediaError> {
        let mut color = format.color;
        if let Some(f) = self.fallback_color {
            if color.range == ColorRange::Unknown {
                color.range = f.range;
            }
            if color.matrix == ColorMatrix::Unknown {
                color.matrix = f.matrix;
            }
            if color.primaries == ColorPrimaries::Unknown {
                color.primaries = f.primaries;
            }
            if color.transfer == TransferFunction::Unknown {
                color.transfer = f.transfer;
            }
        }
        let yuv = matches!(
            format.pixel,
            PixelFormat::Nv12 | PixelFormat::I420 | PixelFormat::Yuy2 | PixelFormat::P010
        );
        if color.primaries == ColorPrimaries::Unknown
            || color.transfer == TransferFunction::Unknown
            || (matches!(color.transfer, TransferFunction::Pq | TransferFunction::Hlg)
                && self.hdr_tone_map.is_none())
            || (color.transfer == TransferFunction::Hlg
                && color.primaries != ColorPrimaries::Bt2020)
            || color.range == ColorRange::Unknown
            || if yuv {
                !matches!(
                    color.matrix,
                    ColorMatrix::Bt601 | ColorMatrix::Bt709 | ColorMatrix::Bt2020
                )
            } else {
                color.matrix != ColorMatrix::Rgb
            }
        {
            return Err(MediaError::Unsupported(
                "CPU preview colorimetry; supply missing metadata or use a capable GPU converter",
            ));
        }
        Ok(color)
    }
}
/// Converts to straight-alpha sRGB RGBA8, applying crop and SPA transform. Chroma uses
/// nearest-neighbor reconstruction. Cursor metadata remains separate for host overlays.
/// BT.709, Display-P3 and BT.2020 primaries are converted in linear light to sRGB.
/// Out-of-gamut output clips; PQ/HLG require an explicit HDR-to-SDR preview policy.
/// The returned resource owns its pixels; the native pool lease can be released immediately.
pub fn video_image(
    frame: &CpuVideoFrame,
    image: ImageId,
    revision: u64,
    options: VideoImageOptions,
) -> Result<ImageResource, MediaError> {
    if revision == 0 {
        return Err(MediaError::InvalidArgument("video image revision"));
    }
    let format = frame.format();
    let metadata = frame.metadata();
    let color = options.resolve_color(format)?;
    let crop = metadata.crop.unwrap_or(VideoRect {
        x: 0,
        y: 0,
        width: format.width,
        height: format.height,
    });
    let (width, height) = if matches!(
        metadata.transform,
        VideoTransform::Rotate90
            | VideoTransform::Rotate270
            | VideoTransform::Flipped90
            | VideoTransform::Flipped270
    ) {
        (crop.height, crop.width)
    } else {
        (crop.width, crop.height)
    };
    let size = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n <= options.max_bytes && *n <= 512 * 1024 * 1024)
        .ok_or(MediaError::ResourceLimit("video preview output"))?;
    let mut pixels = vec![0u8; size];
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = source_position(x, y, crop.width, crop.height, metadata.transform);
            let mut value = sample(frame, (sx + crop.x) as usize, (sy + crop.y) as usize, color);
            let alpha = if options.alpha == ImageAlphaMode::Opaque {
                1.0
            } else if value[3].is_finite() {
                value[3].clamp(0.0, 1.0)
            } else {
                0.0
            };
            for component in &mut value[..3] {
                if options.alpha == ImageAlphaMode::Premultiplied {
                    *component = if alpha > 0.0 { *component / alpha } else { 0.0 };
                }
                if !component.is_finite() {
                    *component = 0.0;
                }
            }
            let converted = color_conversion::convert(
                [value[0], value[1], value[2]],
                color,
                options.hdr_tone_map,
            );
            value[..3].copy_from_slice(&converted);
            let offset = ((y * width + x) * 4) as usize;
            for c in 0..3 {
                pixels[offset + c] = (value[c].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            pixels[offset + 3] = (alpha * 255.0).round() as u8;
        }
    }
    Ok(ImageResource {
        image,
        content_version: revision,
        extent: SizeI {
            width: width as i32,
            height: height as i32,
        },
        color_encoding: ImageColorEncoding::Srgb,
        alpha_mode: ImageAlphaMode::Straight,
        pixel_format: ImagePixelFormat::Rgba8,
        pixels: Arc::from(pixels),
    })
}
fn source_position(x: u32, y: u32, w: u32, h: u32, transform: VideoTransform) -> (u32, u32) {
    // SPA rotations are counter-clockwise. Invert rotation, then the source reflection.
    let (mut sx, sy) = match transform {
        VideoTransform::Normal | VideoTransform::Flipped => (x, y),
        VideoTransform::Rotate90 | VideoTransform::Flipped90 => (w - 1 - y, x),
        VideoTransform::Rotate180 | VideoTransform::Flipped180 => (w - 1 - x, h - 1 - y),
        VideoTransform::Rotate270 | VideoTransform::Flipped270 => (y, h - 1 - x),
    };
    if matches!(
        transform,
        VideoTransform::Flipped
            | VideoTransform::Flipped90
            | VideoTransform::Flipped180
            | VideoTransform::Flipped270
    ) {
        sx = w - 1 - sx;
    }
    (sx, sy)
}
fn sample(frame: &CpuVideoFrame, x: usize, y: usize, color: Colorimetry) -> [f32; 4] {
    let format = frame.format();
    // CpuVideoFrame construction validated every row span, including negative strides.
    let row = frame.plane(0).unwrap().row(y).unwrap();
    let mut rgb = match format.pixel {
        PixelFormat::Rgba8 | PixelFormat::Rgbx8 => [
            row[x * 4] as f32 / 255.0,
            row[x * 4 + 1] as f32 / 255.0,
            row[x * 4 + 2] as f32 / 255.0,
            if format.pixel == PixelFormat::Rgbx8 {
                1.0
            } else {
                row[x * 4 + 3] as f32 / 255.0
            },
        ],
        PixelFormat::Bgra8 | PixelFormat::Bgrx8 => [
            row[x * 4 + 2] as f32 / 255.0,
            row[x * 4 + 1] as f32 / 255.0,
            row[x * 4] as f32 / 255.0,
            if format.pixel == PixelFormat::Bgrx8 {
                1.0
            } else {
                row[x * 4 + 3] as f32 / 255.0
            },
        ],
        PixelFormat::Rgb8 => [
            row[x * 3] as f32 / 255.0,
            row[x * 3 + 1] as f32 / 255.0,
            row[x * 3 + 2] as f32 / 255.0,
            1.0,
        ],
        PixelFormat::Bgr8 => [
            row[x * 3 + 2] as f32 / 255.0,
            row[x * 3 + 1] as f32 / 255.0,
            row[x * 3] as f32 / 255.0,
            1.0,
        ],
        PixelFormat::RgbaF16 => std::array::from_fn(|c| {
            half(u16::from_le_bytes([
                row[x * 8 + c * 2],
                row[x * 8 + c * 2 + 1],
            ]))
        }),
        _ => {
            let (luma, u, v, max, scale) = match format.pixel {
                PixelFormat::Nv12 => {
                    let uv = frame.plane(1).unwrap().row(y / 2).unwrap();
                    (
                        row[x] as f32,
                        uv[x / 2 * 2] as f32,
                        uv[x / 2 * 2 + 1] as f32,
                        255.0,
                        1.0,
                    )
                }
                PixelFormat::I420 => (
                    row[x] as f32,
                    frame.plane(1).unwrap().row(y / 2).unwrap()[x / 2] as f32,
                    frame.plane(2).unwrap().row(y / 2).unwrap()[x / 2] as f32,
                    255.0,
                    1.0,
                ),
                PixelFormat::Yuy2 => (
                    row[x * 2] as f32,
                    row[x / 2 * 4 + 1] as f32,
                    row[x / 2 * 4 + 3] as f32,
                    255.0,
                    1.0,
                ),
                PixelFormat::P010 => {
                    let uv = frame.plane(1).unwrap().row(y / 2).unwrap();
                    let word =
                        |r: &[u8], i: usize| (u16::from_le_bytes([r[i], r[i + 1]]) >> 6) as f32;
                    (
                        word(row, x * 2),
                        word(uv, x / 2 * 4),
                        word(uv, x / 2 * 4 + 2),
                        1023.0,
                        4.0,
                    )
                }
                _ => unreachable!(),
            };
            let (yy, cb, cr) = if color.range == ColorRange::Limited {
                (
                    (luma - 16.0 * scale) / (219.0 * scale),
                    (u - 128.0 * scale) / (224.0 * scale),
                    (v - 128.0 * scale) / (224.0 * scale),
                )
            } else {
                (
                    luma / max,
                    (u - 128.0 * scale) / max,
                    (v - 128.0 * scale) / max,
                )
            };
            let (kr, kb) = match color.matrix {
                ColorMatrix::Bt601 => (0.299, 0.114),
                ColorMatrix::Bt2020 => (0.2627, 0.0593),
                _ => (0.2126, 0.0722),
            };
            let r = yy + 2.0 * (1.0 - kr) * cr;
            let b = yy + 2.0 * (1.0 - kb) * cb;
            let g = (yy - kr * r - kb * b) / (1.0 - kr - kb);
            return [r, g, b, 1.0];
        }
    };
    if color.range == ColorRange::Limited {
        for c in &mut rgb[..3] {
            *c = (*c * 255.0 - 16.0) / 219.0;
        }
    }
    rgb
}
// Rec.709 OETF inverse and sRGB transfer per Linux V4L2 colorspaces-details and IEC sRGB.
fn linearize(v: f32, t: TransferFunction) -> f32 {
    let a = v.abs();
    let linear = match t {
        TransferFunction::Linear => a,
        TransferFunction::Srgb => {
            if a <= 0.04045 {
                a / 12.92
            } else {
                ((a + 0.055) / 1.055).powf(2.4)
            }
        }
        _ => {
            if a < 0.081 {
                a / 4.5
            } else {
                ((a + 0.099) / 1.099).powf(1.0 / 0.45)
            }
        }
    };
    linear.copysign(v)
}
fn srgb_encode(v: f32) -> f32 {
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}
fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = (bits >> 10) & 31;
    let fraction = bits & 1023;
    match exp {
        0 => sign * (fraction as f32) * 2.0f32.powi(-24),
        31 => {
            if fraction == 0 {
                sign * f32::INFINITY
            } else {
                0.0
            }
        }
        _ => sign * (1.0 + fraction as f32 / 1024.0) * 2.0f32.powi(exp as i32 - 15),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_readback_conversion_preserves_channels_and_sets_opaque_padding() {
        let original = [8, 12, 20, 32, 1, 2, 3, 4];
        for (pixel, expected) in [
            (PixelFormat::Rgba8, original),
            (PixelFormat::Bgra8, [20, 12, 8, 32, 3, 2, 1, 4]),
            (PixelFormat::Rgbx8, [8, 12, 20, 255, 1, 2, 3, 255]),
            (PixelFormat::Bgrx8, [20, 12, 8, 255, 3, 2, 1, 255]),
        ] {
            let mut bytes = original;
            rgba_to_packed(&mut bytes, pixel).unwrap();
            assert_eq!(bytes, expected);
        }
        let mut invalid = [1, 2, 3];
        assert!(rgba_to_packed(&mut invalid, PixelFormat::Bgra8).is_err());
        assert_eq!(invalid, [1, 2, 3]);
        let mut bytes = original;
        assert!(rgba_to_packed(&mut bytes, PixelFormat::Nv12).is_err());
        assert_eq!(bytes, original);
    }
    #[test]
    fn cropped_rotated_negative_stride_pixels_are_rendered() {
        let frame = CpuVideoFrame::from_planes(
            VideoFormat::rgba(2, 3, 30),
            vec![CpuPlane {
                bytes: vec![
                    5, 0, 0, 255, 6, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255, 1, 0, 0, 255, 2, 0, 0,
                    255,
                ],
                offset: 16,
                stride: -8,
            }],
            FrameMetadata {
                crop: Some(VideoRect {
                    x: 0,
                    y: 1,
                    width: 2,
                    height: 2,
                }),
                transform: VideoTransform::Rotate90,
                ..Default::default()
            },
        )
        .unwrap();
        let result = video_image(&frame, ImageId(8), 1, VideoImageOptions::default()).unwrap();
        assert_eq!(
            result
                .pixels
                .chunks_exact(4)
                .map(|p| p[0])
                .collect::<Vec<_>>(),
            vec![4, 6, 3, 5]
        );
        assert!(matches!(
            video_image(
                &frame,
                ImageId(8),
                1,
                VideoImageOptions {
                    max_bytes: 15,
                    ..Default::default()
                }
            ),
            Err(MediaError::ResourceLimit(_))
        ));
    }
    #[test]
    fn limited_nv12_black_and_white_and_unknown_metadata() {
        let format = VideoFormat {
            pixel: PixelFormat::Nv12,
            color: Colorimetry::BT709,
            ..VideoFormat::rgba(2, 2, 30)
        };
        let frame = CpuVideoFrame::from_planes(
            format,
            vec![
                CpuPlane {
                    bytes: vec![16, 235, 16, 235],
                    offset: 0,
                    stride: 2,
                },
                CpuPlane {
                    bytes: vec![128, 128],
                    offset: 0,
                    stride: 2,
                },
            ],
            FrameMetadata::default(),
        )
        .unwrap();
        let result = video_image(&frame, ImageId(9), 1, VideoImageOptions::default()).unwrap();
        assert_eq!(&result.pixels[..8], &[0, 0, 0, 255, 255, 255, 255, 255]);
        let frame = CpuVideoFrame::packed(
            VideoFormat {
                color: Colorimetry::UNKNOWN,
                ..VideoFormat::rgba(1, 1, 30)
            },
            vec![128, 64, 32, 255],
            FrameMetadata::default(),
        )
        .unwrap();
        assert!(video_image(&frame, ImageId(9), 1, VideoImageOptions::default()).is_err());
        assert_eq!(
            &*video_image(
                &frame,
                ImageId(9),
                1,
                VideoImageOptions {
                    fallback_color: Some(Colorimetry::SRGB),
                    ..Default::default()
                }
            )
            .unwrap()
            .pixels,
            &[128, 64, 32, 255]
        );
    }
}
