//! Linux kernel power-supply adapter. All native attributes and units stay in this module.
//! Attribute semantics: <https://docs.kernel.org/power/power_supply_class.html>.

use std::{
    fs, io,
    path::Path,
    time::{Duration, SystemTime},
};

use super::{
    BatteryError, BatteryEstimateSource, BatteryKind, BatteryMetrics, BatteryScope,
    BatterySnapshot, BatteryState, BatteryTimeEstimate, ExternalPowerSupply,
};

pub(super) fn read_snapshot(root: &Path) -> Result<BatterySnapshot, BatteryError> {
    let entries = fs::read_dir(root).map_err(|source| error(root, source))?;
    let mut paths = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| error(root, source))?;
    paths.sort();
    let mut batteries = Vec::new();
    let mut external_supplies = Vec::new();
    for path in paths {
        // Class entries are normally symlinks. Follow them through normal attribute reads.
        let Some(kind) = text(&path, "type")? else {
            continue;
        };
        let scope = scope(text(&path, "scope")?.as_deref());
        let id = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if kind == "Battery" || kind == "UPS" {
            let battery = read_battery(&path, id, kind == "UPS", scope)?;
            // Hot-unplug during the read must not leave a phantom device in the result.
            if text(&path, "type")?.is_some() {
                batteries.push(battery);
            }
        } else if kind == "Mains" || kind == "Wireless" || kind.starts_with("USB") {
            let online = number(&path, "online")?.and_then(|value| match value {
                0 => Some(false),
                1 | 2 => Some(true), // Fixed and programmable USB supplies are both online.
                _ => None,
            });
            if text(&path, "type")?.is_some() {
                external_supplies.push(ExternalPowerSupply {
                    id,
                    kind,
                    scope,
                    online,
                });
            }
        }
    }
    let supplies = external_supplies
        .iter()
        .filter(|s| s.scope != BatteryScope::Device)
        .collect::<Vec<_>>();
    let external_power = if supplies.iter().any(|s| s.online == Some(true)) {
        Some(true)
    } else if !supplies.is_empty() && supplies.iter().all(|s| s.online == Some(false)) {
        Some(false)
    } else {
        None
    };
    Ok(BatterySnapshot {
        observed_at: SystemTime::now(),
        batteries,
        external_supplies,
        external_power,
    })
}

fn read_battery(
    path: &Path,
    id: String,
    ups: bool,
    scope: BatteryScope,
) -> Result<BatteryMetrics, BatteryError> {
    // Per the ABI, omission of `present` means present. An absent slot retains its identity,
    // but none of its stale charge or state readings are exposed.
    let present = number(path, "present")? != Some(0);
    let state = if present {
        state(text(path, "status")?.as_deref())
    } else {
        BatteryState::Unknown
    };
    let read = |name| {
        if present {
            number(path, name)
        } else {
            Ok(None)
        }
    };
    let nonnegative =
        |name| read(name).map(|v| v.filter(|v| *v >= 0).map(|v| v as f64 / 1_000_000.0));
    let energy_now_wh = nonnegative("energy_now")?;
    let energy_full_wh = nonnegative("energy_full")?;
    let energy_full_design_wh = nonnegative("energy_full_design")?;
    let energy_empty_wh = nonnegative("energy_empty")?;
    let charge_now_ah = nonnegative("charge_now")?;
    let charge_full_ah = nonnegative("charge_full")?;
    let charge_full_design_ah = nonnegative("charge_full_design")?;
    let charge_empty_ah = nonnegative("charge_empty")?;
    let voltage_volts = nonnegative("voltage_now")?;
    let current_amps = read("current_now")?.map(|v| v as f64 / 1_000_000.0);
    let power_watts = read("power_now")?
        .map(|v| (v as f64).abs() / 1_000_000.0)
        .or_else(|| Some(voltage_volts? * current_amps?.abs()));
    let charge_percent = read("capacity")?
        .filter(|v| (0..=100).contains(v))
        .map(|v| v as f64)
        .or_else(|| capacity(energy_now_wh, energy_full_wh, energy_empty_wh))
        .or_else(|| capacity(charge_now_ah, charge_full_ah, charge_empty_ah));
    let capacity_health_percent = ratio(energy_full_wh, energy_full_design_wh)
        .or_else(|| ratio(charge_full_ah, charge_full_design_ah));
    let time_to_empty = if state == BatteryState::Discharging {
        reported_time(path, "time_to_empty_now", "time_to_empty_avg")?
            .or_else(|| estimate(usable(energy_now_wh, energy_empty_wh), power_watts))
            .or_else(|| {
                estimate(
                    usable(charge_now_ah, charge_empty_ah),
                    current_amps.map(f64::abs),
                )
            })
    } else {
        None
    };
    let time_to_full = if state == BatteryState::Charging {
        reported_time(path, "time_to_full_now", "time_to_full_avg")?
            .or_else(|| estimate(difference(energy_full_wh, energy_now_wh), power_watts))
            .or_else(|| {
                estimate(
                    difference(charge_full_ah, charge_now_ah),
                    current_amps.map(f64::abs),
                )
            })
    } else {
        None
    };
    Ok(BatteryMetrics {
        id,
        kind: if ups {
            BatteryKind::Ups
        } else {
            BatteryKind::Battery
        },
        scope,
        present,
        manufacturer: text(path, "manufacturer")?,
        model: text(path, "model_name")?,
        technology: text(path, "technology")?,
        state,
        charge_percent,
        health: if present { text(path, "health")? } else { None },
        capacity_health_percent,
        cycle_count: read("cycle_count")?.filter(|v| *v > 0).map(|v| v as u64),
        energy_now_wh,
        energy_full_wh,
        energy_full_design_wh,
        energy_empty_wh,
        charge_now_ah,
        charge_full_ah,
        charge_full_design_ah,
        charge_empty_ah,
        power_watts,
        voltage_volts,
        current_amps,
        temperature_celsius: read("temp")?.map(|v| v as f64 / 10.0),
        time_to_empty,
        time_to_full,
    })
}

fn scope(value: Option<&str>) -> BatteryScope {
    match value {
        Some("System") => BatteryScope::System,
        Some("Device") => BatteryScope::Device,
        _ => BatteryScope::Unknown,
    }
}

fn state(value: Option<&str>) -> BatteryState {
    match value {
        Some("Charging") => BatteryState::Charging,
        Some("Discharging") => BatteryState::Discharging,
        Some("Not charging") => BatteryState::NotCharging,
        Some("Full") => BatteryState::Full,
        _ => BatteryState::Unknown,
    }
}

fn ratio(value: Option<f64>, full: Option<f64>) -> Option<f64> {
    let full = full.filter(|v| *v > 0.0)?;
    Some(value? / full * 100.0).filter(|v| v.is_finite())
}

fn usable(value: Option<f64>, empty: Option<f64>) -> Option<f64> {
    Some((value? - empty.unwrap_or(0.0)).max(0.0))
}

fn difference(full: Option<f64>, now: Option<f64>) -> Option<f64> {
    Some((full? - now?).max(0.0))
}

fn capacity(now: Option<f64>, full: Option<f64>, empty: Option<f64>) -> Option<f64> {
    ratio(usable(now, empty), usable(full, empty)).map(|v| v.clamp(0.0, 100.0))
}

fn estimate(remaining: Option<f64>, rate: Option<f64>) -> Option<BatteryTimeEstimate> {
    let seconds = remaining? / rate.filter(|r| *r > 0.0)? * 3600.0;
    Some(BatteryTimeEstimate {
        duration: Duration::try_from_secs_f64(seconds).ok()?,
        source: BatteryEstimateSource::Calculated,
    })
}

fn reported_time(
    path: &Path,
    now: &str,
    average: &str,
) -> Result<Option<BatteryTimeEstimate>, BatteryError> {
    for name in [now, average] {
        if let Some(seconds) = number(path, name)?.filter(|v| *v > 0) {
            return Ok(Some(BatteryTimeEstimate {
                duration: Duration::from_secs(seconds as u64),
                source: BatteryEstimateSource::Reported,
            }));
        }
    }
    Ok(None)
}

fn number(path: &Path, name: &str) -> Result<Option<i64>, BatteryError> {
    Ok(text(path, name)?.and_then(|value| value.parse().ok()))
}

fn text(path: &Path, name: &str) -> Result<Option<String>, BatteryError> {
    let path = path.join(name);
    match fs::read_to_string(&path) {
        Ok(value) => Ok(Some(value.trim().to_owned()).filter(|v| !v.is_empty())),
        Err(source)
            if source.kind() == io::ErrorKind::NotFound
                || matches!(
                    source.raw_os_error(),
                    Some(libc::ENODEV | libc::ENODATA | libc::ENXIO | libc::EINVAL)
                ) =>
        {
            Ok(None)
        }
        Err(source) => Err(error(&path, source)),
    }
}

fn error(path: &Path, source: io::Error) -> BatteryError {
    BatteryError::Read {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests;
