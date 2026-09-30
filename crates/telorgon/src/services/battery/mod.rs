//! Read-only battery and external-power metrics for applications, shells, and embedded hosts.
//!
//! Call [`snapshot`] for a fresh async read, or [`monitor()`] to explicitly start periodic reads.
//! A monitor owns its worker; its cloneable handle supplies a stable UI signal and events.
//! Linux reads the kernel power-supply interface without requiring a desktop session or daemon.
//! Other native platforms return [`BatteryError::Unsupported`]; embedded hosts can implement
//! [`BatteryProvider`] using their own hardware.
//!
//! ```no_run
//! async fn refresh() -> Result<(), telorgon::battery::BatteryError> {
//!     let power = telorgon::battery::snapshot().await?;
//!     for battery in power.batteries {
//!         println!("{}: {:?}% ({:?})", battery.id, battery.charge_percent, battery.state);
//!     }
//!     Ok(())
//! }
//! ```
//!
//! ```no_run
//! use telorgon::battery;
//!
//! async fn observe() -> Result<(), battery::BatteryError> {
//!     let monitor = battery::monitor(battery::BatteryMonitorConfig::default()).await?;
//!     let handle = monitor.handle(); // Pass to UI components; watch handle.signal().
//!     let mut events = handle.subscribe()?;
//!     while let Some(event) = events.next().await {
//!         if event.started_charging() {
//!             // Trigger the application's charging-start sound here.
//!         }
//!         if let battery::BatteryEvent::ResyncRequired = event {
//!             let _current = handle.state(); // Rebuild incremental state; do not replay sounds.
//!         }
//!     }
//!     monitor.shutdown().await?;
//!     Ok(())
//! }
//! ```

#[cfg(target_os = "linux")]
mod linux;
mod model;
mod monitor;
mod status;

pub use model::{
    BatteryError, BatteryEstimateSource, BatteryKind, BatteryMetrics, BatteryProvider,
    BatteryScope, BatterySnapshot, BatteryState, BatteryTimeEstimate, ExternalPowerSupply,
};
pub use monitor::{
    BatteryEvents, BatteryMonitor, BatteryMonitorConfig, BatteryMonitorHandle, monitor,
};
pub use status::{
    BatteryAvailability, BatteryEvent, BatteryHealth, BatteryMonitorState, BatteryStatus,
    BatterySummary,
};

/// Native read-only provider. Construction performs no I/O and claims no hardware resources.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBatteryProvider;

impl BatteryProvider for SystemBatteryProvider {
    fn read_snapshot(&self) -> Result<BatterySnapshot, BatteryError> {
        #[cfg(target_os = "linux")]
        {
            linux::read_snapshot(std::path::Path::new("/sys/class/power_supply"))
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(BatteryError::Unsupported)
        }
    }
}

/// Performs a fresh native read, including device additions and removals since the last call.
///
/// This blocks on native I/O. GUI components should use [`snapshot`] in a task instead.
pub fn read_snapshot() -> Result<BatterySnapshot, BatteryError> {
    SystemBatteryProvider.read_snapshot()
}

/// Reads a fresh snapshot on the SDK's blocking executor, without requiring Tokio.
///
/// The caller owns refresh scheduling. Dropping the future does not cancel an in-flight read,
/// but the read retains no application state and publishes no callback.
pub async fn snapshot() -> Result<BatterySnapshot, BatteryError> {
    blocking::unblock(read_snapshot).await
}
