use std::{cell::Cell, io, sync::mpsc, time::SystemTime};

use super::*;
use crate::services::battery::{BatteryKind, BatteryMetrics, BatteryScope, BatteryState};

fn battery(id: &str, state: BatteryState, percentage: Option<f64>) -> BatteryMetrics {
    BatteryMetrics {
        id: id.into(),
        kind: BatteryKind::Battery,
        scope: BatteryScope::System,
        present: true,
        manufacturer: None,
        model: None,
        technology: None,
        state,
        charge_percent: percentage,
        health: None,
        capacity_health_percent: None,
        cycle_count: None,
        energy_now_wh: None,
        energy_full_wh: None,
        energy_full_design_wh: None,
        energy_empty_wh: None,
        charge_now_ah: None,
        charge_full_ah: None,
        charge_full_design_ah: None,
        charge_empty_ah: None,
        power_watts: None,
        voltage_volts: None,
        current_amps: None,
        temperature_celsius: None,
        time_to_empty: None,
        time_to_full: None,
    }
}

fn snapshot(batteries: Vec<BatteryMetrics>) -> BatterySnapshot {
    BatterySnapshot {
        observed_at: SystemTime::UNIX_EPOCH,
        batteries,
        external_supplies: Vec::new(),
        external_power: Some(false),
    }
}

fn handle(initial: BatterySnapshot, capacity: usize) -> BatteryMonitorHandle {
    BatteryMonitorHandle {
        shared: Arc::new(Shared::new(initial, capacity)),
    }
}

fn drain(events: &mut BatteryEvents) -> Vec<BatteryEvent> {
    std::iter::from_fn(|| events.try_recv()).collect()
}

fn read_error() -> BatteryError {
    BatteryError::Read {
        path: "/fake/battery".into(),
        source: io::Error::from(io::ErrorKind::PermissionDenied),
    }
}

#[test]
fn raw_refreshes_and_fractional_jitter_do_not_invalidate_the_status_signal() {
    let handle = handle(
        snapshot(vec![battery("BAT0", BatteryState::Discharging, Some(50.1))]),
        32,
    );
    let mut events = handle.subscribe().unwrap();
    assert!(drain(&mut events).is_empty());
    let revision = handle.state().revision;
    let mut next = (*handle.snapshot()).clone();
    next.observed_at += Duration::from_secs(2);
    next.batteries[0].charge_percent = Some(50.4);
    next.batteries[0].power_watts = Some(7.5);
    next.batteries[0].temperature_celsius = Some(30.0);
    handle.shared.observe(Ok(next));
    assert_eq!(handle.state().revision, revision);
    assert_eq!(
        handle.snapshot().observed_at,
        SystemTime::UNIX_EPOCH + Duration::from_secs(2)
    );
    assert_eq!(handle.snapshot().batteries[0].charge_percent, Some(50.4));
    assert!(drain(&mut events).is_empty());
    let mut next = (*handle.snapshot()).clone();
    next.batteries[0].charge_percent = Some(50.6);
    handle.shared.observe(Ok(next));
    assert_eq!(handle.state().batteries[0].status.percentage, Some(51));
    assert!(handle.state().revision > revision);
    assert_eq!(
        drain(&mut events),
        vec![BatteryEvent::PercentageChanged {
            id: "BAT0".into(),
            previous: Some(50),
            current: Some(51),
        }]
    );
}

#[test]
fn only_confirmed_state_edges_qualify_for_charging_sounds() {
    let handle = handle(
        snapshot(vec![battery("BAT0", BatteryState::Discharging, Some(50.0))]),
        32,
    );
    let mut events = handle.subscribe().unwrap();
    for (state, starts, stops) in [
        (BatteryState::Charging, true, false),
        (BatteryState::Unknown, false, false),
        (BatteryState::Charging, false, false),
        (BatteryState::Full, false, true),
        (BatteryState::NotCharging, false, false),
    ] {
        handle
            .shared
            .observe(Ok(snapshot(vec![battery("BAT0", state, Some(50.0))])));
        let event = events.try_recv().unwrap();
        assert_eq!(event.started_charging(), starts);
        assert_eq!(event.stopped_charging(), stops);
        assert_eq!(
            event,
            BatteryEvent::StateChanged {
                id: "BAT0".into(),
                previous: match state {
                    BatteryState::Unknown => BatteryState::Charging,
                    BatteryState::Full => BatteryState::Charging,
                    BatteryState::NotCharging => BatteryState::Full,
                    BatteryState::Charging if starts => BatteryState::Discharging,
                    _ => BatteryState::Unknown,
                },
                current: state,
            }
        );
        assert!(events.try_recv().is_none());
    }
}

#[test]
fn outages_retain_last_good_readings_and_recovery_rebaselines_without_sound() {
    let handle = handle(
        snapshot(vec![battery("BAT0", BatteryState::Charging, Some(50.0))]),
        32,
    );
    let mut events = handle.subscribe().unwrap();
    handle.shared.observe(Err(read_error()));
    assert_eq!(
        handle.state().availability,
        BatteryAvailability::Unavailable
    );
    assert!(handle.state().last_error.is_some());
    assert_eq!(handle.snapshot().batteries[0].state, BatteryState::Charging);
    assert_eq!(
        drain(&mut events),
        vec![BatteryEvent::AvailabilityChanged {
            previous: BatteryAvailability::Ready,
            current: BatteryAvailability::Unavailable,
        }]
    );
    let revision = handle.state().revision;
    handle.shared.observe(Err(read_error()));
    assert_eq!(handle.state().revision, revision);
    assert!(events.try_recv().is_none());
    handle.shared.observe(Ok(snapshot(vec![battery(
        "BAT0",
        BatteryState::Full,
        Some(100.0),
    )])));
    assert_eq!(
        drain(&mut events),
        vec![BatteryEvent::AvailabilityChanged {
            previous: BatteryAvailability::Unavailable,
            current: BatteryAvailability::Ready,
        }]
    );
    assert_eq!(handle.state().batteries[0].status.percentage, Some(100));
    assert!(handle.state().last_error.is_none());
    handle.shared.observe(Ok(snapshot(vec![battery(
        "BAT0",
        BatteryState::Charging,
        Some(100.0),
    )])));
    assert!(events.try_recv().unwrap().started_charging());
}

#[test]
fn hotplug_and_absent_slots_do_not_create_charging_edges() {
    let handle = handle(
        snapshot(vec![battery("BAT0", BatteryState::Charging, Some(50.0))]),
        32,
    );
    let mut events = handle.subscribe().unwrap();
    handle.shared.observe(Ok(snapshot(Vec::new())));
    assert_eq!(
        drain(&mut events),
        vec![BatteryEvent::Removed { id: "BAT0".into() }]
    );
    handle.shared.observe(Ok(snapshot(vec![battery(
        "BAT0",
        BatteryState::Charging,
        Some(50.0),
    )])));
    assert_eq!(
        drain(&mut events),
        vec![BatteryEvent::Added { id: "BAT0".into() }]
    );
    let mut absent = battery("BAT0", BatteryState::Charging, Some(50.0));
    absent.present = false;
    handle.shared.observe(Ok(snapshot(vec![absent])));
    assert!(
        drain(&mut events)
            .iter()
            .all(|e| !e.started_charging() && !e.stopped_charging())
    );
    handle.shared.observe(Ok(snapshot(vec![battery(
        "BAT0",
        BatteryState::Charging,
        Some(50.0),
    )])));
    assert!(
        drain(&mut events)
            .iter()
            .all(|e| !e.started_charging() && !e.stopped_charging())
    );
}

#[test]
fn ordering_is_stable_health_updates_publish_and_external_power_remains_independent() {
    let first = battery("A", BatteryState::Full, Some(100.0));
    let second = battery("B", BatteryState::Discharging, Some(50.0));
    let handle = handle(snapshot(vec![second.clone(), first.clone()]), 32);
    let mut events = handle.subscribe().unwrap();
    let revision = handle.state().revision;
    handle.shared.observe(Ok(snapshot(vec![first, second])));
    assert_eq!(handle.state().revision, revision);
    assert!(events.try_recv().is_none());
    let mut next = (*handle.snapshot()).clone();
    next.batteries[0].capacity_health_percent = Some(85.0);
    next.batteries[0].cycle_count = Some(100);
    next.external_power = Some(true);
    handle.shared.observe(Ok(next));
    assert_eq!(
        handle.state().batteries[0]
            .health
            .capacity_retention_percent,
        Some(85.0)
    );
    let event = events.try_recv().unwrap();
    assert_eq!(
        event,
        BatteryEvent::ExternalPowerChanged {
            previous: Some(false),
            connected: Some(true)
        }
    );
    assert!(!event.started_charging());
    assert!(!event.stopped_charging());
}

#[test]
fn queue_overflow_discards_old_sound_events_and_allows_resuming_from_current_state() {
    let handle = handle(
        snapshot(vec![battery("BAT0", BatteryState::Discharging, Some(50.0))]),
        1,
    );
    let mut events = handle.subscribe().unwrap();
    handle.shared.observe(Ok(snapshot(vec![battery(
        "BAT0",
        BatteryState::Charging,
        Some(51.0),
    )])));
    handle.shared.observe(Ok(snapshot(vec![battery(
        "BAT0",
        BatteryState::Full,
        Some(100.0),
    )])));
    assert_eq!(drain(&mut events), vec![BatteryEvent::ResyncRequired]);
    assert_eq!(handle.state().batteries[0].status.state, BatteryState::Full);
    handle.shared.observe(Ok(snapshot(vec![battery(
        "BAT0",
        BatteryState::Charging,
        Some(100.0),
    )])));
    assert!(events.try_recv().unwrap().started_charging());
}

#[test]
fn dropped_subscriptions_release_capacity_and_stopped_handles_reject_new_subscribers() {
    let handle = handle(snapshot(Vec::new()), 1);
    let mut subscriptions = (0..MAX_SUBSCRIPTIONS)
        .map(|_| handle.subscribe().unwrap())
        .collect::<Vec<_>>();
    assert!(matches!(
        handle.subscribe(),
        Err(BatteryError::SubscriberLimitReached)
    ));
    subscriptions.pop();
    assert!(handle.subscribe().is_ok());
    handle.shared.stop.request();
    assert!(matches!(handle.subscribe(), Err(BatteryError::Stopped)));
}

// Cell and Receiver deliberately make this provider Send but not Sync.
struct ControlledProvider {
    initial: BatterySnapshot,
    calls: Cell<usize>,
    owner: Cell<Option<thread::ThreadId>>,
    reading: mpsc::Sender<()>,
    replies: mpsc::Receiver<BatterySnapshot>,
    disposed: mpsc::Sender<()>,
}

impl BatteryProvider for ControlledProvider {
    fn read_snapshot(&self) -> Result<BatterySnapshot, BatteryError> {
        let current = thread::current().id();
        if let Some(owner) = self.owner.get() {
            assert_eq!(
                owner, current,
                "provider reads must remain on the same worker"
            );
        } else {
            self.owner.set(Some(current));
        }
        let call = self.calls.get();
        self.calls.set(call + 1);
        if call == 0 {
            return Ok(self.initial.clone());
        }
        self.reading.send(()).unwrap();
        Ok(self.replies.recv().unwrap())
    }
}

impl Drop for ControlledProvider {
    fn drop(&mut self) {
        let _ = self.disposed.send(());
    }
}

#[test]
fn owner_drop_does_not_join_a_blocked_read_and_late_results_are_discarded() {
    let (reading, reads) = mpsc::channel();
    let (reply, replies) = mpsc::channel();
    let (disposed, disposal) = mpsc::channel();
    let monitor = futures_lite::future::block_on(BatteryMonitor::start_with_provider(
        ControlledProvider {
            initial: snapshot(Vec::new()),
            calls: Cell::new(0),
            owner: Cell::new(None),
            reading,
            replies,
            disposed,
        },
        BatteryMonitorConfig {
            refresh_interval: Duration::from_millis(1),
            event_capacity: 32,
        },
    ))
    .unwrap();
    let handle = monitor.handle();
    let mut events = handle.subscribe().unwrap();
    reads.recv_timeout(Duration::from_secs(5)).unwrap();
    let (dropped, returned) = mpsc::channel();
    let dropper = thread::spawn(move || {
        drop(monitor);
        dropped.send(()).unwrap();
    });
    returned.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(handle.subscribe(), Err(BatteryError::Stopped)));
    reply
        .send(snapshot(vec![battery(
            "late",
            BatteryState::Charging,
            Some(50.0),
        )]))
        .unwrap();
    disposal.recv_timeout(Duration::from_secs(5)).unwrap();
    dropper.join().unwrap();
    assert_eq!(
        futures_lite::future::block_on(events.next()),
        Some(BatteryEvent::AvailabilityChanged {
            previous: BatteryAvailability::Ready,
            current: BatteryAvailability::Stopped,
        })
    );
    assert_eq!(futures_lite::future::block_on(events.next()), None);
    assert!(events.is_closed());
    assert_eq!(handle.state().availability, BatteryAvailability::Stopped);
    assert!(handle.snapshot().batteries.is_empty());
}

struct FixedProvider;
impl BatteryProvider for FixedProvider {
    fn read_snapshot(&self) -> Result<BatterySnapshot, BatteryError> {
        Ok(snapshot(Vec::new()))
    }
}

#[test]
fn shutdown_interrupts_a_long_polling_wait_and_joins_the_worker() {
    let monitor = futures_lite::future::block_on(BatteryMonitor::start_with_provider(
        FixedProvider,
        BatteryMonitorConfig {
            refresh_interval: Duration::from_secs(3600),
            event_capacity: 32,
        },
    ))
    .unwrap();
    let handle = monitor.handle();
    let mut events = handle.subscribe().unwrap();
    futures_lite::future::block_on(monitor.shutdown()).unwrap();
    assert_eq!(handle.state().availability, BatteryAvailability::Stopped);
    assert_eq!(
        drain(&mut events),
        vec![BatteryEvent::AvailabilityChanged {
            previous: BatteryAvailability::Ready,
            current: BatteryAvailability::Stopped,
        }]
    );
    assert_eq!(futures_lite::future::block_on(events.next()), None);
}

#[test]
fn invalid_configuration_is_rejected_before_provider_io() {
    struct MustNotRead;
    impl BatteryProvider for MustNotRead {
        fn read_snapshot(&self) -> Result<BatterySnapshot, BatteryError> {
            panic!("invalid config must not read")
        }
    }
    for config in [
        BatteryMonitorConfig {
            refresh_interval: Duration::ZERO,
            event_capacity: 32,
        },
        BatteryMonitorConfig {
            refresh_interval: Duration::from_secs(2),
            event_capacity: 0,
        },
    ] {
        assert!(matches!(
            futures_lite::future::block_on(BatteryMonitor::start_with_provider(
                MustNotRead,
                config
            )),
            Err(BatteryError::InvalidConfig(_))
        ));
    }
}

#[test]
fn cancelling_startup_releases_the_provider_and_any_started_worker() {
    struct StartupProvider {
        reading: mpsc::Sender<()>,
        proceed: mpsc::Receiver<()>,
        disposed: mpsc::Sender<()>,
    }
    impl BatteryProvider for StartupProvider {
        fn read_snapshot(&self) -> Result<BatterySnapshot, BatteryError> {
            self.reading.send(()).unwrap();
            self.proceed.recv().unwrap();
            Ok(snapshot(Vec::new()))
        }
    }
    impl Drop for StartupProvider {
        fn drop(&mut self) {
            let _ = self.disposed.send(());
        }
    }
    let (reading, started) = mpsc::channel();
    let (proceed, gate) = mpsc::channel();
    let (disposed, disposal) = mpsc::channel();
    let mut startup = Box::pin(BatteryMonitor::start_with_provider(
        StartupProvider {
            reading,
            proceed: gate,
            disposed,
        },
        BatteryMonitorConfig {
            refresh_interval: Duration::from_secs(3600),
            event_capacity: 32,
        },
    ));
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    assert!(std::future::Future::poll(startup.as_mut(), &mut cx).is_pending());
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(startup);
    proceed.send(()).unwrap();
    disposal.recv_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn startup_reports_provider_errors_and_worker_panics() {
    struct FailedProvider;
    impl BatteryProvider for FailedProvider {
        fn read_snapshot(&self) -> Result<BatterySnapshot, BatteryError> {
            Err(BatteryError::Unsupported)
        }
    }
    struct PanickingProvider;
    impl BatteryProvider for PanickingProvider {
        fn read_snapshot(&self) -> Result<BatterySnapshot, BatteryError> {
            panic!("test provider panic")
        }
    }
    assert!(matches!(
        futures_lite::future::block_on(BatteryMonitor::start_with_provider(
            FailedProvider,
            BatteryMonitorConfig::default()
        )),
        Err(BatteryError::Unsupported)
    ));
    assert!(matches!(
        futures_lite::future::block_on(BatteryMonitor::start_with_provider(
            PanickingProvider,
            BatteryMonitorConfig::default()
        )),
        Err(BatteryError::WorkerPanicked)
    ));
}
