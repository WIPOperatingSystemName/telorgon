use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
pub const MAX_DAMAGE_RECTS: usize = 64;
pub const MAX_CURSOR_BYTES: usize = 256 * 256 * 4;
/// Signed strides support bottom-up memory. offset addresses the first logical row;
/// each accessed row must fit the owned allocation. Padding is never exposed as pixels.
#[derive(Clone, Debug)]
pub struct CpuPlane {
    pub bytes: Vec<u8>,
    pub offset: usize,
    pub stride: i32,
}
impl CpuPlane {
    pub(crate) fn validate(&self, shape: PlaneShape) -> Result<(), MediaError> {
        if (self.stride.unsigned_abs() as usize) < shape.row_bytes || shape.rows == 0 {
            return Err(MediaError::InvalidArgument("video plane stride"));
        }
        let last = self.offset as i128 + (shape.rows - 1) as i128 * self.stride as i128;
        let min = last.min(self.offset as i128);
        let max = last.max(self.offset as i128) + shape.row_bytes as i128;
        if min < 0 || max > self.bytes.len() as i128 {
            return Err(MediaError::InvalidArgument("video plane bounds"));
        }
        Ok(())
    }
    pub fn row(&self, row: usize, shape: PlaneShape) -> Option<&[u8]> {
        if row >= shape.rows {
            return None;
        }
        let start = self
            .offset
            .checked_add_signed((self.stride as isize).checked_mul(row.try_into().ok()?)?)?;
        self.bytes.get(start..start.checked_add(shape.row_bytes)?)
    }
    pub(crate) fn row_mut(&mut self, row: usize, shape: PlaneShape) -> Option<&mut [u8]> {
        if row >= shape.rows {
            return None;
        }
        let start = self
            .offset
            .checked_add_signed((self.stride as isize).checked_mul(row.try_into().ok()?)?)?;
        self.bytes
            .get_mut(start..start.checked_add(shape.row_bytes)?)
    }
}
#[derive(Clone, Debug)]
pub struct VideoCursor {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
    /// None means reuse the previous bitmap for this cursor ID. A missing cursor update
    /// preserves previous state; visibility is independently optional.
    pub bitmap: Option<CursorBitmap>,
    pub visible: Option<bool>,
}
#[derive(Clone, Debug)]
pub struct CursorBitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug, Default)]
pub struct FrameMetadata {
    /// Source sequence when supplied by capture. Producers assign their own transport
    /// sequence; a submitted frame's sequence does not override it.
    pub sequence: Option<u64>,
    /// CLOCK_MONOTONIC presentation time. None means the source supplied no timestamp.
    pub timestamp_ns: Option<i64>,
    pub discontinuity: bool,
    pub crop: Option<VideoRect>,
    pub transform: VideoTransform,
    /// Damage relative to the preceding transport frame. Ignore partial rectangles and
    /// redraw the full image when discontinuity is true (including receiver queue backlog).
    /// Empty means unspecified/full damage, not an assertion that pixels are unchanged.
    pub damage: Vec<VideoRect>,
    pub cursor: Option<VideoCursor>,
}
impl FrameMetadata {
    /// PipeWire video timestamps already use CLOCK_MONOTONIC. Missing timestamps require
    /// caller policy rather than pretending arrival time was presentation time.
    pub fn deadline(
        &self,
        monotonic_now_ns: i64,
        maximum_lateness_ns: u64,
    ) -> Option<crate::media::timing::VideoDeadline> {
        Some(crate::media::timing::video_deadline(
            self.timestamp_ns?,
            monotonic_now_ns,
            maximum_lateness_ns,
        ))
    }

    pub(crate) fn allocation_bytes(&self) -> usize {
        self.damage
            .capacity()
            .saturating_mul(std::mem::size_of::<VideoRect>())
            .saturating_add(
                self.cursor
                    .as_ref()
                    .and_then(|c| c.bitmap.as_ref())
                    .map_or(0, |b| b.rgba.capacity()),
            )
    }
    pub fn validate(&self, format: VideoFormat) -> Result<(), MediaError> {
        if self.damage.len() > MAX_DAMAGE_RECTS
            || self.damage.iter().any(|r| !r.fits(format))
            || self.crop.is_some_and(|r| !r.fits(format))
        {
            return Err(MediaError::InvalidArgument("video crop or damage"));
        }
        if let Some(bitmap) = self.cursor.as_ref().and_then(|c| c.bitmap.as_ref()) {
            if bitmap.width == 0
                || bitmap.height == 0
                || bitmap.width > 256
                || bitmap.height > 256
                || bitmap.rgba.len() != bitmap.width as usize * bitmap.height as usize * 4
            {
                return Err(MediaError::InvalidArgument("cursor bitmap"));
            }
        }
        Ok(())
    }
}
pub(crate) struct MemoryBudget {
    used: AtomicUsize,
    gpu_frames: AtomicUsize,
    limit: usize,
    parent: Option<Arc<MemoryBudget>>,
}
impl MemoryBudget {
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicUsize::new(0),
            gpu_frames: AtomicUsize::new(0),
            limit,
            parent: None,
        })
    }
    pub(crate) fn child(parent: Arc<Self>, limit: usize) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicUsize::new(0),
            gpu_frames: AtomicUsize::new(0),
            limit,
            parent: Some(parent),
        })
    }
    pub(crate) fn available(&self) -> usize {
        let local = self.limit.saturating_sub(self.used());
        self.parent
            .as_ref()
            .map_or(local, |parent| local.min(parent.available()))
    }
    pub(crate) fn gpu_frames(&self) -> usize {
        self.gpu_frames.load(Ordering::Acquire)
    }
    pub(crate) fn reserve_gpu_frame(&self, limit: usize) -> Result<(), MediaError> {
        self.gpu_frames
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < limit).then_some(used + 1)
            })
            .map(|_| ())
            .map_err(|_| MediaError::ResourceLimit("held GPU frame count"))
    }
    pub(crate) fn release_gpu_frame(&self) {
        self.gpu_frames.fetch_sub(1, Ordering::AcqRel);
    }
    pub(crate) fn reserve(&self, bytes: usize) -> Result<(), MediaError> {
        if let Some(parent) = &self.parent {
            parent.reserve(bytes)?;
        }
        let result = self
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|n| *n <= self.limit)
            })
            .map(|_| ())
            .map_err(|_| MediaError::ResourceLimit("held video frame bytes"));
        if result.is_err() {
            if let Some(parent) = &self.parent {
                parent.release(bytes);
            }
        }
        result
    }
    pub(crate) fn release(&self, bytes: usize) {
        self.used.fetch_sub(bytes, Ordering::AcqRel);
        if let Some(parent) = &self.parent {
            parent.release(bytes);
        }
    }
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }
}
pub(crate) struct MemoryReservation {
    budget: Arc<MemoryBudget>,
    bytes: usize,
}
impl MemoryReservation {
    pub(crate) fn new(budget: Arc<MemoryBudget>, bytes: usize) -> Result<Self, MediaError> {
        budget.reserve(bytes)?;
        Ok(Self { budget, bytes })
    }
    pub(crate) fn resize(&mut self, bytes: usize) -> Result<(), MediaError> {
        if bytes > self.bytes {
            self.budget.reserve(bytes - self.bytes)?;
        } else {
            self.budget.release(self.bytes - bytes);
        }
        self.bytes = bytes;
        Ok(())
    }
}
impl Drop for MemoryReservation {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}
#[derive(Default)]
pub(crate) struct CapturePixels {
    pub(crate) bytes: Vec<u8>,
    reservation: Option<MemoryReservation>,
}
impl CapturePixels {
    pub(crate) fn capacity(&self) -> usize {
        self.bytes.capacity()
    }

    pub(crate) fn new(bytes: usize, budget: Option<Arc<MemoryBudget>>) -> Result<Self, MediaError> {
        let mut reservation = budget
            .map(|budget| MemoryReservation::new(budget, bytes))
            .transpose()?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(bytes)
            .map_err(|_| MediaError::ResourceLimit("capture pixel allocation"))?;
        pixels.resize(bytes, 0);
        if let Some(charge) = &mut reservation {
            charge.resize(pixels.capacity())?;
        }
        Ok(Self {
            bytes: pixels,
            reservation,
        })
    }
    #[cfg(test)]
    pub(crate) fn unaccounted(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            reservation: None,
        }
    }
}
impl std::ops::Deref for CapturePixels {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}
impl std::ops::DerefMut for CapturePixels {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.bytes
    }
}
pub(crate) struct FrameData {
    pub format: VideoFormat,
    pub planes: Vec<CpuPlane>,
    pub metadata: FrameMetadata,
    pub generation: u64,
    // Fields drop in order: release pixel/metadata allocations before their charge.
    _reservation: Option<MemoryReservation>,
    pixel_reservation: Option<MemoryReservation>,
}
/// Immutable pixels and metadata. Cloning pins the same pool slot; Drop releases that pin
/// from any thread. A frame never borrows PipeWire memory and remains readable after stream
/// removal, shutdown, or format changes. Holding every slot makes capture drop new frames.
#[derive(Clone)]
pub struct CpuVideoFrame {
    pub(crate) data: Arc<FrameData>,
}
impl std::fmt::Debug for CpuVideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpuVideoFrame")
            .field("format", &self.data.format)
            .field("metadata", &self.data.metadata)
            .field("generation", &self.data.generation)
            .finish_non_exhaustive()
    }
}
impl CpuVideoFrame {
    pub fn from_planes(
        mut format: VideoFormat,
        planes: Vec<CpuPlane>,
        metadata: FrameMetadata,
    ) -> Result<Self, MediaError> {
        format.validate()?;
        format.rate = format.rate.reduced();
        metadata.validate(format)?;
        if planes.len() != format.plane_count() {
            return Err(MediaError::InvalidArgument("video plane count"));
        }
        let mut bytes = metadata.allocation_bytes();
        for (index, plane) in planes.iter().enumerate() {
            plane.validate(format.plane(index).unwrap())?;
            bytes = bytes
                .checked_add(plane.bytes.capacity())
                .ok_or(MediaError::InvalidArgument("video plane bytes"))?;
        }
        if bytes > 512 * 1024 * 1024 {
            return Err(MediaError::ResourceLimit("video frame bytes"));
        }
        Ok(Self {
            data: Arc::new(FrameData {
                format,
                planes,
                metadata,
                generation: 0,
                _reservation: None,
                pixel_reservation: None,
            }),
        })
    }
    pub fn packed(
        format: VideoFormat,
        bytes: Vec<u8>,
        metadata: FrameMetadata,
    ) -> Result<Self, MediaError> {
        format.validate()?;
        if format.plane_count() != 1 {
            return Err(MediaError::InvalidArgument("packed video format"));
        }
        let stride = format.plane(0).unwrap().row_bytes as i32;
        Self::from_planes(
            format,
            vec![CpuPlane {
                bytes,
                offset: 0,
                stride,
            }],
            metadata,
        )
    }
    /// Recover a tightly packed single-plane allocation only when no other frame/pool owner
    /// exists. No copy is performed. On success the returned Vec is caller-owned and no longer
    /// charged to its old capture pool; on failure this returns the original lease unchanged.
    pub fn try_into_packed(self) -> Result<Vec<u8>, Self> {
        self.try_into_capture().map(|pixels| pixels.bytes)
    }
    pub(crate) fn packed_capture(
        mut format: VideoFormat,
        pixels: CapturePixels,
        metadata: FrameMetadata,
        metadata_charge: Option<MemoryReservation>,
    ) -> Result<Self, (MediaError, CapturePixels)> {
        let validated = (|| {
            format.validate()?;
            metadata.validate(format)?;
            if format.plane_count() != 1 || format.byte_len()? != pixels.len() {
                return Err(MediaError::InvalidArgument("packed capture layout"));
            }
            let stride = i32::try_from(format.plane(0).unwrap().row_bytes)
                .map_err(|_| MediaError::InvalidArgument("packed capture stride"))?;
            if pixels
                .capacity()
                .checked_add(metadata.allocation_bytes())
                .is_none_or(|bytes| bytes > 512 * 1024 * 1024)
            {
                return Err(MediaError::ResourceLimit("capture frame bytes"));
            }
            Ok(stride)
        })();
        let stride = match validated {
            Ok(stride) => stride,
            Err(error) => return Err((error, pixels)),
        };
        format.rate = format.rate.reduced();
        Ok(Self {
            data: Arc::new(FrameData {
                format,
                planes: vec![CpuPlane {
                    bytes: pixels.bytes,
                    offset: 0,
                    stride,
                }],
                metadata,
                generation: 0,
                _reservation: metadata_charge,
                pixel_reservation: pixels.reservation,
            }),
        })
    }
    pub(crate) fn try_into_capture(self) -> Result<CapturePixels, Self> {
        if self.data.planes.len() != 1
            || self.data.planes[0].offset != 0
            || self.data.planes[0].stride as usize != self.data.format.plane(0).unwrap().row_bytes
            || self.data.planes[0].bytes.len() != self.data.format.byte_len().unwrap_or(0)
        {
            return Err(self);
        }
        match Arc::try_unwrap(self.data) {
            Ok(mut data) => Ok(CapturePixels {
                bytes: std::mem::take(&mut data.planes[0].bytes),
                reservation: data.pixel_reservation.take(),
            }),
            Err(data) => Err(Self { data }),
        }
    }
    pub fn format(&self) -> VideoFormat {
        self.data.format
    }
    pub fn metadata(&self) -> &FrameMetadata {
        &self.data.metadata
    }
    pub fn generation(&self) -> u64 {
        self.data.generation
    }
    pub fn plane(&self, index: usize) -> Option<PlaneView<'_>> {
        Some(PlaneView {
            plane: self.data.planes.get(index)?,
            shape: self.data.format.plane(index)?,
        })
    }
    pub fn byte_len(&self) -> usize {
        self.data.planes.iter().map(|p| p.bytes.len()).sum()
    }
    /// Allocation held by this frame, including unused Vec capacities and metadata.
    pub fn allocation_bytes(&self) -> usize {
        self.data
            .planes
            .iter()
            .map(|p| p.bytes.capacity())
            .sum::<usize>()
            + self.data.metadata.allocation_bytes()
    }
}
pub struct PlaneView<'a> {
    plane: &'a CpuPlane,
    shape: PlaneShape,
}
impl<'a> PlaneView<'a> {
    pub fn shape(&self) -> PlaneShape {
        self.shape
    }
    pub fn stride(&self) -> i32 {
        self.plane.stride
    }
    pub fn row(&self, row: usize) -> Option<&'a [u8]> {
        self.plane.row(row, self.shape)
    }
}
pub(crate) struct FramePool {
    pub slots: Vec<Arc<FrameData>>,
}
impl FramePool {
    pub fn new(
        format: VideoFormat,
        generation: u64,
        count: usize,
        budget: Arc<MemoryBudget>,
    ) -> Result<Self, MediaError> {
        let bytes = format
            .byte_len()?
            .checked_add(MAX_CURSOR_BYTES + MAX_DAMAGE_RECTS * std::mem::size_of::<VideoRect>())
            .ok_or(MediaError::InvalidArgument("video pool bytes"))?;
        if budget
            .used()
            .checked_add(
                bytes
                    .checked_mul(count)
                    .ok_or(MediaError::ResourceLimit("video pool"))?,
            )
            .is_none_or(|n| n > budget.limit)
        {
            return Err(MediaError::ResourceLimit("held video frame bytes"));
        }
        let mut slots = Vec::with_capacity(count);
        for _ in 0..count {
            let reservation = MemoryReservation::new(budget.clone(), bytes)?;
            let frame = FrameData {
                format,
                planes: (0..format.plane_count())
                    .map(|i| {
                        let shape = format.plane(i).unwrap();
                        CpuPlane {
                            bytes: vec![0; shape.row_bytes * shape.rows],
                            offset: 0,
                            stride: shape.row_bytes as i32,
                        }
                    })
                    .collect(),
                metadata: FrameMetadata::default(),
                generation,
                _reservation: Some(reservation),
                pixel_reservation: None,
            };
            slots.push(Arc::new(frame));
        }
        Ok(Self { slots })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_recovery_requires_exclusive_ownership_and_releases_pool_budget() {
        let format = VideoFormat::rgba(2, 2, 120);
        let budget = MemoryBudget::new(1024 * 1024);
        let pool = FramePool::new(format, 1, 2, budget.clone()).unwrap();
        let frame = CpuVideoFrame {
            data: pool.slots[0].clone(),
        };
        let pointer = frame.data.planes[0].bytes.as_ptr();
        let frame = frame.try_into_packed().unwrap_err();
        drop(pool);
        assert!(budget.used() > 0);
        let bytes = frame.try_into_packed().unwrap();
        assert_eq!(bytes.as_ptr(), pointer);
        assert_eq!(bytes.len(), 16);
        assert_eq!(budget.used(), 0);
    }
    #[test]
    fn signed_strides_and_subsampled_planes_are_checked() {
        let format = VideoFormat::rgba(2, 2, 120);
        let frame = CpuVideoFrame::from_planes(
            format,
            vec![CpuPlane {
                bytes: (0..20).collect(),
                offset: 12,
                stride: -12,
            }],
            FrameMetadata::default(),
        )
        .unwrap();
        assert_eq!(
            frame.plane(0).unwrap().row(0).unwrap(),
            &[12, 13, 14, 15, 16, 17, 18, 19]
        );
        assert_eq!(
            frame.plane(0).unwrap().row(1).unwrap(),
            &[0, 1, 2, 3, 4, 5, 6, 7]
        );
        assert!(
            CpuVideoFrame::from_planes(
                format,
                vec![CpuPlane {
                    bytes: vec![0; 16],
                    offset: 0,
                    stride: -8
                }],
                FrameMetadata::default()
            )
            .is_err()
        );
        let format = VideoFormat {
            pixel: PixelFormat::Nv12,
            width: 3,
            height: 3,
            ..format
        };
        assert_eq!(
            format.plane(1),
            Some(PlaneShape {
                row_bytes: 4,
                rows: 2
            })
        );
        assert_eq!(format.byte_len().unwrap(), 17);
    }
    #[test]
    fn retained_frames_pin_memory_and_prevent_pool_reuse() {
        let format = VideoFormat::rgba(2, 2, 60);
        let cost = format.byte_len().unwrap()
            + MAX_CURSOR_BYTES
            + MAX_DAMAGE_RECTS * std::mem::size_of::<VideoRect>();
        let budget = MemoryBudget::new(cost * 2);
        let mut pool = FramePool::new(format, 1, 2, budget.clone()).unwrap();
        let frame = CpuVideoFrame {
            data: pool.slots[0].clone(),
        };
        assert!(Arc::get_mut(&mut pool.slots[0]).is_none());
        drop(pool);
        assert_eq!(budget.used(), cost);
        assert!(FramePool::new(format, 2, 2, budget.clone()).is_err());
        assert_eq!(frame.plane(0).unwrap().row(0).unwrap(), &[0; 8]);
        drop(frame);
        assert_eq!(budget.used(), 0);
        assert!(FramePool::new(format, 2, 2, budget).is_ok());
    }
}
