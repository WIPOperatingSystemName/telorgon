use crate::platform::uefi::{UefiError, UefiResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelMasks {
    pub red: u32,
    pub green: u32,
    pub blue: u32,
    pub reserved: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    RgbReserved,
    BgrReserved,
    BitMask(ChannelMasks),
    BltOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramebufferLayout {
    width: u32,
    height: u32,
    stride_bytes: usize,
    pixel_bytes: usize,
    required_bytes: usize,
    pixel_format: PixelFormat,
}

impl FramebufferLayout {
    pub fn new(
        width: u32,
        height: u32,
        pixels_per_scan_line: u32,
        mapped_bytes: usize,
        format: PixelFormat,
    ) -> UefiResult<Self> {
        if width == 0 || height == 0 {
            return Err(UefiError::InvalidFramebuffer);
        }
        let pixel_bytes = match format {
            PixelFormat::RgbReserved | PixelFormat::BgrReserved => 4,
            PixelFormat::BltOnly => 0,
            PixelFormat::BitMask(masks) => {
                let channels = [masks.red, masks.green, masks.blue, masks.reserved];
                if channels[..3].contains(&0) {
                    return Err(UefiError::InvalidPixelFormat);
                }
                for index in 0..channels.len() {
                    if channels[index + 1..]
                        .iter()
                        .any(|other| channels[index] & *other != 0)
                    {
                        return Err(UefiError::InvalidPixelFormat);
                    }
                }
                let highest_bit =
                    32 - (masks.red | masks.green | masks.blue | masks.reserved).leading_zeros();
                highest_bit.div_ceil(8) as usize
            }
        };
        if pixel_bytes != 0 && pixels_per_scan_line < width {
            return Err(UefiError::InvalidFramebuffer);
        }
        let stride_bytes = (pixels_per_scan_line as usize)
            .checked_mul(pixel_bytes)
            .ok_or(UefiError::InvalidFramebuffer)?;
        let required_bytes = stride_bytes
            .checked_mul(height as usize)
            .ok_or(UefiError::InvalidFramebuffer)?;
        if mapped_bytes < required_bytes {
            return Err(UefiError::InvalidFramebuffer);
        }
        Ok(Self {
            width,
            height,
            stride_bytes,
            pixel_bytes,
            required_bytes,
            pixel_format: format,
        })
    }

    pub fn width(self) -> u32 {
        self.width
    }
    pub fn height(self) -> u32 {
        self.height
    }
    pub fn stride_bytes(self) -> usize {
        self.stride_bytes
    }
    pub fn pixel_bytes(self) -> usize {
        self.pixel_bytes
    }
    pub fn required_bytes(self) -> usize {
        self.required_bytes
    }
    pub fn pixel_format(self) -> PixelFormat {
        self.pixel_format
    }

    pub(crate) fn validate_source(self, width: u32, height: u32, length: usize) -> UefiResult<()> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(UefiError::SurfaceMismatch)?;
        if width != self.width || height != self.height || length != expected {
            return Err(UefiError::SurfaceMismatch);
        }
        Ok(())
    }

    pub(crate) fn encode_pixel(self, rgba: [u8; 4]) -> u32 {
        match self.pixel_format {
            PixelFormat::RgbReserved => u32::from_le_bytes([rgba[0], rgba[1], rgba[2], 0]),
            PixelFormat::BgrReserved => u32::from_le_bytes([rgba[2], rgba[1], rgba[0], 0]),
            PixelFormat::BitMask(masks) => {
                component(rgba[0], masks.red)
                    | component(rgba[1], masks.green)
                    | component(rgba[2], masks.blue)
            }
            PixelFormat::BltOnly => 0,
        }
    }
}

// Pack scaled intensity bits into potentially noncontiguous channel masks.
fn component(value: u8, mask: u32) -> u32 {
    let maximum = (1u64 << mask.count_ones()) - 1;
    let intensity = (u64::from(value) * maximum + 127) / 255;
    let mut output = 0u32;
    let mut source_bit = 0;
    for target_bit in 0..32 {
        if mask & (1 << target_bit) != 0 {
            if intensity & (1 << source_bit) != 0 {
                output |= 1 << target_bit;
            }
            source_bit += 1;
        }
    }
    output
}

/// Convert a tightly packed RGBA8 surface, preserving scan-line padding.
pub fn convert_rgba8(
    layout: FramebufferLayout,
    rgba: &[u8],
    framebuffer: &mut [u8],
) -> UefiResult<()> {
    layout.validate_source(layout.width, layout.height, rgba.len())?;
    if layout.pixel_format == PixelFormat::BltOnly {
        return Err(UefiError::InvalidPixelFormat);
    }
    if framebuffer.len() < layout.required_bytes {
        return Err(UefiError::InvalidFramebuffer);
    }
    for row in 0..layout.height as usize {
        for column in 0..layout.width as usize {
            let input = (row * layout.width as usize + column) * 4;
            let rgba = [
                rgba[input],
                rgba[input + 1],
                rgba[input + 2],
                rgba[input + 3],
            ];
            let target = row * layout.stride_bytes + column * layout.pixel_bytes;
            framebuffer[target..target + layout.pixel_bytes]
                .copy_from_slice(&layout.encode_pixel(rgba).to_le_bytes()[..layout.pixel_bytes]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgr_conversion_preserves_padding_and_ignores_alpha() {
        let layout = FramebufferLayout::new(1, 2, 2, 16, PixelFormat::BgrReserved).unwrap();
        let mut destination = [0xee; 16];
        convert_rgba8(layout, &[1, 2, 3, 255, 4, 5, 6, 128], &mut destination).unwrap();
        assert_eq!(
            destination,
            [
                3, 2, 1, 0, 0xee, 0xee, 0xee, 0xee, 6, 5, 4, 0, 0xee, 0xee, 0xee, 0xee
            ]
        );
    }

    #[test]
    fn rgb565_intensities_are_scaled() {
        let layout = FramebufferLayout::new(
            2,
            1,
            2,
            4,
            PixelFormat::BitMask(ChannelMasks {
                red: 0xf800,
                green: 0x07e0,
                blue: 0x001f,
                reserved: 0,
            }),
        )
        .unwrap();
        let mut destination = [0; 4];
        convert_rgba8(layout, &[255, 0, 0, 0, 0, 255, 255, 255], &mut destination).unwrap();
        assert_eq!(destination, [0, 0xf8, 0xff, 0x07]);
    }

    #[test]
    fn scattered_masks_and_overflowing_geometry_are_handled() {
        let layout = FramebufferLayout::new(
            1,
            1,
            1,
            1,
            PixelFormat::BitMask(ChannelMasks {
                red: 0b0001_0101,
                green: 0b0000_1010,
                blue: 0b1110_0000,
                reserved: 0,
            }),
        )
        .unwrap();
        let mut destination = [0];
        convert_rgba8(layout, &[128, 255, 0, 255], &mut destination).unwrap();
        assert_eq!(destination, [0b0001_1010]);
        assert_eq!(
            FramebufferLayout::new(
                u32::MAX,
                u32::MAX,
                u32::MAX,
                usize::MAX,
                PixelFormat::RgbReserved
            ),
            Err(UefiError::InvalidFramebuffer),
        );
    }

    #[test]
    fn invalid_masks_and_short_surfaces_fail_before_writing() {
        assert_eq!(
            FramebufferLayout::new(
                1,
                1,
                1,
                4,
                PixelFormat::BitMask(ChannelMasks {
                    red: 0xff,
                    green: 0xff,
                    blue: 0xff00,
                    reserved: 0,
                })
            ),
            Err(UefiError::InvalidPixelFormat)
        );
        assert_eq!(
            FramebufferLayout::new(2, 1, 1, 8, PixelFormat::RgbReserved),
            Err(UefiError::InvalidFramebuffer)
        );
        let layout = FramebufferLayout::new(1, 1, 1, 4, PixelFormat::RgbReserved).unwrap();
        let mut destination = [0xee; 4];
        assert_eq!(
            convert_rgba8(layout, &[1, 2, 3], &mut destination),
            Err(UefiError::SurfaceMismatch)
        );
        assert_eq!(destination, [0xee; 4]);
    }
}
