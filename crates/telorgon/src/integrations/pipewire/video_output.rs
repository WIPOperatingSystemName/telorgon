//! Producer-owned native allocations, retired on remove_buffer or stream teardown.
use super::{MediaError, connection::native};
use crate::media::video::*;
use pipewire::{self as pw, spa::sys::*};
use std::{
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
    sync::Arc,
};

pub(super) struct OutputBuffer {
    storage: Storage,
    // Storage is destroyed before the reservation is released.
    _reservation: MemoryReservation,
    _capture_reservation: Option<MemoryReservation>,
}
enum Storage {
    Gpu(Box<dyn VideoGpuOutputBuffer>),
    Cpu(Vec<Mapping>),
}
impl OutputBuffer {
    /// Caller owns add_buffer's live native arrays. No pointer is retained by this owner.
    pub unsafe fn allocate(
        raw: *mut pw::sys::pw_buffer,
        format: VideoFormat,
        modifier: Option<u64>,
        producer: &mut dyn VideoGpuProducer,
        budget: Arc<MemoryBudget>,
        limit: usize,
        capture_budget: Option<Arc<MemoryBudget>>,
    ) -> Result<Self, MediaError> {
        let buffer = unsafe { raw.as_mut().and_then(|r| r.buffer.as_mut()) }
            .ok_or(MediaError::InvalidArgument("output buffer"))?;
        if buffer.datas.is_null() || buffer.n_datas == 0 || buffer.n_datas > 4 {
            return Err(MediaError::InvalidArgument("output data count"));
        }
        let kind = if modifier.is_some() {
            SPA_DATA_DmaBuf
        } else {
            SPA_DATA_MemFd
        };
        for i in 0..buffer.n_datas as usize {
            let data = unsafe { &*buffer.datas.add(i) };
            if data.type_ & (1 << kind) == 0 || data.chunk.is_null() {
                return Err(MediaError::Unsupported("producer allocation data type"));
            }
        }
        if buffer.n_metas > 64 || (buffer.n_metas != 0 && buffer.metas.is_null()) {
            return Err(MediaError::InvalidArgument("output metadata count"));
        }
        let mut metadata_bytes = 0usize;
        for i in 0..buffer.n_metas as usize {
            // SAFETY: caller owns the live native metadata array for add_buffer.
            let meta = unsafe { &*buffer.metas.add(i) };
            metadata_bytes = metadata_bytes
                .checked_add(meta.size as usize)
                .ok_or(MediaError::ResourceLimit("output metadata bytes"))?;
        }
        let available = limit.saturating_sub(budget.used()).min(budget.available());
        let available = available
            .checked_sub(metadata_bytes)
            .ok_or(MediaError::ResourceLimit("output metadata budget"))?;
        let mut reservation = MemoryReservation::new(budget, metadata_bytes)?;
        // Screen GPU storage already carries the renderer's shared charge. Native
        // metadata and CPU fallback storage need their own charge in that same budget.
        let mut capture_reservation = capture_budget
            .map(|budget| MemoryReservation::new(budget, metadata_bytes))
            .transpose()?;
        let storage = if let Some(modifier) = modifier {
            let gpu = producer.allocate(format, modifier, available)?;
            let planes: Vec<_> = gpu
                .planes()
                .iter()
                .map(|p| BorrowedVideoDmaBufPlane {
                    fd: p.fd.as_fd(),
                    offset: p.offset,
                    stride: p.stride,
                    allocation_size: p.allocation_size,
                })
                .collect();
            crate::media::video::gpu::validate_planes(format, modifier, &planes)?;
            if planes.len() != buffer.n_datas as usize
                || planes
                    .iter()
                    .any(|p| p.offset > u32::MAX as u64 || p.stride > i32::MAX as u32)
            {
                return Err(MediaError::InvalidArgument("GPU output plane layout"));
            }
            for plane in gpu.planes() {
                dma_buf_chunk_size(plane.offset, plane.allocation_size)?;
                u32::try_from(plane.allocation_size)
                    .map_err(|_| MediaError::ResourceLimit("GPU output allocation size"))?;
            }
            Storage::Gpu(gpu)
        } else {
            if buffer.n_datas != 1 && buffer.n_datas as usize != format.plane_count() {
                return Err(MediaError::InvalidArgument("CPU output plane count"));
            }
            // Each memfd/mapping consumes complete host pages, including the final
            // partial page. Compute every plane before reserving or allocating any.
            let page_size = Mapping::page_size()?;
            let mut sizes = Vec::with_capacity(buffer.n_datas as usize);
            let mut total = 0usize;
            for i in 0..buffer.n_datas as usize {
                let pixels = if buffer.n_datas == 1 {
                    format.byte_len()?
                } else {
                    let shape = format.plane(i).unwrap();
                    shape
                        .rows
                        .checked_mul(shape.row_bytes)
                        .ok_or(MediaError::ResourceLimit("output plane bytes"))?
                };
                let bytes = Mapping::allocation_size(pixels, page_size)?;
                total = total
                    .checked_add(bytes)
                    .ok_or(MediaError::ResourceLimit("output mapping bytes"))?;
                sizes.push(bytes);
            }
            if total > available {
                return Err(MediaError::ResourceLimit("output allocation budget"));
            }
            reservation.resize(
                metadata_bytes
                    .checked_add(total)
                    .ok_or(MediaError::ResourceLimit("output allocation budget"))?,
            )?;
            if let Some(charge) = &mut capture_reservation {
                charge.resize(
                    metadata_bytes
                        .checked_add(total)
                        .ok_or(MediaError::ResourceLimit("capture CPU allocation budget"))?,
                )?;
            }
            let mut planes = Vec::with_capacity(sizes.len());
            for bytes in sizes {
                planes.push(Mapping::new(bytes)?);
            }
            Storage::Cpu(planes)
        };
        let bytes = match &storage {
            Storage::Gpu(gpu) => gpu.planes().iter().try_fold(0usize, |total, plane| {
                let size = usize::try_from(plane.allocation_size)
                    .map_err(|_| MediaError::ResourceLimit("native GPU allocation size"))?;
                total
                    .checked_add(size)
                    .ok_or(MediaError::ResourceLimit("native GPU allocation size"))
            })?,
            Storage::Cpu(planes) => planes.iter().map(|p| p.bytes).sum(),
        };
        if bytes > available {
            return Err(MediaError::ResourceLimit("native video output pool"));
        }
        reservation.resize(
            metadata_bytes
                .checked_add(bytes)
                .ok_or(MediaError::ResourceLimit("output allocation budget"))?,
        )?;
        let owner = Self {
            storage,
            _reservation: reservation,
            _capture_reservation: capture_reservation,
        };
        // All fallible allocation/validation precedes publication of descriptors to PipeWire.
        for i in 0..buffer.n_datas as usize {
            let data = unsafe { &mut *buffer.datas.add(i) };
            let chunk = unsafe { &mut *data.chunk };
            data.type_ = kind;
            data.flags = SPA_DATA_FLAG_READWRITE;
            data.mapoffset = 0;
            chunk.size = 0;
            chunk.flags = SPA_CHUNK_FLAG_EMPTY as i32;
            match &owner.storage {
                Storage::Gpu(gpu) => {
                    let p = &gpu.planes()[i];
                    data.fd = p.fd.as_raw_fd() as i64;
                    data.data = std::ptr::null_mut();
                    data.maxsize = p.allocation_size as u32;
                    chunk.offset = p.offset as u32;
                    chunk.stride = p.stride as i32;
                    eprintln!("telorgon-pipewire: DMA-BUF output plane={} format={:?} modifier={:#x} offset={} stride={} allocation_bytes={} ready_chunk_bytes={}",
                        i, format, modifier.unwrap(), p.offset, p.stride, p.allocation_size,
                        dma_buf_chunk_size(p.offset, p.allocation_size)?);
                }
                Storage::Cpu(planes) => {
                    data.fd = planes[i].fd.as_raw_fd() as i64;
                    data.data = planes[i].pointer;
                    data.maxsize = planes[i].bytes as u32;
                    chunk.offset = 0;
                    chunk.stride = format.plane(i).unwrap().row_bytes as i32;
                }
            }
        }
        Ok(owner)
    }
    /// Caller holds the dequeued native buffer. Copy completion precedes publication.
    pub unsafe fn produce(
        &mut self,
        raw: *mut pw::sys::pw_buffer,
        format: VideoFormat,
        frame: Option<&GpuVideoFrame>,
        sequence: u64,
        full_damage: bool,
    ) -> Result<bool, MediaError> {
        let raw =
            unsafe { raw.as_mut() }.ok_or(MediaError::InvalidArgument("GPU output buffer"))?;
        let buffer = unsafe { raw.buffer.as_mut() }
            .ok_or(MediaError::InvalidArgument("GPU output SPA buffer"))?;
        let Storage::Gpu(gpu) = &mut self.storage else {
            return Err(MediaError::InvalidArgument("output transport changed"));
        };
        if buffer.datas.is_null() || buffer.n_datas as usize != gpu.planes().len() {
            return Err(MediaError::InvalidArgument("GPU output plane count"));
        }
        raw.size = 0;
        for i in 0..buffer.n_datas as usize {
            let data = unsafe { &mut *buffer.datas.add(i) };
            let chunk = unsafe { data.chunk.as_mut() }
                .ok_or(MediaError::InvalidArgument("GPU output chunk"))?;
            chunk.flags = SPA_CHUNK_FLAG_EMPTY as i32;
            chunk.size = 0;
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
            return Err(MediaError::InvalidArgument("GPU output generation"));
        }
        unsafe {
            super::video_metadata::write_with_damage(buffer, format, None, sequence, full_damage)
        }?;
        unsafe { gpu.copy_from(frame) }?;
        unsafe {
            super::video_metadata::write_with_damage(
                buffer,
                format,
                Some(frame.metadata()),
                sequence,
                full_damage,
            )
        }?;
        for i in 0..buffer.n_datas as usize {
            let p = &gpu.planes()[i];
            let data = unsafe { &mut *buffer.datas.add(i) };
            let chunk = unsafe { &mut *data.chunk };
            chunk.offset = p.offset as u32;
            chunk.stride = p.stride as i32;
            // PipeWire permits zero for DMA-BUF, but older WebRTC consumers discard
            // zero-sized chunks before attempting import. Mark completed storage nonempty.
            chunk.size = dma_buf_chunk_size(p.offset, p.allocation_size)?;
            chunk.flags = 0;
        }
        raw.size = 1;
        Ok(true)
    }
}
fn dma_buf_chunk_size(offset: u64, allocation_size: u64) -> Result<u32, MediaError> {
    allocation_size.checked_sub(offset)
        .filter(|size| *size != 0)
        .and_then(|size| u32::try_from(size).ok())
        .ok_or(MediaError::InvalidArgument("GPU output chunk extent"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dma_buf_payload_uses_storage_extent_including_non_linear_layouts() {
        // The extent comes from the allocation, not width * height or stride * height:
        // modifiers can describe compressed/tiled storage with nonzero plane offsets.
        let size = dma_buf_chunk_size(4096, 12288).unwrap();
        assert_ne!(size, 0, "legacy WebRTC must not discard a completed frame as empty");
        assert_eq!(4096 + u64::from(size), 12288);
        assert!(dma_buf_chunk_size(12288, 12288).is_err());
        assert!(dma_buf_chunk_size(12289, 12288).is_err());
        assert!(dma_buf_chunk_size(0, u64::from(u32::MAX) + 1).is_err());
    }
}
struct Mapping {
    fd: OwnedFd,
    pointer: *mut std::ffi::c_void,
    bytes: usize,
}
impl Mapping {
    fn page_size() -> Result<usize, MediaError> {
        // SAFETY: sysconf reads a process-independent host property and retains no pointers.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        usize::try_from(page)
            .ok()
            .filter(|page| *page > 0)
            .ok_or(MediaError::Unsupported("host mapping page size"))
    }
    fn allocation_size(bytes: usize, page: usize) -> Result<usize, MediaError> {
        bytes
            .checked_add(page - 1)
            .and_then(|rounded| (rounded / page).checked_mul(page))
            .filter(|rounded| {
                *rounded > 0
                    && *rounded <= u32::MAX as usize
                    && *rounded as u128 <= libc::off_t::MAX as u128
            })
            .ok_or(MediaError::ResourceLimit("output mapping size"))
    }
    fn new(bytes: usize) -> Result<Self, MediaError> {
        let fd = unsafe {
            libc::memfd_create(
                c"telorgon-video-output".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        if fd < 0 {
            return Err(native(std::io::Error::last_os_error()));
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe { libc::ftruncate(fd.as_raw_fd(), bytes as libc::off_t) } != 0 {
            return Err(native(std::io::Error::last_os_error()));
        }
        if unsafe {
            libc::fcntl(
                fd.as_raw_fd(),
                libc::F_ADD_SEALS,
                libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL,
            )
        } < 0
        {
            return Err(native(std::io::Error::last_os_error()));
        }
        let pointer = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                bytes,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if pointer == libc::MAP_FAILED {
            return Err(native(std::io::Error::last_os_error()));
        }
        Ok(Self { fd, pointer, bytes })
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.pointer, self.bytes);
        }
    }
}
