use crate::integrations::wayland::server::ffi::{wl_interface, wl_message};
use crate::integrations::wayland::server::{InterfaceSchema, ProtocolSchema};

// Private wrappers apply Sync only to the generated immutable graph, never to arbitrary FFI
// values. Every pointer targets another static in this module or a NUL-terminated string literal.
// libwayland only reads these descriptors. No allocation, mutation, or destruction is involved.
struct Interfaces<const N: usize>([wl_interface; N]);
struct Messages<const N: usize>([wl_message; N]);
struct Types<const N: usize>([*const wl_interface; N]);
unsafe impl<const N: usize> Sync for Interfaces<N> {}
unsafe impl<const N: usize> Sync for Messages<N> {}
unsafe impl<const N: usize> Sync for Types<N> {}

include!(concat!(env!("OUT_DIR"), "/wayland_descriptors.rs"));

/// Access to the immutable desktop protocol descriptors compiled into the library.
/// All returned metadata, C strings, messages, and interface pointers live for the process lifetime.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeProtocol;

impl NativeProtocol {
    pub const fn desktop() -> Self {
        Self
    }

    pub fn schema(&self) -> &'static ProtocolSchema {
        &SCHEMA
    }

    pub fn interface(&self, name: &str) -> Option<&'static wl_interface> {
        SCHEMA
            .interfaces
            .iter()
            .position(|interface| interface.name == name)
            .map(|index| &INTERFACES.0[index])
    }

    pub fn interface_schema(&self, name: &str) -> Option<&'static InterfaceSchema> {
        SCHEMA.interface(name)
    }
}
