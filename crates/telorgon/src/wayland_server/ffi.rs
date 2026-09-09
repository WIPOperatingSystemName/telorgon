//! Exact C-layout declarations for the public `libwayland-server` ABI used by Telorgon.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_void};

#[repr(C)]
pub struct wl_display {
    _private: [u8; 0],
}

#[repr(C)]
pub struct wl_event_loop {
    _private: [u8; 0],
}

#[repr(C)]
pub struct wl_event_source {
    _private: [u8; 0],
}

#[repr(C)]
pub struct wl_client {
    _private: [u8; 0],
}

#[repr(C)]
pub struct wl_resource {
    _private: [u8; 0],
}

#[repr(C)]
pub struct wl_global {
    _private: [u8; 0],
}

#[repr(C)]
pub struct wl_list {
    pub prev: *mut wl_list,
    pub next: *mut wl_list,
}

#[repr(C)]
pub struct wl_listener {
    pub link: wl_list,
    pub notify: Option<unsafe extern "C" fn(*mut wl_listener, *mut c_void)>,
}

#[repr(C)]
pub struct wl_array {
    pub size: usize,
    pub alloc: usize,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct wl_message {
    pub name: *const c_char,
    pub signature: *const c_char,
    pub types: *const *const wl_interface,
}

#[repr(C)]
pub struct wl_interface {
    pub name: *const c_char,
    pub version: c_int,
    pub method_count: c_int,
    pub methods: *const wl_message,
    pub event_count: c_int,
    pub events: *const wl_message,
}

#[repr(C)]
pub union wl_argument {
    pub i: i32,
    pub u: u32,
    pub f: i32,
    pub s: *const c_char,
    pub o: *mut wl_resource,
    pub n: u32,
    pub a: *mut wl_array,
    pub h: i32,
}

pub type wl_global_bind_func_t =
    Option<unsafe extern "C" fn(client: *mut wl_client, data: *mut c_void, version: u32, id: u32)>;
pub type wl_resource_destroy_func_t = Option<unsafe extern "C" fn(resource: *mut wl_resource)>;
pub type wl_dispatcher_func_t = Option<
    unsafe extern "C" fn(
        implementation: *const c_void,
        target: *mut c_void,
        opcode: u32,
        message: *const wl_message,
        arguments: *mut wl_argument,
    ) -> c_int,
>;
pub type wl_event_loop_fd_func_t =
    Option<unsafe extern "C" fn(fd: c_int, mask: u32, data: *mut c_void) -> c_int>;
pub type wl_event_loop_timer_func_t = Option<unsafe extern "C" fn(data: *mut c_void) -> c_int>;

pub const WL_EVENT_READABLE: u32 = 0x01;
pub const WL_EVENT_WRITABLE: u32 = 0x02;
pub const WL_EVENT_HANGUP: u32 = 0x04;
pub const WL_EVENT_ERROR: u32 = 0x08;

#[link(name = "wayland-server")]
unsafe extern "C" {
    pub fn wl_display_create() -> *mut wl_display;
    pub fn wl_display_destroy(display: *mut wl_display);
    pub fn wl_display_destroy_clients(display: *mut wl_display);
    pub fn wl_display_set_global_filter(
        display: *mut wl_display,
        filter: Option<
            unsafe extern "C" fn(*const wl_client, *const wl_global, *mut c_void) -> bool,
        >,
        data: *mut c_void,
    );
    pub fn wl_global_get_interface(global: *const wl_global) -> *const wl_interface;
    pub fn wl_display_get_event_loop(display: *mut wl_display) -> *mut wl_event_loop;
    pub fn wl_display_add_socket(display: *mut wl_display, name: *const c_char) -> c_int;
    pub fn wl_display_add_socket_auto(display: *mut wl_display) -> *const c_char;
    pub fn wl_display_flush_clients(display: *mut wl_display);
    pub fn wl_display_get_serial(display: *mut wl_display) -> u32;
    pub fn wl_display_next_serial(display: *mut wl_display) -> u32;
    pub fn wl_display_terminate(display: *mut wl_display);

    pub fn wl_event_loop_dispatch(event_loop: *mut wl_event_loop, timeout: c_int) -> c_int;
    pub fn wl_event_loop_dispatch_idle(event_loop: *mut wl_event_loop);
    pub fn wl_event_loop_add_fd(
        event_loop: *mut wl_event_loop,
        fd: c_int,
        mask: u32,
        callback: wl_event_loop_fd_func_t,
        data: *mut c_void,
    ) -> *mut wl_event_source;
    pub fn wl_event_loop_add_timer(
        event_loop: *mut wl_event_loop,
        callback: wl_event_loop_timer_func_t,
        data: *mut c_void,
    ) -> *mut wl_event_source;
    pub fn wl_event_source_timer_update(source: *mut wl_event_source, ms_delay: c_int) -> c_int;
    pub fn wl_event_source_fd_update(source: *mut wl_event_source, mask: u32) -> c_int;
    pub fn wl_event_source_remove(source: *mut wl_event_source) -> c_int;

    pub fn wl_global_create(
        display: *mut wl_display,
        interface: *const wl_interface,
        version: c_int,
        data: *mut c_void,
        bind: wl_global_bind_func_t,
    ) -> *mut wl_global;
    pub fn wl_global_remove(global: *mut wl_global);
    pub fn wl_global_destroy(global: *mut wl_global);

    pub fn wl_resource_create(
        client: *mut wl_client,
        interface: *const wl_interface,
        version: c_int,
        id: u32,
    ) -> *mut wl_resource;
    pub fn wl_resource_destroy(resource: *mut wl_resource);
    pub fn wl_resource_set_dispatcher(
        resource: *mut wl_resource,
        dispatcher: wl_dispatcher_func_t,
        implementation: *const c_void,
        data: *mut c_void,
        destroy: wl_resource_destroy_func_t,
    );
    pub fn wl_resource_post_error(
        resource: *mut wl_resource,
        code: u32,
        message: *const c_char,
        ...
    );
    pub fn wl_resource_get_client(resource: *mut wl_resource) -> *mut wl_client;
    pub fn wl_resource_get_id(resource: *mut wl_resource) -> u32;
    pub fn wl_resource_get_version(resource: *mut wl_resource) -> c_int;
    pub fn wl_resource_get_user_data(resource: *mut wl_resource) -> *mut c_void;
    pub fn wl_resource_set_user_data(resource: *mut wl_resource, data: *mut c_void);
    pub fn wl_resource_post_event_array(
        resource: *mut wl_resource,
        opcode: u32,
        arguments: *mut wl_argument,
    );
    pub fn wl_resource_post_no_memory(resource: *mut wl_resource);

    pub fn wl_client_get_credentials(
        client: *mut wl_client,
        pid: *mut i32,
        uid: *mut u32,
        gid: *mut u32,
    );
    pub fn wl_client_create(display: *mut wl_display, fd: c_int) -> *mut wl_client;
    pub fn wl_client_destroy(client: *mut wl_client);
    pub fn wl_client_add_destroy_listener(client: *mut wl_client, listener: *mut wl_listener);
    pub fn wl_list_remove(link: *mut wl_list);
    pub fn wl_client_flush(client: *mut wl_client);
    pub fn wl_client_post_no_memory(client: *mut wl_client);
    pub fn wl_client_get_object(client: *mut wl_client, id: u32) -> *mut wl_resource;

    pub static wl_callback_interface: wl_interface;
    pub static wl_compositor_interface: wl_interface;
    pub static wl_surface_interface: wl_interface;
    pub static wl_region_interface: wl_interface;
    pub static wl_shm_interface: wl_interface;
    pub static wl_shm_pool_interface: wl_interface;
    pub static wl_buffer_interface: wl_interface;
    pub static wl_subcompositor_interface: wl_interface;
    pub static wl_subsurface_interface: wl_interface;
    pub static wl_output_interface: wl_interface;
    pub static wl_seat_interface: wl_interface;
    pub static wl_pointer_interface: wl_interface;
    pub static wl_keyboard_interface: wl_interface;
    pub static wl_touch_interface: wl_interface;
    pub static wl_data_device_manager_interface: wl_interface;
    pub static wl_data_device_interface: wl_interface;
    pub static wl_data_source_interface: wl_interface;
    pub static wl_data_offer_interface: wl_interface;
}
