#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_uint, c_void};

macro_rules! opaque {
    ($($name:ident),* $(,)?) => {$(
        #[repr(C)]
        pub struct $name { _private: [u8; 0] }
    )*};
}

opaque!(drmModeAtomicReq, gbm_device, gbm_bo);

pub type drmEventHandler = Option<
    unsafe extern "C" fn(
        fd: c_int,
        sequence: c_uint,
        tv_sec: c_uint,
        tv_usec: c_uint,
        user_data: *mut c_void,
    ),
>;
pub type drmPageFlipHandler2 = Option<
    unsafe extern "C" fn(
        fd: c_int,
        sequence: c_uint,
        tv_sec: c_uint,
        tv_usec: c_uint,
        crtc_id: c_uint,
        user_data: *mut c_void,
    ),
>;
pub type drmSequenceHandler =
    Option<unsafe extern "C" fn(fd: c_int, sequence: u64, ns: u64, user_data: u64)>;

#[repr(C)]
pub struct drmEventContext {
    pub version: c_int,
    pub vblank_handler: drmEventHandler,
    pub page_flip_handler: drmEventHandler,
    pub page_flip_handler2: drmPageFlipHandler2,
    pub sequence_handler: drmSequenceHandler,
}

#[repr(C)]
pub struct drmModeRes {
    pub count_fbs: c_int,
    pub fbs: *mut c_uint,
    pub count_crtcs: c_int,
    pub crtcs: *mut c_uint,
    pub count_connectors: c_int,
    pub connectors: *mut c_uint,
    pub count_encoders: c_int,
    pub encoders: *mut c_uint,
    pub min_width: c_uint,
    pub max_width: c_uint,
    pub min_height: c_uint,
    pub max_height: c_uint,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct drmModeModeInfo {
    pub clock: c_uint,
    pub hdisplay: u16,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub hskew: u16,
    pub vdisplay: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub vscan: u16,
    pub vrefresh: c_uint,
    pub flags: c_uint,
    pub type_: c_uint,
    pub name: [c_char; 32],
}

#[repr(C)]
pub struct drmModeConnector {
    pub connector_id: c_uint,
    pub encoder_id: c_uint,
    pub connector_type: c_uint,
    pub connector_type_id: c_uint,
    pub connection: c_uint,
    pub mm_width: c_uint,
    pub mm_height: c_uint,
    pub subpixel: c_uint,
    pub count_modes: c_int,
    pub modes: *mut drmModeModeInfo,
    pub count_props: c_int,
    pub props: *mut c_uint,
    pub prop_values: *mut u64,
    pub count_encoders: c_int,
    pub encoders: *mut c_uint,
}

#[repr(C)]
pub struct drmModeEncoder {
    pub encoder_id: c_uint,
    pub encoder_type: c_uint,
    pub crtc_id: c_uint,
    pub possible_crtcs: c_uint,
    pub possible_clones: c_uint,
}

#[repr(C)]
pub struct drmModePlaneRes {
    pub count_planes: c_uint,
    pub planes: *mut c_uint,
}

#[repr(C)]
pub struct drmModePlane {
    pub count_formats: c_uint,
    pub formats: *mut c_uint,
    pub plane_id: c_uint,
    pub crtc_id: c_uint,
    pub fb_id: c_uint,
    pub crtc_x: c_uint,
    pub crtc_y: c_uint,
    pub x: c_uint,
    pub y: c_uint,
    pub possible_crtcs: c_uint,
    pub gamma_size: c_uint,
}

#[repr(C)]
pub struct drmModeObjectProperties {
    pub count_props: c_uint,
    pub props: *mut c_uint,
    pub prop_values: *mut u64,
}

#[repr(C)]
pub struct drmModePropertyEnum {
    pub value: u64,
    pub name: [c_char; 32],
}

#[repr(C)]
pub struct drmModePropertyRes {
    pub prop_id: c_uint,
    pub flags: c_uint,
    pub name: [c_char; 32],
    pub count_values: c_int,
    pub values: *mut u64,
    pub count_enums: c_int,
    pub enums: *mut drmModePropertyEnum,
    pub count_blobs: c_int,
    pub blob_ids: *mut c_uint,
}

pub const DRM_MODE_ATOMIC_TEST_ONLY: u32 = 0x0100;
pub const DRM_MODE_ATOMIC_NONBLOCK: u32 = 0x0200;
pub const DRM_MODE_ATOMIC_ALLOW_MODESET: u32 = 0x0400;
pub const DRM_MODE_PAGE_FLIP_EVENT: u32 = 0x01;
pub const DRM_MODE_FB_MODIFIERS: u32 = 0x02;
pub const DRM_EVENT_CONTEXT_VERSION: c_int = 4;
#[repr(C)]
pub struct drmModePropertyBlobRes {
    pub id: u32,
    pub length: u32,
    pub data: *mut c_void,
}

pub const DRM_CAP_DUMB_BUFFER: u64 = 1;
pub const DRM_CAP_ADDFB2_MODIFIERS: u64 = 0x10;
pub const DRM_CAP_CURSOR_WIDTH: u64 = 0x8;
pub const DRM_CAP_CURSOR_HEIGHT: u64 = 0x9;

pub const GBM_BO_USE_SCANOUT: u32 = 1 << 0;
pub const GBM_BO_USE_CURSOR: u32 = 1 << 1;
pub const GBM_BO_USE_RENDERING: u32 = 1 << 2;
pub const GBM_BO_USE_WRITE: u32 = 1 << 3;
pub const GBM_BO_USE_LINEAR: u32 = 1 << 4;
pub const GBM_BO_TRANSFER_WRITE: u32 = 1 << 1;
pub const GBM_BO_TRANSFER_READ_WRITE: u32 = (1 << 0) | GBM_BO_TRANSFER_WRITE;

#[link(name = "drm")]
unsafe extern "C" {
    pub fn drmModeGetPropertyBlob(fd: c_int, id: u32) -> *mut drmModePropertyBlobRes;
    pub fn drmModeFreePropertyBlob(blob: *mut drmModePropertyBlobRes);
    pub fn drmGetCap(fd: c_int, capability: u64, value: *mut u64) -> c_int;
    pub fn drmHandleEvent(fd: c_int, context: *mut drmEventContext) -> c_int;
    pub fn drmModeGetResources(fd: c_int) -> *mut drmModeRes;
    pub fn drmModeFreeResources(resources: *mut drmModeRes);
    pub fn drmModeGetConnector(fd: c_int, connector_id: c_uint) -> *mut drmModeConnector;
    pub fn drmModeFreeConnector(connector: *mut drmModeConnector);
    pub fn drmModeGetEncoder(fd: c_int, encoder_id: c_uint) -> *mut drmModeEncoder;
    pub fn drmModeFreeEncoder(encoder: *mut drmModeEncoder);
    pub fn drmModeGetPlaneResources(fd: c_int) -> *mut drmModePlaneRes;
    pub fn drmModeFreePlaneResources(resources: *mut drmModePlaneRes);
    pub fn drmModeGetPlane(fd: c_int, plane_id: c_uint) -> *mut drmModePlane;
    pub fn drmModeFreePlane(plane: *mut drmModePlane);
    pub fn drmModeObjectGetProperties(
        fd: c_int,
        object_id: c_uint,
        object_type: c_uint,
    ) -> *mut drmModeObjectProperties;
    pub fn drmModeFreeObjectProperties(properties: *mut drmModeObjectProperties);
    pub fn drmModeGetProperty(fd: c_int, property_id: c_uint) -> *mut drmModePropertyRes;
    pub fn drmModeFreeProperty(property: *mut drmModePropertyRes);
    pub fn drmSetClientCap(fd: c_int, capability: u64, value: u64) -> c_int;
    pub fn drmModeAtomicAlloc() -> *mut drmModeAtomicReq;
    pub fn drmModeAtomicFree(request: *mut drmModeAtomicReq);
    pub fn drmModeAtomicAddProperty(
        request: *mut drmModeAtomicReq,
        object_id: c_uint,
        property_id: c_uint,
        value: u64,
    ) -> c_int;
    pub fn drmModeAtomicCommit(
        fd: c_int,
        request: *mut drmModeAtomicReq,
        flags: c_uint,
        user_data: *mut c_void,
    ) -> c_int;
    pub fn drmModeCreatePropertyBlob(
        fd: c_int,
        data: *const c_void,
        length: usize,
        id: *mut c_uint,
    ) -> c_int;
    pub fn drmModeDestroyPropertyBlob(fd: c_int, id: c_uint) -> c_int;
    pub fn drmModeAddFB2WithModifiers(
        fd: c_int,
        width: c_uint,
        height: c_uint,
        pixel_format: c_uint,
        bo_handles: *const c_uint,
        pitches: *const c_uint,
        offsets: *const c_uint,
        modifier: *const u64,
        buffer_id: *mut c_uint,
        flags: c_uint,
    ) -> c_int;
    pub fn drmModeAddFB2(
        fd: c_int,
        width: c_uint,
        height: c_uint,
        pixel_format: c_uint,
        bo_handles: *const c_uint,
        pitches: *const c_uint,
        offsets: *const c_uint,
        buffer_id: *mut c_uint,
        flags: c_uint,
    ) -> c_int;
    pub fn drmModeRmFB(fd: c_int, buffer_id: c_uint) -> c_int;
    pub fn drmPrimeFDToHandle(fd: c_int, prime_fd: c_int, handle: *mut c_uint) -> c_int;
    pub fn drmIoctl(fd: c_int, request: c_ulong, argument: *mut c_void) -> c_int;
}

pub type c_ulong = usize;

#[link(name = "gbm")]
unsafe extern "C" {
    pub fn gbm_create_device(fd: c_int) -> *mut gbm_device;
    pub fn gbm_device_destroy(device: *mut gbm_device);
    pub fn gbm_bo_create(
        device: *mut gbm_device,
        width: c_uint,
        height: c_uint,
        format: c_uint,
        flags: c_uint,
    ) -> *mut gbm_bo;
    pub fn gbm_bo_destroy(buffer: *mut gbm_bo);
    pub fn gbm_bo_get_width(buffer: *mut gbm_bo) -> c_uint;
    pub fn gbm_bo_get_height(buffer: *mut gbm_bo) -> c_uint;
    pub fn gbm_bo_get_format(buffer: *mut gbm_bo) -> c_uint;
    pub fn gbm_bo_get_modifier(buffer: *mut gbm_bo) -> u64;
    pub fn gbm_bo_get_plane_count(buffer: *mut gbm_bo) -> c_int;
    pub fn gbm_bo_get_fd_for_plane(buffer: *mut gbm_bo, plane: c_int) -> c_int;
    pub fn gbm_bo_get_stride_for_plane(buffer: *mut gbm_bo, plane: c_int) -> c_uint;
    pub fn gbm_bo_get_offset(buffer: *mut gbm_bo, plane: c_int) -> c_uint;
    pub fn gbm_bo_get_handle_for_plane(buffer: *mut gbm_bo, plane: c_int) -> c_uint;
    pub fn gbm_bo_map(
        buffer: *mut gbm_bo,
        x: c_uint,
        y: c_uint,
        width: c_uint,
        height: c_uint,
        flags: c_uint,
        stride: *mut c_uint,
        map_data: *mut *mut c_void,
    ) -> *mut c_void;
    pub fn gbm_bo_unmap(buffer: *mut gbm_bo, map_data: *mut c_void);
    pub fn gbm_device_get_backend_name(device: *mut gbm_device) -> *const c_char;
}

#[link(name = "dl")]
unsafe extern "C" {
    pub fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
unsafe extern "C" {
    pub fn __errno_location() -> *mut c_int;
    pub fn mmap(
        addr: *mut c_void,
        length: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: std::ffi::c_long,
    ) -> *mut c_void;
    pub fn munmap(addr: *mut c_void, length: usize) -> c_int;
}
