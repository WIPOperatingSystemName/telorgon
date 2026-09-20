//! Direct Linux desktop-session, input, and keyboard boundaries for the Telorgon compositor.
//!
//! This crate deliberately wraps the original C projects (`libseat`, `libinput`, `libudev`, and
//! `libxkbcommon`) rather than adopting a Rust compositor framework. It starts no threads and owns
//! no event loop; the compositor dispatches each source from its Wayland loop.

mod model;

#[cfg(target_os = "linux")]
pub mod ffi;
#[cfg(target_os = "linux")]
mod input;
#[cfg(target_os = "linux")]
mod keymap;
#[cfg(target_os = "linux")]
mod session;

#[cfg(target_os = "linux")]
pub use input::LibInputContext;
#[cfg(target_os = "linux")]
pub use keymap::{KeyDirection, KeymapFile, XkbKeyboard, XkbModifiers};
pub use model::{
    DeviceIdentity, LinuxInputEvent, LinuxInputEventKind, LinuxPlatformError,
    LinuxPlatformErrorKind, LinuxSessionConfig,
};
#[cfg(target_os = "linux")]
pub use session::{LinuxSeat, SeatDevice, SeatState};

pub const NATIVE_LINUX_PLATFORM_AVAILABLE: bool = cfg!(target_os = "linux");

#[cfg(target_os = "linux")]
pub(crate) fn monotonic_time_microseconds() -> Option<u64> {
    let mut time = std::mem::MaybeUninit::<ffi::timespec>::uninit();
    if unsafe { ffi::clock_gettime(ffi::CLOCK_MONOTONIC, time.as_mut_ptr()) } != 0 {
        return None;
    }
    let time = unsafe { time.assume_init() };
    let seconds = u64::try_from(time.tv_sec).ok()?;
    let nanoseconds = u64::try_from(time.tv_nsec).ok()?;
    seconds
        .checked_mul(1_000_000)?
        .checked_add(nanoseconds / 1_000)
}
