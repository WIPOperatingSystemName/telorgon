#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_double, c_int, c_long, c_uint, c_ulong, c_void};

pub const CLOCK_MONOTONIC: c_int = 1;
pub const EFD_CLOEXEC: c_int = 0x80000;
pub const EFD_NONBLOCK: c_int = 0x800;
pub const DMA_BUF_SYNC_READ: u32 = 1 << 0;

/// Linux's `_IOWR(DMA_BUF_BASE, 2, struct dma_buf_export_sync_file)` on the supported
/// asm-generic ioctl ABI used by x86_64 and aarch64.
pub const DMA_BUF_IOCTL_EXPORT_SYNC_FILE: c_ulong = 0xC008_6202;

#[repr(C)]
pub struct dma_buf_export_sync_file {
    pub flags: u32,
    pub fd: c_int,
}

const _: () = assert!(std::mem::size_of::<dma_buf_export_sync_file>() == 8);

#[repr(C)]
pub struct timespec {
    pub tv_sec: c_long,
    pub tv_nsec: c_long,
}

macro_rules! opaque {
    ($($name:ident),* $(,)?) => {$(
        #[repr(C)]
        pub struct $name { _private: [u8; 0] }
    )*};
}

opaque!(
    libseat,
    libinput,
    libinput_event,
    libinput_device,
    udev,
    xkb_context,
    xkb_keymap,
    xkb_state,
);

#[repr(C)]
pub struct libseat_seat_listener {
    pub enable_seat: Option<unsafe extern "C" fn(*mut libseat, *mut c_void)>,
    pub disable_seat: Option<unsafe extern "C" fn(*mut libseat, *mut c_void)>,
}

#[repr(C)]
pub struct libinput_interface {
    pub open_restricted: Option<unsafe extern "C" fn(*const c_char, c_int, *mut c_void) -> c_int>,
    pub close_restricted: Option<unsafe extern "C" fn(c_int, *mut c_void)>,
}

#[repr(C)]
pub struct xkb_rule_names {
    pub rules: *const c_char,
    pub model: *const c_char,
    pub layout: *const c_char,
    pub variant: *const c_char,
    pub options: *const c_char,
}

#[link(name = "seat")]
unsafe extern "C" {
    pub fn libseat_open_seat(
        listener: *const libseat_seat_listener,
        data: *mut c_void,
    ) -> *mut libseat;
    pub fn libseat_close_seat(seat: *mut libseat) -> c_int;
    pub fn libseat_get_fd(seat: *mut libseat) -> c_int;
    pub fn libseat_dispatch(seat: *mut libseat, timeout: c_int) -> c_int;
    pub fn libseat_disable_seat(seat: *mut libseat) -> c_int;
    pub fn libseat_open_device(seat: *mut libseat, path: *const c_char, fd: *mut c_int) -> c_int;
    pub fn libseat_close_device(seat: *mut libseat, device_id: c_int) -> c_int;
    pub fn libseat_seat_name(seat: *mut libseat) -> *const c_char;
    pub fn libseat_switch_session(seat: *mut libseat, session: c_int) -> c_int;
}

#[link(name = "udev")]
unsafe extern "C" {
    pub fn udev_new() -> *mut udev;
    pub fn udev_unref(udev: *mut udev) -> *mut udev;
}

#[link(name = "input")]
unsafe extern "C" {
    pub fn libinput_udev_create_context(
        interface: *const libinput_interface,
        user_data: *mut c_void,
        udev: *mut udev,
    ) -> *mut libinput;
    pub fn libinput_udev_assign_seat(input: *mut libinput, seat_id: *const c_char) -> c_int;
    pub fn libinput_unref(input: *mut libinput) -> *mut libinput;
    pub fn libinput_get_fd(input: *mut libinput) -> c_int;
    pub fn libinput_dispatch(input: *mut libinput) -> c_int;
    pub fn libinput_suspend(input: *mut libinput);
    pub fn libinput_resume(input: *mut libinput) -> c_int;
    pub fn libinput_get_event(input: *mut libinput) -> *mut libinput_event;
    pub fn libinput_event_destroy(event: *mut libinput_event);
    pub fn libinput_event_get_type(event: *mut libinput_event) -> c_uint;
    pub fn libinput_event_get_device(event: *mut libinput_event) -> *mut libinput_device;
    pub fn libinput_event_pointer_get_time_usec(event: *mut libinput_event) -> u64;
    pub fn libinput_event_pointer_get_dx(event: *mut libinput_event) -> c_double;
    pub fn libinput_event_pointer_get_dy(event: *mut libinput_event) -> c_double;
    pub fn libinput_event_pointer_get_dx_unaccelerated(event: *mut libinput_event) -> c_double;
    pub fn libinput_event_pointer_get_dy_unaccelerated(event: *mut libinput_event) -> c_double;
    pub fn libinput_event_pointer_get_button(event: *mut libinput_event) -> c_uint;
    pub fn libinput_event_pointer_get_button_state(event: *mut libinput_event) -> c_uint;
    pub fn libinput_event_pointer_get_absolute_x_transformed(
        event: *mut libinput_event,
        width: c_uint,
    ) -> c_double;
    pub fn libinput_event_pointer_get_absolute_y_transformed(
        event: *mut libinput_event,
        height: c_uint,
    ) -> c_double;
    pub fn libinput_event_pointer_has_axis(event: *mut libinput_event, axis: c_uint) -> c_int;
    pub fn libinput_event_pointer_get_axis_value(
        event: *mut libinput_event,
        axis: c_uint,
    ) -> c_double;
    pub fn libinput_event_pointer_get_axis_value_discrete(
        event: *mut libinput_event,
        axis: c_uint,
    ) -> c_double;
    pub fn libinput_event_keyboard_get_time_usec(event: *mut libinput_event) -> u64;
    pub fn libinput_event_keyboard_get_key(event: *mut libinput_event) -> c_uint;
    pub fn libinput_event_keyboard_get_key_state(event: *mut libinput_event) -> c_uint;
    pub fn libinput_event_touch_get_time_usec(event: *mut libinput_event) -> u64;
    pub fn libinput_event_touch_get_seat_slot(event: *mut libinput_event) -> c_int;
    pub fn libinput_event_touch_get_x_transformed(
        event: *mut libinput_event,
        width: c_uint,
    ) -> c_double;
    pub fn libinput_event_touch_get_y_transformed(
        event: *mut libinput_event,
        height: c_uint,
    ) -> c_double;
}

#[link(name = "xkbcommon")]
unsafe extern "C" {
    pub fn xkb_context_new(flags: c_uint) -> *mut xkb_context;
    pub fn xkb_context_unref(context: *mut xkb_context);
    pub fn xkb_context_include_path_append(context: *mut xkb_context, path: *const c_char)
    -> c_int;
    pub fn xkb_keymap_new_from_names(
        context: *mut xkb_context,
        names: *const xkb_rule_names,
        flags: c_uint,
    ) -> *mut xkb_keymap;
    pub fn xkb_keymap_unref(keymap: *mut xkb_keymap);
    pub fn xkb_keymap_get_as_string(keymap: *mut xkb_keymap, format: c_uint) -> *mut c_char;
    pub fn xkb_state_new(keymap: *mut xkb_keymap) -> *mut xkb_state;
    pub fn xkb_state_unref(state: *mut xkb_state);
    pub fn xkb_state_update_key(state: *mut xkb_state, key: c_uint, direction: c_uint) -> c_uint;
    pub fn xkb_state_key_get_one_sym(state: *mut xkb_state, key: c_uint) -> c_uint;
    pub fn xkb_state_key_get_utf8(
        state: *mut xkb_state,
        key: c_uint,
        buffer: *mut c_char,
        size: usize,
    ) -> c_int;
    pub fn xkb_state_mod_name_is_active(
        state: *mut xkb_state,
        name: *const c_char,
        components: c_uint,
    ) -> c_int;
    pub fn xkb_state_serialize_mods(state: *mut xkb_state, components: c_uint) -> c_uint;
    pub fn xkb_state_serialize_layout(state: *mut xkb_state, components: c_uint) -> c_uint;
}

#[link(name = "c")]
unsafe extern "C" {
    pub fn clock_gettime(clock_id: c_int, time: *mut timespec) -> c_int;
    pub fn eventfd(initial_value: c_uint, flags: c_int) -> c_int;
    pub fn read(fd: c_int, buffer: *mut c_void, count: usize) -> isize;
    pub fn write(fd: c_int, buffer: *const c_void, count: usize) -> isize;
    pub fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    pub fn free(pointer: *mut c_void);
    pub fn memfd_create(name: *const c_char, flags: c_uint) -> c_int;
    pub fn ftruncate(fd: c_int, length: i64) -> c_int;
}
