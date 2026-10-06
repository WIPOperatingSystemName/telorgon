use super::{request::Completion, *};
use crate::authoring::compose::{Signal, SignalWriter};
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::Instant,
};

pub(super) struct Job {
    pub id: NetworkRequestId,
    pub command: NetworkCommand,
    pub completion: Arc<Completion>,
    pub deadline: Instant,
}
pub(super) struct State {
    pub started: bool,
    pub stopped: bool,
    pub queue: VecDeque<Job>,
    pub active: Option<Arc<Completion>>,
}
pub(super) struct Shared {
    pub config: NetworkConfig,
    pub state: Mutex<State>,
    pub wake: Condvar,
    pub signal: Signal<NetworkSnapshot>,
    pub writer: SignalWriter<NetworkSnapshot>,
    pub challenges: Signal<Vec<NetworkCredentialChallenge>>,
    pub challenge_writer: SignalWriter<Vec<NetworkCredentialChallenge>>,
}
impl Shared {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Clone)]
pub struct NetworkObserver {
    pub(super) shared: Arc<Shared>,
}
impl NetworkObserver {
    pub fn signal(&self) -> Signal<NetworkSnapshot> {
        self.shared.signal.clone()
    }
    /// Separate credential prompts. Passwords are never published in either signal.
    pub fn challenges(&self) -> Signal<Vec<NetworkCredentialChallenge>> {
        self.shared.challenges.clone()
    }
}
#[derive(Clone)]
pub struct NetworkHandle {
    pub(super) shared: Arc<Shared>,
}
impl std::fmt::Debug for NetworkHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkHandle").finish_non_exhaustive()
    }
}
impl PartialEq for NetworkHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
}
impl NetworkHandle {
    pub fn observer(&self) -> NetworkObserver {
        NetworkObserver {
            shared: self.shared.clone(),
        }
    }
    pub fn signal(&self) -> Signal<NetworkSnapshot> {
        self.shared.signal.clone()
    }
    /// Enqueues a request without native I/O. Completion reflects backend readback.
    pub fn execute(&self, command: NetworkCommand) -> Result<NetworkRequest, NetworkError> {
        command.validate()?;
        let mut state = self.shared.lock();
        if state.stopped {
            return Err(NetworkError::Stopped);
        }
        if !state.started {
            return Err(NetworkError::Unavailable);
        }
        validate_target(&command, &self.shared.signal.snapshot())?;
        state.queue.retain(|job| !job.completion.terminal());
        if state.queue.len() + usize::from(state.active.is_some())
            >= self.shared.config.queue_capacity
        {
            return Err(NetworkError::Busy);
        }
        let id = NetworkRequestId::new();
        let completion = Completion::new();
        if command.needs_credentials() {
            completion.await_credentials();
        }
        state.queue.push_back(Job {
            id,
            command,
            completion: completion.clone(),
            deadline: Instant::now() + self.shared.config.request_timeout,
        });
        drop(state);
        self.shared.wake.notify_one();
        Ok(NetworkRequest { id, completion })
    }
    pub fn scan_wifi(&self, interface: NetworkInterfaceId) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::ScanWifi(interface))
    }
    pub fn connect_wifi(
        &self,
        interface: NetworkInterfaceId,
        options: WifiConnectOptions,
    ) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::ConnectWifi { interface, options })
    }
    pub fn activate(
        &self,
        profile: NetworkProfileId,
        interface: NetworkInterfaceId,
    ) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::ActivateProfile { interface, profile })
    }
    pub fn disconnect(
        &self,
        interface: NetworkInterfaceId,
        policy: DisconnectPolicy,
    ) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::Disconnect { interface, policy })
    }
    pub fn update_profile(
        &self,
        profile: NetworkProfileId,
        ip: NetworkIpConfig,
        persistence: NetworkPersistence,
    ) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::UpdateProfile {
            profile,
            ip,
            persistence,
        })
    }
    pub fn set_autoconnect(
        &self,
        profile: NetworkProfileId,
        enabled: bool,
        persistence: NetworkPersistence,
    ) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::SetAutoconnect {
            profile,
            enabled,
            persistence,
        })
    }
    pub fn forget_profile(
        &self,
        profile: NetworkProfileId,
    ) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::ForgetProfile(profile))
    }
    pub fn reapply(&self, interface: NetworkInterfaceId) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::Reapply(interface))
    }
    pub fn set_wifi_enabled(&self, enabled: bool) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::SetWifiEnabled(enabled))
    }
    pub fn set_networking_enabled(&self, enabled: bool) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::SetNetworkingEnabled(enabled))
    }
    pub fn renew_dhcp(
        &self,
        interface: NetworkInterfaceId,
        family: IpFamily,
    ) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::RenewDhcp { interface, family })
    }
    pub fn flush_dns_cache(&self) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::FlushDnsCache)
    }
    pub fn clear_addresses(
        &self,
        interface: NetworkInterfaceId,
        family: IpFamily,
    ) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::ClearAddresses { interface, family })
    }
    pub fn clear_routes(
        &self,
        interface: NetworkInterfaceId,
        family: IpFamily,
    ) -> Result<NetworkRequest, NetworkError> {
        self.execute(NetworkCommand::ClearRoutes { interface, family })
    }
    /// Supplies a password to a still-queued prompt. This resumes that same request.
    pub fn supply_credentials(
        &self,
        request: NetworkRequestId,
        secret: NetworkSecret,
    ) -> Result<(), NetworkError> {
        let mut state = self.shared.lock();
        if state.stopped {
            return Err(NetworkError::Stopped);
        }
        let job = state
            .queue
            .iter_mut()
            .find(|j| j.id == request && !j.completion.terminal())
            .ok_or(NetworkError::Stale)?;
        if !job.command.needs_credentials() || Instant::now() >= job.deadline {
            return Err(NetworkError::Stale);
        }
        let NetworkCommand::ConnectWifi { options, .. } = &mut job.command else {
            return Err(NetworkError::Stale);
        };
        options.credentials = Some(secret);
        if let Err(error) = options.validate() {
            options.credentials = None;
            return Err(error);
        }
        drop(state);
        self.shared.wake.notify_one();
        Ok(())
    }
}

pub(super) fn validate_target(
    command: &NetworkCommand,
    state: &NetworkSnapshot,
) -> Result<(), NetworkError> {
    if state.state != NetworkServiceState::Ready {
        return Err(NetworkError::Unavailable);
    }
    let device = |id| {
        state
            .interfaces
            .iter()
            .find(|d| d.id == id)
            .ok_or(NetworkError::Stale)
    };
    let profile = |id| {
        state
            .profiles
            .iter()
            .find(|p| p.id == id)
            .ok_or(NetworkError::Stale)
    };
    let supported = |yes| {
        if yes {
            Ok(())
        } else {
            Err(NetworkError::Unsupported)
        }
    };
    match command {
        NetworkCommand::ScanWifi(id) => supported(device(*id)?.capabilities.scan_wifi),
        NetworkCommand::ConnectWifi { interface, options } => {
            let d = device(*interface)?;
            supported(d.capabilities.connect_wifi)?;
            if !matches!(
                options.security,
                WifiSecurity::Open | WifiSecurity::WpaPersonal | WifiSecurity::Wpa3Personal
            ) {
                return Err(NetworkError::Unsupported);
            }
            if options.security == WifiSecurity::Wpa3Personal {
                supported(d.capabilities.wpa3_personal)?;
            }
            if let Some(ap) = options.access_point {
                let ap = d
                    .access_points
                    .iter()
                    .find(|a| a.id == ap)
                    .ok_or(NetworkError::Stale)?;
                if ap.ssid.as_ref() != Some(&options.ssid) || ap.security != options.security {
                    return Err(NetworkError::InvalidConfig(
                        "access point does not match requested network",
                    ));
                }
            }
            Ok(())
        }
        NetworkCommand::ActivateProfile {
            interface,
            profile: p,
        } => {
            let p = profile(*p)?;
            let d = device(*interface)?;
            if p.kind != d.kind || p.interface_name.as_ref().is_some_and(|name| name != &d.name) {
                return Err(NetworkError::InvalidConfig(
                    "profile does not match this interface",
                ));
            }
            supported(d.capabilities.activate_profile)
        }
        NetworkCommand::Disconnect { interface, .. } => {
            supported(device(*interface)?.capabilities.disconnect)
        }
        NetworkCommand::Reapply(id) => supported(device(*id)?.capabilities.reapply),
        NetworkCommand::RenewDhcp { interface, .. } => {
            supported(device(*interface)?.capabilities.renew_dhcp)
        }
        NetworkCommand::ClearAddresses { interface, .. } => {
            supported(device(*interface)?.capabilities.clear_addresses)
        }
        NetworkCommand::ClearRoutes { interface, .. } => {
            supported(device(*interface)?.capabilities.clear_routes)
        }
        NetworkCommand::UpdateProfile { profile: p, .. } => {
            profile(*p)?;
            supported(state.capabilities.edit_profiles && state.capabilities.configure_ip)
        }
        NetworkCommand::SetAutoconnect { profile: p, .. } | NetworkCommand::ForgetProfile(p) => {
            profile(*p)?;
            supported(state.capabilities.edit_profiles)
        }
        NetworkCommand::FlushDnsCache => supported(state.capabilities.flush_dns_cache),
        NetworkCommand::SetWifiEnabled(_) => supported(state.capabilities.set_wifi_enabled),
        NetworkCommand::SetNetworkingEnabled(_) => {
            supported(state.capabilities.set_networking_enabled)
        }
    }
}

/// Owns backend work. Dropping a handle never disconnects a network; dropping the owner stops observation.
pub struct NetworkController {
    pub(super) shared: Arc<Shared>,
    provider: Option<Box<dyn NetworkProvider>>,
    worker: Option<JoinHandle<()>>,
}
impl NetworkController {
    pub fn with_provider(
        config: NetworkConfig,
        provider: impl NetworkProvider,
    ) -> Result<Self, NetworkError> {
        config.validate()?;
        let (signal, writer) = Signal::new(NetworkSnapshot::default());
        let (challenges, challenge_writer) = Signal::new(Vec::new());
        Ok(Self {
            shared: Arc::new(Shared {
                config,
                state: Mutex::new(State {
                    started: false,
                    stopped: false,
                    queue: VecDeque::new(),
                    active: None,
                }),
                wake: Condvar::new(),
                signal,
                writer,
                challenges,
                challenge_writer,
            }),
            provider: Some(Box::new(provider)),
            worker: None,
        })
    }
    pub fn handle(&self) -> NetworkHandle {
        NetworkHandle {
            shared: self.shared.clone(),
        }
    }
    pub fn observer(&self) -> NetworkObserver {
        self.handle().observer()
    }
    pub fn start(&mut self) -> Result<(), NetworkError> {
        {
            let mut state = self.shared.lock();
            if state.started || state.stopped {
                return Err(NetworkError::Unavailable);
            }
            state.started = true;
        }
        let provider = self.provider.take().ok_or(NetworkError::Unavailable)?;
        let shared = self.shared.clone();
        match std::thread::Builder::new()
            .name("telorgon-network".into())
            .spawn(move || super::worker::run(shared, provider))
        {
            Ok(worker) => {
                self.worker = Some(worker);
                Ok(())
            }
            Err(_) => {
                self.shared.lock().stopped = true;
                Err(NetworkError::Unavailable)
            }
        }
    }
    pub fn shutdown(&mut self) -> Result<(), NetworkError> {
        self.shared.lock().stopped = true;
        self.shared.wake.notify_one();
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| NetworkError::BackendFailure)?;
        } else {
            let mut snapshot = (*self.shared.signal.snapshot()).clone();
            snapshot.state = NetworkServiceState::Stopped;
            self.shared.writer.publish(snapshot);
        }
        Ok(())
    }
}
impl Drop for NetworkController {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl std::fmt::Debug for NetworkController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkController")
            .field("state", &self.shared.signal.snapshot().state)
            .finish_non_exhaustive()
    }
}
