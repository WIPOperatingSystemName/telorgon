use super::*;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct Fixture {
    snapshot: NetworkSnapshot,
    reads: usize,
    writes: usize,
    finish: bool,
    panic: bool,
    password_received: bool,
}
struct Fake {
    state: Arc<Mutex<Fixture>>,
    pending: Option<NetworkResult>,
}
impl NetworkProvider for Fake {
    fn snapshot(&mut self) -> Result<NetworkSnapshot, NetworkError> {
        let mut state = self.state.lock().unwrap();
        state.reads += 1;
        Ok(state.snapshot.clone())
    }
    fn execute(&mut self, command: NetworkCommand) -> Result<NetworkDispatch, NetworkError> {
        let mut state = self.state.lock().unwrap();
        state.writes += 1;
        if state.panic {
            drop(state);
            panic!("fake backend failure");
        }
        let result = match command {
            NetworkCommand::ScanWifi(id) => NetworkResult::ScanComplete(id),
            NetworkCommand::ConnectWifi { options, .. } => {
                state.password_received = options.credentials.is_some();
                NetworkResult::Connected(NetworkConnectionId::new())
            }
            NetworkCommand::SetWifiEnabled(enabled) => {
                state.snapshot.wifi_enabled = enabled;
                NetworkResult::WifiEnabled(enabled)
            }
            _ => return Err(NetworkError::Unsupported),
        };
        self.pending = Some(result);
        Ok(NetworkDispatch::Pending(1))
    }
    fn poll(&mut self, _: u64, _: &NetworkSnapshot) -> Result<Option<NetworkResult>, NetworkError> {
        Ok(if self.state.lock().unwrap().finish {
            self.pending.clone()
        } else {
            None
        })
    }
    fn abandon(&mut self, _: u64) {
        self.pending = None;
    }
}
fn fixture() -> (NetworkController, Arc<Mutex<Fixture>>, NetworkInterfaceId) {
    let id = NetworkInterfaceId::new();
    let capabilities = NetworkCapabilities {
        set_wifi_enabled: true,
        scan_wifi: true,
        connect_wifi: true,
        disconnect: true,
        edit_profiles: true,
        configure_ip: true,
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(Fixture {
        snapshot: NetworkSnapshot {
            state: NetworkServiceState::Ready,
            capabilities: capabilities.clone(),
            interfaces: vec![NetworkInterface {
                device: Default::default(),
                id,
                name: "wifi-test".into(),
                kind: NetworkInterfaceKind::Wifi,
                managed: true,
                state: NetworkConnectionState::Disconnected,
                failure: None,
                capabilities,
                active_connection: None,
                ipv4: Default::default(),
                ipv6: Default::default(),
                access_points: Vec::new(),
                active_access_point: None,
            }],
            ..Default::default()
        },
        reads: 0,
        writes: 0,
        finish: false,
        panic: false,
        password_received: false,
    }));
    let config = NetworkConfig {
        poll_interval: Duration::from_millis(10),
        request_timeout: Duration::from_secs(2),
        queue_capacity: 8,
    };
    let controller = NetworkController::with_provider(
        config,
        Fake {
            state: state.clone(),
            pending: None,
        },
    )
    .unwrap();
    (controller, state, id)
}
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "test condition timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn start(controller: &mut NetworkController) {
    controller.start().unwrap();
    until(|| controller.observer().signal().snapshot().state == NetworkServiceState::Ready);
}

#[test]
fn construction_is_idle_and_confirmation_waits_for_backend_observation() {
    let (mut controller, state, id) = fixture();
    assert_eq!(state.lock().unwrap().reads, 0);
    let handle = controller.handle();
    assert_eq!(handle.scan_wifi(id).unwrap_err(), NetworkError::Unavailable);
    start(&mut controller);
    let request = handle.scan_wifi(id).unwrap();
    until(|| state.lock().unwrap().writes == 1);
    assert_eq!(request.state(), NetworkRequestState::Dispatched);
    state.lock().unwrap().finish = true;
    assert_eq!(
        futures_lite::future::block_on(request.completion()),
        NetworkOutcome::Applied(NetworkResult::ScanComplete(id))
    );
}
#[test]
fn missing_credentials_prompt_separately_and_resume_the_same_request() {
    let (mut controller, state, id) = fixture();
    start(&mut controller);
    let handle = controller.handle();
    let observer = controller.observer();
    let request = handle
        .connect_wifi(
            id,
            WifiConnectOptions::new(
                Ssid::new(b"Cafe".to_vec()).unwrap(),
                WifiSecurity::WpaPersonal,
            ),
        )
        .unwrap();
    until(|| !observer.challenges().snapshot().is_empty());
    assert_eq!(request.state(), NetworkRequestState::AwaitingCredentials);
    assert_eq!(observer.challenges().snapshot()[0].request, request.id());
    assert_eq!(state.lock().unwrap().writes, 0);
    assert!(
        handle
            .supply_credentials(request.id(), NetworkSecret::new("short"))
            .is_err()
    );
    handle
        .supply_credentials(request.id(), NetworkSecret::new("test-password"))
        .unwrap();
    until(|| state.lock().unwrap().password_received);
    until(|| observer.challenges().snapshot().is_empty());
    assert!(!format!("{:?}", *observer.signal().snapshot()).contains("test-password"));
    assert_eq!(
        format!("{:?}", NetworkSecret::new("test-password")),
        "NetworkSecret([redacted])"
    );
    state.lock().unwrap().finish = true;
    assert!(matches!(
        futures_lite::future::block_on(request.completion()),
        NetworkOutcome::Applied(NetworkResult::Connected(_))
    ));
}
#[test]
fn cancelling_a_queued_request_does_not_mutate_the_backend() {
    let (mut controller, state, id) = fixture();
    start(&mut controller);
    let handle = controller.handle();
    let active = handle.scan_wifi(id).unwrap();
    until(|| state.lock().unwrap().writes == 1);
    let queued = handle.scan_wifi(id).unwrap();
    assert_eq!(
        queued.cancel(),
        NetworkCancellation::CancelledBeforeDispatch
    );
    assert_eq!(active.cancel(), NetworkCancellation::TooLate);
    assert_eq!(
        futures_lite::future::block_on(queued.completion()),
        NetworkOutcome::Cancelled
    );
    state.lock().unwrap().finish = true;
    let _ = futures_lite::future::block_on(active.completion());
    assert_eq!(state.lock().unwrap().writes, 1);
}
#[test]
fn stale_and_unsupported_targets_fail_without_native_writes() {
    let (mut controller, state, id) = fixture();
    start(&mut controller);
    let handle = controller.handle();
    assert_eq!(
        handle.renew_dhcp(id, IpFamily::V4).unwrap_err(),
        NetworkError::Unsupported
    );
    assert_eq!(
        handle.flush_dns_cache().unwrap_err(),
        NetworkError::Unsupported
    );
    assert_eq!(
        handle.scan_wifi(NetworkInterfaceId::new()).unwrap_err(),
        NetworkError::Stale
    );
    state.lock().unwrap().snapshot.interfaces.clear();
    until(|| handle.signal().snapshot().interfaces.is_empty());
    assert_eq!(handle.scan_wifi(id).unwrap_err(), NetworkError::Stale);
    assert_eq!(state.lock().unwrap().writes, 0);
}
#[test]
fn queued_targets_are_revalidated_before_dispatch() {
    let (mut controller, state, id) = fixture();
    start(&mut controller);
    let handle = controller.handle();
    let active = handle.scan_wifi(id).unwrap();
    until(|| state.lock().unwrap().writes == 1);
    let queued = handle.scan_wifi(id).unwrap();
    {
        let mut state = state.lock().unwrap();
        state.snapshot.interfaces.clear();
        state.finish = true;
    }
    let _ = futures_lite::future::block_on(active.completion());
    assert_eq!(
        futures_lite::future::block_on(queued.completion()),
        NetworkOutcome::Failed(NetworkError::Stale)
    );
    assert_eq!(state.lock().unwrap().writes, 1);
}
#[test]
fn dropping_owner_stops_surviving_handles_and_finishes_pending_requests() {
    let (mut controller, state, id) = fixture();
    start(&mut controller);
    let handle = controller.handle();
    let request = handle.scan_wifi(id).unwrap();
    until(|| state.lock().unwrap().writes == 1);
    drop(controller);
    assert_eq!(
        futures_lite::future::block_on(request.completion()),
        NetworkOutcome::Unconfirmed(NetworkError::Stopped)
    );
    assert_eq!(
        handle.signal().snapshot().state,
        NetworkServiceState::Stopped
    );
    assert_eq!(handle.scan_wifi(id).unwrap_err(), NetworkError::Stopped);
}
#[test]
fn backend_panics_terminate_requests_instead_of_leaving_waiters_hanging() {
    let (mut controller, state, id) = fixture();
    start(&mut controller);
    state.lock().unwrap().panic = true;
    let request = controller.handle().scan_wifi(id).unwrap();
    until(|| controller.observer().signal().snapshot().state == NetworkServiceState::Stopped);
    assert_eq!(
        futures_lite::future::block_on(request.completion()),
        NetworkOutcome::Unconfirmed(NetworkError::BackendFailure)
    );
}
#[test]
fn timed_out_dispatched_requests_are_unconfirmed_and_never_reported_applied() {
    let (mut controller, _, id) = fixture();
    Arc::get_mut(&mut controller.shared)
        .unwrap()
        .config
        .request_timeout = Duration::from_millis(120);
    start(&mut controller);
    let request = controller.handle().scan_wifi(id).unwrap();
    assert_eq!(
        futures_lite::future::block_on(request.completion()),
        NetworkOutcome::Unconfirmed(NetworkError::TimedOut)
    );
}
#[test]
fn ip_validation_rejects_invalid_prefixes_families_and_empty_static_settings() {
    assert!(IpAddress::new("10.0.0.1".parse().unwrap(), 33).is_err());
    assert!(IpAddress::new("::1".parse().unwrap(), 129).is_err());
    assert!(
        IpSettings {
            method: IpMethod::Static(Vec::new()),
            ..Default::default()
        }
        .validate(IpFamily::V4)
        .is_err()
    );
    assert!(
        IpSettings {
            dns: vec!["::1".parse().unwrap()],
            ..Default::default()
        }
        .validate(IpFamily::V4)
        .is_err()
    );
    assert!(
        IpSettings {
            method: IpMethod::Disabled,
            gateway: Some("10.0.0.1".parse().unwrap()),
            ..Default::default()
        }
        .validate(IpFamily::V4)
        .is_err()
    );
    assert!(Ssid::new(Vec::new()).is_err());
    assert!(Ssid::new(vec![b'a'; 33]).is_err());
    assert_eq!(
        Ssid::new(vec![0xff, 0xfe]).unwrap().as_bytes(),
        &[0xff, 0xfe]
    );
}

#[test]
fn queue_capacity_includes_dispatched_requests_and_releases_after_completion() {
    let (mut controller, state, id) = fixture();
    Arc::get_mut(&mut controller.shared)
        .unwrap()
        .config
        .queue_capacity = 1;
    start(&mut controller);
    let handle = controller.handle();
    let request = handle.scan_wifi(id).unwrap();
    until(|| state.lock().unwrap().writes == 1);
    assert_eq!(handle.scan_wifi(id).unwrap_err(), NetworkError::Busy);
    state.lock().unwrap().finish = true;
    let _ = futures_lite::future::block_on(request.completion());
    until(|| controller.shared.lock().active.is_none());
    assert!(handle.scan_wifi(id).is_ok());
}
#[test]
fn cancelling_a_password_prompt_closes_it_without_dispatching_or_accepting_late_secrets() {
    let (mut controller, state, id) = fixture();
    start(&mut controller);
    let handle = controller.handle();
    let request = handle
        .connect_wifi(
            id,
            WifiConnectOptions::new(
                Ssid::new(b"Cafe".to_vec()).unwrap(),
                WifiSecurity::WpaPersonal,
            ),
        )
        .unwrap();
    until(|| !handle.observer().challenges().snapshot().is_empty());
    assert_eq!(
        request.cancel(),
        NetworkCancellation::CancelledBeforeDispatch
    );
    until(|| handle.observer().challenges().snapshot().is_empty());
    assert_eq!(
        handle.supply_credentials(request.id(), NetworkSecret::new("test-password")),
        Err(NetworkError::Stale)
    );
    assert_eq!(state.lock().unwrap().writes, 0);
}
