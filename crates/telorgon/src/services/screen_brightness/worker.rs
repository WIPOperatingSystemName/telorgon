use super::{
    controller::{Endpoint, Shared, identity},
    request::Completion,
    *,
};
use crate::platform::contracts::{PlatformError, PlatformErrorKind};
use std::{
    collections::HashSet,
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) fn start(
    shared: Arc<Shared>,
    mut provider: Box<dyn ScreenBrightnessProvider>,
) -> Result<Vec<JoinHandle<()>>, ScreenBrightnessError> {
    shared.lock().started = true;
    let worker_shared = shared.clone();
    let io = thread::Builder::new()
        .name("telorgon-screen-brightness".into())
        .spawn(move || {
            let _exit = WorkerExit(worker_shared.clone());
            run(&worker_shared, provider.as_mut());
        })
        .map_err(|_| {
            shared.lock().started = false;
            ScreenBrightnessError::Unavailable
        })?;
    let watch_shared = shared.clone();
    let watch = thread::Builder::new()
        .name("telorgon-brightness-deadlines".into())
        .spawn(move || watchdog(watch_shared));
    match watch {
        Ok(watch) => Ok(vec![io, watch]),
        Err(_) => {
            shared.stop();
            Err(ScreenBrightnessError::Unavailable)
        }
    }
}
struct WorkerExit(Arc<Shared>);
impl Drop for WorkerExit {
    fn drop(&mut self) {
        let (queued, active) = {
            let mut state = self.0.lock();
            state.done = true;
            state.stopped = true;
            if thread::panicking() {
                state.last_error = Some(ScreenBrightnessError::WorkerPanicked);
            }
            (
                state.queue.drain(..).collect::<Vec<_>>(),
                state.active.take(),
            )
        };
        for command in queued {
            command
                .completion
                .finish(ScreenBrightnessOutcome::Failed(failure(
                    PlatformErrorKind::Unavailable,
                )));
        }
        if let Some(active) = active {
            active.completion.finish(unconfirmed(
                ScreenBrightnessConfirmationFailure::Transport,
                None,
            ));
        }
        self.0.publish();
        self.0.wake.notify_all();
    }
}
fn failure(kind: PlatformErrorKind) -> PlatformError {
    PlatformError::new(kind, "screen brightness operation")
}
fn unconfirmed(
    reason: ScreenBrightnessConfirmationFailure,
    observed: Option<ScreenBrightnessLevel>,
) -> ScreenBrightnessOutcome {
    ScreenBrightnessOutcome::Unconfirmed { reason, observed }
}
fn watchdog(shared: Arc<Shared>) {
    loop {
        super::keys::repeat(&shared);
        let (expired, active, changed) = {
            let mut state = shared.lock();
            if state.stopped {
                return;
            }
            let now = Instant::now();
            let before = state.queue.len();
            let mut expired = Vec::new();
            let mut kept = std::collections::VecDeque::new();
            while let Some(command) = state.queue.pop_front() {
                if command.completion.terminal() {
                    continue;
                }
                let outcome = if command.epoch != state.epoch
                    || !state.grants.contains_key(&command.authority)
                {
                    Some(ScreenBrightnessOutcome::Stale)
                } else if now >= command.deadline {
                    Some(ScreenBrightnessOutcome::Failed(failure(
                        PlatformErrorKind::TimedOut,
                    )))
                } else {
                    None
                };
                if let Some(outcome) = outcome {
                    expired.push((command.completion, outcome));
                } else {
                    kept.push_back(command);
                }
            }
            state.queue = kept;
            let active = state.active.as_ref().and_then(|c| {
                if c.completion.terminal() {
                    return None;
                }
                let reason = if c.epoch != state.epoch || !state.grants.contains_key(&c.authority) {
                    Some(ScreenBrightnessConfirmationFailure::AuthorityLost)
                } else if now >= c.deadline {
                    Some(ScreenBrightnessConfirmationFailure::Timeout)
                } else {
                    None
                };
                reason.map(|r| (c.completion.clone(), r))
            });
            let changed = state.queue.len() != before || active.is_some();
            if active
                .as_ref()
                .is_some_and(|(_, reason)| *reason == ScreenBrightnessConfirmationFailure::Timeout)
            {
                state.last_error = Some(ScreenBrightnessError::TimedOut);
            }
            (expired, active, changed)
        };
        for (completion, outcome) in expired {
            completion.finish(outcome);
        }
        if let Some((completion, reason)) = active {
            completion.finish(unconfirmed(reason, None));
        }
        if changed {
            shared.publish();
        }
        let state = shared.lock();
        let _ = shared
            .wake
            .wait_timeout(state, Duration::from_millis(20))
            .unwrap_or_else(|e| e.into_inner());
    }
}
fn run(shared: &Arc<Shared>, provider: &mut dyn ScreenBrightnessProvider) {
    let mut next_refresh = Instant::now();
    loop {
        if shared.lock().stopped {
            return;
        }
        let requested = {
            let mut state = shared.lock();
            std::mem::take(&mut state.refresh_requested)
        };
        if requested || provider.has_changes() || Instant::now() >= next_refresh {
            refresh(shared, provider);
            next_refresh = Instant::now() + shared.config.poll_interval;
        }
        let command = {
            let mut state = shared.lock();
            if state.stopped {
                return;
            }
            state.queue.pop_front()
        };
        if let Some(command) = command {
            let completion = command.completion.clone();
            if !completion.begin() {
                shared.publish();
                continue;
            }
            let descriptor = {
                let mut state = shared.lock();
                let valid = valid_command(&state, &command);
                let descriptor = state
                    .devices
                    .iter()
                    .find(|e| e.handle == command.device)
                    .map(|e| e.descriptor.clone());
                if valid && descriptor.is_some() && Instant::now() < command.deadline {
                    state.active = Some(command);
                    descriptor
                } else {
                    None
                }
            };
            if let Some(descriptor) = descriptor {
                shared.publish();
                dispatch(shared, provider, &descriptor, &completion);
                shared.lock().active = None;
            } else {
                completion.finish(ScreenBrightnessOutcome::Stale);
            }
            shared.publish();
            continue;
        }
        let state = shared.lock();
        if state.stopped {
            return;
        }
        if !state.queue.is_empty() {
            continue;
        }
        let _ = shared
            .wake
            .wait_timeout(
                state,
                next_refresh
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(50)),
            )
            .unwrap_or_else(|e| e.into_inner());
    }
}
fn valid_command(state: &super::controller::State, command: &super::controller::Command) -> bool {
    !state.stopped
        && state.host.active
        && !state.host.locked
        && command.epoch == state.epoch
        && state.grants.get(&command.authority).is_some_and(|scope| {
            scope
                .devices
                .as_ref()
                .is_none_or(|allowed| allowed.contains(&command.device))
        })
}
fn refresh(shared: &Shared, provider: &mut dyn ScreenBrightnessProvider) {
    let epoch = shared.lock().epoch;
    let result = provider.discover().and_then(|devices| {
        let mut keys = HashSet::new();
        if devices.len() > 64
            || devices.iter().any(|e| {
                let outputs = match &e.association {
                    ScreenBrightnessAssociation::Confirmed(v)
                    | ScreenBrightnessAssociation::HostConfigured(v) => v.as_slice(),
                    _ => &[],
                };
                e.maximum == 0
                    || e.key.is_empty()
                    || e.key.len() > 256
                    || e.name.len() > 256
                    || outputs.len() > 64
                    || outputs
                        .iter()
                        .enumerate()
                        .any(|(i, o)| outputs[..i].contains(o))
                    || !keys.insert(e.key.clone())
            })
        {
            return Err(ScreenBrightnessError::InvalidData);
        }
        let readings = devices
            .into_iter()
            .map(|descriptor| {
                let reading = provider
                    .read(&descriptor.key)
                    .and_then(|r| r.validate(descriptor.maximum))
                    .ok();
                (descriptor, reading)
            })
            .collect::<Vec<_>>();
        Ok(readings)
    });
    {
        let mut state = shared.lock();
        if state.stopped || state.epoch != epoch {
            return;
        }
        match result {
            Ok(readings) => {
                let mut previous = std::mem::take(&mut state.devices);
                let mut devices = Vec::new();
                for (descriptor, reading) in readings {
                    let old = previous.iter().position(|e| {
                        e.descriptor.key == descriptor.key
                            && e.descriptor.maximum == descriptor.maximum
                            && e.descriptor.kind == descriptor.kind
                            && e.descriptor.association == descriptor.association
                    });
                    let handle = if let Some(i) = old {
                        previous.remove(i).handle
                    } else {
                        let Ok(device) = identity() else {
                            state.last_error = Some(ScreenBrightnessError::Unavailable);
                            continue;
                        };
                        ScreenBrightnessDeviceHandle {
                            controller: shared.id,
                            device,
                        }
                    };
                    devices.push(Endpoint {
                        descriptor,
                        handle,
                        reading,
                        observed: Instant::now(),
                    });
                }
                state.devices = devices;
                state.last_error = None;
            }
            Err(error) => {
                state.devices.clear();
                state.last_error = Some(error);
            }
        }
    }
    shared.publish();
}
fn dispatch(
    shared: &Shared,
    provider: &mut dyn ScreenBrightnessProvider,
    descriptor: &ScreenBrightnessProviderDevice,
    completion: &Arc<Completion>,
) {
    let current = match provider
        .read(&descriptor.key)
        .and_then(|r| r.validate(descriptor.maximum))
    {
        Ok(reading) => reading,
        Err(error) => {
            shared.lock().last_error = Some(error.clone());
            completion.finish(match error {
                ScreenBrightnessError::StaleDevice => ScreenBrightnessOutcome::Stale,
                ScreenBrightnessError::PermissionDenied
                | ScreenBrightnessError::SessionInactive
                | ScreenBrightnessError::Locked => ScreenBrightnessOutcome::Denied,
                _ => ScreenBrightnessOutcome::Failed(failure(PlatformErrorKind::TransportFailure)),
            });
            return;
        }
    };
    let (wanted, device, epoch) = {
        let mut state = shared.lock();
        let active = state.active.as_ref().expect("active request owns dispatch");
        if !valid_command(&state, active) || !descriptor.permission.allows_use() {
            drop(state);
            completion.finish(unconfirmed(
                ScreenBrightnessConfirmationFailure::AuthorityLost,
                None,
            ));
            return;
        }
        if completion.terminal() || Instant::now() >= active.deadline {
            drop(state);
            completion.finish(unconfirmed(
                ScreenBrightnessConfirmationFailure::Timeout,
                None,
            ));
            return;
        }
        let minimum = shared.config.native_minimum(descriptor.maximum);
        let wanted = match active.action {
            ScreenBrightnessAction::Set(level) => level
                .raw(descriptor.maximum)
                .clamp(minimum, descriptor.maximum),
            ScreenBrightnessAction::Adjust(delta) => {
                let base = i32::from(
                    ScreenBrightnessLevel::from_raw(current.configured, descriptor.maximum)
                        .as_basis_points(),
                );
                let level = ScreenBrightnessLevel::basis_points(
                    (base + i32::from(delta.as_basis_points())).clamp(0, 10_000) as u16,
                )
                .unwrap();
                let mut raw = level.raw(descriptor.maximum);
                if raw == current.configured && delta.as_basis_points() != 0 {
                    raw = if delta.as_basis_points() > 0 {
                        raw.saturating_add(1)
                    } else {
                        raw.saturating_sub(1)
                    };
                }
                raw.clamp(minimum, descriptor.maximum)
            }
        };
        let device = active.device;
        let epoch = active.epoch;
        state.active.as_mut().unwrap().action = ScreenBrightnessAction::Set(
            ScreenBrightnessLevel::from_raw(wanted, descriptor.maximum),
        );
        (wanted, device, epoch)
    };
    shared.publish();
    // Check again after publication callbacks, immediately before the native write.
    {
        let state = shared.lock();
        let active = state.active.as_ref().unwrap();
        let reason = if !valid_command(&state, active) {
            Some(ScreenBrightnessConfirmationFailure::AuthorityLost)
        } else if completion.terminal() || Instant::now() >= active.deadline {
            Some(ScreenBrightnessConfirmationFailure::Timeout)
        } else {
            None
        };
        if let Some(reason) = reason {
            drop(state);
            completion.finish(unconfirmed(reason, None));
            return;
        }
    }
    let write = if wanted == current.configured {
        Ok(())
    } else {
        provider.set(&descriptor.key, wanted)
    };
    let readback = provider
        .read(&descriptor.key)
        .and_then(|r| r.validate(descriptor.maximum));
    let observed = readback
        .as_ref()
        .ok()
        .map(|r| ScreenBrightnessLevel::from_raw(r.configured, descriptor.maximum));
    let expired = {
        let mut state = shared.lock();
        if state.stopped
            || state.epoch != epoch
            || !valid_command(&state, state.active.as_ref().unwrap())
        {
            drop(state);
            completion.finish(unconfirmed(
                ScreenBrightnessConfirmationFailure::AuthorityLost,
                None,
            ));
            return;
        }
        let Some(endpoint) = state.devices.iter_mut().find(|e| e.handle == device) else {
            drop(state);
            completion.finish(unconfirmed(
                ScreenBrightnessConfirmationFailure::DeviceLost,
                None,
            ));
            return;
        };
        endpoint.reading = readback.as_ref().ok().copied();
        endpoint.observed = Instant::now();
        state.last_error = write
            .as_ref()
            .err()
            .or_else(|| readback.as_ref().err())
            .cloned();
        Instant::now() >= state.active.as_ref().unwrap().deadline
    };
    let revision = shared.publish();
    if expired {
        completion.finish(unconfirmed(
            ScreenBrightnessConfirmationFailure::Timeout,
            observed,
        ));
        return;
    }
    let result = match (write, readback) {
        (Ok(()), Ok(reading)) if reading.configured == wanted => {
            ScreenBrightnessOutcome::Applied(ScreenBrightnessApplied {
                level: ScreenBrightnessLevel::from_raw(wanted, descriptor.maximum),
                reported_actual: reading
                    .actual
                    .map(|v| ScreenBrightnessLevel::from_raw(v, descriptor.maximum)),
                revision,
                verification: descriptor.verification,
            })
        }
        (
            Err(
                ScreenBrightnessError::PermissionDenied
                | ScreenBrightnessError::SessionInactive
                | ScreenBrightnessError::Locked,
            ),
            _,
        ) => ScreenBrightnessOutcome::Denied,
        (Err(_), _) => unconfirmed(ScreenBrightnessConfirmationFailure::Transport, observed),
        (Ok(()), Err(_)) => unconfirmed(
            ScreenBrightnessConfirmationFailure::ReadbackUnavailable,
            None,
        ),
        _ => unconfirmed(
            ScreenBrightnessConfirmationFailure::ReadbackMismatch,
            observed,
        ),
    };
    completion.finish(result);
}
