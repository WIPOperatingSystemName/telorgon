use super::{BatteryKind, BatteryMetrics, BatteryScope, BatteryState, BatteryTimeEstimate};

/// Whole-percent status for UI display; raw fractional capacity remains in `BatteryMetrics`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatteryStatus {
    pub present: bool,
    pub percentage: Option<u8>,
    pub state: BatteryState,
    pub time_to_empty: Option<BatteryTimeEstimate>,
    pub time_to_full: Option<BatteryTimeEstimate>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatteryHealth {
    pub capacity_retention_percent: Option<f64>,
    pub cycle_count: Option<u64>,
    pub condition: Option<String>,
}

impl BatteryMetrics {
    pub fn status(&self) -> BatteryStatus {
        BatteryStatus {
            present: self.present,
            percentage: self
                .present
                .then_some(self.charge_percent)
                .flatten()
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                .map(|v| v.round() as u8),
            state: if self.present {
                self.state
            } else {
                BatteryState::Unknown
            },
            time_to_empty: if self.present && self.state == BatteryState::Discharging {
                self.time_to_empty
            } else {
                None
            },
            time_to_full: if self.present && self.state == BatteryState::Charging {
                self.time_to_full
            } else {
                None
            },
        }
    }

    pub fn health(&self) -> BatteryHealth {
        BatteryHealth {
            capacity_retention_percent: self
                .present
                .then_some(self.capacity_health_percent)
                .flatten()
                .filter(|v| v.is_finite() && *v >= 0.0),
            cycle_count: if self.present { self.cycle_count } else { None },
            condition: if self.present {
                self.health.clone()
            } else {
                None
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatterySummary {
    pub id: String,
    pub kind: BatteryKind,
    pub scope: BatteryScope,
    pub model: Option<String>,
    pub status: BatteryStatus,
    pub health: BatteryHealth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryAvailability {
    Ready,
    Unavailable,
    Stopped,
}

/// Stable UI projection, without observation timestamps or noisy electrical readings.
/// During an outage or after shutdown, the batteries and external power retain the last
/// successful status. Check `availability` before treating these readings as current.
#[derive(Clone, Debug, PartialEq)]
pub struct BatteryMonitorState {
    pub availability: BatteryAvailability,
    pub batteries: Vec<BatterySummary>,
    pub external_power: Option<bool>,
    pub last_error: Option<String>,
}

/// Changes observed between successful refreshes. Coalescing or polling may miss transitions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatteryEvent {
    Added {
        id: String,
    },
    Removed {
        id: String,
    },
    StateChanged {
        id: String,
        previous: BatteryState,
        current: BatteryState,
    },
    PercentageChanged {
        id: String,
        previous: Option<u8>,
        current: Option<u8>,
    },
    ExternalPowerChanged {
        previous: Option<bool>,
        connected: Option<bool>,
    },
    AvailabilityChanged {
        previous: BatteryAvailability,
        current: BatteryAvailability,
    },
    /// Queued changes were lost. Read the handle's latest state and discard incremental history.
    /// This event does not describe a charging transition and should not trigger a sound.
    ResyncRequired,
}

impl BatteryEvent {
    /// True only when the same observed battery moves from a known non-charging state to charging.
    /// Startup, recovery, discovery, removal, and unknown states never qualify.
    pub fn started_charging(&self) -> bool {
        matches!(self, Self::StateChanged { previous, current: BatteryState::Charging, .. }
            if *previous != BatteryState::Unknown && *previous != BatteryState::Charging)
    }

    /// Includes reaching full charge or a charging limit; use external-power events for unplugging.
    pub fn stopped_charging(&self) -> bool {
        matches!(self, Self::StateChanged { previous: BatteryState::Charging, current, .. }
            if *current != BatteryState::Unknown && *current != BatteryState::Charging)
    }
}
