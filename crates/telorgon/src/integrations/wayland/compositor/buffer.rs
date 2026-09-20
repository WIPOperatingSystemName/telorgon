use std::fmt;

use crate::foundation::SizeI;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShmFormat {
    Argb8888,
    Xrgb8888,
    Abgr8888,
    Xbgr8888,
    Rgb565,
    Other(u32),
}

impl ShmFormat {
    pub const fn bytes_per_pixel(self) -> Option<u32> {
        match self {
            Self::Argb8888 | Self::Xrgb8888 | Self::Abgr8888 | Self::Xbgr8888 => Some(4),
            Self::Rgb565 => Some(2),
            Self::Other(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShmPool {
    pub size: usize,
}

impl ShmPool {
    pub fn new(size: i32, maximum_bytes: usize) -> Result<Self, BufferError> {
        let size = usize::try_from(size).map_err(|_| BufferError::InvalidPoolSize)?;
        if size == 0 || size > maximum_bytes {
            return Err(BufferError::InvalidPoolSize);
        }
        Ok(Self { size })
    }

    pub fn resize(&mut self, size: i32, maximum_bytes: usize) -> Result<(), BufferError> {
        let size = usize::try_from(size).map_err(|_| BufferError::InvalidPoolSize)?;
        if size <= self.size || size > maximum_bytes {
            return Err(BufferError::InvalidPoolResize);
        }
        self.size = size;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShmBuffer {
    pub offset: usize,
    pub size: SizeI,
    pub stride: u32,
    pub format: ShmFormat,
}

impl ShmBuffer {
    pub fn new(
        pool: ShmPool,
        offset: i32,
        width: i32,
        height: i32,
        stride: i32,
        format: ShmFormat,
    ) -> Result<Self, BufferError> {
        let offset = usize::try_from(offset).map_err(|_| BufferError::InvalidOffset)?;
        let stride = u32::try_from(stride).map_err(|_| BufferError::InvalidStride)?;
        if width <= 0 || height <= 0 {
            return Err(BufferError::InvalidDimensions);
        }
        let bytes_per_pixel = format
            .bytes_per_pixel()
            .ok_or(BufferError::UnsupportedFormat)?;
        let minimum_stride = u32::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(bytes_per_pixel))
            .ok_or(BufferError::ArithmeticOverflow)?;
        if stride < minimum_stride {
            return Err(BufferError::InvalidStride);
        }
        let end = usize::try_from(height)
            .ok()
            .and_then(|height| height.checked_mul(stride as usize))
            .and_then(|bytes| offset.checked_add(bytes))
            .ok_or(BufferError::ArithmeticOverflow)?;
        if end > pool.size {
            return Err(BufferError::OutOfPoolBounds);
        }
        Ok(Self {
            offset,
            size: SizeI { width, height },
            stride,
            format,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DmaBufFlags {
    pub y_invert: bool,
    pub interlaced: bool,
    pub bottom_field_first: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaBufPlane {
    pub index: u8,
    pub fd_token: u64,
    pub offset: u32,
    pub stride: u32,
    pub modifier: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DmaBufDescriptor {
    pub size: SizeI,
    pub format: u32,
    pub flags: DmaBufFlags,
    pub planes: Vec<DmaBufPlane>,
}

impl DmaBufDescriptor {
    pub fn new(
        size: SizeI,
        format: u32,
        flags: DmaBufFlags,
        mut planes: Vec<DmaBufPlane>,
    ) -> Result<Self, BufferError> {
        if size.width <= 0 || size.height <= 0 {
            return Err(BufferError::InvalidDimensions);
        }
        if planes.is_empty() || planes.len() > 4 {
            return Err(BufferError::InvalidPlaneCount);
        }
        planes.sort_unstable_by_key(|plane| plane.index);
        for (expected, plane) in planes.iter().enumerate() {
            if usize::from(plane.index) != expected {
                return Err(BufferError::NonContiguousPlanes);
            }
            if plane.fd_token == 0 || plane.stride == 0 {
                return Err(BufferError::InvalidPlane);
            }
            let _ = u64::from(plane.offset)
                .checked_add(u64::from(plane.stride) * size.height as u64)
                .ok_or(BufferError::ArithmeticOverflow)?;
        }
        Ok(Self {
            size,
            format,
            flags,
            planes,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BufferDescriptor {
    Shm(ShmBuffer),
    DmaBuf(DmaBufDescriptor),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferError {
    InvalidPoolSize,
    InvalidPoolResize,
    InvalidOffset,
    InvalidStride,
    InvalidDimensions,
    UnsupportedFormat,
    OutOfPoolBounds,
    InvalidPlaneCount,
    NonContiguousPlanes,
    InvalidPlane,
    ArithmeticOverflow,
}

impl fmt::Display for BufferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Wayland buffer validation failed: {self:?}")
    }
}

impl std::error::Error for BufferError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shm_buffers_cannot_escape_the_mapped_pool() {
        let pool = ShmPool::new(4096, 8192).unwrap();
        assert!(ShmBuffer::new(pool, 0, 16, 16, 64, ShmFormat::Argb8888).is_ok());
        assert_eq!(
            ShmBuffer::new(pool, 4000, 16, 16, 64, ShmFormat::Argb8888),
            Err(BufferError::OutOfPoolBounds)
        );
    }

    #[test]
    fn dma_buf_planes_must_be_contiguous() {
        let result = DmaBufDescriptor::new(
            SizeI {
                width: 100,
                height: 100,
            },
            875_713_112,
            DmaBufFlags::default(),
            vec![DmaBufPlane {
                index: 1,
                fd_token: 2,
                offset: 0,
                stride: 400,
                modifier: 0,
            }],
        );
        assert_eq!(result, Err(BufferError::NonContiguousPlanes));
    }
}
