use std::fmt;
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::foundation::{PointI, RectI, SizeI};
use crate::graphics::presentation::kms::ffi;
use crate::graphics::presentation::kms::{
    AtomicProperty, DRM_FORMAT_MOD_INVALID, GbmBuffer, KmsFramebufferId, KmsPropertyId,
};
use crate::graphics::presentation::kms::{KmsConnectorId, KmsCrtcId, KmsObjectProperties, KmsPlaneId};

const DRM_CLIENT_CAP_UNIVERSAL_PLANES: u64 = 2;
const DRM_CLIENT_CAP_ATOMIC: u64 = 3;
const DRM_CLIENT_CAP_CURSOR_PLANE_HOTSPOT: u64 = 6;

pub struct KmsDevice {
    fd: OwnedFd,
    page_flip_events: Box<AtomicU64>,
    cursor_plane_hotspot: bool,
}

impl KmsDevice {
    pub fn new(fd: OwnedFd) -> Result<Self, KmsError> {
        for capability in [DRM_CLIENT_CAP_UNIVERSAL_PLANES, DRM_CLIENT_CAP_ATOMIC] {
            let result = unsafe { ffi::drmSetClientCap(fd.as_raw_fd(), capability, 1) };
            if result != 0 {
                return Err(KmsError::native(
                    KmsErrorKind::Unsupported,
                    "DRM device does not support required atomic modesetting capabilities",
                    result,
                ));
            }
        }
        // Only para-virtualized drivers require this capability. Physical drivers commonly return
        // EOPNOTSUPP, so failure here is an expected indication that no hotspot properties exist.
        let cursor_plane_hotspot = unsafe {
            ffi::drmSetClientCap(fd.as_raw_fd(), DRM_CLIENT_CAP_CURSOR_PLANE_HOTSPOT, 1) == 0
        };
        Ok(Self {
            fd,
            page_flip_events: Box::new(AtomicU64::new(0)),
            cursor_plane_hotspot,
        })
    }

    pub fn fd(&self) -> &OwnedFd {
        &self.fd
    }

    pub fn atomic_request(&self) -> Result<AtomicRequest<'_>, KmsError> {
        AtomicRequest::new(self)
    }

    pub const fn cursor_plane_hotspot_capable(&self) -> bool {
        self.cursor_plane_hotspot
    }

    /// Dispatches readable DRM events and records completed nonblocking page flips.
    pub fn dispatch_events(&self) -> Result<(), KmsError> {
        let mut context = ffi::drmEventContext {
            version: ffi::DRM_EVENT_CONTEXT_VERSION,
            vblank_handler: None,
            page_flip_handler: Some(page_flip_handler),
            page_flip_handler2: None,
            sequence_handler: None,
        };
        let result = unsafe { ffi::drmHandleEvent(self.fd.as_raw_fd(), &mut context) };
        if result != 0 {
            Err(KmsError::native(
                KmsErrorKind::Native,
                "DRM event dispatch failed",
                result,
            ))
        } else {
            Ok(())
        }
    }

    /// Takes the number of page flips completed since the previous call.
    pub fn take_completed_page_flips(&self) -> u64 {
        self.page_flip_events.swap(0, Ordering::AcqRel)
    }

    pub fn capability(&self, capability: u64) -> Result<u64, KmsError> {
        let mut value = 0;
        if unsafe { ffi::drmGetCap(self.fd.as_raw_fd(), capability, &mut value) } != 0 {
            return Err(KmsError::last_os_error(
                KmsErrorKind::Native,
                "DRM capability query failed",
            ));
        }
        Ok(value)
    }

    pub fn cursor_size(&self) -> Result<SizeI, KmsError> {
        let mut width = 0_u64;
        let mut height = 0_u64;
        for (capability, value) in [
            (ffi::DRM_CAP_CURSOR_WIDTH, &mut width),
            (ffi::DRM_CAP_CURSOR_HEIGHT, &mut height),
        ] {
            let result = unsafe { ffi::drmGetCap(self.fd.as_raw_fd(), capability, value) };
            if result != 0 {
                return Err(KmsError::native(
                    KmsErrorKind::Unsupported,
                    "DRM cursor-size capability query failed",
                    result,
                ));
            }
        }
        if width == 0 || height == 0 || width > 1024 || height > 1024 {
            return Err(KmsError::new(
                KmsErrorKind::Unsupported,
                "DRM reported an unusable hardware-cursor extent",
            ));
        }
        Ok(SizeI {
            width: width as i32,
            height: height as i32,
        })
    }

    fn page_flip_user_data(&self) -> *mut std::ffi::c_void {
        std::ptr::from_ref(self.page_flip_events.as_ref())
            .cast_mut()
            .cast()
    }

    pub fn add_framebuffer<'device>(
        &'device self,
        buffer: &GbmBuffer<'_, '_>,
    ) -> Result<KmsFramebuffer<'device>, KmsError> {
        self.add_framebuffer_with_layout(buffer, false)
    }

    pub(crate) fn add_framebuffer_with_layout<'device>(
        &'device self,
        buffer: &GbmBuffer<'_, '_>,
        implicit: bool,
    ) -> Result<KmsFramebuffer<'device>, KmsError> {
        let size = buffer.size();
        let mut format = buffer.format();
        if implicit {
            format.modifier = DRM_FORMAT_MOD_INVALID;
        }
        let count = buffer.plane_count()?;
        let mut handles = [0_u32; 4];
        let mut pitches = [0_u32; 4];
        let mut offsets = [0_u32; 4];

        for index in 0..count {
            handles[index] =
                unsafe { ffi::gbm_bo_get_handle_for_plane(buffer.raw(), index as i32) };
            pitches[index] =
                unsafe { ffi::gbm_bo_get_stride_for_plane(buffer.raw(), index as i32) };
            offsets[index] = unsafe { ffi::gbm_bo_get_offset(buffer.raw(), index as i32) };
            if handles[index] == 0 || pitches[index] == 0 {
                return Err(KmsError::new(
                    KmsErrorKind::InvalidState,
                    "GBM returned invalid KMS plane metadata",
                ));
            }
        }
        self.add_framebuffer_layout(size, format, handles, pitches, offsets)
    }

    pub(crate) fn add_framebuffer_layout<'device>(
        &'device self,
        size: SizeI,
        format: super::ScanoutFormat,
        handles: [u32; 4],
        pitches: [u32; 4],
        offsets: [u32; 4],
    ) -> Result<KmsFramebuffer<'device>, KmsError> {
        let mut id = 0;
        let result = if format.modifier == DRM_FORMAT_MOD_INVALID {
            unsafe {
                ffi::drmModeAddFB2(
                    self.fd.as_raw_fd(),
                    size.width as u32,
                    size.height as u32,
                    format.fourcc,
                    handles.as_ptr(),
                    pitches.as_ptr(),
                    offsets.as_ptr(),
                    &mut id,
                    0,
                )
            }
        } else {
            let modifiers = framebuffer_modifiers(handles, format.modifier);
            unsafe {
                ffi::drmModeAddFB2WithModifiers(
                    self.fd.as_raw_fd(),
                    size.width as u32,
                    size.height as u32,
                    format.fourcc,
                    handles.as_ptr(),
                    pitches.as_ptr(),
                    offsets.as_ptr(),
                    modifiers.as_ptr(),
                    &mut id,
                    ffi::DRM_MODE_FB_MODIFIERS,
                )
            }
        };
        if result != 0 {
            return Err(KmsError::last_os_error(
                KmsErrorKind::Native,
                "DRM framebuffer creation failed",
            ));
        }
        Ok(KmsFramebuffer {
            device: self,
            id: KmsFramebufferId::from_raw(id).expect("successful AddFB2 returns a nonzero id"),
        })
    }

    pub fn create_property_blob<'device>(
        &'device self,
        bytes: &[u8],
    ) -> Result<PropertyBlob<'device>, KmsError> {
        if bytes.is_empty() {
            return Err(KmsError::new(
                KmsErrorKind::InvalidState,
                "DRM property blobs must not be empty",
            ));
        }
        let mut id = 0;
        let result = unsafe {
            ffi::drmModeCreatePropertyBlob(
                self.fd.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                &mut id,
            )
        };
        if result != 0 || id == 0 {
            Err(KmsError::native(
                KmsErrorKind::Native,
                "DRM property blob creation failed",
                result,
            ))
        } else {
            Ok(PropertyBlob { device: self, id })
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn primary_modeset_request(
        &self,
        connector: KmsConnectorId,
        connector_properties: &KmsObjectProperties,
        crtc: KmsCrtcId,
        crtc_properties: &KmsObjectProperties,
        plane: KmsPlaneId,
        plane_properties: &KmsObjectProperties,
        mode_blob: u32,
        framebuffer: KmsFramebufferId,
        width: u32,
        height: u32,
    ) -> Result<AtomicRequest<'_>, KmsError> {
        if mode_blob == 0
            || width == 0
            || height == 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
        {
            return Err(KmsError::new(
                KmsErrorKind::InvalidState,
                "atomic modeset requires a mode blob and positive extent",
            ));
        }
        let mut request = self.atomic_request()?;
        add_named(
            &mut request,
            connector.get(),
            connector_properties,
            "CRTC_ID",
            u64::from(crtc.get()),
        )?;
        add_named(
            &mut request,
            crtc.get(),
            crtc_properties,
            "MODE_ID",
            u64::from(mode_blob),
        )?;
        add_named(&mut request, crtc.get(), crtc_properties, "ACTIVE", 1)?;
        request.set_plane(
            plane,
            plane_properties,
            crtc,
            framebuffer,
            RectI {
                x: 0,
                y: 0,
                width: width as i32,
                height: height as i32,
            },
            RectI {
                x: 0,
                y: 0,
                width: width as i32,
                height: height as i32,
            },
        )?;
        Ok(request)
    }
}

fn add_named(
    request: &mut AtomicRequest<'_>,
    object: u32,
    properties: &KmsObjectProperties,
    name: &'static str,
    value: u64,
) -> Result<(), KmsError> {
    let property = properties.named(name).ok_or_else(|| {
        KmsError::new(
            KmsErrorKind::Unsupported,
            format!("DRM object has no required atomic property {name}"),
        )
    })?;
    request.add(object, property.id, value)
}

pub struct KmsFramebuffer<'device> {
    device: &'device KmsDevice,
    id: KmsFramebufferId,
}

impl KmsFramebuffer<'_> {
    pub const fn id(&self) -> KmsFramebufferId {
        self.id
    }
}

impl Drop for KmsFramebuffer<'_> {
    fn drop(&mut self) {
        let result = unsafe { ffi::drmModeRmFB(self.device.fd.as_raw_fd(), self.id.get()) };
        if result != 0 {
            // Device/seat loss during rollback must not panic while unwinding.
            eprintln!(
                "telorgon-kms: {}",
                KmsError::last_os_error(KmsErrorKind::Native, "DRM framebuffer removal failed")
            );
        }
    }
}

pub struct PropertyBlob<'device> {
    device: &'device KmsDevice,
    id: u32,
}

impl PropertyBlob<'_> {
    pub const fn id(&self) -> u32 {
        self.id
    }
}

impl Drop for PropertyBlob<'_> {
    fn drop(&mut self) {
        let result =
            unsafe { ffi::drmModeDestroyPropertyBlob(self.device.fd.as_raw_fd(), self.id) };
        debug_assert_eq!(result, 0, "DRM property blob removal failed");
    }
}

pub struct AtomicRequest<'device> {
    device: &'device KmsDevice,
    raw: NonNull<ffi::drmModeAtomicReq>,
    properties: Vec<AtomicProperty>,
}

impl<'device> AtomicRequest<'device> {
    fn new(device: &'device KmsDevice) -> Result<Self, KmsError> {
        let raw = NonNull::new(unsafe { ffi::drmModeAtomicAlloc() }).ok_or_else(|| {
            KmsError::new(
                KmsErrorKind::Allocation,
                "DRM atomic request allocation failed",
            )
        })?;
        Ok(Self {
            device,
            raw,
            properties: Vec::new(),
        })
    }

    pub fn add(
        &mut self,
        object: u32,
        property: KmsPropertyId,
        value: u64,
    ) -> Result<(), KmsError> {
        if object == 0 {
            return Err(KmsError::new(
                KmsErrorKind::InvalidState,
                "DRM atomic property object must be nonzero",
            ));
        }
        let result = unsafe {
            ffi::drmModeAtomicAddProperty(self.raw.as_ptr(), object, property.get(), value)
        };
        if result < 0 {
            return Err(KmsError::native(
                KmsErrorKind::Native,
                "DRM rejected an atomic property",
                result,
            ));
        }
        self.properties.push(AtomicProperty {
            object,
            property,
            value,
        });
        Ok(())
    }

    pub fn set_plane(
        &mut self,
        plane: KmsPlaneId,
        properties: &KmsObjectProperties,
        crtc: KmsCrtcId,
        framebuffer: KmsFramebufferId,
        source: RectI,
        destination: RectI,
    ) -> Result<(), KmsError> {
        if source.x < 0
            || source.y < 0
            || source.width <= 0
            || source.height <= 0
            || destination.width <= 0
            || destination.height <= 0
        {
            return Err(KmsError::new(
                KmsErrorKind::InvalidState,
                "atomic plane rectangles are invalid",
            ));
        }
        let source_x = u64::try_from(source.x)
            .ok()
            .and_then(|value| value.checked_shl(16))
            .ok_or_else(|| KmsError::new(KmsErrorKind::InvalidState, "plane source overflow"))?;
        let source_y = u64::try_from(source.y)
            .ok()
            .and_then(|value| value.checked_shl(16))
            .ok_or_else(|| KmsError::new(KmsErrorKind::InvalidState, "plane source overflow"))?;
        let source_width = u64::try_from(source.width)
            .ok()
            .and_then(|value| value.checked_shl(16))
            .ok_or_else(|| KmsError::new(KmsErrorKind::InvalidState, "plane source overflow"))?;
        let source_height = u64::try_from(source.height)
            .ok()
            .and_then(|value| value.checked_shl(16))
            .ok_or_else(|| KmsError::new(KmsErrorKind::InvalidState, "plane source overflow"))?;
        for (name, value) in [
            ("FB_ID", u64::from(framebuffer.get())),
            ("CRTC_ID", u64::from(crtc.get())),
            ("SRC_X", source_x),
            ("SRC_Y", source_y),
            ("SRC_W", source_width),
            ("SRC_H", source_height),
            ("CRTC_X", signed_property_value(destination.x)),
            ("CRTC_Y", signed_property_value(destination.y)),
            ("CRTC_W", destination.width as u64),
            ("CRTC_H", destination.height as u64),
        ] {
            add_named(self, plane.get(), properties, name, value)?;
        }
        Ok(())
    }

    pub fn disable_plane(
        &mut self,
        plane: KmsPlaneId,
        properties: &KmsObjectProperties,
    ) -> Result<(), KmsError> {
        add_named(self, plane.get(), properties, "FB_ID", 0)?;
        add_named(self, plane.get(), properties, "CRTC_ID", 0)
    }

    pub fn include_active_crtc(
        &mut self,
        crtc: KmsCrtcId,
        properties: &KmsObjectProperties,
    ) -> Result<(), KmsError> {
        add_named(self, crtc.get(), properties, "ACTIVE", 1)
    }

    pub fn set_cursor_hotspot(
        &mut self,
        plane: KmsPlaneId,
        properties: &KmsObjectProperties,
        hotspot: PointI,
    ) -> Result<(), KmsError> {
        add_named(
            self,
            plane.get(),
            properties,
            "HOTSPOT_X",
            signed_property_value(hotspot.x),
        )?;
        add_named(
            self,
            plane.get(),
            properties,
            "HOTSPOT_Y",
            signed_property_value(hotspot.y),
        )
    }

    pub fn properties(&self) -> &[AtomicProperty] {
        &self.properties
    }

    pub fn test(&self, allow_modeset: bool) -> Result<(), KmsError> {
        self.commit_inner(
            ffi::DRM_MODE_ATOMIC_TEST_ONLY
                | if allow_modeset {
                    ffi::DRM_MODE_ATOMIC_ALLOW_MODESET
                } else {
                    0
                },
        )
    }

    pub fn commit(self, allow_modeset: bool, page_flip_event: bool) -> Result<(), KmsError> {
        let flags = (if allow_modeset {
            ffi::DRM_MODE_ATOMIC_ALLOW_MODESET
        } else {
            0
        }) | if page_flip_event {
            ffi::DRM_MODE_PAGE_FLIP_EVENT | ffi::DRM_MODE_ATOMIC_NONBLOCK
        } else {
            0
        };
        self.commit_inner(flags)
    }

    fn commit_inner(&self, flags: u32) -> Result<(), KmsError> {
        if self.properties.is_empty() {
            return Err(KmsError::new(
                KmsErrorKind::InvalidState,
                "DRM atomic request contains no properties",
            ));
        }
        let result = unsafe {
            ffi::drmModeAtomicCommit(
                self.device.fd.as_raw_fd(),
                self.raw.as_ptr(),
                flags,
                if flags & ffi::DRM_MODE_PAGE_FLIP_EVENT != 0 {
                    self.device.page_flip_user_data()
                } else {
                    std::ptr::null_mut()
                },
            )
        };
        if result != 0 {
            Err(KmsError::native(
                KmsErrorKind::Native,
                "DRM atomic commit failed",
                result,
            ))
        } else {
            Ok(())
        }
    }
}

const fn signed_property_value(value: i32) -> u64 {
    (value as i64) as u64
}

unsafe extern "C" fn page_flip_handler(
    _fd: i32,
    _sequence: u32,
    _tv_sec: u32,
    _tv_usec: u32,
    user_data: *mut std::ffi::c_void,
) {
    let Some(events) = NonNull::new(user_data.cast::<AtomicU64>()) else {
        return;
    };
    unsafe { events.as_ref() }.fetch_add(1, Ordering::Release);
}

impl Drop for AtomicRequest<'_> {
    fn drop(&mut self) {
        unsafe { ffi::drmModeAtomicFree(self.raw.as_ptr()) };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KmsErrorKind {
    Unsupported,
    Allocation,
    InvalidState,
    Native,
}

#[derive(Debug)]
pub struct KmsError {
    kind: KmsErrorKind,
    context: String,
    native_code: Option<i32>,
}

impl KmsError {
    pub fn new(kind: KmsErrorKind, context: impl Into<String>) -> Self {
        Self {
            kind,
            context: context.into(),
            native_code: None,
        }
    }

    pub fn native(kind: KmsErrorKind, context: impl Into<String>, native_code: i32) -> Self {
        Self {
            kind,
            context: context.into(),
            native_code: if native_code == -1 {
                std::io::Error::last_os_error().raw_os_error()
            } else {
                Some(native_code)
            },
        }
    }

    pub(crate) fn annotate(&mut self, detail: impl AsRef<str>) {
        self.context.push_str(": ");
        self.context.push_str(detail.as_ref());
    }

    pub fn last_os_error(kind: KmsErrorKind, context: impl Into<String>) -> Self {
        let error = std::io::Error::last_os_error();
        Self {
            kind,
            context: context.into(),
            native_code: error.raw_os_error(),
        }
    }

    /// Stop negotiation on resource exhaustion, lost devices, or seat/permission errors.
    pub fn retryable(&self) -> bool {
        !matches!(
            self.native_code.map(i32::saturating_abs),
            Some(1 | 5 | 6 | 9 | 12 | 13 | 16 | 19 | 23 | 24 | 28)
        ) && self.kind != KmsErrorKind::InvalidState
    }

    pub const fn kind(&self) -> KmsErrorKind {
        self.kind
    }

    pub const fn native_code(&self) -> Option<i32> {
        self.native_code
    }
}

impl fmt::Display for KmsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.context)?;
        if let Some(code) = self.native_code {
            write!(
                formatter,
                " (native={code}: {})",
                std::io::Error::from_raw_os_error(code.saturating_abs())
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for KmsError {}

// AddFB2 requires zero modifiers in unused slots, even for non-linear layouts.
// Active planes have nonzero handles; multiple planes may share the same handle.
fn framebuffer_modifiers(handles: [u32; 4], modifier: u64) -> [u64; 4] {
    handles.map(|handle| if handle == 0 { 0 } else { modifier })
}

#[cfg(test)]
mod tests {
    use super::framebuffer_modifiers;

    #[test]
    fn tiled_rgb_framebuffer_leaves_unused_modifiers_zero() {
        // Layout observed in the Quadro RTX 4000 startup failure. The kernel
        // rejects nonzero modifiers in slots 1..4 for this single-plane buffer.
        let modifier = 0x0300_0000_0060_6014;
        assert_eq!(
            framebuffer_modifiers([7, 0, 0, 0], modifier),
            [modifier, 0, 0, 0]
        );
    }

    #[test]
    fn framebuffer_modifier_is_retained_for_every_active_plane() {
        let modifier = 0x0100_0000_0000_0002;
        assert_eq!(
            framebuffer_modifiers([7, 7, 0, 0], modifier),
            [modifier, modifier, 0, 0]
        );
        assert_eq!(
            framebuffer_modifiers([7, 8, 9, 0], modifier),
            [modifier, modifier, modifier, 0]
        );
        assert_eq!(
            framebuffer_modifiers([7, 8, 9, 10], modifier),
            [modifier; 4]
        );
    }

    #[test]
    fn linear_framebuffer_modifiers_remain_zero() {
        assert_eq!(framebuffer_modifiers([7, 0, 0, 0], 0), [0; 4]);
    }
}
