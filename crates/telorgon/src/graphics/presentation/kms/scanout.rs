//! Presenter-owned CPU compatibility buffers; no renderer or Vulkan dependency.
use super::{
    DRM_FORMAT_MOD_INVALID, DRM_FORMAT_MOD_LINEAR, GbmBuffer, KmsDevice, KmsError, KmsErrorKind,
    KmsFramebuffer, ScanoutFormat, ffi, gbm,
};
use crate::foundation::{RectI, SizeI};
use std::os::fd::AsRawFd;
use std::ptr::NonNull;

pub enum ScanoutBuffer<'a> {
    Gbm {
        buffer: GbmBuffer<'a, 'a>,
        implicit: bool,
    },
    Dumb(DumbBuffer<'a>),
}

impl ScanoutBuffer<'_> {
    pub fn format(&self) -> ScanoutFormat {
        match self {
            Self::Gbm { buffer, implicit } => {
                let mut format = buffer.format();
                if *implicit {
                    format.modifier = DRM_FORMAT_MOD_INVALID;
                }
                format
            }
            Self::Dumb(buffer) => ScanoutFormat {
                fourcc: buffer.fourcc,
                modifier: DRM_FORMAT_MOD_LINEAR,
            },
        }
    }
    pub fn gbm(&self) -> Option<&GbmBuffer<'_, '_>> {
        match self {
            Self::Gbm { buffer, .. } => Some(buffer),
            Self::Dumb(_) => None,
        }
    }
    pub fn framebuffer<'kms>(
        &self,
        kms: &'kms KmsDevice,
    ) -> Result<KmsFramebuffer<'kms>, KmsError> {
        match self {
            Self::Gbm { buffer, implicit } => kms.add_framebuffer_with_layout(buffer, *implicit),
            Self::Dumb(buffer) => kms.add_framebuffer_layout(
                buffer.size,
                ScanoutFormat {
                    fourcc: buffer.fourcc,
                    modifier: DRM_FORMAT_MOD_INVALID,
                },
                [buffer.handle, 0, 0, 0],
                [buffer.pitch, 0, 0, 0],
                [0; 4],
            ),
        }
    }
    pub fn test_cpu_write(&mut self) -> Result<(), KmsError> {
        match self {
            Self::Gbm { buffer, .. } => buffer.map_write().map(|mut mapping| mapping.clear()),
            Self::Dumb(_) => Ok(()), // Mapped and initialized during construction.
        }
    }
    pub fn write_rgba8_region(&mut self, source: &[u8], region: RectI) -> Result<(), KmsError> {
        match self {
            Self::Gbm { buffer, .. } => buffer.map_write()?.write_rgba8_region(source, region),
            Self::Dumb(buffer) => {
                let target = unsafe {
                    std::slice::from_raw_parts_mut(buffer.mapping.unwrap().as_ptr(), buffer.length)
                };
                gbm::write_rgba8_region(
                    target,
                    buffer.pitch as usize,
                    buffer.size,
                    buffer.fourcc,
                    source,
                    region,
                )
            }
        }
    }
}

pub struct DumbBuffer<'kms> {
    kms: &'kms KmsDevice,
    handle: u32,
    pitch: u32,
    length: usize,
    mapping: Option<NonNull<u8>>,
    size: SizeI,
    fourcc: u32,
    api: DumbApi,
}

impl<'kms> DumbBuffer<'kms> {
    pub fn new(kms: &'kms KmsDevice, size: SizeI, fourcc: u32) -> Result<Self, KmsError> {
        if size.width <= 0
            || size.height <= 0
            || !matches!(
                fourcc,
                super::DRM_FORMAT_XRGB8888 | super::DRM_FORMAT_ARGB8888
            )
        {
            return Err(KmsError::new(
                KmsErrorKind::InvalidState,
                "invalid CPU dumb-buffer format/extent",
            ));
        }
        if kms.capability(ffi::DRM_CAP_DUMB_BUFFER)? == 0 {
            return Err(KmsError::new(
                KmsErrorKind::Unsupported,
                "DRM dumb buffers are unavailable",
            ));
        }
        let api = dumb_api().ok_or_else(|| {
            KmsError::new(
                KmsErrorKind::Unsupported,
                "libdrm dumb-buffer entrypoints are unavailable",
            )
        })?;
        let mut handle = 0;
        let mut pitch = 0;
        let mut bytes = 0;
        let result = unsafe {
            (api.create)(
                kms.fd().as_raw_fd(),
                size.width as u32,
                size.height as u32,
                32,
                0,
                &mut handle,
                &mut pitch,
                &mut bytes,
            )
        };
        if result != 0 {
            return Err(KmsError::native(
                KmsErrorKind::Allocation,
                "DRM dumb-buffer allocation failed",
                result.saturating_abs(),
            ));
        }
        // Own the handle immediately so all subsequent error paths release it.
        let mut buffer = Self {
            kms,
            handle,
            pitch,
            length: 0,
            mapping: None,
            size,
            fourcc,
            api,
        };
        buffer.length = validate_layout(size, pitch, bytes)?;
        let mut offset = 0;
        let result = unsafe { (api.map)(kms.fd().as_raw_fd(), handle, &mut offset) };
        if result != 0 {
            return Err(KmsError::native(
                KmsErrorKind::Native,
                "DRM dumb-buffer mapping query failed",
                result.saturating_abs(),
            ));
        }
        let offset = std::ffi::c_long::try_from(offset).map_err(|_| {
            KmsError::new(
                KmsErrorKind::Unsupported,
                "DRM mmap offset exceeds platform range",
            )
        })?;
        let address = unsafe {
            ffi::mmap(
                std::ptr::null_mut(),
                buffer.length,
                3,
                1,
                kms.fd().as_raw_fd(),
                offset,
            )
        };
        if address as isize == -1 {
            return Err(KmsError::last_os_error(
                KmsErrorKind::Native,
                "DRM dumb-buffer mmap failed",
            ));
        }
        let Some(mapping) = NonNull::new(address.cast::<u8>()) else {
            unsafe { ffi::munmap(address, buffer.length) };
            return Err(KmsError::new(
                KmsErrorKind::Unsupported,
                "DRM returned a null mapping address",
            ));
        };
        buffer.mapping = Some(mapping);
        unsafe { std::ptr::write_bytes(mapping.as_ptr(), 0, buffer.length) };
        Ok(buffer)
    }
}

fn validate_layout(size: SizeI, pitch: u32, bytes: u64) -> Result<usize, KmsError> {
    let length = usize::try_from(bytes)
        .ok()
        .filter(|n| *n <= isize::MAX as usize);
    let rows = (pitch as u64).checked_mul(size.height.max(0) as u64);
    if size.width <= 0
        || size.height <= 0
        || u64::from(pitch) < size.width as u64 * 4
        || rows.is_none_or(|rows| rows > bytes)
        || length.is_none()
    {
        return Err(KmsError::new(
            KmsErrorKind::InvalidState,
            "DRM returned invalid dumb-buffer pitch/size",
        ));
    }
    Ok(length.unwrap())
}

impl Drop for DumbBuffer<'_> {
    fn drop(&mut self) {
        if let Some(mapping) = self.mapping {
            unsafe { ffi::munmap(mapping.as_ptr().cast(), self.length) };
        }
        unsafe { (self.api.destroy)(self.kms.fd().as_raw_fd(), self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn padded_cpu_pitch_is_respected_and_short_allocations_are_rejected() {
        let size = SizeI {
            width: 3,
            height: 2,
        };
        assert_eq!(validate_layout(size, 16, 32).unwrap(), 32);
        assert!(validate_layout(size, 8, 32).is_err());
        assert!(validate_layout(size, 16, 31).is_err());
        let mut bytes = [0xcc; 32];
        let mut source = [0; 24];
        source[12..16].copy_from_slice(&[10, 20, 30, 255]);
        gbm::write_rgba8_region(
            &mut bytes,
            16,
            size,
            super::super::DRM_FORMAT_XRGB8888,
            &source,
            RectI {
                x: 0,
                y: 1,
                width: 1,
                height: 1,
            },
        )
        .unwrap();
        assert_eq!(&bytes[16..20], &[30, 20, 10, 255]);
        assert!(bytes[..16].iter().chain(&bytes[20..]).all(|b| *b == 0xcc));
    }
}

#[derive(Clone, Copy)]
struct DumbApi {
    create: unsafe extern "C" fn(i32, u32, u32, u32, u32, *mut u32, *mut u32, *mut u64) -> i32,
    map: unsafe extern "C" fn(i32, u32, *mut u64) -> i32,
    destroy: unsafe extern "C" fn(i32, u32) -> i32,
}
fn dumb_api() -> Option<DumbApi> {
    static API: std::sync::OnceLock<Option<DumbApi>> = std::sync::OnceLock::new();
    *API.get_or_init(|| {
        let lookup = |name: &std::ffi::CStr| {
            NonNull::new(unsafe { ffi::dlsym(std::ptr::null_mut(), name.as_ptr()) })
        };
        // These exact signatures are declared by xf86drmMode.h. The linked libdrm
        // remains loaded for the entire lifetime of these function pointers.
        Some(unsafe {
            DumbApi {
                create: std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn(
                        i32,
                        u32,
                        u32,
                        u32,
                        u32,
                        *mut u32,
                        *mut u32,
                        *mut u64,
                    ) -> i32,
                >(lookup(c"drmModeCreateDumbBuffer")?.as_ptr()),
                map: std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn(i32, u32, *mut u64) -> i32,
                >(lookup(c"drmModeMapDumbBuffer")?.as_ptr()),
                destroy: std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn(i32, u32) -> i32,
                >(lookup(c"drmModeDestroyDumbBuffer")?.as_ptr()),
            }
        })
    })
}
