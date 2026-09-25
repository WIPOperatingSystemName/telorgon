//! Owned DMA-BUF images and the deliberate graphics-backend extension boundary.
use super::*;
use std::{
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoDmaBufFormat {
    pub pixel: PixelFormat,
    pub modifier: u64,
    pub planes: u32,
}
/// Allocation metadata, not a CPU mapping. Modifiers can describe tiled/compressed storage.
#[derive(Debug)]
pub struct VideoDmaBufPlane {
    pub fd: OwnedFd,
    pub offset: u64,
    pub stride: u32,
    pub allocation_size: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct BorrowedVideoDmaBufPlane<'a> {
    pub fd: BorrowedFd<'a>,
    pub offset: u64,
    pub stride: u32,
    pub allocation_size: u64,
}
/// Read-only input valid for one graphics transfer call. Duplicating an FD extends allocation
/// lifetime, but does not extend the right to read content after the transfer returns.
pub struct BorrowedGpuVideoFrame<'a> {
    pub format: VideoFormat,
    pub modifier: u64,
    pub planes: &'a [BorrowedVideoDmaBufPlane<'a>],
    pub metadata: &'a FrameMetadata,
}
pub(crate) struct GpuReservation {
    budget: Arc<MemoryBudget>,
    bytes: usize,
}
impl GpuReservation {
    pub(crate) fn resize(&mut self, bytes: usize) -> Result<(), MediaError> {
        if bytes > self.bytes {
            self.budget.reserve(bytes - self.bytes)?;
            self.bytes = bytes;
        } else {
            self.shrink(bytes);
        }
        Ok(())
    }
    pub(crate) fn shrink(&mut self, bytes: usize) {
        assert!(bytes <= self.bytes, "GPU reservation cannot grow");
        self.budget.release(self.bytes - bytes);
        self.bytes = bytes;
    }
    pub(crate) fn new(
        budget: Arc<MemoryBudget>,
        bytes: usize,
        max_frames: usize,
    ) -> Result<Arc<Self>, MediaError> {
        budget.reserve_gpu_frame(max_frames)?;
        if let Err(error) = budget.reserve(bytes) {
            budget.release_gpu_frame();
            return Err(error);
        }
        Ok(Arc::new(Self { budget, bytes }))
    }
}
impl Drop for GpuReservation {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
        self.budget.release_gpu_frame();
    }
}
pub(crate) struct GpuFrameData {
    pub format: VideoFormat,
    pub modifier: u64,
    pub planes: Vec<VideoDmaBufPlane>,
    pub metadata: FrameMetadata,
    pub generation: u64,
    accounting: Option<Arc<GpuReservation>>,
    importing: AtomicBool,
}
/// Immutable independently owned GPU image. Clones retain the same allocation and accounting
/// reservation. Native capture buffers have already been released; resize/disconnect do not
/// revoke pixels. Safe renderer bridges retain this owner until GPU use completes.
#[derive(Clone)]
pub struct GpuVideoFrame {
    pub(crate) data: Arc<GpuFrameData>,
}
impl std::fmt::Debug for GpuVideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuVideoFrame")
            .field("format", &self.data.format)
            .field("modifier", &self.data.modifier)
            .field("generation", &self.data.generation)
            .finish_non_exhaustive()
    }
}
impl GpuVideoFrame {
    /// Import an externally allocated, finished image into the media ownership model.
    ///
    /// # Safety
    /// Every FD must be a real DMA-BUF of the declared format, modifier, size and layout.
    /// All writes must have completed and no external owner may write or reuse the allocation
    /// while this image or any exported renderer lease exists. FDs alone do not ensure this.
    /// This is an advanced interoperability boundary; ordinary callers use graphics exporters.
    pub unsafe fn from_completed_dma_buf(
        mut format: VideoFormat,
        modifier: u64,
        planes: Vec<VideoDmaBufPlane>,
        metadata: FrameMetadata,
    ) -> Result<Self, MediaError> {
        format.validate()?;
        metadata.validate(format)?;
        let borrowed: Vec<_> = planes
            .iter()
            .map(|p| BorrowedVideoDmaBufPlane {
                fd: p.fd.as_fd(),
                offset: p.offset,
                stride: p.stride,
                allocation_size: p.allocation_size,
            })
            .collect();
        validate_planes(format, modifier, &borrowed)?;
        let bytes = planes
            .iter()
            .map(|p| p.allocation_size as usize)
            .sum::<usize>();
        if bytes.saturating_add(metadata.allocation_bytes()) > 512 * 1024 * 1024 {
            return Err(MediaError::ResourceLimit("GPU frame including metadata"));
        }
        format.rate = format.rate.reduced();
        Ok(Self {
            data: Arc::new(GpuFrameData {
                format,
                modifier,
                planes,
                metadata,
                generation: 0,
                accounting: None,
                importing: AtomicBool::new(false),
            }),
        })
    }
    pub fn format(&self) -> VideoFormat {
        self.data.format
    }
    pub fn modifier(&self) -> u64 {
        self.data.modifier
    }
    pub fn metadata(&self) -> &FrameMetadata {
        &self.data.metadata
    }
    pub fn generation(&self) -> u64 {
        self.data.generation
    }
    pub fn allocation_bytes(&self) -> usize {
        self.data
            .planes
            .iter()
            .map(|p| p.allocation_size as usize)
            .sum::<usize>()
            + self.data.metadata.allocation_bytes()
    }
    /// Borrow descriptors for a read-only external import. Keeping a borrowed descriptor does
    /// not transfer this owner's lifetime or synchronization obligations.
    ///
    /// # Safety
    /// Imported GPU resources must retain a clone of this frame until all reads complete.
    /// Callers must never write to these allocations or use them as mutable render targets.
    /// Synchronize image layout/ownership transitions with every other import, including
    /// safe renderer bridges; those bridges do not coordinate with this unsafe escape hatch.
    pub unsafe fn planes(&self) -> Vec<BorrowedVideoDmaBufPlane<'_>> {
        self.data
            .planes
            .iter()
            .map(|p| BorrowedVideoDmaBufPlane {
                fd: p.fd.as_fd(),
                offset: p.offset,
                stride: p.stride,
                allocation_size: p.allocation_size,
            })
            .collect()
    }
    /// A renderer performs image layout/ownership transitions even for read-only sampling.
    /// Serialize independent imports of one allocation, including imports on different devices.
    pub(crate) fn begin_import(&self) -> Result<GpuFrameImport, MediaError> {
        self.data
            .importing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| MediaError::ResourceLimit("GPU frame already has a renderer lease"))?;
        Ok(GpuFrameImport(self.data.clone()))
    }
    pub(crate) fn account(
        &mut self,
        budget: Arc<MemoryBudget>,
        generation: u64,
        max_frames: usize,
    ) -> Result<(), MediaError> {
        let reservation = GpuReservation::new(budget, self.allocation_bytes(), max_frames)?;
        self.account_reserved(reservation, generation)
    }
    pub(crate) fn account_reserved(
        &mut self,
        reservation: Arc<GpuReservation>,
        generation: u64,
    ) -> Result<(), MediaError> {
        if self.allocation_bytes() > reservation.bytes {
            return Err(MediaError::ResourceLimit("GPU frame exceeds reservation"));
        }
        let data = Arc::get_mut(&mut self.data).ok_or(MediaError::InvalidArgument(
            "GPU transfer returned a shared image",
        ))?;
        if data.accounting.is_some() {
            return Err(MediaError::InvalidArgument("GPU image already accounted"));
        }
        data.accounting = Some(reservation);
        data.generation = generation;
        Ok(())
    }
}
pub(crate) struct GpuFrameImport(Arc<GpuFrameData>);
impl Drop for GpuFrameImport {
    fn drop(&mut self) {
        self.0.importing.store(false, Ordering::Release);
    }
}
/// Concrete graphics implementations copy native storage into independently owned GPU images.
/// Methods execute serially on the PipeWire control worker and must have bounded resource use.
/// No host UI or audio realtime APIs may be called here.
///
/// # Safety
/// `copy_to_owned` must finish *all reads* of input allocations before returning, including
/// error paths, and honor their implicit DMA-BUF producer fences. It must not retain input FDs
/// or mappings for later work. Return a unique, immutable, completed allocation within
/// `byte_limit`; apply the input's format and metadata without changing their meaning.
/// Do not implement this trait merely by duplicating the producer's FDs.
pub unsafe trait VideoGpuTransfer: Send + 'static {
    fn formats(&self) -> Vec<VideoDmaBufFormat>;
    fn copy_to_owned(
        &mut self,
        input: BorrowedGpuVideoFrame<'_>,
        byte_limit: usize,
    ) -> Result<GpuVideoFrame, MediaError>;
}
#[derive(Clone, Debug)]
pub enum VideoFrame {
    Cpu(CpuVideoFrame),
    Gpu(GpuVideoFrame),
}

impl VideoFrame {
    pub fn metadata(&self) -> &FrameMetadata {
        match self {
            Self::Cpu(frame) => frame.metadata(),
            Self::Gpu(frame) => frame.metadata(),
        }
    }
    pub fn generation(&self) -> u64 {
        match self {
            Self::Cpu(frame) => frame.generation(),
            Self::Gpu(frame) => frame.generation(),
        }
    }
    pub fn format(&self) -> VideoFormat {
        match self {
            Self::Cpu(frame) => frame.format(),
            Self::Gpu(frame) => frame.format(),
        }
    }
}

pub(crate) fn validate_planes(
    format: VideoFormat,
    modifier: u64,
    planes: &[BorrowedVideoDmaBufPlane<'_>],
) -> Result<(), MediaError> {
    format.validate()?;
    if planes.is_empty() || planes.len() > 4 || modifier == 0x00ff_ffff_ffff_ffff {
        return Err(MediaError::Unsupported("explicit DMA-BUF layout required"));
    }
    let mut total = 0u64;
    for (index, plane) in planes.iter().enumerate() {
        total = total
            .checked_add(plane.allocation_size)
            .ok_or(MediaError::ResourceLimit("GPU allocation size"))?;
        if plane.allocation_size == 0
            || total > 512 * 1024 * 1024
            || plane.offset >= plane.allocation_size
            || plane.stride == 0
        {
            return Err(MediaError::InvalidArgument("DMA-BUF plane allocation"));
        }
        if modifier == 0 {
            if planes.len() != format.plane_count() {
                return Err(MediaError::InvalidArgument("linear DMA-BUF plane count"));
            }
            let shape = format.plane(index).unwrap();
            let end = plane
                .offset
                .checked_add((shape.rows as u64 - 1) * plane.stride as u64)
                .and_then(|n| n.checked_add(shape.row_bytes as u64));
            if (plane.stride as usize) < shape.row_bytes
                || end.is_none_or(|end| end > plane.allocation_size)
            {
                return Err(MediaError::InvalidArgument("linear DMA-BUF plane bounds"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_renderer_import_retains_budget_and_excludes_overlapping_imports() {
        let budget = MemoryBudget::new(16);
        // Ownership-only fixture: this descriptor is never imported or used for GPU access.
        let mut frame = GpuVideoFrame {
            data: Arc::new(GpuFrameData {
                format: VideoFormat::rgba(2, 2, 30),
                modifier: 0,
                planes: vec![VideoDmaBufPlane {
                    fd: std::fs::File::open("/dev/null").unwrap().into(),
                    offset: 0,
                    stride: 8,
                    allocation_size: 16,
                }],
                metadata: FrameMetadata::default(),
                generation: 0,
                accounting: None,
                importing: AtomicBool::new(false),
            }),
        };
        frame.account(budget.clone(), 7, 1).unwrap();
        assert!(budget.reserve_gpu_frame(1).is_err());
        let retained = frame.clone();
        let lease = frame.begin_import().unwrap();
        assert!(retained.begin_import().is_err());
        drop(frame);
        assert_eq!(budget.used(), 16);
        drop(lease);
        let next = retained.begin_import().unwrap();
        drop(retained);
        assert_eq!(budget.used(), 16);
        drop(next);
        assert_eq!(budget.used(), 0);
        assert_eq!(budget.gpu_frames(), 0);
    }
    #[test]
    fn pending_exports_share_one_reservation_until_final_retirement() {
        let budget = MemoryBudget::new(64);
        let pending = GpuReservation::new(budget.clone(), 48, 2).unwrap();
        let submitted = pending.clone();
        // A failed byte reservation must roll back its frame slot as well.
        assert!(GpuReservation::new(budget.clone(), 32, 2).is_err());
        assert_eq!(budget.gpu_frames(), 1);
        let delivered = pending.clone();
        drop(pending);
        drop(delivered);
        assert_eq!(budget.used(), 48);
        assert_eq!(budget.gpu_frames(), 1);
        drop(submitted);
        assert_eq!(budget.used(), 0);
        assert_eq!(budget.gpu_frames(), 0);
        assert!(GpuReservation::new(budget.clone(), 64, 1).is_ok());
        assert_eq!(budget.used(), 0);
    }
    #[test]
    fn dma_buf_layout_checks_distinguish_linear_bounds_from_modifiers() {
        let file = std::fs::File::open("/dev/null").unwrap();
        // Pure descriptor validation only: this FD is never imported/mapped as a DMA-BUF.
        let mut plane = BorrowedVideoDmaBufPlane {
            fd: file.as_fd(),
            offset: 16,
            stride: 16,
            allocation_size: 44,
        };
        let format = VideoFormat::rgba(3, 2, 30);
        assert!(validate_planes(format, 0, &[plane]).is_ok());
        plane.allocation_size = 43;
        assert!(validate_planes(format, 0, &[plane]).is_err());
        plane.allocation_size = 44;
        plane.stride = 8;
        assert!(validate_planes(format, 0, &[plane]).is_err());
        // A non-linear modifier's storage is checked by the graphics API, not linear math.
        assert!(validate_planes(format, 9, &[plane]).is_ok());
        assert!(validate_planes(format, 0x00ff_ffff_ffff_ffff, &[plane]).is_err());
    }
}
