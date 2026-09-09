use std::collections::BTreeMap;
use std::ffi::{CString, c_void};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use crate::core::PointF;

use crate::platform_linux::ffi;
use crate::platform_linux::{
    LinuxInputEvent, LinuxInputEventKind, LinuxPlatformError, LinuxPlatformErrorKind, LinuxSeat,
};

const DEVICE_ADDED: u32 = 1;
const DEVICE_REMOVED: u32 = 2;
const KEYBOARD_KEY: u32 = 300;
const POINTER_MOTION: u32 = 400;
const POINTER_MOTION_ABSOLUTE: u32 = 401;
const POINTER_BUTTON: u32 = 402;
const POINTER_AXIS: u32 = 403;
const TOUCH_DOWN: u32 = 500;
const TOUCH_UP: u32 = 501;
const TOUCH_MOTION: u32 = 502;
const TOUCH_CANCEL: u32 = 503;

const AXIS_VERTICAL: u32 = 0;
const AXIS_HORIZONTAL: u32 = 1;
const NORMALIZED_EXTENT: u32 = 1_000_000;

struct DeviceBroker {
    seat: *mut ffi::libseat,
    devices: BTreeMap<i32, i32>,
}

unsafe extern "C" fn open_restricted(
    path: *const std::ffi::c_char,
    _flags: i32,
    data: *mut c_void,
) -> i32 {
    let broker = unsafe { &mut *data.cast::<DeviceBroker>() };
    let mut fd = -1;
    let id = unsafe { ffi::libseat_open_device(broker.seat, path, &mut fd) };
    if id >= 0 && fd >= 0 {
        broker.devices.insert(fd, id);
        fd
    } else {
        -1
    }
}

unsafe extern "C" fn close_restricted(fd: i32, data: *mut c_void) {
    let broker = unsafe { &mut *data.cast::<DeviceBroker>() };
    if let Some(id) = broker.devices.remove(&fd) {
        unsafe { ffi::libseat_close_device(broker.seat, id) };
    }
}

static INTERFACE: ffi::libinput_interface = ffi::libinput_interface {
    open_restricted: Some(open_restricted),
    close_restricted: Some(close_restricted),
};

pub struct LibInputContext<'seat> {
    raw: NonNull<ffi::libinput>,
    udev: NonNull<ffi::udev>,
    broker: Box<DeviceBroker>,
    marker: PhantomData<(&'seat LinuxSeat, Rc<()>)>,
}

impl<'seat> LibInputContext<'seat> {
    pub fn new(seat: &'seat LinuxSeat, seat_name: &str) -> Result<Self, LinuxPlatformError> {
        let udev = NonNull::new(unsafe { ffi::udev_new() }).ok_or_else(|| {
            LinuxPlatformError::new(
                LinuxPlatformErrorKind::Allocation,
                "libudev allocation failed",
            )
        })?;
        let mut broker = Box::new(DeviceBroker {
            seat: seat.raw(),
            devices: BTreeMap::new(),
        });
        let raw = unsafe {
            ffi::libinput_udev_create_context(
                &INTERFACE,
                (&mut *broker as *mut DeviceBroker).cast::<c_void>(),
                udev.as_ptr(),
            )
        };
        let Some(raw) = NonNull::new(raw) else {
            unsafe { ffi::udev_unref(udev.as_ptr()) };
            return Err(LinuxPlatformError::new(
                LinuxPlatformErrorKind::Input,
                "libinput could not create a udev context",
            ));
        };
        let seat_name = CString::new(seat_name).map_err(|_| {
            LinuxPlatformError::new(
                LinuxPlatformErrorKind::Input,
                "seat name contains an interior NUL",
            )
        })?;
        let result = unsafe { ffi::libinput_udev_assign_seat(raw.as_ptr(), seat_name.as_ptr()) };
        if result != 0 {
            unsafe {
                ffi::libinput_unref(raw.as_ptr());
                ffi::udev_unref(udev.as_ptr());
            }
            return Err(LinuxPlatformError::native(
                LinuxPlatformErrorKind::Input,
                "libinput could not assign the requested seat",
                result,
            ));
        }
        Ok(Self {
            raw,
            udev,
            broker,
            marker: PhantomData,
        })
    }

    pub fn event_fd(&self) -> i32 {
        unsafe { ffi::libinput_get_fd(self.raw.as_ptr()) }
    }

    /// Close input devices and discard all queued pre-suspend/device-removal events.
    pub fn suspend(&self) {
        unsafe { ffi::libinput_suspend(self.raw.as_ptr()) };
        while let Some(event) = NonNull::new(unsafe { ffi::libinput_get_event(self.raw.as_ptr()) })
        {
            unsafe { ffi::libinput_event_destroy(event.as_ptr()) };
        }
    }

    pub fn resume(&self) -> Result<(), LinuxPlatformError> {
        let result = unsafe { ffi::libinput_resume(self.raw.as_ptr()) };
        if result != 0 {
            Err(LinuxPlatformError::native(
                LinuxPlatformErrorKind::Input,
                "libinput resume failed",
                result,
            ))
        } else {
            Ok(())
        }
    }

    pub fn dispatch(&self) -> Result<(), LinuxPlatformError> {
        let result = unsafe { ffi::libinput_dispatch(self.raw.as_ptr()) };
        if result != 0 {
            Err(LinuxPlatformError::native(
                LinuxPlatformErrorKind::Input,
                "libinput dispatch failed",
                result,
            ))
        } else {
            Ok(())
        }
    }

    pub fn next_event(&self) -> Option<LinuxInputEvent> {
        let raw = NonNull::new(unsafe { ffi::libinput_get_event(self.raw.as_ptr()) })?;
        let event_type = unsafe { ffi::libinput_event_get_type(raw.as_ptr()) };
        let device = unsafe { ffi::libinput_event_get_device(raw.as_ptr()) } as usize as u64;
        let event = match event_type {
            DEVICE_ADDED => Some(LinuxInputEvent {
                time_microseconds: 0,
                device_token: device,
                kind: LinuxInputEventKind::DeviceAdded,
            }),
            DEVICE_REMOVED => Some(LinuxInputEvent {
                time_microseconds: 0,
                device_token: device,
                kind: LinuxInputEventKind::DeviceRemoved,
            }),
            KEYBOARD_KEY => Some(LinuxInputEvent {
                time_microseconds: unsafe {
                    ffi::libinput_event_keyboard_get_time_usec(raw.as_ptr())
                },
                device_token: device,
                kind: LinuxInputEventKind::KeyboardKey {
                    keycode: unsafe { ffi::libinput_event_keyboard_get_key(raw.as_ptr()) },
                    pressed: unsafe { ffi::libinput_event_keyboard_get_key_state(raw.as_ptr()) }
                        != 0,
                },
            }),
            POINTER_MOTION => Some(LinuxInputEvent {
                time_microseconds: unsafe {
                    ffi::libinput_event_pointer_get_time_usec(raw.as_ptr())
                },
                device_token: device,
                kind: LinuxInputEventKind::PointerMotion {
                    delta: PointF {
                        x: unsafe { ffi::libinput_event_pointer_get_dx(raw.as_ptr()) } as f32,
                        y: unsafe { ffi::libinput_event_pointer_get_dy(raw.as_ptr()) } as f32,
                    },
                    unaccelerated: PointF {
                        x: unsafe { ffi::libinput_event_pointer_get_dx_unaccelerated(raw.as_ptr()) }
                            as f32,
                        y: unsafe { ffi::libinput_event_pointer_get_dy_unaccelerated(raw.as_ptr()) }
                            as f32,
                    },
                },
            }),
            POINTER_BUTTON => Some(LinuxInputEvent {
                time_microseconds: unsafe {
                    ffi::libinput_event_pointer_get_time_usec(raw.as_ptr())
                },
                device_token: device,
                kind: LinuxInputEventKind::PointerButton {
                    button: unsafe { ffi::libinput_event_pointer_get_button(raw.as_ptr()) },
                    pressed: unsafe { ffi::libinput_event_pointer_get_button_state(raw.as_ptr()) }
                        != 0,
                },
            }),
            POINTER_MOTION_ABSOLUTE => Some(LinuxInputEvent {
                time_microseconds: unsafe {
                    ffi::libinput_event_pointer_get_time_usec(raw.as_ptr())
                },
                device_token: device,
                kind: LinuxInputEventKind::PointerAbsolute {
                    normalized: PointF {
                        x: (unsafe {
                            ffi::libinput_event_pointer_get_absolute_x_transformed(
                                raw.as_ptr(),
                                NORMALIZED_EXTENT,
                            )
                        } / f64::from(NORMALIZED_EXTENT)) as f32,
                        y: (unsafe {
                            ffi::libinput_event_pointer_get_absolute_y_transformed(
                                raw.as_ptr(),
                                NORMALIZED_EXTENT,
                            )
                        } / f64::from(NORMALIZED_EXTENT)) as f32,
                    },
                },
            }),
            POINTER_AXIS => Some(LinuxInputEvent {
                time_microseconds: unsafe {
                    ffi::libinput_event_pointer_get_time_usec(raw.as_ptr())
                },
                device_token: device,
                kind: LinuxInputEventKind::PointerAxis {
                    horizontal: axis_value(raw.as_ptr(), AXIS_HORIZONTAL),
                    vertical: axis_value(raw.as_ptr(), AXIS_VERTICAL),
                    discrete_x: axis_discrete(raw.as_ptr(), AXIS_HORIZONTAL),
                    discrete_y: axis_discrete(raw.as_ptr(), AXIS_VERTICAL),
                },
            }),
            TOUCH_DOWN | TOUCH_MOTION => Some(LinuxInputEvent {
                time_microseconds: unsafe { ffi::libinput_event_touch_get_time_usec(raw.as_ptr()) },
                device_token: device,
                kind: if event_type == TOUCH_DOWN {
                    LinuxInputEventKind::TouchDown {
                        slot: unsafe { ffi::libinput_event_touch_get_seat_slot(raw.as_ptr()) },
                        normalized: touch_position(raw.as_ptr()),
                    }
                } else {
                    LinuxInputEventKind::TouchMotion {
                        slot: unsafe { ffi::libinput_event_touch_get_seat_slot(raw.as_ptr()) },
                        normalized: touch_position(raw.as_ptr()),
                    }
                },
            }),
            TOUCH_UP => Some(LinuxInputEvent {
                time_microseconds: unsafe { ffi::libinput_event_touch_get_time_usec(raw.as_ptr()) },
                device_token: device,
                kind: LinuxInputEventKind::TouchUp {
                    slot: unsafe { ffi::libinput_event_touch_get_seat_slot(raw.as_ptr()) },
                },
            }),
            TOUCH_CANCEL => Some(LinuxInputEvent {
                time_microseconds: unsafe { ffi::libinput_event_touch_get_time_usec(raw.as_ptr()) },
                device_token: device,
                kind: LinuxInputEventKind::TouchCancel,
            }),
            _ => None,
        };
        unsafe { ffi::libinput_event_destroy(raw.as_ptr()) };
        event
    }

    pub fn opened_device_count(&self) -> usize {
        self.broker.devices.len()
    }
}

fn axis_value(event: *mut ffi::libinput_event, axis: u32) -> f64 {
    if unsafe { ffi::libinput_event_pointer_has_axis(event, axis) } != 0 {
        unsafe { ffi::libinput_event_pointer_get_axis_value(event, axis) }
    } else {
        0.0
    }
}

fn axis_discrete(event: *mut ffi::libinput_event, axis: u32) -> i32 {
    if unsafe { ffi::libinput_event_pointer_has_axis(event, axis) } != 0 {
        unsafe { ffi::libinput_event_pointer_get_axis_value_discrete(event, axis) }
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    } else {
        0
    }
}

fn touch_position(event: *mut ffi::libinput_event) -> PointF {
    PointF {
        x: (unsafe { ffi::libinput_event_touch_get_x_transformed(event, NORMALIZED_EXTENT) }
            / f64::from(NORMALIZED_EXTENT)) as f32,
        y: (unsafe { ffi::libinput_event_touch_get_y_transformed(event, NORMALIZED_EXTENT) }
            / f64::from(NORMALIZED_EXTENT)) as f32,
    }
}

impl Drop for LibInputContext<'_> {
    fn drop(&mut self) {
        unsafe {
            ffi::libinput_unref(self.raw.as_ptr());
            ffi::udev_unref(self.udev.as_ptr());
        }
        debug_assert!(
            self.broker.devices.is_empty(),
            "libinput retained restricted devices"
        );
    }
}
