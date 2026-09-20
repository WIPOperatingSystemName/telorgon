//! Owned SHM destination; pixel writes belong on a delivery worker, never dispatch.
use super::*;
use std::fs::File;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub(super) struct CaptureCancellation(Arc<AtomicBool>);
impl CaptureCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}
impl Drop for CaptureCancellation {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(super) struct CaptureDestination {
    descriptor: ShmBuffer,
    file: File,
    row_bytes: usize,
    extent: u64,
    cancelled: Arc<AtomicBool>,
}
impl CaptureDestination {
    pub fn new(
        descriptor: ShmBuffer,
        file: File,
        size: crate::core::SizeI,
        cancellation: &CaptureCancellation,
    ) -> Result<Self, NativeCompositorError> {
        let invalid = || NativeCompositorError::new("capture buffer constraints mismatch");
        if descriptor.size != size
            || size.width <= 0
            || size.height <= 0
            || descriptor.format != ShmFormat::Argb8888
        {
            return Err(invalid());
        }
        let row_bytes = (size.width as usize).checked_mul(4).ok_or_else(invalid)?;
        if (descriptor.stride as usize) < row_bytes {
            return Err(invalid());
        }
        let extent = (descriptor.offset as u64)
            .checked_add(
                u64::from(descriptor.stride)
                    .checked_mul(size.height as u64)
                    .ok_or_else(invalid)?,
            )
            .ok_or_else(invalid)?;
        if file.metadata().map_err(error)?.len() < extent {
            return Err(invalid());
        }
        Ok(Self {
            descriptor,
            file,
            row_bytes,
            extent,
            cancelled: cancellation.0.clone(),
        })
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn write_rgba(&self, rgba: &[u8]) -> Result<(), NativeCompositorError> {
        self.check_cancelled()?;
        let length = self
            .row_bytes
            .checked_mul(self.descriptor.size.height as usize)
            .ok_or_else(|| NativeCompositorError::new("capture byte length overflow"))?;
        if rgba.len() != length || self.file.metadata().map_err(error)?.len() < self.extent {
            return Err(NativeCompositorError::new(
                "capture pixels or backing file changed size",
            ));
        }
        let mut row = vec![0; self.row_bytes];
        for (y, source) in rgba.chunks_exact(self.row_bytes).enumerate() {
            for (source, target) in source.chunks_exact(4).zip(row.chunks_exact_mut(4)) {
                let argb = (u32::from(source[3]) << 24)
                    | (u32::from(source[0]) << 16)
                    | (u32::from(source[1]) << 8)
                    | u32::from(source[2]);
                target.copy_from_slice(&argb.to_ne_bytes());
            }
            self.check_cancelled()?;
            self.file
                .write_all_at(
                    &row,
                    self.descriptor.offset as u64 + y as u64 * u64::from(self.descriptor.stride),
                )
                .map_err(error)?;
        }
        self.check_cancelled()
    }

    fn check_cancelled(&self) -> Result<(), NativeCompositorError> {
        if self.is_cancelled() {
            Err(NativeCompositorError::new("capture delivery cancelled"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_destination_converts_channels_preserves_padding_and_rejects_truncation() {
        let path = std::env::temp_dir().join(format!(
            "telorgon-capture-destination-{}",
            std::process::id()
        ));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(path).unwrap();
        file.set_len(32).unwrap();
        file.write_all_at(&[0x5a; 32], 0).unwrap();
        let size = crate::core::SizeI {
            width: 2,
            height: 2,
        };
        let descriptor = ShmBuffer {
            offset: 4,
            size,
            stride: 12,
            format: ShmFormat::Argb8888,
        };
        let cancellation = CaptureCancellation::default();
        let destination =
            CaptureDestination::new(descriptor, file.try_clone().unwrap(), size, &cancellation)
                .unwrap();
        let pixels = [1, 2, 3, 255, 4, 5, 6, 128, 7, 8, 9, 255, 10, 11, 12, 255];
        destination.write_rgba(&pixels).unwrap();
        let mut bytes = [0; 32];
        file.read_exact_at(&mut bytes, 0).unwrap();
        assert_eq!(&bytes[4..8], &0xff010203u32.to_ne_bytes());
        assert_eq!(&bytes[8..12], &0x80040506u32.to_ne_bytes());
        assert_eq!(&bytes[16..20], &0xff070809u32.to_ne_bytes());
        assert_eq!(&bytes[20..24], &0xff0a0b0cu32.to_ne_bytes());
        for range in [0..4, 12..16, 24..32] {
            assert!(bytes[range].iter().all(|byte| *byte == 0x5a));
        }
        assert!(destination.write_rgba(&pixels[..12]).is_err());
        // The writer can outlive its native frame; dropping the owner cancels it.
        assert!(!destination.is_cancelled());
        drop(cancellation);
        assert!(destination.is_cancelled());
        file.write_all_at(&[0x5a; 32], 0).unwrap();
        assert!(destination.write_rgba(&pixels).is_err());
        file.read_exact_at(&mut bytes, 0).unwrap();
        assert_eq!(bytes, [0x5a; 32]);
        let active = CaptureCancellation::default();
        let destination =
            CaptureDestination::new(descriptor, file.try_clone().unwrap(), size, &active).unwrap();
        file.set_len(8).unwrap();
        assert!(destination.write_rgba(&pixels).is_err());
        assert_eq!(file.metadata().unwrap().len(), 8);
    }
}
