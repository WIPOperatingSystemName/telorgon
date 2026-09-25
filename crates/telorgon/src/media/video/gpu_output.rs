use super::*;

/// Allocates the reusable images owned by a native video producer. Called serially on the
/// connection's control worker. Allocation capability probing must not use physical capture.
///
/// # Safety
/// Return correctly described, independently owned DMA-BUF allocations within `byte_limit`.
/// No background writer may access them. Allocation/Drop must leave no GPU work outstanding.
pub unsafe trait VideoGpuProducer: Send + 'static {
    fn formats(&self) -> Vec<VideoDmaBufFormat>;
    fn allocate(
        &mut self,
        format: VideoFormat,
        modifier: u64,
        byte_limit: usize,
    ) -> Result<Box<dyn VideoGpuOutputBuffer>, MediaError>;
}

// SAFETY: boxing preserves the implementation's exclusive mutable calls and ownership.
unsafe impl<T: VideoGpuProducer + ?Sized> VideoGpuProducer for Box<T> {
    fn formats(&self) -> Vec<VideoDmaBufFormat> {
        (**self).formats()
    }
    fn allocate(
        &mut self,
        format: VideoFormat,
        modifier: u64,
        byte_limit: usize,
    ) -> Result<Box<dyn VideoGpuOutputBuffer>, MediaError> {
        (**self).allocate(format, modifier, byte_limit)
    }
}

/// One fixed allocation; descriptor identities and layouts must remain unchanged until Drop.
/// The transport keeps this owner through native buffer removal, including renegotiation.
///
/// # Safety
/// Descriptors must denote the allocated format/modifier from `VideoGpuProducer::allocate`.
/// `copy_from` must wait destination implicit read/write fences, serialize source imports,
/// preserve encoded pixels, and finish all reads/writes before returning, including errors.
/// It must not retain input frames or descriptors after the call.
pub unsafe trait VideoGpuOutputBuffer: Send {
    fn planes(&self) -> &[VideoDmaBufPlane];
    /// # Safety
    /// The caller must exclusively own the native output buffer, with no new consumer access
    /// starting before this call returns. Existing implicit consumer fences must be honored by
    /// the implementation. Merely owning an FD does not grant permission to overwrite pixels.
    unsafe fn copy_from(&mut self, frame: &GpuVideoFrame) -> Result<(), MediaError>;
}

pub(crate) enum GpuBackend {
    Capture(Box<dyn VideoGpuTransfer>),
    Produce(Box<dyn VideoGpuProducer>),
}
