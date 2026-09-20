use std::fmt;

use crate::foundation::PointF;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinuxSessionConfig {
    pub seat_name: String,
    pub allow_session_switching: bool,
}

impl Default for LinuxSessionConfig {
    fn default() -> Self {
        Self {
            seat_name: "seat0".to_owned(),
            allow_session_switching: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub system_name: String,
    pub display_name: String,
    pub vendor: u32,
    pub product: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinuxInputEvent {
    pub time_microseconds: u64,
    pub device_token: u64,
    pub kind: LinuxInputEventKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LinuxInputEventKind {
    PointerMotion {
        /// Libinput's normalized accelerated movement; the desktop applies it in logical units.
        delta: PointF,
        /// Device-space unaccelerated movement, independent of the output pixel density.
        unaccelerated: PointF,
    },
    PointerAbsolute {
        normalized: PointF,
    },
    PointerButton {
        button: u32,
        pressed: bool,
    },
    PointerAxis {
        horizontal: f64,
        vertical: f64,
        discrete_x: i32,
        discrete_y: i32,
    },
    KeyboardKey {
        keycode: u32,
        pressed: bool,
    },
    TouchDown {
        slot: i32,
        normalized: PointF,
    },
    TouchMotion {
        slot: i32,
        normalized: PointF,
    },
    TouchUp {
        slot: i32,
    },
    TouchCancel,
    DeviceAdded,
    DeviceRemoved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinuxPlatformErrorKind {
    Unsupported,
    Allocation,
    Session,
    Device,
    Input,
    Keymap,
    Native,
    InvalidState,
}

#[derive(Debug)]
pub struct LinuxPlatformError {
    kind: LinuxPlatformErrorKind,
    context: String,
    native_code: Option<i32>,
}

impl LinuxPlatformError {
    pub fn new(kind: LinuxPlatformErrorKind, context: impl Into<String>) -> Self {
        Self {
            kind,
            context: context.into(),
            native_code: None,
        }
    }

    pub fn native(
        kind: LinuxPlatformErrorKind,
        context: impl Into<String>,
        native_code: i32,
    ) -> Self {
        Self {
            kind,
            context: context.into(),
            native_code: Some(native_code),
        }
    }

    pub const fn kind(&self) -> LinuxPlatformErrorKind {
        self.kind
    }

    pub const fn native_code(&self) -> Option<i32> {
        self.native_code
    }
}

impl fmt::Display for LinuxPlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.context)
    }
}

impl std::error::Error for LinuxPlatformError {}
