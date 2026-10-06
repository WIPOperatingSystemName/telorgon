use super::{request::Completion, *};
use crate::{Signal, SignalWriter, platform::contracts::PermissionState};
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::Instant,
};

static IDENTITIES: AtomicU64 = AtomicU64::new(1);
pub(crate) fn identity() -> Result<u64, ScreenBrightnessError> {
    IDENTITIES
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
        .map_err(|_| ScreenBrightnessError::Unavailable)
}
pub(crate) struct Endpoint {
    pub descriptor: ScreenBrightnessProviderDevice,
    pub handle: ScreenBrightnessDeviceHandle,
    pub reading: Option<ScreenBrightnessReading>,
    pub observed: Instant,
}
pub(crate) struct Command {
    pub id: ScreenBrightnessRequestId,
    pub device: ScreenBrightnessDeviceHandle,
    pub action: ScreenBrightnessAction,
    pub authority: u64,
    pub epoch: u64,
    pub deadline: Instant,
    pub completion: Arc<Completion>,
    pub mode: ScreenBrightnessWriteMode,
    pub key: Option<u32>,
}
pub(crate) struct Grant {
    pub parent: Option<u64>,
    pub devices: Option<BTreeSet<ScreenBrightnessDeviceHandle>>,
}
pub(crate) struct State {
    pub started: bool,
    pub stopped: bool,
    pub done: bool,
    pub host: ScreenBrightnessHostState,
    pub epoch: u64,
    pub revision: u64,
    pub devices: Vec<Endpoint>,
    pub queue: VecDeque<Command>,
    pub active: Option<Command>,
    pub grants: HashMap<u64, Grant>,
    pub last_error: Option<ScreenBrightnessError>,
    pub keys: HashMap<u32, super::keys::HeldKey>,
    pub refresh_requested: bool,
}
pub(crate) struct Shared {
    pub id: u64,
    pub config: ScreenBrightnessConfig,
    pub state: Mutex<State>,
    pub wake: Condvar,
    pub signal: Signal<ScreenBrightnessSnapshot>,
    pub writer: SignalWriter<ScreenBrightnessSnapshot>,
    publication: Mutex<()>,
}
impl Shared {
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(crate) fn publish(&self) -> u64 {
        // Serializes publications without keeping the state lock across Signal callbacks.
        let _publication = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        let snapshot = {
            let mut state = self.lock();
            state.revision = state
                .revision
                .checked_add(1)
                .expect("screen brightness revision exhausted");
            let permission = if !state.host.active || state.host.locked {
                PermissionState::Restricted
            } else {
                PermissionState::Granted
            };
            ScreenBrightnessSnapshot {
                revision: state.revision,
                state: if state.stopped {
                    ScreenBrightnessServiceState::Stopped
                } else if !state.started {
                    ScreenBrightnessServiceState::Unstarted
                } else if !state.host.active || state.host.locked {
                    ScreenBrightnessServiceState::Restricted
                } else if state.devices.is_empty() {
                    ScreenBrightnessServiceState::Unavailable
                } else {
                    ScreenBrightnessServiceState::Ready
                },
                devices: state
                    .devices
                    .iter()
                    .map(|e| ScreenBrightnessDeviceSnapshot {
                        device: e.handle,
                        name: e.descriptor.name.clone(),
                        kind: e.descriptor.kind,
                        association: e.descriptor.association.clone(),
                        permission: if permission == PermissionState::Restricted {
                            permission
                        } else {
                            e.descriptor.permission
                        },
                        minimum: ScreenBrightnessLevel::from_raw(
                            self.config.native_minimum(e.descriptor.maximum),
                            e.descriptor.maximum,
                        ),
                        native_maximum: e.descriptor.maximum,
                        configured: e.reading.map(|r| {
                            ScreenBrightnessLevel::from_raw(r.configured, e.descriptor.maximum)
                        }),
                        reported_actual: e
                            .reading
                            .and_then(|r| r.actual)
                            .map(|r| ScreenBrightnessLevel::from_raw(r, e.descriptor.maximum)),
                        pending: state
                            .active
                            .as_ref()
                            .filter(|c| c.device == e.handle)
                            .and_then(|c| match c.action {
                                ScreenBrightnessAction::Set(v) => Some((c.id, v)),
                                _ => None,
                            }),
                        observed_at: e.observed,
                    })
                    .collect(),
                pending: state.queue.len() + usize::from(state.active.is_some()),
                last_error: state.last_error.clone(),
            }
        };
        let revision = snapshot.revision;
        let (_, notify) = self.writer.publish_deferred(snapshot);
        drop(_publication);
        notify();
        revision
    }
    pub(crate) fn stop(&self) {
        let (queued, active) = {
            let mut state = self.lock();
            state.stopped = true;
            state.keys.clear();
            state.epoch += 1;
            (
                state.queue.drain(..).collect::<Vec<_>>(),
                state.active.as_ref().map(|c| c.completion.clone()),
            )
        };
        for command in queued {
            command
                .completion
                .finish(ScreenBrightnessOutcome::Cancelled);
        }
        if let Some(active) = active {
            active.finish(ScreenBrightnessOutcome::Unconfirmed {
                reason: ScreenBrightnessConfirmationFailure::AuthorityLost,
                observed: None,
            });
        }
        self.wake.notify_all();
        self.publish();
    }
}
#[derive(Clone)]
pub struct ScreenBrightnessObserver {
    signal: Signal<ScreenBrightnessSnapshot>,
}
impl ScreenBrightnessObserver {
    pub fn signal(&self) -> Signal<ScreenBrightnessSnapshot> {
        self.signal.clone()
    }
}
#[derive(Clone)]
pub struct ScreenBrightnessHandle {
    pub(crate) shared: Arc<Shared>,
    pub(super) authority: u64,
}
impl std::fmt::Debug for ScreenBrightnessHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenBrightnessHandle")
            .field("controller", &self.shared.id)
            .finish_non_exhaustive()
    }
}
impl PartialEq for ScreenBrightnessHandle {
    fn eq(&self, other: &Self) -> bool {
        self.shared.id == other.shared.id && self.authority == other.authority
    }
}
impl ScreenBrightnessHandle {
    pub fn observer(&self) -> ScreenBrightnessObserver {
        ScreenBrightnessObserver {
            signal: self.signal(),
        }
    }
    pub fn signal(&self) -> Signal<ScreenBrightnessSnapshot> {
        self.shared.signal.clone()
    }
    /// Narrows adjustment authority. The owner may revoke this scope independently.
    pub fn restrict_to(
        &self,
        devices: &[ScreenBrightnessDeviceHandle],
    ) -> Result<Self, ScreenBrightnessError> {
        let mut state = self.shared.lock();
        let parent = state
            .grants
            .get(&self.authority)
            .ok_or(ScreenBrightnessError::PermissionDenied)?;
        let devices = devices.iter().copied().collect::<BTreeSet<_>>();
        if devices.is_empty()
            || devices.iter().any(|d| {
                d.controller != self.shared.id
                    || !state.devices.iter().any(|e| e.handle == *d)
                    || parent
                        .devices
                        .as_ref()
                        .is_some_and(|allowed| !allowed.contains(d))
            })
        {
            return Err(ScreenBrightnessError::PermissionDenied);
        }
        if state.grants.len() >= 64 {
            return Err(ScreenBrightnessError::QueueFull);
        }
        let authority = identity()?;
        state.grants.insert(
            authority,
            Grant {
                parent: Some(self.authority),
                devices: Some(devices),
            },
        );
        Ok(Self {
            shared: self.shared.clone(),
            authority,
        })
    }
    pub fn execute(
        &self,
        target: ScreenBrightnessTarget,
        action: ScreenBrightnessAction,
    ) -> Result<ScreenBrightnessRequest, ScreenBrightnessError> {
        self.execute_with_mode(target, action, ScreenBrightnessWriteMode::Ordered)
    }
    /// No native I/O. Output/default selectors bind to a current incarnation at admission.
    pub fn execute_with_mode(
        &self,
        target: ScreenBrightnessTarget,
        action: ScreenBrightnessAction,
        mode: ScreenBrightnessWriteMode,
    ) -> Result<ScreenBrightnessRequest, ScreenBrightnessError> {
        self.enqueue(target, action, mode, None, None)
    }
    pub(super) fn enqueue(
        &self,
        target: ScreenBrightnessTarget,
        action: ScreenBrightnessAction,
        mode: ScreenBrightnessWriteMode,
        key: Option<u32>,
        expected_epoch: Option<u64>,
    ) -> Result<ScreenBrightnessRequest, ScreenBrightnessError> {
        if mode == ScreenBrightnessWriteMode::ReplacePendingSet
            && !matches!(action, ScreenBrightnessAction::Set(_))
        {
            return Err(ScreenBrightnessError::InvalidConfig(
                "only absolute sets can replace pending work",
            ));
        }
        let (request, superseded) = {
            let mut state = self.shared.lock();
            if expected_epoch.is_some_and(|e| {
                e != state.epoch || key.is_some_and(|key| !state.keys.contains_key(&key))
            }) {
                return Err(ScreenBrightnessError::StaleDevice);
            }
            if state.stopped || !state.started {
                return Err(ScreenBrightnessError::Unavailable);
            }
            if !state.host.active {
                return Err(ScreenBrightnessError::SessionInactive);
            }
            if state.host.locked {
                return Err(ScreenBrightnessError::Locked);
            }
            let candidates = state
                .devices
                .iter()
                .filter(|e| match target {
                    ScreenBrightnessTarget::Device(device) => e.handle == device,
                    ScreenBrightnessTarget::DefaultInternal => {
                        e.descriptor.kind == ScreenBrightnessKind::InternalBacklight
                    }
                    ScreenBrightnessTarget::Output(output) => match &e.descriptor.association {
                        ScreenBrightnessAssociation::Confirmed(outputs)
                        | ScreenBrightnessAssociation::HostConfigured(outputs) => {
                            outputs.contains(&output)
                        }
                        _ => false,
                    },
                })
                .collect::<Vec<_>>();
            let endpoint = match candidates.as_slice() {
                [device] => *device,
                [] => {
                    return Err(if matches!(target, ScreenBrightnessTarget::Device(_)) {
                        ScreenBrightnessError::StaleDevice
                    } else {
                        ScreenBrightnessError::Unsupported
                    });
                }
                _ => return Err(ScreenBrightnessError::AmbiguousTarget),
            };
            let allowed = state
                .grants
                .get(&self.authority)
                .ok_or(ScreenBrightnessError::PermissionDenied)?;
            if allowed
                .devices
                .as_ref()
                .is_some_and(|devices| !devices.contains(&endpoint.handle))
                || !endpoint.descriptor.permission.allows_use()
            {
                return Err(ScreenBrightnessError::PermissionDenied);
            }
            if let ScreenBrightnessAction::Set(level) = action {
                if (!self.shared.config.allow_zero && level == ScreenBrightnessLevel::ZERO)
                    || level
                        < ScreenBrightnessLevel::from_raw(
                            self.shared
                                .config
                                .native_minimum(endpoint.descriptor.maximum),
                            endpoint.descriptor.maximum,
                        )
                {
                    return Err(ScreenBrightnessError::BelowMinimum);
                }
            }
            let device = endpoint.handle;
            let replace = mode == ScreenBrightnessWriteMode::ReplacePendingSet
                && state.queue.back().is_some_and(|c| {
                    c.device == device
                        && c.authority == self.authority
                        && c.mode == mode
                        && matches!(c.action, ScreenBrightnessAction::Set(_))
                        && !c.completion.terminal()
                });
            if !replace
                && state.queue.len() + usize::from(state.active.is_some())
                    >= self.shared.config.queue_capacity
            {
                return Err(ScreenBrightnessError::QueueFull);
            }
            let completion = Completion::new();
            let id = ScreenBrightnessRequestId(identity()?);
            let command = Command {
                id,
                device,
                action,
                authority: self.authority,
                epoch: state.epoch,
                deadline: Instant::now() + self.shared.config.request_timeout,
                completion: completion.clone(),
                mode,
                key,
            };
            let superseded = if replace {
                state.queue.pop_back()
            } else {
                None
            };
            state.queue.push_back(command);
            (
                ScreenBrightnessRequest {
                    id,
                    completion,
                    device,
                    epoch: state.epoch,
                },
                superseded,
            )
        };
        if let Some(old) = superseded {
            old.completion.finish(ScreenBrightnessOutcome::Superseded);
        }
        self.shared.wake.notify_all();
        self.shared.publish();
        Ok(request)
    }
}
/// Owns workers; construction performs no I/O and handles cannot prolong their lifetime.
pub struct ScreenBrightnessController {
    pub(crate) shared: Arc<Shared>,
    provider: Option<Box<dyn ScreenBrightnessProvider>>,
    workers: Vec<JoinHandle<()>>,
    #[cfg(all(target_os = "linux", feature = "shell-screen-brightness-linux"))]
    linux: crate::platform::linux::screen_brightness::ScreenBrightnessLinuxConfig,
}
impl ScreenBrightnessController {
    pub fn new(config: ScreenBrightnessConfig) -> Result<Self, ScreenBrightnessError> {
        config.validate()?;
        let id = identity()?;
        let (signal, writer) = Signal::new(ScreenBrightnessSnapshot {
            revision: 1,
            state: ScreenBrightnessServiceState::Unstarted,
            devices: Vec::new(),
            pending: 0,
            last_error: None,
        });
        Ok(Self {
            shared: Arc::new(Shared {
                id,
                config,
                state: Mutex::new(State {
                    started: false,
                    stopped: false,
                    done: false,
                    host: ScreenBrightnessHostState::default(),
                    epoch: 1,
                    revision: 1,
                    devices: Vec::new(),
                    queue: VecDeque::new(),
                    active: None,
                    grants: HashMap::from([(
                        id,
                        Grant {
                            parent: None,
                            devices: None,
                        },
                    )]),
                    last_error: None,
                    keys: HashMap::new(),
                    refresh_requested: false,
                }),
                wake: Condvar::new(),
                signal,
                writer,
                publication: Mutex::new(()),
            }),
            provider: None,
            workers: Vec::new(),
            #[cfg(all(target_os = "linux", feature = "shell-screen-brightness-linux"))]
            linux: Default::default(),
        })
    }
    pub fn with_provider<P: ScreenBrightnessProvider>(
        config: ScreenBrightnessConfig,
        provider: P,
    ) -> Result<Self, ScreenBrightnessError> {
        let mut owner = Self::new(config)?;
        owner.provider = Some(Box::new(provider));
        Ok(owner)
    }
    pub fn handle(&self) -> ScreenBrightnessHandle {
        ScreenBrightnessHandle {
            shared: self.shared.clone(),
            authority: self.shared.id,
        }
    }
    /// Starts an explicitly injected provider. Set host authority separately before writing.
    pub fn start(&mut self) -> Result<(), ScreenBrightnessError> {
        {
            let state = self.shared.lock();
            if state.started || state.stopped {
                return Err(ScreenBrightnessError::Unavailable);
            }
        }
        let provider = self
            .provider
            .take()
            .ok_or(ScreenBrightnessError::Unsupported)?;
        self.workers = super::worker::start(self.shared.clone(), provider)?;
        Ok(())
    }
    pub fn set_host_state(&self, host: ScreenBrightnessHostState) {
        let (queued, active) = {
            let mut state = self.shared.lock();
            if state.host == host {
                return;
            }
            state.host = host;
            state.epoch += 1;
            state.keys.clear();
            (
                state.queue.drain(..).collect::<Vec<_>>(),
                state.active.as_ref().map(|c| c.completion.clone()),
            )
        };
        for command in queued {
            command.completion.finish(ScreenBrightnessOutcome::Stale);
        }
        if let Some(active) = active {
            active.finish(ScreenBrightnessOutcome::Unconfirmed {
                reason: ScreenBrightnessConfirmationFailure::AuthorityLost,
                observed: None,
            });
        }
        self.shared.wake.notify_all();
        self.shared.publish();
    }
    pub fn revoke(&self, handle: &ScreenBrightnessHandle) -> Result<(), ScreenBrightnessError> {
        if handle.shared.id != self.shared.id {
            return Err(ScreenBrightnessError::PermissionDenied);
        }
        let (queued, active) = {
            let mut state = self.shared.lock();
            let mut revoked = BTreeSet::from([handle.authority]);
            loop {
                let children = state
                    .grants
                    .iter()
                    .filter_map(|(&id, grant)| {
                        grant
                            .parent
                            .filter(|parent| revoked.contains(parent))
                            .map(|_| id)
                    })
                    .collect::<Vec<_>>();
                let before = revoked.len();
                revoked.extend(children);
                if revoked.len() == before {
                    break;
                }
            }
            state.grants.retain(|id, _| !revoked.contains(id));
            state
                .keys
                .retain(|_, key| !revoked.contains(&key.authority));
            let mut queued = Vec::new();
            state.queue.retain(|command| {
                if revoked.contains(&command.authority) {
                    queued.push(command.completion.clone());
                    false
                } else {
                    true
                }
            });
            let active = state
                .active
                .as_ref()
                .filter(|c| revoked.contains(&c.authority))
                .map(|c| c.completion.clone());
            (queued, active)
        };
        for completion in queued {
            completion.finish(ScreenBrightnessOutcome::Stale);
        }
        if let Some(completion) = active {
            completion.finish(ScreenBrightnessOutcome::Unconfirmed {
                reason: ScreenBrightnessConfirmationFailure::AuthorityLost,
                observed: None,
            });
        }
        self.shared.wake.notify_all();
        self.shared.publish();
        Ok(())
    }
    pub fn stop(&self) {
        self.shared.stop();
    }
    pub async fn shutdown(&mut self) -> Result<(), ScreenBrightnessError> {
        self.stop();
        let shared = self.shared.clone();
        let workers = std::mem::take(&mut self.workers);
        blocking::unblock(move || {
            let deadline = Instant::now() + shared.config.request_timeout;
            let mut state = shared.lock();
            while !state.done && state.started {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(ScreenBrightnessError::TimedOut);
                }
                state = shared
                    .wake
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            drop(state);
            for worker in workers {
                worker
                    .join()
                    .map_err(|_| ScreenBrightnessError::WorkerPanicked)?;
            }
            Ok(())
        })
        .await
    }
    #[cfg(all(target_os = "linux", feature = "shell-screen-brightness-linux"))]
    pub fn linux(
        mut self,
        config: crate::platform::linux::screen_brightness::ScreenBrightnessLinuxConfig,
    ) -> Self {
        self.linux = config;
        self
    }
    pub(crate) fn start_for_shell(
        &mut self,
        output: crate::shell::OutputId,
    ) -> Result<(), ScreenBrightnessError> {
        if self.provider.is_some() {
            return self.start();
        }
        #[cfg(all(target_os = "linux", feature = "shell-screen-brightness-linux"))]
        {
            self.provider = Some(Box::new(
                crate::platform::linux::screen_brightness::ScreenBrightnessLinuxProvider::new(
                    self.linux.clone(),
                    output,
                )?,
            ));
            self.start()
        }
        #[cfg(not(all(target_os = "linux", feature = "shell-screen-brightness-linux")))]
        {
            let _ = output;
            Err(ScreenBrightnessError::Unsupported)
        }
    }
}
impl std::fmt::Debug for ScreenBrightnessController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenBrightnessController")
            .field("controller", &self.shared.id)
            .field("started", &self.shared.lock().started)
            .finish_non_exhaustive()
    }
}
impl Drop for ScreenBrightnessController {
    fn drop(&mut self) {
        self.shared.stop();
    }
}
