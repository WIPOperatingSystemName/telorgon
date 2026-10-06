use crate::{
    platform::contracts::{PermissionState, PlatformError},
    shell::OutputId,
};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct ScreenBrightnessLevel(u16);
impl ScreenBrightnessLevel {
    pub const MAX: Self = Self(10_000);
    pub const ZERO: Self = Self(0);
    pub fn percent(value: f32) -> Result<Self, ScreenBrightnessError> {
        if !value.is_finite() || !(0.0..=100.0).contains(&value) {
            return Err(ScreenBrightnessError::InvalidLevel);
        }
        Self::basis_points((value * 100.0).round() as u16)
    }
    pub const fn basis_points(value: u16) -> Result<Self, ScreenBrightnessError> {
        if value > 10_000 {
            Err(ScreenBrightnessError::InvalidLevel)
        } else {
            Ok(Self(value))
        }
    }
    pub const fn as_basis_points(self) -> u16 {
        self.0
    }
    pub fn as_percent(self) -> f32 {
        self.0 as f32 / 100.0
    }
    pub(crate) fn from_raw(raw: u32, maximum: u32) -> Self {
        Self(
            ((u64::from(raw.min(maximum)) * 10_000 + u64::from(maximum) / 2) / u64::from(maximum))
                as u16,
        )
    }
    pub(crate) fn raw(self, maximum: u32) -> u32 {
        ((u64::from(self.0) * u64::from(maximum) + 5_000) / 10_000) as u32
    }
}
impl TryFrom<u16> for ScreenBrightnessLevel {
    type Error = ScreenBrightnessError;
    fn try_from(v: u16) -> Result<Self, Self::Error> {
        Self::basis_points(v)
    }
}
impl From<ScreenBrightnessLevel> for u16 {
    fn from(v: ScreenBrightnessLevel) -> Self {
        v.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "i16", into = "i16")]
pub struct ScreenBrightnessDelta(i16);
impl ScreenBrightnessDelta {
    pub fn percentage_points(value: f32) -> Result<Self, ScreenBrightnessError> {
        if !value.is_finite() || !(-100.0..=100.0).contains(&value) {
            return Err(ScreenBrightnessError::InvalidLevel);
        }
        Self::basis_points((value * 100.0).round() as i16)
    }
    pub const fn basis_points(value: i16) -> Result<Self, ScreenBrightnessError> {
        if value < -10_000 || value > 10_000 {
            Err(ScreenBrightnessError::InvalidLevel)
        } else {
            Ok(Self(value))
        }
    }
    pub const fn as_basis_points(self) -> i16 {
        self.0
    }
}
impl TryFrom<i16> for ScreenBrightnessDelta {
    type Error = ScreenBrightnessError;
    fn try_from(v: i16) -> Result<Self, Self::Error> {
        Self::basis_points(v)
    }
}
impl From<ScreenBrightnessDelta> for i16 {
    fn from(v: ScreenBrightnessDelta) -> Self {
        v.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenBrightnessDeviceHandle {
    pub(crate) controller: u64,
    pub(crate) device: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenBrightnessTarget {
    Device(ScreenBrightnessDeviceHandle),
    Output(OutputId),
    DefaultInternal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenBrightnessAction {
    Set(ScreenBrightnessLevel),
    Adjust(ScreenBrightnessDelta),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScreenBrightnessWriteMode {
    #[default]
    Ordered,
    ReplacePendingSet,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenBrightnessKind {
    InternalBacklight,
    ExternalMonitor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenBrightnessVerification {
    DriverSetting,
    MonitorVcp,
    Provider,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScreenBrightnessAssociation {
    Unknown,
    Confirmed(Vec<OutputId>),
    HostConfigured(Vec<OutputId>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenBrightnessServiceState {
    Unstarted,
    Ready,
    Restricted,
    Unavailable,
    Stopped,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenBrightnessDeviceSnapshot {
    pub device: ScreenBrightnessDeviceHandle,
    pub name: String,
    pub kind: ScreenBrightnessKind,
    pub association: ScreenBrightnessAssociation,
    pub permission: PermissionState,
    pub minimum: ScreenBrightnessLevel,
    pub native_maximum: u32,
    pub configured: Option<ScreenBrightnessLevel>,
    pub reported_actual: Option<ScreenBrightnessLevel>,
    pub pending: Option<(ScreenBrightnessRequestId, ScreenBrightnessLevel)>,
    pub observed_at: Instant,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenBrightnessSnapshot {
    pub revision: u64,
    pub state: ScreenBrightnessServiceState,
    pub devices: Vec<ScreenBrightnessDeviceSnapshot>,
    pub pending: usize,
    pub last_error: Option<ScreenBrightnessError>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScreenBrightnessRequestId(pub(crate) u64);
impl ScreenBrightnessRequestId {
    pub const fn get(self) -> u64 {
        self.0
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenBrightnessApplied {
    pub level: ScreenBrightnessLevel,
    pub reported_actual: Option<ScreenBrightnessLevel>,
    pub revision: u64,
    pub verification: ScreenBrightnessVerification,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenBrightnessConfirmationFailure {
    Timeout,
    Transport,
    ReadbackUnavailable,
    ReadbackMismatch,
    AuthorityLost,
    DeviceLost,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ScreenBrightnessOutcome {
    Applied(ScreenBrightnessApplied),
    Unconfirmed {
        reason: ScreenBrightnessConfirmationFailure,
        observed: Option<ScreenBrightnessLevel>,
    },
    Superseded,
    Cancelled,
    Denied,
    Stale,
    Failed(PlatformError),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScreenBrightnessError {
    InvalidLevel,
    InvalidConfig(&'static str),
    BelowMinimum,
    Unsupported,
    Unavailable,
    PermissionDenied,
    SessionInactive,
    Locked,
    StaleDevice,
    AmbiguousTarget,
    QueueFull,
    TimedOut,
    InvalidData,
    Transport,
    WorkerPanicked,
}
impl fmt::Display for ScreenBrightnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "screen brightness: {self:?}")
    }
}
impl std::error::Error for ScreenBrightnessError {}

#[derive(Clone, Debug)]
pub struct ScreenBrightnessConfig {
    pub minimum: ScreenBrightnessLevel,
    pub allow_zero: bool,
    pub queue_capacity: usize,
    pub poll_interval: Duration,
    pub request_timeout: Duration,
}
impl Default for ScreenBrightnessConfig {
    fn default() -> Self {
        Self {
            minimum: ScreenBrightnessLevel(100),
            allow_zero: false,
            queue_capacity: 32,
            poll_interval: Duration::from_secs(2),
            request_timeout: Duration::from_secs(5),
        }
    }
}
impl ScreenBrightnessConfig {
    pub(crate) fn validate(&self) -> Result<(), ScreenBrightnessError> {
        if !(1..=32).contains(&self.queue_capacity)
            || self.poll_interval < Duration::from_millis(50)
            || self.poll_interval > Duration::from_secs(3600)
            || self.request_timeout < Duration::from_millis(50)
            || self.request_timeout > Duration::from_secs(60)
        {
            return Err(ScreenBrightnessError::InvalidConfig("capacity or timing"));
        }
        Ok(())
    }
    pub(crate) fn native_minimum(&self, max: u32) -> u32 {
        let floor = (u64::from(self.minimum.0) * u64::from(max)).div_ceil(10_000) as u32;
        floor.max(u32::from(!self.allow_zero))
    }
}

/// Supplied only by the owner, never by widget handles. A change revokes admitted work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScreenBrightnessHostState {
    pub active: bool,
    pub locked: bool,
}
