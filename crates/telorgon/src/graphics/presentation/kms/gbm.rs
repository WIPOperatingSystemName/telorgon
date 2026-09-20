use std::ffi::CStr;
use std::os::fd::{FromRawFd, OwnedFd};
use std::ptr::NonNull;
use std::slice;

use crate::foundation::{RectI, SizeI};

use crate::graphics::presentation::kms::ffi;
use crate::graphics::presentation::kms::{
    DRM_FORMAT_ARGB8888, DRM_FORMAT_XRGB8888, KmsError, KmsErrorKind, ScanoutFormat,
};

pub struct GbmDevice<'fd> {
    raw: NonNull<ffi::gbm_device>,
    drm_fd: &'fd OwnedFd,
}

impl<'fd> GbmDevice<'fd> {
    pub fn new(drm_fd: &'fd OwnedFd) -> Result<Self, KmsError> {
        unsafe { *ffi::__errno_location() = 0 };
        let raw = NonNull::new(unsafe {
            ffi::gbm_create_device(std::os::fd::AsRawFd::as_raw_fd(drm_fd))
        })
        .ok_or_else(|| {
            KmsError::last_os_error(KmsErrorKind::Allocation, "GBM device creation failed")
        })?;
        Ok(Self { raw, drm_fd })
    }

    pub fn backend_name(&self) -> Option<String> {
        let raw = unsafe { ffi::gbm_device_get_backend_name(self.raw.as_ptr()) };
        NonNull::new(raw.cast_mut()).map(|raw| {
            unsafe { CStr::from_ptr(raw.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        })
    }

    pub fn allocate(
        &self,
        size: SizeI,
        format: ScanoutFormat,
        candidate_modifiers: &[u64],
    ) -> Result<GbmBuffer<'_, 'fd>, KmsError> {
        self.allocate_with_usage(
            size,
            format,
            candidate_modifiers,
            ffi::GBM_BO_USE_SCANOUT | ffi::GBM_BO_USE_RENDERING,
        )
    }

    pub fn allocate_cursor(&self, size: SizeI) -> Result<GbmBuffer<'_, 'fd>, KmsError> {
        let format = ScanoutFormat {
            fourcc: DRM_FORMAT_ARGB8888,
            modifier: crate::graphics::presentation::kms::DRM_FORMAT_MOD_LINEAR,
        };
        let usage = ffi::GBM_BO_USE_CURSOR | ffi::GBM_BO_USE_WRITE | ffi::GBM_BO_USE_LINEAR;
        self.allocate_with_usage(
            size,
            format,
            &[crate::graphics::presentation::kms::DRM_FORMAT_MOD_LINEAR],
            usage,
        )
        .or_else(|_| self.allocate_legacy(size, format.fourcc, usage))
    }

    pub(crate) fn allocate_legacy(
        &self,
        size: SizeI,
        format: u32,
        usage: u32,
    ) -> Result<GbmBuffer<'_, 'fd>, KmsError> {
        if size.width <= 0 || size.height <= 0 {
            return Err(KmsError::new(
                KmsErrorKind::InvalidState,
                "GBM allocation needs positive dimensions",
            ));
        }
        unsafe { *ffi::__errno_location() = 0 };
        let raw = unsafe {
            ffi::gbm_bo_create(
                self.raw.as_ptr(),
                size.width as u32,
                size.height as u32,
                format,
                usage,
            )
        };
        let raw = NonNull::new(raw).ok_or_else(|| {
            let mut error =
                KmsError::last_os_error(KmsErrorKind::Allocation, "legacy GBM allocation failed");
            error.annotate(format!(
                "{}x{} fourcc={format:#x} usage={usage:#x}",
                size.width, size.height
            ));
            error
        })?;
        Ok(GbmBuffer { raw, device: self })
    }

    pub(crate) fn allocate_with_usage(
        &self,
        size: SizeI,
        format: ScanoutFormat,
        candidate_modifiers: &[u64],
        usage: u32,
    ) -> Result<GbmBuffer<'_, 'fd>, KmsError> {
        if size.width <= 0 || size.height <= 0 || candidate_modifiers.is_empty() {
            return Err(KmsError::new(
                KmsErrorKind::InvalidState,
                "GBM allocation needs positive dimensions and at least one modifier",
            ));
        }
        let count = u32::try_from(candidate_modifiers.len())
            .map_err(|_| KmsError::new(KmsErrorKind::InvalidState, "too many GBM modifiers"))?;
        let create = modifiers2().ok_or_else(|| {
            KmsError::new(
                KmsErrorKind::Unsupported,
                "GBM explicit-modifier allocation with usage flags is unavailable",
            )
        })?;
        unsafe { *ffi::__errno_location() = 0 };
        let raw = unsafe {
            create(
                self.raw.as_ptr(),
                size.width as u32,
                size.height as u32,
                format.fourcc,
                candidate_modifiers.as_ptr(),
                count,
                usage,
            )
        };
        let raw = NonNull::new(raw).ok_or_else(|| {
            let mut error =
                KmsError::last_os_error(KmsErrorKind::Allocation, "GBM scanout allocation failed");
            error.annotate(format!(
                "{}x{} fourcc={:#x} modifiers={:x?} ({} total) usage={usage:#x}",
                size.width,
                size.height,
                format.fourcc,
                &candidate_modifiers[..candidate_modifiers.len().min(8)],
                candidate_modifiers.len()
            ));
            error
        })?;
        let buffer = GbmBuffer { raw, device: self };
        let actual = buffer.format();
        if buffer.size() != size
            || actual.fourcc != format.fourcc
            || !candidate_modifiers.contains(&actual.modifier)
            || actual.modifier == super::DRM_FORMAT_MOD_INVALID
        {
            return Err(KmsError::new(
                KmsErrorKind::InvalidState,
                "GBM allocation metadata violates the requested layout",
            ));
        }
        Ok(buffer)
    }

    pub fn drm_fd(&self) -> &OwnedFd {
        self.drm_fd
    }
}

impl Drop for GbmDevice<'_> {
    fn drop(&mut self) {
        unsafe { ffi::gbm_device_destroy(self.raw.as_ptr()) };
    }
}

pub struct GbmBuffer<'device, 'fd> {
    raw: NonNull<ffi::gbm_bo>,
    device: &'device GbmDevice<'fd>,
}

impl GbmBuffer<'_, '_> {
    pub fn size(&self) -> SizeI {
        SizeI {
            width: unsafe { ffi::gbm_bo_get_width(self.raw.as_ptr()) } as i32,
            height: unsafe { ffi::gbm_bo_get_height(self.raw.as_ptr()) } as i32,
        }
    }

    pub fn format(&self) -> ScanoutFormat {
        ScanoutFormat {
            fourcc: unsafe { ffi::gbm_bo_get_format(self.raw.as_ptr()) },
            modifier: unsafe { ffi::gbm_bo_get_modifier(self.raw.as_ptr()) },
        }
    }

    pub fn plane_count(&self) -> Result<usize, KmsError> {
        let count = unsafe { ffi::gbm_bo_get_plane_count(self.raw.as_ptr()) };
        if !(1..=4).contains(&count) {
            Err(KmsError::new(
                KmsErrorKind::Unsupported,
                "GBM buffer plane count is outside KMS AddFB2 limits",
            ))
        } else {
            Ok(count as usize)
        }
    }

    pub fn export_planes(&self) -> Result<Vec<GbmPlane>, KmsError> {
        (0..self.plane_count()?)
            .map(|index| {
                let index = index as i32;
                let fd = unsafe { ffi::gbm_bo_get_fd_for_plane(self.raw.as_ptr(), index) };
                if fd < 0 {
                    return Err(KmsError::native(
                        KmsErrorKind::Native,
                        "GBM could not export a DMA-BUF plane",
                        fd,
                    ));
                }
                Ok(GbmPlane {
                    fd: unsafe { OwnedFd::from_raw_fd(fd) },
                    stride: unsafe { ffi::gbm_bo_get_stride_for_plane(self.raw.as_ptr(), index) },
                    offset: unsafe { ffi::gbm_bo_get_offset(self.raw.as_ptr(), index) },
                })
            })
            .collect()
    }

    pub fn map_write(&mut self) -> Result<GbmWriteMapping<'_>, KmsError> {
        let size = self.size();
        let mut stride = 0;
        let mut map_data = std::ptr::null_mut();
        unsafe { *ffi::__errno_location() = 0 };
        let pixels = unsafe {
            ffi::gbm_bo_map(
                self.raw.as_ptr(),
                0,
                0,
                size.width as u32,
                size.height as u32,
                ffi::GBM_BO_TRANSFER_READ_WRITE,
                &mut stride,
                &mut map_data,
            )
        };
        let pixels = NonNull::new(pixels.cast::<u8>()).ok_or_else(|| {
            KmsError::last_os_error(KmsErrorKind::Native, "GBM scanout-buffer mapping failed")
        })?;
        let length = (stride as usize)
            .checked_mul(size.height as usize)
            .filter(|n| *n <= isize::MAX as usize);
        if u64::from(stride) < size.width as u64 * 4 || length.is_none() {
            unsafe { ffi::gbm_bo_unmap(self.raw.as_ptr(), map_data) };
            return Err(KmsError::new(
                KmsErrorKind::InvalidState,
                "GBM returned an invalid mapped row stride",
            ));
        }
        Ok(GbmWriteMapping {
            buffer: self.raw,
            pixels,
            map_data,
            length: length.unwrap(),
            stride: stride as usize,
            size,
            format: self.format(),
            marker: std::marker::PhantomData,
        })
    }

    pub(crate) fn raw(&self) -> *mut ffi::gbm_bo {
        self.raw.as_ptr()
    }

    pub fn handle(&self) -> Result<u32, KmsError> {
        let handle = unsafe { ffi::gbm_bo_get_handle_for_plane(self.raw.as_ptr(), 0) };
        if handle == 0 {
            Err(KmsError::new(
                KmsErrorKind::Native,
                "GBM returned cursor buffer handle zero",
            ))
        } else {
            Ok(handle)
        }
    }

    pub fn device(&self) -> &GbmDevice<'_> {
        self.device
    }
}

impl Drop for GbmBuffer<'_, '_> {
    fn drop(&mut self) {
        unsafe { ffi::gbm_bo_destroy(self.raw.as_ptr()) };
    }
}

#[derive(Debug)]
pub struct GbmPlane {
    pub fd: OwnedFd,
    pub stride: u32,
    pub offset: u32,
}

pub struct GbmWriteMapping<'buffer> {
    buffer: NonNull<ffi::gbm_bo>,
    pixels: NonNull<u8>,
    map_data: *mut std::ffi::c_void,
    length: usize,
    stride: usize,
    size: SizeI,
    format: ScanoutFormat,
    marker: std::marker::PhantomData<&'buffer mut ffi::gbm_bo>,
}

impl GbmWriteMapping<'_> {
    pub(crate) fn clear(&mut self) {
        unsafe { std::ptr::write_bytes(self.pixels.as_ptr(), 0, self.length) };
    }

    pub const fn stride(&self) -> usize {
        self.stride
    }

    pub const fn size(&self) -> SizeI {
        self.size
    }

    pub fn write_rgba8(&mut self, source: &[u8]) -> Result<(), KmsError> {
        self.write_rgba8_region(
            source,
            RectI {
                x: 0,
                y: 0,
                width: self.size.width,
                height: self.size.height,
            },
        )
    }

    pub fn write_rgba8_region(&mut self, source: &[u8], region: RectI) -> Result<(), KmsError> {
        let target = unsafe { slice::from_raw_parts_mut(self.pixels.as_ptr(), self.length) };
        write_rgba8_region(
            target,
            self.stride,
            self.size,
            self.format.fourcc,
            source,
            region,
        )
    }
}

impl Drop for GbmWriteMapping<'_> {
    fn drop(&mut self) {
        unsafe { ffi::gbm_bo_unmap(self.buffer.as_ptr(), self.map_data) };
    }
}

pub(crate) fn write_rgba8_region(
    target: &mut [u8],
    stride: usize,
    size: SizeI,
    fourcc: u32,
    source: &[u8],
    region: RectI,
) -> Result<(), KmsError> {
    if size.width <= 0
        || size.height <= 0
        || stride < size.width as usize * 4
        || stride
            .checked_mul(size.height as usize)
            .is_none_or(|required| required > target.len())
    {
        return Err(KmsError::new(
            KmsErrorKind::InvalidState,
            "invalid CPU mapping layout",
        ));
    }
    if !matches!(fourcc, DRM_FORMAT_ARGB8888 | DRM_FORMAT_XRGB8888) {
        return Err(KmsError::new(
            KmsErrorKind::Unsupported,
            "software scanout supports DRM ARGB8888 and XRGB8888",
        ));
    }
    if region.x < 0
        || region.y < 0
        || region.width <= 0
        || region.height <= 0
        || region
            .x
            .checked_add(region.width)
            .is_none_or(|end| end > size.width)
        || region
            .y
            .checked_add(region.height)
            .is_none_or(|end| end > size.height)
    {
        return Err(KmsError::new(
            KmsErrorKind::InvalidState,
            "software scanout update region is outside the buffer",
        ));
    }
    let source_stride = size.width as usize * 4;
    let required = source_stride
        .checked_mul(size.height as usize)
        .ok_or_else(|| KmsError::new(KmsErrorKind::InvalidState, "pixel extent overflow"))?;
    if source.len() != required {
        return Err(KmsError::new(
            KmsErrorKind::InvalidState,
            "software frame does not match the scanout extent",
        ));
    }
    let row_bytes = region.width as usize * 4;
    let x = region.x as usize * 4;
    for row in region.y as usize..(region.y + region.height) as usize {
        let source = &source[row * source_stride + x..row * source_stride + x + row_bytes];
        let target = &mut target[row * stride + x..row * stride + x + row_bytes];
        for (source, target) in source
            .as_chunks::<4>()
            .0
            .iter()
            .zip(target.as_chunks_mut::<4>().0.iter_mut())
        {
            // DRM ARGB/XRGB8888 are native-endian packed values, which are BGRA/BGRX bytes on
            // the little-endian Linux systems supported by this KMS path.
            target.copy_from_slice(&[
                source[2],
                source[1],
                source[0],
                if fourcc == DRM_FORMAT_ARGB8888 {
                    source[3]
                } else {
                    255
                },
            ]);
        }
    }
    Ok(())
}

// Resolve this optional entrypoint at runtime so older GBM installations can use
// the CPU legacy/dumb path without a process-loader failure.
type CreateModifiers2 = unsafe extern "C" fn(
    *mut ffi::gbm_device,
    u32,
    u32,
    u32,
    *const u64,
    u32,
    u32,
) -> *mut ffi::gbm_bo;
fn modifiers2() -> Option<CreateModifiers2> {
    static FUNCTION: std::sync::OnceLock<Option<CreateModifiers2>> = std::sync::OnceLock::new();
    *FUNCTION.get_or_init(|| {
        let symbol = unsafe {
            ffi::dlsym(
                std::ptr::null_mut(),
                c"gbm_bo_create_with_modifiers2".as_ptr(),
            )
        };
        if symbol.is_null() {
            None
        } else {
            Some(unsafe { std::mem::transmute::<*mut std::ffi::c_void, CreateModifiers2>(symbol) })
        }
    })
}
