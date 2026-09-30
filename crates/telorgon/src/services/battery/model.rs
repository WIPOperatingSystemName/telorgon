use std::{
    io,
    path::PathBuf,
    time::{Duration, SystemTime},
};

/// Whether a battery powers the host, an attached device, or an unspecified scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BatteryScope {
    System,
    Device,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryKind {
    Battery,
    Ups,
}

/// Charging state is reported independently of whether external power is connected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BatteryState {
    Charging,
    Discharging,
    NotCharging,
    Full,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryEstimateSource {
    /// A duration reported by the hardware or native power service.
    Reported,
    /// A constant-rate approximation from the current energy/power or charge/current readings.
    Calculated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryTimeEstimate {
    pub duration: Duration,
    pub source: BatteryEstimateSource,
}

/// One battery's readings. `None` means the metric is unavailable, never a zero reading.
///
/// Field suffixes specify units. Capacity and current use separate charge and energy units;
/// they are never substituted for one another. Device IDs are stable while the native device
/// exists, but may change after replacement or reconnection.
#[derive(Clone, Debug, PartialEq)]
pub struct BatteryMetrics {
    pub id: String,
    pub kind: BatteryKind,
    pub scope: BatteryScope,
    pub present: bool,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub technology: Option<String>,
    pub state: BatteryState,
    /// Remaining usable capacity, between 0 and 100 percent.
    pub charge_percent: Option<f64>,
    /// Native health description, such as `Good` or `Overheat`.
    pub health: Option<String>,
    /// Learned full capacity divided by design capacity, in percent. May exceed 100.
    /// This measures capacity retention, independently of native health/fault status.
    pub capacity_health_percent: Option<f64>,
    pub cycle_count: Option<u64>,
    pub energy_now_wh: Option<f64>,
    pub energy_full_wh: Option<f64>,
    pub energy_full_design_wh: Option<f64>,
    pub energy_empty_wh: Option<f64>,
    pub charge_now_ah: Option<f64>,
    pub charge_full_ah: Option<f64>,
    pub charge_full_design_ah: Option<f64>,
    pub charge_empty_ah: Option<f64>,
    /// Magnitude of instantaneous charging or discharging power.
    pub power_watts: Option<f64>,
    pub voltage_volts: Option<f64>,
    /// Native signed current. Some drivers report a magnitude for both charging and discharging;
    /// consult `state` rather than inferring the charging state from this sign.
    pub current_amps: Option<f64>,
    pub temperature_celsius: Option<f64>,
    /// Populated only while discharging. Calculated values are approximate, not predictions.
    pub time_to_empty: Option<BatteryTimeEstimate>,
    /// Populated only while charging. Calculated values do not model charging taper.
    pub time_to_full: Option<BatteryTimeEstimate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalPowerSupply {
    pub id: String,
    /// Native type, including mains, USB variants, or wireless charging.
    pub kind: String,
    pub scope: BatteryScope,
    pub online: Option<bool>,
}

/// A fresh observation; individual device attributes are read sequentially, not atomically.
#[derive(Clone, Debug, PartialEq)]
pub struct BatterySnapshot {
    pub observed_at: SystemTime,
    /// Includes system, peripheral, and UPS batteries. Filter `scope` and `kind` as appropriate.
    /// An empty list means no batteries were discovered, not an unsupported platform.
    pub batteries: Vec<BatteryMetrics>,
    pub external_supplies: Vec<ExternalPowerSupply>,
    /// Whether any non-device external supply is online. `None` means unknown or unreported.
    /// Unspecified supply scopes are included for older hardware that does not report a scope.
    pub external_power: Option<bool>,
}

/// A caller-owned source for native or embedded battery metrics.
///
/// Implementations perform a bounded synchronous observation and own no implicit refresh loop.
/// Battery IDs must be unique within each snapshot and stable for the same device. A provider
/// used by a monitor is moved onto its worker and need not support simultaneous reads.
pub trait BatteryProvider {
    fn read_snapshot(&self) -> Result<BatterySnapshot, BatteryError>;
}

#[derive(Debug, thiserror::Error)]
pub enum BatteryError {
    #[error("battery metrics are unsupported on this platform")]
    Unsupported,
    #[error("cannot read battery metrics at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid battery monitor configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("cannot start battery monitor: {0}")]
    Start(#[source] io::Error),
    #[error("battery monitor has stopped")]
    Stopped,
    #[error("battery monitor subscription limit reached")]
    SubscriberLimitReached,
    #[error("battery monitor worker panicked")]
    WorkerPanicked,
}
