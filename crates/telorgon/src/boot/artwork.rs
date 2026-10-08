//! Bounded boot artwork decoding, including the `.splash` section of a UKI.

use std::io::Cursor;
use std::ops::Range;
use std::sync::Arc;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

use crate::foundation::SizeI;
use crate::graphics::render::{
    ImageAlphaMode, ImageColorEncoding, ImagePixelFormat, ImageResource,
};
use crate::ui::ImageId;

pub const MAX_EFI_ARTWORK_SOURCE_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_PE_HEADER_BYTES: usize = 64 * 1024;
pub const MAX_BOOT_ARTWORK_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_BOOT_ARTWORK_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_DECODER_ALLOCATION: u64 = 64 * 1024 * 1024;
const PE_HEADER_SIZE: usize = 24;
const PE_SECTION_SIZE: usize = 40;

#[derive(Debug, thiserror::Error)]
pub enum BootArtworkError {
    #[error("EFI artwork source exceeds the 512 MiB limit")]
    EfiTooLarge,
    #[error("PE artwork headers exceed the 64 KiB limit")]
    HeadersTooLarge,
    #[error("invalid PE image: {0}")]
    InvalidPe(&'static str),
    #[error("PE image contains multiple .splash sections")]
    DuplicateSplash,
    #[error("boot artwork exceeds the 8 MiB encoded size limit")]
    ArtworkTooLarge,
    #[error("boot artwork dimensions exceed the 4 million pixel limit")]
    DimensionsTooLarge,
    #[error("boot artwork must be a BMP or PNG image")]
    UnsupportedFormat,
    #[error("could not decode boot artwork: {0}")]
    Decode(#[from] image::ImageError),
}

/// Decodes the file-backed `.splash` section, returning `None` when absent.
/// This parses artwork only; it does not validate executable authenticity.
pub fn embedded_splash(
    efi: &[u8],
    image_id: ImageId,
) -> Result<Option<ImageResource>, BootArtworkError> {
    let Some(bytes) = splash_bytes(efi)? else {
        return Ok(None);
    };
    decode_artwork(bytes, image_id).map(Some)
}

/// Locates file-backed splash bytes from at most 64 KiB of PE headers.
/// The caller must check the returned range against the actual file length.
pub fn embedded_splash_range(headers: &[u8]) -> Result<Option<Range<usize>>, BootArtworkError> {
    if headers.len() > MAX_PE_HEADER_BYTES {
        return Err(BootArtworkError::HeadersTooLarge);
    }
    splash_range(headers, None)
}

/// Decodes BMP or PNG data into an owned, straight-alpha sRGB render resource.
pub fn decode_artwork(bytes: &[u8], image_id: ImageId) -> Result<ImageResource, BootArtworkError> {
    if bytes.len() > MAX_BOOT_ARTWORK_BYTES {
        return Err(BootArtworkError::ArtworkTooLarge);
    }
    let format = image::guess_format(bytes).map_err(|_| BootArtworkError::UnsupportedFormat)?;
    if !matches!(format, ImageFormat::Bmp | ImageFormat::Png) {
        return Err(BootArtworkError::UnsupportedFormat);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_BOOT_ARTWORK_PIXELS as u32);
    limits.max_image_height = Some(MAX_BOOT_ARTWORK_PIXELS as u32);
    limits.max_alloc = Some(MAX_DECODER_ALLOCATION);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_BOOT_ARTWORK_PIXELS {
        return Err(BootArtworkError::DimensionsTooLarge);
    }
    // Inspect the dimensions before allocating a decoded pixel buffer. BMP and
    // PNG have at most eight bytes per pixel in their native decoded formats.
    if decoder.total_bytes() > MAX_BOOT_ARTWORK_PIXELS * 8 {
        return Err(BootArtworkError::DimensionsTooLarge);
    }
    let rgba = DynamicImage::from_decoder(decoder)?.into_rgba8();
    Ok(ImageResource {
        image: image_id,
        content_version: 1,
        extent: SizeI {
            width: width as i32,
            height: height as i32,
        },
        color_encoding: ImageColorEncoding::Srgb,
        alpha_mode: ImageAlphaMode::Straight,
        pixel_format: ImagePixelFormat::Rgba8,
        pixels: Arc::from(rgba.into_raw()),
    })
}

fn splash_bytes(efi: &[u8]) -> Result<Option<&[u8]>, BootArtworkError> {
    if efi.len() > MAX_EFI_ARTWORK_SOURCE_BYTES {
        return Err(BootArtworkError::EfiTooLarge);
    }
    let headers = &efi[..efi.len().min(MAX_PE_HEADER_BYTES)];
    let range = splash_range(headers, Some(efi.len()))?;
    range
        .map(|range| {
            efi.get(range)
                .ok_or(BootArtworkError::InvalidPe("file range is truncated"))
        })
        .transpose()
}

fn splash_range(
    headers: &[u8],
    file_length: Option<usize>,
) -> Result<Option<Range<usize>>, BootArtworkError> {
    if headers.len() < 64 || headers.get(..2) != Some(b"MZ") {
        return Err(BootArtworkError::InvalidPe("missing DOS header"));
    }
    let pe_offset = le_u32(headers, 0x3c)? as usize;
    if pe_offset < 64 {
        return Err(BootArtworkError::InvalidPe("PE header overlaps DOS header"));
    }
    let header = byte_range(headers, pe_offset, PE_HEADER_SIZE)?;
    if header.get(..4) != Some(b"PE\0\0") {
        return Err(BootArtworkError::InvalidPe("missing PE signature"));
    }
    let section_count = le_u16(header, 6)? as usize;
    let optional_size = le_u16(header, 20)? as usize;
    let optional_offset = pe_offset
        .checked_add(PE_HEADER_SIZE)
        .ok_or(BootArtworkError::InvalidPe("header offset overflows"))?;
    let optional = byte_range(headers, optional_offset, optional_size)?;
    let minimum_optional_size = match le_u16(optional, 0)? {
        0x10b => 96,
        0x20b => 112,
        _ => return Err(BootArtworkError::InvalidPe("unsupported optional header")),
    };
    if optional_size < minimum_optional_size {
        return Err(BootArtworkError::InvalidPe("truncated optional header"));
    }
    let sections_offset = optional_offset
        .checked_add(optional_size)
        .ok_or(BootArtworkError::InvalidPe("section offset overflows"))?;
    let table_size = section_count
        .checked_mul(PE_SECTION_SIZE)
        .ok_or(BootArtworkError::InvalidPe("section table size overflows"))?;
    let sections = byte_range(headers, sections_offset, table_size)?;
    let header_end = sections_offset + table_size;
    let mut splash = None;
    for section in sections.chunks_exact(PE_SECTION_SIZE) {
        let virtual_size = le_u32(section, 8)? as usize;
        let raw_size = le_u32(section, 16)? as usize;
        let raw_offset = le_u32(section, 20)? as usize;
        if raw_size != 0 && raw_offset < header_end {
            return Err(BootArtworkError::InvalidPe("section overlaps PE headers"));
        }
        let raw_end = raw_offset
            .checked_add(raw_size)
            .ok_or(BootArtworkError::InvalidPe("section range overflows"))?;
        if raw_end > MAX_EFI_ARTWORK_SOURCE_BYTES {
            return Err(BootArtworkError::EfiTooLarge);
        }
        if raw_size != 0 && file_length.is_some_and(|length| raw_end > length) {
            return Err(BootArtworkError::InvalidPe("file range is truncated"));
        }
        if section.get(..8) != Some(b".splash\0") {
            continue;
        }
        if splash.is_some() {
            return Err(BootArtworkError::DuplicateSplash);
        }
        // SizeOfRawData may include alignment padding; VirtualSize names the
        // meaningful payload. Zero-filled virtual bytes are not file artwork.
        let payload_size = if virtual_size == 0 {
            raw_size
        } else {
            virtual_size.min(raw_size)
        };
        if payload_size > MAX_BOOT_ARTWORK_BYTES {
            return Err(BootArtworkError::ArtworkTooLarge);
        }
        splash = Some(raw_offset..raw_offset + payload_size);
    }
    Ok(splash)
}

fn byte_range(bytes: &[u8], offset: usize, size: usize) -> Result<&[u8], BootArtworkError> {
    let end = offset
        .checked_add(size)
        .ok_or(BootArtworkError::InvalidPe("file range overflows"))?;
    bytes
        .get(offset..end)
        .ok_or(BootArtworkError::InvalidPe("file range is truncated"))
}

fn le_u16(bytes: &[u8], offset: usize) -> Result<u16, BootArtworkError> {
    let field = byte_range(bytes, offset, 2)?;
    Ok(u16::from_le_bytes([field[0], field[1]]))
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, BootArtworkError> {
    let field = byte_range(bytes, offset, 4)?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bmp() -> Vec<u8> {
        let image = image::RgbImage::from_raw(2, 1, vec![255, 0, 0, 0, 128, 255]).unwrap();
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, ImageFormat::Bmp).unwrap();
        output.into_inner()
    }

    fn pe(section_name: &[u8; 8], payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 512 + payload.len() + 32];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[70..72].copy_from_slice(&1u16.to_le_bytes());
        bytes[84..86].copy_from_slice(&112u16.to_le_bytes());
        bytes[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        bytes[200..208].copy_from_slice(section_name);
        bytes[208..212].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes[216..220].copy_from_slice(&((payload.len() + 32) as u32).to_le_bytes());
        bytes[220..224].copy_from_slice(&512u32.to_le_bytes());
        bytes[512..512 + payload.len()].copy_from_slice(payload);
        bytes
    }

    #[test]
    fn extracts_decodes_and_trims_section_padding() {
        let bmp = bmp();
        let efi = pe(b".splash\0", &bmp);
        assert_eq!(splash_bytes(&efi).unwrap(), Some(bmp.as_slice()));
        let image = embedded_splash(&efi, ImageId(17)).unwrap().unwrap();
        assert_eq!(image.image, ImageId(17));
        assert_eq!(
            image.extent,
            SizeI {
                width: 2,
                height: 1
            }
        );
        assert_eq!(&*image.pixels, &[255, 0, 0, 255, 0, 128, 255, 255]);
        assert_eq!(image.color_encoding, ImageColorEncoding::Srgb);
        assert_eq!(image.alpha_mode, ImageAlphaMode::Straight);
    }

    #[test]
    fn absent_splash_is_not_an_error() {
        assert!(
            embedded_splash(&pe(b".text\0\0\0", &[1, 2]), ImageId(0))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn locates_splash_from_headers_without_reading_its_payload() {
        let bmp = bmp();
        let bytes = pe(b".splash\0", &bmp);
        let range = embedded_splash_range(&bytes[..240]).unwrap().unwrap();
        assert_eq!(range, 512..512 + bmp.len());
        assert!(matches!(
            embedded_splash(&bytes[..240], ImageId(0)),
            Err(BootArtworkError::InvalidPe(_))
        ));
        assert!(matches!(
            embedded_splash_range(&vec![0; MAX_PE_HEADER_BYTES + 1]),
            Err(BootArtworkError::HeadersTooLarge)
        ));
    }

    #[test]
    fn rejects_corrupt_headers_and_truncated_section_tables() {
        let valid = pe(b".splash\0", &bmp());
        for end in [0, 1, 63, 80, 100, 220, 239] {
            assert!(matches!(
                embedded_splash(&valid[..end], ImageId(0)),
                Err(BootArtworkError::InvalidPe(_))
            ));
        }
        let mut invalid = valid.clone();
        invalid[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            embedded_splash(&invalid, ImageId(0)),
            Err(BootArtworkError::InvalidPe(_))
        ));
        invalid = valid;
        invalid[64] = b'X';
        assert!(matches!(
            embedded_splash(&invalid, ImageId(0)),
            Err(BootArtworkError::InvalidPe(_))
        ));
    }

    #[test]
    fn rejects_out_of_bounds_and_header_overlapping_raw_sections() {
        for offset in [64, u32::MAX] {
            let mut bytes = pe(b".splash\0", &bmp());
            bytes[220..224].copy_from_slice(&offset.to_le_bytes());
            assert!(matches!(
                embedded_splash(&bytes, ImageId(0)),
                Err(BootArtworkError::InvalidPe(_) | BootArtworkError::EfiTooLarge)
            ));
        }
        let mut bytes = pe(b".splash\0", &bmp());
        bytes[216..220].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            embedded_splash(&bytes, ImageId(0)),
            Err(BootArtworkError::InvalidPe(_) | BootArtworkError::EfiTooLarge)
        ));
    }

    #[test]
    fn empty_splash_cannot_index_beyond_the_file() {
        let mut bytes = pe(b".splash\0", &[]);
        bytes[216..220].copy_from_slice(&0u32.to_le_bytes());
        bytes[220..224].copy_from_slice(&4096u32.to_le_bytes());
        assert!(matches!(
            embedded_splash(&bytes, ImageId(0)),
            Err(BootArtworkError::InvalidPe(_))
        ));
    }

    #[test]
    fn rejects_duplicate_splash_sections() {
        let mut bytes = pe(b".splash\0", &bmp());
        bytes[70..72].copy_from_slice(&2u16.to_le_bytes());
        let first = bytes[200..240].to_vec();
        bytes[240..280].copy_from_slice(&first);
        assert!(matches!(
            embedded_splash(&bytes, ImageId(0)),
            Err(BootArtworkError::DuplicateSplash)
        ));
    }

    #[test]
    fn rejects_invalid_artwork_and_excessive_dimensions() {
        assert!(matches!(
            embedded_splash(&pe(b".splash\0", b"not an image"), ImageId(0)),
            Err(BootArtworkError::UnsupportedFormat)
        ));
        let mut bytes = bmp();
        bytes[18..22].copy_from_slice(&4096i32.to_le_bytes());
        bytes[22..26].copy_from_slice(&4096i32.to_le_bytes());
        assert!(matches!(
            decode_artwork(&bytes, ImageId(0)),
            Err(BootArtworkError::DimensionsTooLarge)
        ));
        let oversized = vec![0; MAX_BOOT_ARTWORK_BYTES + 1];
        assert!(matches!(
            decode_artwork(&oversized, ImageId(0)),
            Err(BootArtworkError::ArtworkTooLarge)
        ));
    }

    #[test]
    fn supports_png_without_discarding_transparency() {
        let png = image::RgbaImage::from_raw(1, 1, vec![4, 5, 6, 7]).unwrap();
        let mut output = Cursor::new(Vec::new());
        png.write_to(&mut output, ImageFormat::Png).unwrap();
        let decoded = decode_artwork(&output.into_inner(), ImageId(2)).unwrap();
        assert_eq!(&*decoded.pixels, &[4, 5, 6, 7]);
    }
}
