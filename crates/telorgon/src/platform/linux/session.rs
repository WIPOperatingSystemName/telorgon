use std::cell::Cell;
use std::ffi::{CStr, CString, c_void};
use std::marker::PhantomData;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::ptr::NonNull;
use std::rc::Rc;

use crate::platform::linux::ffi;
use crate::platform::linux::{LinuxPlatformError, LinuxPlatformErrorKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatState {
    Disabled,
    Enabled,
}

struct CallbackState {
    enabled: Cell<bool>,
    defer_disable: bool,
    disable_pending: Cell<bool>,
}

unsafe extern "C" fn enable_seat(_seat: *mut ffi::libseat, data: *mut c_void) {
    let state = unsafe { &*(data.cast::<CallbackState>()) };
    state.enabled.set(true);
}

unsafe extern "C" fn disable_seat(seat: *mut ffi::libseat, data: *mut c_void) {
    let state = unsafe { &*(data.cast::<CallbackState>()) };
    state.enabled.set(false);
    if state.defer_disable {
        state.disable_pending.set(true);
    } else {
        unsafe { ffi::libseat_disable_seat(seat) };
    }
}

static LISTENER: ffi::libseat_seat_listener = ffi::libseat_seat_listener {
    enable_seat: Some(enable_seat),
    disable_seat: Some(disable_seat),
};

pub struct LinuxSeat {
    raw: NonNull<ffi::libseat>,
    callbacks: Box<CallbackState>,
    marker: PhantomData<Rc<()>>,
}

impl LinuxSeat {
    pub fn open() -> Result<Self, LinuxPlatformError> {
        Self::open_inner(false)
    }

    /// The caller must suspend input/device use and promptly acknowledge each
    /// pending disable after dispatch. Ordinary `open` retains automatic acknowledgement.
    pub fn open_with_deferred_disable() -> Result<Self, LinuxPlatformError> {
        Self::open_inner(true)
    }

    fn open_inner(defer_disable: bool) -> Result<Self, LinuxPlatformError> {
        let mut callbacks = Box::new(CallbackState {
            enabled: Cell::new(false),
            defer_disable,
            disable_pending: Cell::new(false),
        });
        let raw = unsafe {
            ffi::libseat_open_seat(
                &LISTENER,
                (&mut *callbacks as *mut CallbackState).cast::<c_void>(),
            )
        };
        let raw = NonNull::new(raw).ok_or_else(|| {
            LinuxPlatformError::new(
                LinuxPlatformErrorKind::Session,
                "libseat could not open a desktop session seat",
            )
        })?;
        Ok(Self {
            raw,
            callbacks,
            marker: PhantomData,
        })
    }

    pub fn disable_pending(&self) -> bool {
        self.callbacks.disable_pending.get()
    }

    pub fn acknowledge_disable(&self) -> Result<(), LinuxPlatformError> {
        if !self.disable_pending() {
            return Ok(());
        }
        let result = unsafe { ffi::libseat_disable_seat(self.raw.as_ptr()) };
        if result < 0 {
            return Err(LinuxPlatformError::native(
                LinuxPlatformErrorKind::Session,
                "libseat disable acknowledgement failed",
                result,
            ));
        }
        self.callbacks.disable_pending.set(false);
        Ok(())
    }

    pub fn state(&self) -> SeatState {
        if self.callbacks.enabled.get() {
            SeatState::Enabled
        } else {
            SeatState::Disabled
        }
    }

    pub fn name(&self) -> Result<String, LinuxPlatformError> {
        let name = unsafe { ffi::libseat_seat_name(self.raw.as_ptr()) };
        let name = NonNull::new(name.cast_mut()).ok_or_else(|| {
            LinuxPlatformError::new(
                LinuxPlatformErrorKind::Session,
                "libseat returned no seat name",
            )
        })?;
        Ok(unsafe { CStr::from_ptr(name.as_ptr()) }
            .to_string_lossy()
            .into_owned())
    }

    pub fn event_fd(&self) -> i32 {
        unsafe { ffi::libseat_get_fd(self.raw.as_ptr()) }
    }

    pub fn dispatch(&self, timeout_ms: i32) -> Result<(), LinuxPlatformError> {
        let result = unsafe { ffi::libseat_dispatch(self.raw.as_ptr(), timeout_ms) };
        if result < 0 {
            Err(LinuxPlatformError::native(
                LinuxPlatformErrorKind::Session,
                "libseat dispatch failed",
                result,
            ))
        } else {
            Ok(())
        }
    }

    pub fn open_device(&self, path: &str) -> Result<SeatDevice<'_>, LinuxPlatformError> {
        let path = CString::new(path).map_err(|_| {
            LinuxPlatformError::new(
                LinuxPlatformErrorKind::Device,
                "device path contains an interior NUL",
            )
        })?;
        let mut fd = -1;
        let id = unsafe { ffi::libseat_open_device(self.raw.as_ptr(), path.as_ptr(), &mut fd) };
        if id < 0 || fd < 0 {
            return Err(LinuxPlatformError::native(
                LinuxPlatformErrorKind::Device,
                "libseat could not open the requested device",
                id,
            ));
        }
        Ok(SeatDevice { seat: self, id, fd })
    }

    pub fn switch_session(&self, session: i32) -> Result<(), LinuxPlatformError> {
        if session <= 0 {
            return Err(LinuxPlatformError::new(
                LinuxPlatformErrorKind::InvalidState,
                "session number must be positive",
            ));
        }
        let result = unsafe { ffi::libseat_switch_session(self.raw.as_ptr(), session) };
        if result < 0 {
            Err(LinuxPlatformError::native(
                LinuxPlatformErrorKind::Session,
                "libseat could not switch virtual terminal session",
                result,
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) fn raw(&self) -> *mut ffi::libseat {
        self.raw.as_ptr()
    }
}

impl Drop for LinuxSeat {
    fn drop(&mut self) {
        let result = unsafe { ffi::libseat_close_seat(self.raw.as_ptr()) };
        debug_assert!(result >= 0, "libseat close failed");
    }
}

pub struct SeatDevice<'seat> {
    seat: &'seat LinuxSeat,
    id: i32,
    fd: i32,
}

impl SeatDevice<'_> {
    pub const fn fd(&self) -> i32 {
        self.fd
    }

    pub const fn id(&self) -> i32 {
        self.id
    }

    pub fn try_clone_fd(&self) -> Result<OwnedFd, LinuxPlatformError> {
        unsafe { BorrowedFd::borrow_raw(self.fd) }
            .try_clone_to_owned()
            .map_err(|error| {
                LinuxPlatformError::new(LinuxPlatformErrorKind::Device, error.to_string())
            })
    }
}

impl Drop for SeatDevice<'_> {
    fn drop(&mut self) {
        let result = unsafe { ffi::libseat_close_device(self.seat.raw.as_ptr(), self.id) };
        debug_assert!(result >= 0, "libseat device close failed");
    }
}

#[cfg(test)]
mod transition_tests {
    use super::*;
    #[test]
    fn deferred_disable_remains_pending_even_if_enable_arrives_before_owner_cleanup() {
        let mut state = CallbackState {
            enabled: Cell::new(true),
            defer_disable: true,
            disable_pending: Cell::new(false),
        };
        let data = (&mut state as *mut CallbackState).cast::<c_void>();
        unsafe { disable_seat(std::ptr::null_mut(), data) };
        assert!(!state.enabled.get());
        assert!(state.disable_pending.get());
        unsafe { enable_seat(std::ptr::null_mut(), data) };
        assert!(state.enabled.get());
        assert!(state.disable_pending.get());
    }
}
