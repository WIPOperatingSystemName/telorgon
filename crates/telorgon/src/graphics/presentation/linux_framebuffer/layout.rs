use super::abi::{Bitfield, FixedInfo, VariableInfo};
use crate::foundation::SizeI;
use crate::graphics::presentation::{PresentationError, PresentationErrorKind, PresentationResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Layout {
    pub extent: SizeI,
    pub mapping_bytes: usize,
    pub physical_start: libc::c_ulong,
    pub stride: usize,
    pub visible_offset: usize,
    pub pixel_bytes: usize,
    pub channels: [Bitfield; 4],
}

impl Layout {
    pub fn read(fixed: &FixedInfo, variable: &VariableInfo) -> PresentationResult<Self> {
        if fixed.kind != 0
            || fixed.visual != 2
            || variable.grayscale != 0
            || variable.nonstd != 0
            || variable.rotate != 0
            || variable.vmode & 256 != 0
            || !matches!(variable.bits_per_pixel, 16 | 24 | 32)
        {
            return Err(unsupported(
                "fbdev requires standard packed truecolor pixels without rotation or y-wrap",
            ));
        }
        if variable.xres == 0
            || variable.yres == 0
            || variable.xres > i32::MAX as u32
            || variable.yres > i32::MAX as u32
            || variable
                .xoffset
                .checked_add(variable.xres)
                .is_none_or(|n| n > variable.xres_virtual)
            || variable
                .yoffset
                .checked_add(variable.yres)
                .is_none_or(|n| n > variable.yres_virtual)
        {
            return Err(invalid(
                "fbdev visible dimensions or pan offsets exceed virtual dimensions",
            ));
        }
        let pixel_bytes = variable.bits_per_pixel as usize / 8;
        let stride = fixed.line_length as usize;
        let virtual_row_bytes = (variable.xres_virtual as usize)
            .checked_mul(pixel_bytes)
            .ok_or_else(|| invalid("fbdev virtual row byte length overflows"))?;
        if stride < virtual_row_bytes || fixed.smem_len == 0 {
            return Err(invalid(
                "fbdev stride or mapping length is smaller than its declared image",
            ));
        }
        let visible_offset = (variable.yoffset as usize)
            .checked_mul(stride)
            .and_then(|offset| {
                (variable.xoffset as usize)
                    .checked_mul(pixel_bytes)
                    .and_then(|x| offset.checked_add(x))
            })
            .ok_or_else(|| invalid("fbdev visible offset overflows"))?;
        let end = (variable.yres as usize - 1)
            .checked_mul(stride)
            .and_then(|last_row| visible_offset.checked_add(last_row))
            .and_then(|offset| {
                (variable.xres as usize)
                    .checked_mul(pixel_bytes)
                    .and_then(|width| offset.checked_add(width))
            })
            .ok_or_else(|| invalid("fbdev visible byte range overflows"))?;
        if end > fixed.smem_len as usize || fixed.smem_len as usize > isize::MAX as usize {
            return Err(invalid("fbdev visible image exceeds the mapped memory"));
        }
        let channels = [
            variable.red,
            variable.green,
            variable.blue,
            variable.transparency,
        ];
        let mut used = 0u64;
        for (index, field) in channels.iter().enumerate() {
            if field.length == 0 && index == 3 {
                continue;
            }
            if field.length == 0
                || field.msb_right != 0
                || field
                    .offset
                    .checked_add(field.length)
                    .is_none_or(|end| end > variable.bits_per_pixel)
            {
                return Err(unsupported(
                    "fbdev channel bitfield is missing, reversed, or outside its pixel",
                ));
            }
            let mask = ((1u64 << field.length) - 1) << field.offset;
            if used & mask != 0 {
                return Err(invalid("fbdev channel bitfields overlap"));
            }
            used |= mask;
        }
        Ok(Self {
            extent: SizeI {
                width: variable.xres as i32,
                height: variable.yres as i32,
            },
            mapping_bytes: fixed.smem_len as usize,
            physical_start: fixed.smem_start,
            stride,
            visible_offset,
            pixel_bytes,
            channels,
        })
    }

    pub fn row_bytes(&self) -> usize {
        self.extent.width as usize * self.pixel_bytes
    }

    pub fn encode(&self, rgba: &[u8], output: &mut [u8]) {
        for (source, destination) in rgba
            .chunks_exact(4)
            .zip(output.chunks_exact_mut(self.pixel_bytes))
        {
            let mut pixel = 0u32;
            for (index, field) in self.channels.iter().enumerate() {
                if field.length == 0 {
                    continue;
                }
                // A framebuffer is opaque. The optional transparency channel is set fully opaque;
                // rendered source alpha is not treated as a hardware-reserved channel.
                let channel = if index == 3 { 255 } else { source[index] };
                let maximum = (1u64 << field.length) - 1;
                let value = (u64::from(channel) * maximum + 127) / 255;
                pixel |= (value << field.offset) as u32;
            }
            let bytes = pixel.to_ne_bytes();
            #[cfg(target_endian = "little")]
            destination.copy_from_slice(&bytes[..self.pixel_bytes]);
            #[cfg(target_endian = "big")]
            destination.copy_from_slice(&bytes[4 - self.pixel_bytes..]);
        }
    }
}

fn unsupported(message: &str) -> PresentationError {
    PresentationError::new(PresentationErrorKind::Unsupported, message)
}
fn invalid(message: &str) -> PresentationError {
    PresentationError::new(PresentationErrorKind::InvalidState, message)
}
