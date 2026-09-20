use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use crate::assets::{
    AssetBundle, AssetEntry, AssetError, AssetKey, AssetKind, CursorAsset, IconAsset, ImageAsset,
    asset_image_id,
};
use crate::foundation::{ColorRgba8, SizeI};
use crate::graphics::render::{ImageAlphaMode, ImageColorEncoding, ImagePixelFormat, ImageResource};

const MAX_DIMENSION: u32 = 4096;
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SVG_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetRasterSize {
    pub width: u32,
    pub height: u32,
}

impl AssetRasterSize {
    pub fn new(width: u32, height: u32) -> Result<Self, AssetMediaError> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(AssetMediaError::InvalidRasterSize { width, height });
        }
        Ok(Self { width, height })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedAssetImage {
    pub key: AssetKey,
    pub extent: SizeI,
    pub alpha_mode: ImageAlphaMode,
    pub pixels_rgba8: Arc<[u8]>,
    image_id: crate::ui::ImageId,
    content_version: u64,
}

impl DecodedAssetImage {
    pub const fn content_version(&self) -> u64 {
        self.content_version
    }

    pub fn render_resource(&self) -> ImageResource {
        ImageResource {
            image: self.image_id,
            content_version: self.content_version,
            extent: self.extent,
            color_encoding: ImageColorEncoding::Srgb,
            alpha_mode: self.alpha_mode,
            pixel_format: ImagePixelFormat::Rgba8,
            pixels: Arc::clone(&self.pixels_rgba8),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CacheKey {
    asset: AssetKey,
    size: Option<AssetRasterSize>,
    tint: Option<u32>,
}

/// Bounded decoder and raster cache shared by GUI, shell, cursor, and native icon adapters.
pub struct AssetMediaCache {
    bundle: AssetBundle,
    decoded: BTreeMap<CacheKey, Arc<DecodedAssetImage>>,
}

impl AssetMediaCache {
    pub fn new(bundle: AssetBundle) -> Result<Self, AssetMediaError> {
        Ok(Self {
            bundle: bundle.validate().map_err(AssetMediaError::Catalog)?,
            decoded: BTreeMap::new(),
        })
    }

    pub const fn bundle(&self) -> AssetBundle {
        self.bundle
    }

    pub fn icon(
        &mut self,
        icon: IconAsset,
        size: Option<AssetRasterSize>,
    ) -> Result<Arc<DecodedAssetImage>, AssetMediaError> {
        self.decode(icon.key(), AssetKind::Icon, size, None)
    }

    pub fn tinted_icon(
        &mut self,
        icon: IconAsset,
        size: Option<AssetRasterSize>,
        tint: ColorRgba8,
    ) -> Result<Arc<DecodedAssetImage>, AssetMediaError> {
        self.decode(icon.key(), AssetKind::Icon, size, Some(tint))
    }

    pub fn image(
        &mut self,
        image: ImageAsset,
        size: Option<AssetRasterSize>,
    ) -> Result<Arc<DecodedAssetImage>, AssetMediaError> {
        self.decode(image.key(), AssetKind::Image, size, None)
    }

    pub fn cursor(
        &mut self,
        cursor: CursorAsset,
        size: Option<AssetRasterSize>,
    ) -> Result<Arc<DecodedAssetImage>, AssetMediaError> {
        self.decode(cursor.key(), AssetKind::Cursor, size, None)
    }

    pub fn tinted_cursor(
        &mut self,
        cursor: CursorAsset,
        size: Option<AssetRasterSize>,
        tint: ColorRgba8,
    ) -> Result<Arc<DecodedAssetImage>, AssetMediaError> {
        self.decode(cursor.key(), AssetKind::Cursor, size, Some(tint))
    }

    /// Decodes the intrinsic representation for every GUI-renderable catalog entry.
    pub fn preload_render_resources(&mut self) -> Result<Vec<ImageResource>, AssetMediaError> {
        let entries = self.bundle.iter().copied().collect::<Vec<_>>();
        let mut resources = Vec::new();
        for entry in entries {
            if !matches!(entry.kind, AssetKind::Icon | AssetKind::Image) {
                continue;
            }
            let decoded = self.decode(entry.key, entry.kind, None, None)?;
            resources.push(decoded.render_resource());
        }
        Ok(resources)
    }

    fn decode(
        &mut self,
        key: AssetKey,
        expected: AssetKind,
        size: Option<AssetRasterSize>,
        tint: Option<ColorRgba8>,
    ) -> Result<Arc<DecodedAssetImage>, AssetMediaError> {
        let cache_key = CacheKey {
            asset: key,
            size,
            tint: tint.map(packed_color),
        };
        if let Some(image) = self.decoded.get(&cache_key) {
            return Ok(Arc::clone(image));
        }
        let entry = self.bundle.get(key).ok_or(AssetError::NotFound(key))?;
        if entry.kind != expected {
            return Err(AssetError::KindMismatch {
                key,
                expected,
                actual: entry.kind,
            }
            .into());
        }
        let image = Arc::new(decode_entry(entry, size, tint)?);
        self.decoded.insert(cache_key, Arc::clone(&image));
        Ok(image)
    }
}

fn decode_entry(
    entry: &AssetEntry,
    requested: Option<AssetRasterSize>,
    tint: Option<ColorRgba8>,
) -> Result<DecodedAssetImage, AssetMediaError> {
    let mut decoded = if entry.media_type == "image/svg+xml" {
        decode_svg(entry, requested)
    } else {
        decode_raster(entry, requested)
    }?;
    if let Some(tint) = tint {
        colorize_alpha_mask(&mut decoded, tint);
        decoded.image_id = tinted_asset_image_id(entry.key, tint);
        decoded.content_version = content_version(
            entry,
            AssetRasterSize {
                width: decoded.extent.width.max(1) as u32,
                height: decoded.extent.height.max(1) as u32,
            },
            Some(tint),
        );
    }
    Ok(decoded)
}

fn decode_svg(
    entry: &AssetEntry,
    requested: Option<AssetRasterSize>,
) -> Result<DecodedAssetImage, AssetMediaError> {
    if entry.bytes.len() > MAX_SVG_BYTES {
        return Err(AssetMediaError::SvgTooLarge(entry.key));
    }
    let options = resvg::usvg::Options {
        resources_dir: None,
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..resvg::usvg::Options::default()
    };
    let tree = resvg::usvg::Tree::from_data(entry.bytes, &options)
        .map_err(|error| AssetMediaError::decode(entry.key, error))?;
    let intrinsic = tree.size();
    let size = match requested {
        Some(size) => size,
        None => AssetRasterSize::new(
            intrinsic.width().ceil().max(1.0) as u32,
            intrinsic.height().ceil().max(1.0) as u32,
        )?,
    };
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width, size.height).ok_or(
        AssetMediaError::InvalidRasterSize {
            width: size.width,
            height: size.height,
        },
    )?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        size.width as f32 / intrinsic.width(),
        size.height as f32 / intrinsic.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let pixels = Arc::<[u8]>::from(pixmap.take());
    Ok(decoded_image(
        entry,
        size,
        ImageAlphaMode::Premultiplied,
        pixels,
        None,
    ))
}

fn decode_raster(
    entry: &AssetEntry,
    requested: Option<AssetRasterSize>,
) -> Result<DecodedAssetImage, AssetMediaError> {
    let mut reader = image::ImageReader::new(Cursor::new(entry.bytes))
        .with_guessed_format()
        .map_err(|error| AssetMediaError::decode(entry.key, error))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| AssetMediaError::decode(entry.key, error))?;
    let rgba = match requested {
        Some(size) if decoded.width() != size.width || decoded.height() != size.height => decoded
            .resize_exact(
                size.width,
                size.height,
                image::imageops::FilterType::Lanczos3,
            )
            .to_rgba8(),
        _ => decoded.to_rgba8(),
    };
    let size = AssetRasterSize::new(rgba.width(), rgba.height())?;
    Ok(decoded_image(
        entry,
        size,
        ImageAlphaMode::Straight,
        Arc::from(rgba.into_raw()),
        None,
    ))
}

fn decoded_image(
    entry: &AssetEntry,
    size: AssetRasterSize,
    alpha_mode: ImageAlphaMode,
    pixels_rgba8: Arc<[u8]>,
    tint: Option<ColorRgba8>,
) -> DecodedAssetImage {
    DecodedAssetImage {
        key: entry.key,
        extent: SizeI {
            width: size.width as i32,
            height: size.height as i32,
        },
        alpha_mode,
        pixels_rgba8,
        image_id: tint.map_or_else(
            || asset_image_id(entry.key),
            |tint| tinted_asset_image_id(entry.key, tint),
        ),
        content_version: content_version(entry, size, tint),
    }
}

fn content_version(entry: &AssetEntry, size: AssetRasterSize, tint: Option<ColorRgba8>) -> u64 {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for byte in entry
        .bytes
        .iter()
        .copied()
        .chain(size.width.to_le_bytes())
        .chain(size.height.to_le_bytes())
        .chain(
            tint.into_iter()
                .flat_map(|color| [color.r, color.g, color.b, color.a]),
        )
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash.max(1)
}

fn colorize_alpha_mask(image: &mut DecodedAssetImage, tint: ColorRgba8) {
    let mut pixels = image.pixels_rgba8.to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]) * u16::from(tint.a) / 255;
        pixel[0] = tint.r;
        pixel[1] = tint.g;
        pixel[2] = tint.b;
        pixel[3] = alpha as u8;
    }
    image.pixels_rgba8 = Arc::from(pixels);
    image.alpha_mode = ImageAlphaMode::Straight;
}

fn packed_color(color: ColorRgba8) -> u32 {
    u32::from_le_bytes([color.r, color.g, color.b, color.a])
}

fn tinted_asset_image_id(key: AssetKey, tint: ColorRgba8) -> crate::ui::ImageId {
    let mut hash = asset_image_id(key).0 ^ 0x6d2b_79f5;
    for byte in [tint.r, tint.g, tint.b, tint.a] {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    crate::ui::ImageId(hash | (1 << 31))
}

#[derive(Debug, thiserror::Error)]
pub enum AssetMediaError {
    #[error(transparent)]
    Catalog(#[from] crate::assets::AssetCatalogError),
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error(
        "asset raster size {width}x{height} is outside the supported 1..={MAX_DIMENSION} range"
    )]
    InvalidRasterSize { width: u32, height: u32 },
    #[error("SVG asset `{0}` exceeds the {MAX_SVG_BYTES} byte parsing limit")]
    SvgTooLarge(AssetKey),
    #[error("could not decode asset `{key}`: {message}")]
    Decode { key: AssetKey, message: String },
}

impl AssetMediaError {
    fn decode(key: AssetKey, error: impl std::fmt::Display) -> Self {
        Self::Decode {
            key,
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: IconAsset = IconAsset::new(AssetKey::new("icons/test.svg"));
    static ENTRIES: [AssetEntry; 1] = [AssetEntry::embedded(
        SVG.key(),
        AssetKind::Icon,
        "image/svg+xml",
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4"><rect width="8" height="4" fill="red"/></svg>"#,
    )];

    #[test]
    fn svg_decode_is_bounded_cached_and_render_ready() {
        let mut cache = AssetMediaCache::new(AssetBundle::new(&ENTRIES)).unwrap();
        let size = AssetRasterSize::new(16, 16).unwrap();
        let first = cache.icon(SVG, Some(size)).unwrap();
        let second = cache.icon(SVG, Some(size)).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            first.extent,
            SizeI {
                width: 16,
                height: 16
            }
        );
        assert_eq!(first.pixels_rgba8.len(), 16 * 16 * 4);
        assert_eq!(first.render_resource().image, SVG.image_id());
    }

    #[test]
    fn tint_recolors_svg_alpha_and_uses_a_distinct_cache_entry() {
        let mut cache = AssetMediaCache::new(AssetBundle::new(&ENTRIES)).unwrap();
        let original = cache.icon(SVG, None).unwrap();
        let white = ColorRgba8::rgba(255, 255, 255, 192);
        let tinted = cache.tinted_icon(SVG, None, white).unwrap();
        let tinted_again = cache.tinted_icon(SVG, None, white).unwrap();

        assert!(Arc::ptr_eq(&tinted, &tinted_again));
        assert!(!Arc::ptr_eq(&original, &tinted));
        assert_ne!(
            original.render_resource().image,
            tinted.render_resource().image
        );
        assert!(
            tinted
                .pixels_rgba8
                .chunks_exact(4)
                .all(|pixel| pixel == [255, 255, 255, 192])
        );
        assert_eq!(tinted.alpha_mode, ImageAlphaMode::Straight);
    }
}
