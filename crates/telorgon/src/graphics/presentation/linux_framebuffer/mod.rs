//! Explicit Linux fbdev presentation. The caller arranges exclusive display ownership; this
//! adapter never switches VTs, changes modes, discovers devices, or starts an event loop.

mod abi;
mod layout;
#[cfg(test)]
mod tests;

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::ptr::NonNull;

use crate::foundation::{ColorRgba8, SizeF, SizeI};
use crate::graphics::presentation::{
    AlphaMode, ColorSpace, PresentationError, PresentationErrorKind, PresentationResult,
    SurfaceMetrics, SurfaceRevision,
};
use crate::graphics::renderers::software::SoftwareSurface;
use layout::Layout;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FramebufferExit {
    #[default]
    KeepLastFrame,
    Clear(ColorRgba8),
}

pub struct LinuxFramebufferPresenter {
    file: File,
    layout: Layout,
    mapping: NonNull<u8>,
    scratch: Vec<u8>,
}

impl LinuxFramebufferPresenter {
    /// Opens exactly the supplied framebuffer. No display contents change until presentation.
    pub fn open(path: impl AsRef<Path>, maximum_mapping_bytes: usize) -> PresentationResult<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| native("could not open framebuffer", error))?;
        let layout = query(&file)?;
        if layout.mapping_bytes > maximum_mapping_bytes {
            return Err(PresentationError::new(
                PresentationErrorKind::OutOfMemory,
                "framebuffer mapping exceeds the configured byte budget",
            ));
        }
        let mut scratch = Vec::new();
        scratch.try_reserve_exact(layout.row_bytes()).map_err(|_| {
            PresentationError::new(
                PresentationErrorKind::OutOfMemory,
                "could not reserve framebuffer conversion row",
            )
        })?;
        scratch.resize(layout.row_bytes(), 0);
        // The fbdev fd owns this mapping; length is checked against the driver-reported smem_len.
        // We retain the fd until after unmapping and never expose references to device memory.
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                layout.mapping_bytes,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(native(
                "could not map framebuffer",
                std::io::Error::last_os_error(),
            ));
        }
        let Some(mapping) = NonNull::new(raw.cast::<u8>()) else {
            unsafe {
                libc::munmap(raw, layout.mapping_bytes);
            }
            return Err(PresentationError::new(
                PresentationErrorKind::Native,
                "framebuffer mapping unexpectedly used the null address",
            ));
        };
        Ok(Self {
            file,
            layout,
            mapping,
            scratch,
        })
    }

    pub fn extent(&self) -> SizeI {
        self.layout.extent
    }

    pub fn metrics(&self, scale_factor: f64) -> PresentationResult<SurfaceMetrics> {
        let metrics = SurfaceMetrics {
            revision: SurfaceRevision::new(1),
            logical_extent: SizeF {
                width: (f64::from(self.layout.extent.width) / scale_factor) as f32,
                height: (f64::from(self.layout.extent.height) / scale_factor) as f32,
            },
            physical_extent: self.layout.extent,
            scale_factor,
            color_space: ColorSpace::Srgb,
            alpha_mode: AlphaMode::Opaque,
        };
        metrics.validate()
    }

    pub fn present(&mut self, surface: &SoftwareSurface) -> PresentationResult<()> {
        self.present_rgba8(surface.framebuffer_extent(), surface.pixels_rgba8())
    }

    /// Copies a full opaque frame into the current visible pan region, respecting driver stride.
    /// This synchronous copy does not promise vblank synchronization or display completion.
    pub fn present_rgba8(&mut self, extent: SizeI, pixels: &[u8]) -> PresentationResult<()> {
        self.check_layout()?;
        let rgba_row = self.layout.extent.width as usize * 4;
        let expected = rgba_row
            .checked_mul(self.layout.extent.height as usize)
            .ok_or_else(|| {
                PresentationError::new(
                    PresentationErrorKind::InvalidState,
                    "software frame byte length overflows",
                )
            })?;
        if extent != self.layout.extent || pixels.len() != expected {
            return Err(PresentationError::new(
                PresentationErrorKind::InvalidState,
                "software frame does not match the framebuffer extent",
            ));
        }
        for (row, source) in pixels.chunks_exact(rgba_row).enumerate() {
            self.layout.encode(source, &mut self.scratch);
            self.write_row(row);
        }
        Ok(())
    }

    pub fn shutdown(mut self, exit: FramebufferExit) -> PresentationResult<()> {
        if let FramebufferExit::Clear(color) = exit {
            self.check_layout()?;
            let rgba = [color.r, color.g, color.b, color.a];
            let mut pixel = [0u8; 4];
            self.layout
                .encode(&rgba, &mut pixel[..self.layout.pixel_bytes]);
            for target in self.scratch.chunks_exact_mut(self.layout.pixel_bytes) {
                target.copy_from_slice(&pixel[..self.layout.pixel_bytes]);
            }
            for row in 0..self.layout.extent.height as usize {
                self.write_row(row);
            }
        }
        Ok(())
    }

    fn check_layout(&self) -> PresentationResult<()> {
        if query(&self.file)? != self.layout {
            return Err(PresentationError::new(
                PresentationErrorKind::SurfaceLost,
                "framebuffer mode, pan region, or memory changed; reopen presentation",
            ));
        }
        Ok(())
    }

    fn write_row(&mut self, row: usize) {
        let offset = self.layout.visible_offset + row * self.layout.stride;
        // Layout validation proves every byte of every visible row fits the mapping. Mutable
        // presenter access serializes writes; volatile stores preserve writes to device memory.
        for (column, byte) in self.scratch.iter().copied().enumerate() {
            unsafe {
                self.mapping
                    .as_ptr()
                    .add(offset + column)
                    .write_volatile(byte);
            }
        }
    }
}

impl Drop for LinuxFramebufferPresenter {
    fn drop(&mut self) {
        // Mapping and fd remain owned here, with no outstanding public device-memory borrows.
        unsafe {
            libc::munmap(self.mapping.as_ptr().cast(), self.layout.mapping_bytes);
        }
    }
}

fn query(file: &File) -> PresentationResult<Layout> {
    let mut fixed = abi::FixedInfo::default();
    let mut variable = abi::VariableInfo::default();
    // The requests write exactly the corresponding repr(C) linux/fb.h UAPI structure.
    if unsafe { libc::ioctl(file.as_raw_fd(), abi::GET_FIXED, &mut fixed) } < 0 {
        return Err(native(
            "could not query fixed framebuffer info",
            std::io::Error::last_os_error(),
        ));
    }
    if unsafe { libc::ioctl(file.as_raw_fd(), abi::GET_VARIABLE, &mut variable) } < 0 {
        return Err(native(
            "could not query variable framebuffer info",
            std::io::Error::last_os_error(),
        ));
    }
    Layout::read(&fixed, &variable)
}

fn native(context: &str, error: std::io::Error) -> PresentationError {
    let code = error.raw_os_error().map(i64::from);
    let context = format!("{context}: {error}");
    match code {
        Some(code) => {
            PresentationError::with_backend_code(PresentationErrorKind::Native, context, code)
        }
        None => PresentationError::new(PresentationErrorKind::Native, context),
    }
}
