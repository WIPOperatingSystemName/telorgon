use super::*;
use crate::{platform::contracts::PermissionState, shell::OutputId};
use std::{
    sync::{Arc, Condvar, Mutex, mpsc},
    time::{Duration, Instant},
};
struct Native {
    values: [u32; 2],
    writes: Vec<u32>,
    maximum: u32,
    block: bool,
    mismatch: bool,
}
struct Fake {
    native: Arc<Mutex<Native>>,
    gate: Arc<(Mutex<bool>, Condvar)>,
    entered: mpsc::SyncSender<()>,
}
impl ScreenBrightnessProvider for Fake {
    fn discover(&mut self) -> Result<Vec<ScreenBrightnessProviderDevice>, ScreenBrightnessError> {
        Ok((0..2)
            .map(|index| ScreenBrightnessProviderDevice {
                key: index.to_string(),
                name: format!("Panel {index}"),
                kind: ScreenBrightnessKind::InternalBacklight,
                association: ScreenBrightnessAssociation::HostConfigured(vec![
                    OutputId::from_raw(index + 1).unwrap(),
                ]),
                maximum: self.native.lock().unwrap().maximum,
                permission: PermissionState::Granted,
                verification: ScreenBrightnessVerification::Provider,
            })
            .collect())
    }
    fn read(&mut self, key: &str) -> Result<ScreenBrightnessReading, ScreenBrightnessError> {
        let index: usize = key
            .parse()
            .map_err(|_| ScreenBrightnessError::StaleDevice)?;
        Ok(ScreenBrightnessReading {
            configured: self.native.lock().unwrap().values[index],
            actual: None,
        })
    }
    fn set(&mut self, key: &str, value: u32) -> Result<(), ScreenBrightnessError> {
        let blocked = self.native.lock().unwrap().block;
        if blocked {
            let _ = self.entered.try_send(());
            let (lock, wake) = &*self.gate;
            let gate = lock.lock().unwrap();
            let _ = wake
                .wait_timeout_while(gate, Duration::from_secs(3), |open| !*open)
                .unwrap();
        }
        let mut native = self.native.lock().unwrap();
        native.writes.push(value);
        if !native.mismatch {
            native.values[key.parse::<usize>().unwrap()] = value;
        }
        Ok(())
    }
}
struct Fixture {
    owner: ScreenBrightnessController,
    handle: ScreenBrightnessHandle,
    native: Arc<Mutex<Native>>,
    gate: Arc<(Mutex<bool>, Condvar)>,
    entered: mpsc::Receiver<()>,
}
impl Fixture {
    fn new(config: ScreenBrightnessConfig, maximum: u32) -> Self {
        let native = Arc::new(Mutex::new(Native {
            values: [maximum * 40 / 100, maximum * 20 / 100],
            writes: vec![],
            maximum,
            block: false,
            mismatch: false,
        }));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (entered, receiver) = mpsc::sync_channel(1);
        let mut owner = ScreenBrightnessController::with_provider(
            config,
            Fake {
                native: native.clone(),
                gate: gate.clone(),
                entered,
            },
        )
        .unwrap();
        owner.set_host_state(ScreenBrightnessHostState {
            active: true,
            locked: false,
        });
        let handle = owner.handle();
        owner.start().unwrap();
        wait(|| handle.signal().snapshot().devices.len() == 2);
        Self {
            owner,
            handle,
            native,
            gate,
            entered: receiver,
        }
    }
    fn set(&self, percent: f32) -> ScreenBrightnessRequest {
        self.handle
            .execute(
                ScreenBrightnessTarget::Output(OutputId::MIN),
                ScreenBrightnessAction::Set(ScreenBrightnessLevel::percent(percent).unwrap()),
            )
            .unwrap()
    }
    fn block(&self) {
        self.native.lock().unwrap().block = true;
    }
    fn release(&self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.release();
        self.owner.stop();
    }
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(2);
    while !predicate() {
        assert!(Instant::now() < end, "timed out waiting for observed state");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn complete(request: ScreenBrightnessRequest) -> ScreenBrightnessOutcome {
    futures_lite::future::block_on(request.completion())
}
#[test]
fn values_reject_nonfinite_and_invalid_deserialization() {
    for value in [f32::NAN, f32::INFINITY, -1.0, 101.0] {
        assert!(ScreenBrightnessLevel::percent(value).is_err());
    }
    let de = serde::de::value::U16Deserializer::<serde::de::value::Error>::new(10_001);
    assert!(<ScreenBrightnessLevel as serde::Deserialize>::deserialize(de).is_err());
    assert_eq!(
        ScreenBrightnessDelta::percentage_points(-5.0)
            .unwrap()
            .as_basis_points(),
        -500
    );
}
#[test]
fn ordered_adjustments_accumulate_and_completion_sees_published_state() {
    let f = Fixture::new(Default::default(), 100);
    let action =
        ScreenBrightnessAction::Adjust(ScreenBrightnessDelta::percentage_points(5.0).unwrap());
    let first = f
        .handle
        .execute(ScreenBrightnessTarget::Output(OutputId::MIN), action)
        .unwrap();
    let second = f
        .handle
        .execute(ScreenBrightnessTarget::Output(OutputId::MIN), action)
        .unwrap();
    assert!(matches!(
        complete(first),
        ScreenBrightnessOutcome::Applied(_)
    ));
    let ScreenBrightnessOutcome::Applied(applied) = complete(second) else {
        panic!("adjustment did not apply");
    };
    assert_eq!(applied.level.as_percent(), 50.0);
    assert!(f.handle.signal().snapshot().revision >= applied.revision);
    assert_eq!(f.native.lock().unwrap().writes, [45, 50]);
}
#[test]
fn scoped_handles_and_host_revocation_cannot_dispatch_queued_work() {
    let f = Fixture::new(Default::default(), 100);
    let device = f.handle.signal().snapshot().devices[0].device;
    let scoped = f.handle.restrict_to(&[device]).unwrap();
    assert!(matches!(
        scoped.execute(
            ScreenBrightnessTarget::Output(OutputId::from_raw(2).unwrap()),
            ScreenBrightnessAction::Set(ScreenBrightnessLevel::MAX)
        ),
        Err(ScreenBrightnessError::PermissionDenied)
    ));
    f.block();
    let active = f.set(60.0);
    f.entered.recv_timeout(Duration::from_secs(1)).unwrap();
    let queued = scoped
        .execute(
            ScreenBrightnessTarget::Device(device),
            ScreenBrightnessAction::Set(ScreenBrightnessLevel::MAX),
        )
        .unwrap();
    f.owner.revoke(&scoped).unwrap();
    assert_eq!(complete(queued), ScreenBrightnessOutcome::Stale);
    f.owner.set_host_state(ScreenBrightnessHostState {
        active: false,
        locked: false,
    });
    assert!(matches!(
        complete(active),
        ScreenBrightnessOutcome::Unconfirmed {
            reason: ScreenBrightnessConfirmationFailure::AuthorityLost,
            ..
        }
    ));
    assert!(matches!(
        f.handle.execute(
            ScreenBrightnessTarget::Device(device),
            ScreenBrightnessAction::Set(ScreenBrightnessLevel::MAX)
        ),
        Err(ScreenBrightnessError::SessionInactive)
    ));
    f.release();
}
#[test]
fn sliders_supersede_only_adjacent_pending_sets_and_cancellation_prevents_dispatch() {
    let f = Fixture::new(Default::default(), 100);
    f.block();
    let active = f.set(60.0);
    f.entered.recv_timeout(Duration::from_secs(1)).unwrap();
    let set = |value| {
        f.handle
            .execute_with_mode(
                ScreenBrightnessTarget::Output(OutputId::MIN),
                ScreenBrightnessAction::Set(ScreenBrightnessLevel::percent(value).unwrap()),
                ScreenBrightnessWriteMode::ReplacePendingSet,
            )
            .unwrap()
    };
    let old = set(70.0);
    let newest = set(80.0);
    assert_eq!(complete(old), ScreenBrightnessOutcome::Superseded);
    assert_eq!(
        newest.cancel(),
        ScreenBrightnessCancellation::CancelledBeforeDispatch
    );
    assert_eq!(complete(newest), ScreenBrightnessOutcome::Cancelled);
    f.release();
    assert!(matches!(
        complete(active),
        ScreenBrightnessOutcome::Applied(_)
    ));
    wait(|| f.handle.signal().snapshot().pending == 0);
    assert_eq!(f.native.lock().unwrap().writes, [60]);
}
#[test]
fn deadline_keeps_capacity_reserved_until_native_io_finishes() {
    let f = Fixture::new(
        ScreenBrightnessConfig {
            request_timeout: Duration::from_millis(100),
            queue_capacity: 1,
            ..Default::default()
        },
        100,
    );
    f.block();
    let active = f.set(60.0);
    f.entered.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(
        complete(active),
        ScreenBrightnessOutcome::Unconfirmed {
            reason: ScreenBrightnessConfirmationFailure::Timeout,
            ..
        }
    ));
    assert!(matches!(
        f.handle.execute(
            ScreenBrightnessTarget::Output(OutputId::MIN),
            ScreenBrightnessAction::Set(ScreenBrightnessLevel::MAX)
        ),
        Err(ScreenBrightnessError::QueueFull)
    ));
    f.release();
    wait(|| f.handle.signal().snapshot().pending == 0);
}
#[test]
fn readback_mismatch_is_not_reported_as_applied() {
    let f = Fixture::new(Default::default(), 100);
    f.native.lock().unwrap().mismatch = true;
    assert!(matches!(
        complete(f.set(60.0)),
        ScreenBrightnessOutcome::Unconfirmed {
            reason: ScreenBrightnessConfirmationFailure::ReadbackMismatch,
            ..
        }
    ));
}
#[test]
fn coarse_devices_keep_nonzero_floor_and_move_at_least_one_native_step() {
    let f = Fixture::new(Default::default(), 16);
    assert!(matches!(
        f.handle.execute(
            ScreenBrightnessTarget::Output(OutputId::MIN),
            ScreenBrightnessAction::Set(ScreenBrightnessLevel::ZERO)
        ),
        Err(ScreenBrightnessError::BelowMinimum)
    ));
    let request = f
        .handle
        .execute(
            ScreenBrightnessTarget::Output(OutputId::MIN),
            ScreenBrightnessAction::Adjust(ScreenBrightnessDelta::percentage_points(0.1).unwrap()),
        )
        .unwrap();
    assert!(matches!(
        complete(request),
        ScreenBrightnessOutcome::Applied(_)
    ));
    assert_eq!(f.native.lock().unwrap().values[0], 7);
}
#[test]
fn firmware_keys_do_not_write_and_release_stops_shell_repeats() {
    let f = Fixture::new(Default::default(), 100);
    let target = ScreenBrightnessTarget::Output(OutputId::MIN);
    f.handle
        .key_press(
            target,
            10,
            true,
            ScreenBrightnessKeyConfig {
                handling: ScreenBrightnessKeyHandling::FirmwareAdjusts,
                ..Default::default()
            },
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(f.native.lock().unwrap().writes.is_empty());
    f.handle
        .key_press(
            target,
            10,
            true,
            ScreenBrightnessKeyConfig {
                repeat_delay: Duration::from_millis(100),
                repeat_interval: Duration::from_millis(50),
                ..Default::default()
            },
        )
        .unwrap();
    wait(|| f.native.lock().unwrap().values[0] >= 50);
    f.owner.release_key(10);
    wait(|| f.handle.signal().snapshot().pending == 0);
    let count = f.native.lock().unwrap().writes.len();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(f.native.lock().unwrap().writes.len(), count);
}
#[test]
fn quick_key_release_preserves_the_admitted_initial_adjustment() {
    let f = Fixture::new(Default::default(), 100);
    f.block();
    let active = f.set(60.0);
    f.entered.recv_timeout(Duration::from_secs(1)).unwrap();
    f.handle
        .key_press(
            ScreenBrightnessTarget::Output(OutputId::MIN),
            10,
            true,
            Default::default(),
        )
        .unwrap();
    f.owner.release_key(10);
    f.release();
    assert!(matches!(
        complete(active),
        ScreenBrightnessOutcome::Applied(_)
    ));
    wait(|| f.handle.signal().snapshot().pending == 0);
    assert_eq!(f.native.lock().unwrap().writes, [60, 65]);
}
#[test]
fn stale_foreign_device_and_ambiguous_default_are_rejected() {
    let first = Fixture::new(Default::default(), 100);
    let second = Fixture::new(Default::default(), 100);
    let foreign = second.handle.signal().snapshot().devices[0].device;
    assert!(matches!(
        first.handle.execute(
            ScreenBrightnessTarget::Device(foreign),
            ScreenBrightnessAction::Set(ScreenBrightnessLevel::MAX)
        ),
        Err(ScreenBrightnessError::StaleDevice)
    ));
    assert!(matches!(
        first.handle.execute(
            ScreenBrightnessTarget::DefaultInternal,
            ScreenBrightnessAction::Set(ScreenBrightnessLevel::MAX)
        ),
        Err(ScreenBrightnessError::AmbiguousTarget)
    ));
}

#[test]
fn revoking_parent_revokes_descendants_and_queued_requests() {
    let f = Fixture::new(Default::default(), 100);
    let device = f.handle.signal().snapshot().devices[0].device;
    let parent = f.handle.restrict_to(&[device]).unwrap();
    let child = parent.restrict_to(&[device]).unwrap();
    f.block();
    let active = f.set(60.0);
    f.entered.recv_timeout(Duration::from_secs(1)).unwrap();
    let queued = child
        .execute(
            ScreenBrightnessTarget::Device(device),
            ScreenBrightnessAction::Set(ScreenBrightnessLevel::MAX),
        )
        .unwrap();
    f.owner.revoke(&parent).unwrap();
    assert_eq!(complete(queued), ScreenBrightnessOutcome::Stale);
    assert!(matches!(
        child.execute(
            ScreenBrightnessTarget::Device(device),
            ScreenBrightnessAction::Set(ScreenBrightnessLevel::MAX)
        ),
        Err(ScreenBrightnessError::PermissionDenied)
    ));
    assert!(child.restrict_to(&[device]).is_err());
    f.release();
    assert!(matches!(
        complete(active),
        ScreenBrightnessOutcome::Applied(_)
    ));
    assert_eq!(f.native.lock().unwrap().writes, [60]);
}
#[test]
fn publication_callbacks_can_reenter_authority_updates() {
    let f = Fixture::new(Default::default(), 100);
    let shared = f.handle.shared.clone();
    let invoked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = invoked.clone();
    let dependency = f.handle.signal().dependency(0);
    let _subscription = dependency.subscribe(Arc::new(move || {
        if !flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
            shared.stop();
        }
    }));
    f.handle.shared.publish();
    assert!(invoked.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(
        f.handle.signal().snapshot().state,
        ScreenBrightnessServiceState::Stopped
    );
}
