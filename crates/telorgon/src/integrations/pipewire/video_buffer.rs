//! Checked mapped CPU-plane boundary. No native pointer or borrowed mapping escapes a call.
use super::MediaError;
use crate::media::video::*;
use pipewire::{self as pw, spa::sys::*};
use std::ptr::NonNull;
pub(super) struct Buffer<'a> {
    raw: NonNull<pw::sys::pw_buffer>,
    stream: &'a pw::stream::Stream,
}
impl<'a> Buffer<'a> {
    pub fn dequeue(stream: &'a pw::stream::Stream) -> Option<Self> {
        // SAFETY: only called from this stream's serial non-RT process callback.
        NonNull::new(unsafe { stream.dequeue_raw_buffer() }).map(|raw| Self { raw, stream })
    }
    pub fn identity(&self) -> usize {
        self.raw.as_ptr() as usize
    }
    pub fn produce_gpu(
        &mut self,
        output: &mut super::video_output::OutputBuffer,
        format: VideoFormat,
        frame: Option<&GpuVideoFrame>,
        sequence: u64,
        full_damage: bool,
    ) -> Result<bool, MediaError> {
        // SAFETY: this RAII lease owns the native buffer exclusively until Drop requeues it.
        unsafe { output.produce(self.raw.as_ptr(), format, frame, sequence, full_damage) }
    }
    pub fn capture(&self, frame: &mut FrameData) -> Result<bool, MediaError> {
        // SAFETY: dequeued buffer and its SPA arrays/mappings remain live until Drop.
        let buffer = unsafe { self.raw.as_ref().buffer.as_ref() }
            .ok_or(MediaError::InvalidArgument("missing SPA video buffer"))?;
        if buffer.n_datas > 0 && !buffer.datas.is_null() {
            let data = unsafe { &*buffer.datas };
            if let Some(chunk) = unsafe { data.chunk.as_ref() } {
                if chunk.size == 0 || chunk.flags & SPA_CHUNK_FLAG_EMPTY as i32 != 0 {
                    return Ok(false);
                }
            }
        }
        let planes = unsafe { planes(buffer, frame.format, false) }?;
        for (index, plane) in planes.iter().enumerate().take(frame.format.plane_count()) {
            let plane = plane.as_ref().unwrap();
            for row in 0..plane.shape.rows {
                let dest = frame.planes[index]
                    .row_mut(row, plane.shape)
                    .ok_or(MediaError::InvalidArgument("video copy destination"))?;
                // SAFETY: planes validated every row range. The owned destination is separate
                // from native mappings. No Rust reference to native mutable storage is retained.
                unsafe {
                    std::ptr::copy_nonoverlapping(plane.row(row), dest.as_mut_ptr(), dest.len());
                }
            }
        }
        frame.metadata = unsafe { super::video_metadata::read(buffer, frame.format) }?;
        Ok(true)
    }
    pub fn capture_gpu(
        &self,
        format: VideoFormat,
        modifier: u64,
        count: u32,
        transfer: &mut dyn VideoGpuTransfer,
        limit: usize,
        previous: Option<u64>,
    ) -> Result<Option<GpuVideoFrame>, MediaError> {
        use std::os::fd::BorrowedFd;
        // SAFETY: exclusive dequeued buffer; all native arrays/FDs stay live for this call.
        let buffer = unsafe { self.raw.as_ref().buffer.as_ref() }
            .ok_or(MediaError::InvalidArgument("missing GPU SPA buffer"))?;
        if buffer.datas.is_null() || buffer.n_datas != count || count == 0 || count > 4 {
            return Err(MediaError::InvalidArgument("GPU video plane count"));
        }
        let mut planes = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            let data = unsafe { &*buffer.datas.add(index) };
            let chunk =
                unsafe { data.chunk.as_ref() }.ok_or(MediaError::InvalidArgument("GPU chunk"))?;
            if chunk.flags & SPA_CHUNK_FLAG_EMPTY as i32 != 0 {
                return Ok(None);
            }
            if data.type_ != SPA_DATA_DmaBuf
                || data.fd < 0
                || data.fd > i32::MAX as i64
                || chunk.stride <= 0
            {
                return Err(MediaError::InvalidArgument("GPU plane FD or stride"));
            }
            let fd = unsafe { BorrowedFd::borrow_raw(data.fd as i32) };
            planes.push(BorrowedVideoDmaBufPlane {
                fd,
                offset: (data.mapoffset as u64)
                    .checked_add(chunk.offset as u64)
                    .ok_or(MediaError::InvalidArgument("GPU offset"))?,
                stride: chunk.stride as u32,
                allocation_size: super::video_dma_buf::allocation_size(fd)?,
            });
        }
        crate::media::video::gpu::validate_planes(format, modifier, &planes)?;
        let mut metadata = unsafe { super::video_metadata::read(buffer, format) }?;
        metadata.discontinuity |= metadata.sequence.is_none();
        if let Some(sequence) = metadata.sequence {
            metadata.discontinuity |= previous.is_none_or(|old| old.wrapping_add(1) != sequence);
        }
        let frame = transfer.copy_to_owned(
            BorrowedGpuVideoFrame {
                format,
                modifier,
                planes: &planes,
                metadata: &metadata,
            },
            limit,
        )?;
        if frame.allocation_bytes() > limit {
            return Err(MediaError::ResourceLimit(
                "GPU transfer exceeded allocation budget",
            ));
        }
        if frame.format() != format {
            return Err(MediaError::InvalidArgument(
                "GPU transfer changed frame format",
            ));
        }
        Ok(Some(frame))
    }
    pub fn produce(
        &mut self,
        format: VideoFormat,
        frame: Option<&CpuVideoFrame>,
        sequence: u64,
        full_damage: bool,
    ) -> Result<bool, MediaError> {
        // SAFETY: exclusively dequeued buffer, valid until Drop.
        let raw = unsafe { self.raw.as_mut() };
        let buffer = unsafe { raw.buffer.as_mut() }
            .ok_or(MediaError::InvalidArgument("missing SPA video buffer"))?;
        raw.size = 0;
        if buffer.n_datas == 0 || buffer.n_datas > 3 || buffer.datas.is_null() {
            return Err(MediaError::InvalidArgument("video planes"));
        }
        // Initialize all chunks as empty even on a later validation failure.
        for index in 0..buffer.n_datas as usize {
            let data = unsafe { &mut *buffer.datas.add(index) };
            if let Some(chunk) = unsafe { data.chunk.as_mut() } {
                chunk.size = 0;
                chunk.offset = 0;
                chunk.flags = SPA_CHUNK_FLAG_EMPTY as i32;
            }
        }
        let Some(frame) = frame else {
            unsafe {
                super::video_metadata::write_with_damage(
                    buffer,
                    format,
                    None,
                    sequence,
                    full_damage,
                )
            }?;
            return Ok(false);
        };
        if frame.format() != format {
            return Err(MediaError::InvalidArgument("video frame generation"));
        }
        let layouts = unsafe { planes(buffer, format, true) }?;
        for (index, plane) in layouts.iter().enumerate().take(format.plane_count()) {
            let plane = plane.as_ref().unwrap();
            let source = frame.plane(index).unwrap();
            for row in 0..plane.shape.rows {
                let bytes = source
                    .row(row)
                    .ok_or(MediaError::InvalidArgument("video source row"))?;
                // SAFETY: validated native row ranges and separate owned source storage.
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), plane.row(row), bytes.len());
                }
            }
        }
        unsafe {
            super::video_metadata::write_with_damage(
                buffer,
                format,
                Some(frame.metadata()),
                sequence,
                full_damage,
            )
        }?;
        if buffer.n_datas == 1 {
            let data = unsafe { &mut *buffer.datas };
            let chunk = unsafe { &mut *data.chunk };
            chunk.offset = 0;
            chunk.stride = format.plane(0).unwrap().row_bytes as i32;
            chunk.size = format.byte_len()? as u32;
            chunk.flags = 0;
        } else {
            for index in 0..buffer.n_datas as usize {
                let shape = format.plane(index).unwrap();
                let data = unsafe { &mut *buffer.datas.add(index) };
                let chunk = unsafe { &mut *data.chunk };
                chunk.offset = 0;
                chunk.stride = shape.row_bytes as i32;
                chunk.size = (shape.rows * shape.row_bytes) as u32;
                chunk.flags = 0;
            }
        }
        raw.size = 1;
        Ok(true)
    }
}
impl Drop for Buffer<'_> {
    fn drop(&mut self) {
        // SAFETY: exact stream that supplied this buffer; sole lease, returned exactly once.
        unsafe {
            self.stream.queue_raw_buffer(self.raw.as_ptr());
        }
    }
}
struct NativePlane {
    data: *mut u8,
    offset: usize,
    stride: i32,
    shape: PlaneShape,
}
impl NativePlane {
    unsafe fn row(&self, row: usize) -> *mut u8 {
        let offset = (self.offset as i128 + row as i128 * self.stride as i128) as usize;
        // SAFETY: planes validated extrema for every row; row is less than shape.rows.
        unsafe { self.data.add(offset) }
    }
}
#[allow(non_upper_case_globals)]
unsafe fn planes(
    buffer: &spa_buffer,
    format: VideoFormat,
    output: bool,
) -> Result<[Option<NativePlane>; 3], MediaError> {
    let bad = || MediaError::InvalidArgument("mapped video plane layout");
    let count = buffer.n_datas as usize;
    if (count != 1 && count != format.plane_count()) || buffer.datas.is_null() {
        return Err(bad());
    }
    let mut result = [None, None, None];
    let mut next_offset = 0usize;
    let mut base_stride = 0i32;
    for index in 0..format.plane_count() {
        // SAFETY: native SPA array is valid for n_datas entries; index was bounded above.
        let data = unsafe { &*buffer.datas.add(if count == 1 { 0 } else { index }) };
        if data.data.is_null() || !matches!(data.type_, SPA_DATA_MemFd | SPA_DATA_MemPtr) {
            return Err(bad());
        }
        let chunk = unsafe { data.chunk.as_ref() }.ok_or_else(bad)?;
        let shape = format.plane(index).unwrap();
        if !output
            && (chunk.flags & (SPA_CHUNK_FLAG_CORRUPTED | SPA_CHUNK_FLAG_EMPTY) as i32 != 0
                || chunk.size == 0)
        {
            return Err(bad());
        }
        let stride = if output {
            shape.row_bytes as i32
        } else if count > 1 || index == 0 {
            if chunk.stride == 0 {
                shape.row_bytes as i32
            } else {
                chunk.stride
            }
        } else {
            if base_stride < 0 {
                return Err(bad());
            }
            match format.pixel {
                PixelFormat::I420 => (base_stride + 1) / 2,
                _ => base_stride,
            }
            .max(shape.row_bytes as i32)
        };
        if index == 0 {
            base_stride = stride;
            next_offset = if output { 0 } else { chunk.offset as usize };
        }
        if (stride.unsigned_abs() as usize) < shape.row_bytes {
            return Err(bad());
        }
        let offset = if count == 1 {
            next_offset
        } else if output {
            0
        } else {
            chunk.offset as usize
        };
        let last = offset as i128 + (shape.rows - 1) as i128 * stride as i128;
        let low = last.min(offset as i128);
        let high = last.max(offset as i128) + shape.row_bytes as i128;
        if low < 0
            || high > data.maxsize as i128
            || (!output
                && (high - low > chunk.size as i128
                    || count == 1
                        && format.plane_count() > 1
                        && high > chunk.offset as i128 + chunk.size as i128))
        {
            return Err(bad());
        }
        result[index] = Some(NativePlane {
            data: data.data.cast(),
            offset,
            stride,
            shape,
        });
        next_offset = offset
            .checked_add(
                (stride.unsigned_abs() as usize)
                    .checked_mul(shape.rows)
                    .ok_or_else(bad)?,
            )
            .ok_or_else(bad)?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separate_yuv_planes_preserve_padding_and_reject_short_chunks() {
        let format = VideoFormat {
            pixel: PixelFormat::Nv12,
            color: Colorimetry::BT709,
            ..VideoFormat::rgba(4, 2, 30)
        };
        let mut y = [16u8; 16];
        let mut uv = [128u8; 8];
        let mut yc = spa_chunk {
            offset: 2,
            size: 12,
            stride: 8,
            flags: 0,
        };
        let mut uvc = spa_chunk {
            offset: 1,
            size: 4,
            stride: 6,
            flags: 0,
        };
        let mut data = [
            spa_data {
                type_: SPA_DATA_MemPtr,
                flags: 0,
                fd: -1,
                mapoffset: 0,
                maxsize: y.len() as u32,
                data: y.as_mut_ptr().cast(),
                chunk: &mut yc,
            },
            spa_data {
                type_: SPA_DATA_MemPtr,
                flags: 0,
                fd: -1,
                mapoffset: 0,
                maxsize: uv.len() as u32,
                data: uv.as_mut_ptr().cast(),
                chunk: &mut uvc,
            },
        ];
        let buffer = spa_buffer {
            n_metas: 0,
            n_datas: 2,
            metas: std::ptr::null_mut(),
            datas: data.as_mut_ptr(),
        };
        // SAFETY: all native-shaped arrays, chunks and mappings refer to live local storage.
        let layouts = unsafe { planes(&buffer, format, false) }.unwrap();
        let y_plane = layouts[0].as_ref().unwrap();
        let uv_plane = layouts[1].as_ref().unwrap();
        assert_eq!(
            unsafe { std::slice::from_raw_parts(y_plane.row(1), 4) },
            &[16; 4]
        );
        assert_eq!(
            unsafe { std::slice::from_raw_parts(uv_plane.row(0), 4) },
            &[128; 4]
        );
        unsafe {
            (*data[0].chunk).size = 3;
        }
        assert!(unsafe { planes(&buffer, format, false) }.is_err());
        unsafe {
            (*data[0].chunk).size = 12;
            (*data[0].chunk).offset = 10;
            (*data[0].chunk).stride = -8;
        }
        assert!(unsafe { planes(&buffer, format, false) }.is_ok());
        unsafe {
            (*data[0].chunk).offset = 0;
        }
        assert!(unsafe { planes(&buffer, format, false) }.is_err());
    }
}
