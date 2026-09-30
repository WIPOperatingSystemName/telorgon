use std::sync::Arc;

use super::{
    BatteryAvailability, BatteryError, BatteryEvent, BatteryMonitorState, BatterySnapshot,
};
use crate::services::battery::BatterySummary;

pub(super) struct Observation {
    pub(super) latest: Arc<BatterySnapshot>,
    pub(super) status: BatteryMonitorState,
    baseline: Option<Arc<BatterySnapshot>>,
}

impl Observation {
    pub(super) fn new(snapshot: BatterySnapshot) -> Self {
        let snapshot = Arc::new(normalize(snapshot));
        Self {
            status: status(&snapshot),
            latest: snapshot.clone(),
            baseline: Some(snapshot),
        }
    }

    pub(super) fn update(
        &mut self,
        result: Result<BatterySnapshot, BatteryError>,
    ) -> Vec<BatteryEvent> {
        let mut events = Vec::new();
        let availability = self.status.availability;
        match result {
            Err(error) => {
                self.status.availability = BatteryAvailability::Unavailable;
                self.status.last_error = Some(error.to_string());
                // Loss of observation breaks continuity. Recovery supplies a new baseline,
                // avoiding synthetic charging or device transitions across the blind interval.
                self.baseline = None;
            }
            Ok(snapshot) => {
                let snapshot = Arc::new(normalize(snapshot));
                if let Some(previous) = &self.baseline {
                    diff(previous, &snapshot, &mut events);
                }
                self.status = status(&snapshot);
                self.latest = snapshot.clone();
                self.baseline = Some(snapshot);
            }
        }
        if availability != self.status.availability {
            events.insert(
                0,
                BatteryEvent::AvailabilityChanged {
                    previous: availability,
                    current: self.status.availability,
                },
            );
        }
        events
    }

    pub(super) fn resync(
        &mut self,
        result: Result<BatterySnapshot, BatteryError>,
    ) -> Vec<BatteryEvent> {
        // Lost kernel notifications invalidate transition history just like a failed read.
        self.baseline = None;
        self.update(result)
    }
}

fn normalize(mut snapshot: BatterySnapshot) -> BatterySnapshot {
    snapshot.batteries.sort_by(|a, b| a.id.cmp(&b.id));
    snapshot.external_supplies.sort_by(|a, b| a.id.cmp(&b.id));
    snapshot
}

fn status(snapshot: &BatterySnapshot) -> BatteryMonitorState {
    BatteryMonitorState {
        availability: BatteryAvailability::Ready,
        batteries: snapshot
            .batteries
            .iter()
            .map(|battery| BatterySummary {
                id: battery.id.clone(),
                kind: battery.kind,
                scope: battery.scope,
                model: battery.model.clone(),
                status: battery.status(),
                health: battery.health(),
            })
            .collect(),
        external_power: snapshot.external_power,
        last_error: None,
    }
}

fn diff(previous: &BatterySnapshot, current: &BatterySnapshot, events: &mut Vec<BatteryEvent>) {
    for old in &previous.batteries {
        if !current
            .batteries
            .iter()
            .any(|b| b.id == old.id && b.kind == old.kind && b.scope == old.scope)
        {
            events.push(BatteryEvent::Removed { id: old.id.clone() });
        }
    }
    for battery in &current.batteries {
        let Some(old) = previous
            .batteries
            .iter()
            .find(|b| b.id == battery.id && b.kind == battery.kind && b.scope == battery.scope)
        else {
            events.push(BatteryEvent::Added {
                id: battery.id.clone(),
            });
            continue;
        };
        let before = old.status();
        let after = battery.status();
        if before.state != after.state {
            events.push(BatteryEvent::StateChanged {
                id: battery.id.clone(),
                previous: before.state,
                current: after.state,
            });
        }
        if before.percentage != after.percentage {
            events.push(BatteryEvent::PercentageChanged {
                id: battery.id.clone(),
                previous: before.percentage,
                current: after.percentage,
            });
        }
    }
    if previous.external_power != current.external_power {
        events.push(BatteryEvent::ExternalPowerChanged {
            previous: previous.external_power,
            connected: current.external_power,
        });
    }
}
