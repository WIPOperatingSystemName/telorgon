use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;

struct Supplies(PathBuf);

impl Supplies {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "telorgon-battery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn device(&self, name: &str, attributes: &[(&str, &str)]) {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        for (key, value) in attributes {
            fs::write(path.join(key), format!("{value}\n")).unwrap();
        }
    }

    fn snapshot(&self) -> BatterySnapshot {
        read_snapshot(&self.0).unwrap()
    }
}

impl Drop for Supplies {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn energy_metrics_keep_units_health_and_estimates_separate() {
    let supplies = Supplies::new();
    supplies.device(
        "BAT0",
        &[
            ("type", "Battery"),
            ("scope", "System"),
            ("status", "Discharging"),
            ("manufacturer", "Example"),
            ("model_name", "Portable"),
            ("health", "Good"),
            ("energy_now", "30000000"),
            ("energy_full", "60000000"),
            ("energy_full_design", "80000000"),
            ("energy_empty", "0"),
            ("power_now", "10000000"),
            ("voltage_now", "12000000"),
            ("current_now", "-833333"),
            ("temp", "315"),
            ("cycle_count", "128"),
        ],
    );
    supplies.device("AC", &[("type", "Mains"), ("online", "0")]);
    let snapshot = supplies.snapshot();
    assert_eq!(snapshot.external_power, Some(false));
    let battery = &snapshot.batteries[0];
    assert_eq!(battery.scope, BatteryScope::System);
    assert_eq!(battery.charge_percent, Some(50.0));
    assert_eq!(battery.capacity_health_percent, Some(75.0));
    assert_eq!(battery.health.as_deref(), Some("Good"));
    assert_eq!(battery.energy_now_wh, Some(30.0));
    assert_eq!(battery.charge_now_ah, None);
    assert_eq!(battery.voltage_volts, Some(12.0));
    assert_eq!(battery.current_amps, Some(-0.833333));
    assert_eq!(battery.temperature_celsius, Some(31.5));
    assert_eq!(battery.cycle_count, Some(128));
    assert_eq!(
        battery.time_to_empty,
        Some(BatteryTimeEstimate {
            duration: Duration::from_secs(10_800),
            source: BatteryEstimateSource::Calculated,
        })
    );
    assert_eq!(battery.time_to_full, None);
}

#[test]
fn charge_only_hardware_uses_current_without_inventing_energy() {
    let supplies = Supplies::new();
    supplies.device(
        "battery",
        &[
            ("type", "Battery"),
            ("status", "Charging"),
            ("charge_now", "2000000"),
            ("charge_empty", "1000000"),
            ("charge_full", "5000000"),
            ("charge_full_design", "5000000"),
            ("current_now", "1500000"),
            ("voltage_now", "4000000"),
            ("cycle_count", "0"),
        ],
    );
    let snapshot = supplies.snapshot();
    let battery = &snapshot.batteries[0];
    assert_eq!(battery.charge_percent, Some(25.0));
    assert_eq!(battery.energy_now_wh, None);
    assert_eq!(battery.charge_now_ah, Some(2.0));
    assert_eq!(battery.power_watts, Some(6.0));
    assert_eq!(battery.cycle_count, None);
    assert_eq!(
        battery.time_to_full.unwrap().duration,
        Duration::from_secs(7200)
    );
    assert_eq!(battery.time_to_empty, None);
}

#[test]
fn native_capacity_and_duration_take_precedence_over_calculation() {
    let supplies = Supplies::new();
    supplies.device(
        "BAT0",
        &[
            ("type", "Battery"),
            ("status", "Discharging"),
            ("capacity", "42"),
            ("energy_now", "30000000"),
            ("energy_full", "60000000"),
            ("power_now", "10000000"),
            ("time_to_empty_now", "1200"),
            ("time_to_empty_avg", "2400"),
        ],
    );
    let snapshot = supplies.snapshot();
    let battery = &snapshot.batteries[0];
    assert_eq!(battery.charge_percent, Some(42.0));
    assert_eq!(
        battery.time_to_empty,
        Some(BatteryTimeEstimate {
            duration: Duration::from_secs(1200),
            source: BatteryEstimateSource::Reported,
        })
    );
}

#[test]
fn malformed_unknown_and_absent_readings_never_become_zero_capacity() {
    let supplies = Supplies::new();
    supplies.device(
        "BAT0",
        &[
            ("type", "Battery"),
            ("status", "Future state"),
            ("capacity", "255"),
            ("energy_now", "-1"),
            ("energy_full", "0"),
            ("power_now", "0"),
            ("cycle_count", "-1"),
            ("temp", "unknown"),
            ("time_to_empty_now", "-1"),
        ],
    );
    supplies.device(
        "BAT1",
        &[
            ("type", "Battery"),
            ("present", "0"),
            ("status", "Full"),
            ("capacity", "100"),
            ("energy_now", "50000000"),
        ],
    );
    let snapshot = supplies.snapshot();
    assert_eq!(snapshot.external_power, None);
    let unknown = &snapshot.batteries[0];
    assert!(unknown.present);
    assert_eq!(unknown.state, BatteryState::Unknown);
    assert_eq!(unknown.charge_percent, None);
    assert_eq!(unknown.energy_now_wh, None);
    assert_eq!(unknown.temperature_celsius, None);
    assert_eq!(unknown.time_to_empty, None);
    assert!(!snapshot.batteries[1].present);
    assert_eq!(snapshot.batteries[1].state, BatteryState::Unknown);
    assert_eq!(snapshot.batteries[1].charge_percent, None);
    assert_eq!(snapshot.batteries[1].energy_now_wh, None);
}

#[test]
fn device_power_does_not_override_host_external_power_and_refresh_discovers_removal() {
    let supplies = Supplies::new();
    supplies.device("AC", &[("type", "Mains"), ("online", "0")]);
    supplies.device(
        "USB",
        &[("type", "USB_PD"), ("online", "2"), ("scope", "System")],
    );
    supplies.device(
        "mouse-charger",
        &[("type", "Wireless"), ("online", "1"), ("scope", "Device")],
    );
    supplies.device(
        "mouse",
        &[("type", "Battery"), ("capacity", "72"), ("scope", "Device")],
    );
    supplies.device(
        "UPS",
        &[("type", "UPS"), ("status", "Full"), ("capacity", "100")],
    );
    let snapshot = supplies.snapshot();
    assert_eq!(snapshot.external_power, Some(true));
    assert_eq!(snapshot.batteries.len(), 2);
    assert_eq!(snapshot.batteries[0].kind, BatteryKind::Ups);
    assert_eq!(snapshot.batteries[1].scope, BatteryScope::Device);
    fs::remove_dir_all(supplies.0.join("USB")).unwrap();
    fs::remove_dir_all(supplies.0.join("mouse")).unwrap();
    let snapshot = supplies.snapshot();
    assert_eq!(snapshot.external_power, Some(false));
    assert_eq!(snapshot.batteries.len(), 1);
    supplies.device("unknown", &[("type", "USB")]);
    assert_eq!(supplies.snapshot().external_power, None);
}

#[test]
fn zero_rates_produce_no_estimate_but_real_zero_charge_is_preserved() {
    let supplies = Supplies::new();
    supplies.device(
        "BAT0",
        &[
            ("type", "Battery"),
            ("status", "Discharging"),
            ("capacity", "0"),
            ("energy_now", "0"),
            ("energy_full", "60000000"),
            ("power_now", "0"),
            ("current_now", "0"),
            ("time_to_empty_now", "0"),
        ],
    );
    let snapshot = supplies.snapshot();
    assert_eq!(snapshot.batteries[0].charge_percent, Some(0.0));
    assert_eq!(snapshot.batteries[0].time_to_empty, None);
}

#[test]
fn averaged_duration_is_used_when_instant_duration_is_unknown_and_suppressed_when_full() {
    let supplies = Supplies::new();
    supplies.device(
        "BAT0",
        &[
            ("type", "Battery"),
            ("status", "Charging"),
            ("time_to_full_now", "0"),
            ("time_to_full_avg", "1800"),
        ],
    );
    assert_eq!(
        supplies.snapshot().batteries[0].time_to_full,
        Some(BatteryTimeEstimate {
            duration: Duration::from_secs(1800),
            source: BatteryEstimateSource::Reported,
        })
    );
    supplies.device("BAT0", &[("status", "Full")]);
    assert_eq!(supplies.snapshot().batteries[0].time_to_full, None);
}

#[test]
fn empty_supported_directory_and_unreadable_backend_are_distinct() {
    let supplies = Supplies::new();
    let empty = supplies.snapshot();
    assert!(empty.batteries.is_empty());
    assert_eq!(empty.external_power, None);
    let missing = supplies.0.join("missing");
    assert!(
        matches!(read_snapshot(&missing), Err(BatteryError::Read { path, .. }) if path == missing)
    );
    supplies.device("broken", &[("type", "Battery")]);
    fs::create_dir(supplies.0.join("broken/capacity")).unwrap();
    assert!(
        matches!(read_snapshot(&supplies.0), Err(BatteryError::Read { path, .. }) if path.ends_with("broken/capacity"))
    );
}

#[test]
fn class_symlinks_are_followed_without_replacing_the_public_device_id() {
    let supplies = Supplies::new();
    let devices = Supplies::new();
    devices.device("native-device", &[("type", "Battery"), ("capacity", "64")]);
    std::os::unix::fs::symlink(devices.0.join("native-device"), supplies.0.join("BAT0")).unwrap();
    let snapshot = supplies.snapshot();
    assert_eq!(snapshot.batteries[0].id, "BAT0");
    assert_eq!(snapshot.batteries[0].charge_percent, Some(64.0));
    fs::remove_dir_all(devices.0.join("native-device")).unwrap();
    assert!(supplies.snapshot().batteries.is_empty());
}
