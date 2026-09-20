use super::*;

#[derive(Debug)]
pub struct ShmBufferReader {
    pub(super) descriptor: ShmBuffer,
    pub(super) file: std::fs::File,
}

impl ShmBufferReader {
    pub fn read_full(self) -> Result<ShmImage, NativeCompositorError> {
        let height = usize::try_from(self.descriptor.size.height)
            .map_err(|_| NativeCompositorError::new("invalid SHM image height"))?;
        let length = (self.descriptor.stride as usize)
            .checked_mul(height)
            .ok_or_else(|| NativeCompositorError::new("SHM image length overflow"))?;
        let mut pixels = vec![0_u8; length];
        read_shm_exact(
            &self.file,
            self.descriptor.offset as u64,
            &mut pixels,
            "shared-memory buffer ended before its declared extent",
        )?;
        Ok(ShmImage {
            descriptor: self.descriptor,
            pixels,
        })
    }

    pub fn read_region(self, rect: RectI) -> Result<ShmImageRegion, NativeCompositorError> {
        let right = i64::from(rect.x) + i64::from(rect.width);
        let bottom = i64::from(rect.y) + i64::from(rect.height);
        if rect.x < 0
            || rect.y < 0
            || rect.width <= 0
            || rect.height <= 0
            || right > i64::from(self.descriptor.size.width)
            || bottom > i64::from(self.descriptor.size.height)
        {
            return Err(NativeCompositorError::new(
                "SHM read rectangle lies outside the buffer",
            ));
        }
        let bytes_per_pixel = self
            .descriptor
            .format
            .bytes_per_pixel()
            .ok_or_else(|| NativeCompositorError::new("unsupported SHM pixel format"))?
            as usize;
        let row_bytes = (rect.width as usize)
            .checked_mul(bytes_per_pixel)
            .ok_or_else(|| NativeCompositorError::new("SHM region row size overflow"))?;
        let length = row_bytes
            .checked_mul(rect.height as usize)
            .ok_or_else(|| NativeCompositorError::new("SHM region size overflow"))?;
        let x_bytes = (rect.x as usize)
            .checked_mul(bytes_per_pixel)
            .ok_or_else(|| NativeCompositorError::new("SHM x offset overflow"))?;
        let mut pixels = vec![0_u8; length];
        for row in 0..rect.height as usize {
            let file_offset = self
                .descriptor
                .offset
                .checked_add(
                    (rect.y as usize + row)
                        .checked_mul(self.descriptor.stride as usize)
                        .ok_or_else(|| NativeCompositorError::new("SHM row offset overflow"))?,
                )
                .and_then(|offset| offset.checked_add(x_bytes))
                .ok_or_else(|| NativeCompositorError::new("SHM region offset overflow"))?;
            read_shm_exact(
                &self.file,
                file_offset as u64,
                &mut pixels[row * row_bytes..(row + 1) * row_bytes],
                "shared-memory buffer ended before its damaged region",
            )?;
        }
        Ok(ShmImageRegion {
            descriptor: self.descriptor,
            rect,
            row_bytes,
            pixels,
        })
    }
}

pub(super) fn read_shm_exact(
    file: &std::fs::File,
    offset: u64,
    target: &mut [u8],
    eof_context: &'static str,
) -> Result<(), NativeCompositorError> {
    let mut read = 0;
    while read < target.len() {
        let read_offset = offset
            .checked_add(read as u64)
            .ok_or_else(|| NativeCompositorError::new("SHM read offset overflow"))?;
        let count = file
            .read_at(&mut target[read..], read_offset)
            .map_err(error)?;
        if count == 0 {
            return Err(NativeCompositorError::new(eof_context));
        }
        read += count;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShmImage {
    pub descriptor: ShmBuffer,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShmImageRegion {
    pub descriptor: ShmBuffer,
    pub rect: RectI,
    pub row_bytes: usize,
    pub pixels: Vec<u8>,
}
