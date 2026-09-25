use super::{ConnectionState, ObjectHandle, RegistrySnapshot, registry::RegistryState};
use std::{
    collections::VecDeque,
    os::fd::OwnedFd,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    #[error("PipeWire is not ready")]
    NotReady,
    #[error("connection is closed")]
    Disconnected,
    #[error("object belongs to a removed object or previous connection")]
    StaleHandle,
    #[error("bounded queue is full")]
    QueueFull,
    #[error("resource limit reached: {0}")]
    ResourceLimit(&'static str),
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
    #[error("permission denied")]
    PermissionDenied,
    #[error("unsupported capability: {0}")]
    Unsupported(&'static str),
    #[error("request cancelled before execution")]
    Cancelled,
    #[error("request timed out")]
    Timeout,
    #[error("native PipeWire error: {0}")]
    Native(String),
}
pub(crate) fn native(error: impl std::fmt::Display) -> MediaError {
    MediaError::Native(error.to_string().chars().take(512).collect())
}

/// The FD is consumed by PipeWire. Reconnect a portal using a newly authorized FD.
pub enum Remote {
    Default,
    Named(String),
    Portal(OwnedFd),
}
#[derive(Clone, Debug)]
pub struct ConnectionConfig {
    pub application_name: String,
    pub command_capacity: usize,
    pub max_objects: usize,
    pub max_subscriptions: usize,
    pub connect_timeout: Duration,
}
impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            application_name: "Telorgon".into(),
            command_capacity: 64,
            max_objects: 4096,
            max_subscriptions: 32,
            connect_timeout: Duration::from_secs(5),
        }
    }
}
impl ConnectionConfig {
    fn validate(&self) -> Result<(), MediaError> {
        if self.application_name.len() > 256
            || self.application_name.contains('\0')
            || self.application_name.is_empty()
            || !(1..=4096).contains(&self.command_capacity)
            || !(1..=65536).contains(&self.max_objects)
            || !(1..=256).contains(&self.max_subscriptions)
            || self.connect_timeout.is_zero()
            || self.connect_timeout > Duration::from_secs(60)
        {
            return Err(MediaError::InvalidArgument("connection configuration"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub enum ConnectionEvent {
    StateChanged(ConnectionState),
    ObjectAdded(ObjectHandle),
    ObjectRemoved(ObjectHandle),
    ObjectChanged(ObjectHandle),
    /// Events were lost. Discard incremental state and obtain a fresh snapshot.
    ResyncRequired,
    /// A nonfatal native protocol request error; the connection remains usable.
    ProtocolError(MediaError),
}
struct EventQueue {
    events: VecDeque<ConnectionEvent>,
    capacity: usize,
    overflow: bool,
}
struct Subscriber {
    queue: Mutex<EventQueue>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
/// FIFO until overflow, then one ResyncRequired replaces stale queued deltas. Dropping
/// this value unsubscribes. A wake callback runs on the control worker, outside locks;
/// it must be fast, nonblocking and must not join/shut down that worker.
pub struct Subscription {
    inner: Arc<Subscriber>,
}
impl Subscription {
    pub fn try_recv(&self) -> Option<ConnectionEvent> {
        let mut q = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
        if q.overflow {
            q.overflow = false;
            q.events.clear();
            return Some(ConnectionEvent::ResyncRequired);
        }
        q.events.pop_front()
    }
}

pub(crate) struct Shared {
    pub registry: Mutex<RegistryState>,
    subscribers: Mutex<Vec<Weak<Subscriber>>>,
    pub stop: AtomicBool,
}
impl Shared {
    pub fn emit(&self, event: ConnectionEvent) {
        let subscribers: Vec<_> = {
            let mut list = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
            list.retain(|s| s.strong_count() > 0);
            list.iter().filter_map(Weak::upgrade).collect()
        };
        for sub in subscribers {
            let overflow = {
                let mut q = sub.queue.lock().unwrap_or_else(|e| e.into_inner());
                if q.overflow {
                    false
                } else if q.events.len() == q.capacity {
                    q.overflow = true;
                    true
                } else {
                    q.events.push_back(event.clone());
                    false
                }
            };
            if overflow {
                self.registry
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .snapshot
                    .diagnostics
                    .event_overflows += 1;
            }
            // A host callback must never unwind across PipeWire's C callback boundary.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (sub.wake)()));
        }
    }
    pub fn state(&self, state: ConnectionState) {
        {
            let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(state, ConnectionState::Failed(_)) {
                registry.snapshot.diagnostics.connection_failures += 1;
            }
            if matches!(state, ConnectionState::Failed(_) | ConnectionState::Stopped) {
                registry.snapshot.objects.clear();
            }
            registry.snapshot.state = state.clone();
            registry.snapshot.revision += 1;
        }
        self.emit(ConnectionEvent::StateChanged(state));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestState {
    Queued,
    Executing,
    Complete(Result<(), MediaError>),
}
/// A request is owned independently of event delivery. Cancellation succeeds only before
/// execution; dropping a request does not undo accepted work. Completion remains queryable
/// even if a subscription overflows. Never wait on the realtime or UI thread.
pub struct Request {
    pub(crate) state: Arc<Mutex<RequestState>>,
}
impl Request {
    pub fn state(&self) -> RequestState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn cancel(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if *state != RequestState::Queued {
            return false;
        }
        *state = RequestState::Complete(Err(MediaError::Cancelled));
        true
    }
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RequestState::Queued)),
        }
    }
}
pub(crate) struct Completion(pub Arc<Mutex<RequestState>>);
impl Completion {
    pub fn begin(&self) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if *state != RequestState::Queued {
            return false;
        }
        *state = RequestState::Executing;
        true
    }
    pub fn finish(&self, result: Result<(), MediaError>) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(*state, RequestState::Complete(_)) {
            *state = RequestState::Complete(result);
        }
    }
}
impl Drop for Completion {
    fn drop(&mut self) {
        self.finish(Err(MediaError::Disconnected));
    }
}
pub(crate) enum Command {
    #[cfg(feature = "video-linux")]
    CreateVideo(
        u64,
        crate::media::video::VideoConfig,
        Arc<crate::media::video::VideoShared>,
        Option<crate::media::video::GpuBackend>,
    ),
    #[cfg(feature = "video-linux")]
    Video(u64, super::video::VideoCommand, Completion),
    #[cfg(feature = "midi-linux")]
    CreateMidi(
        crate::media::midi::MidiConfig,
        Arc<crate::media::midi::MidiShared>,
        crate::media::midi::MidiEndpoint,
    ),
    Barrier(Completion),
    #[cfg(feature = "audio-linux")]
    CreateFilter(
        u64,
        crate::media::audio::FilterConfig,
        Arc<crate::media::audio::FilterShared>,
        Box<dyn crate::media::audio::FilterProcessor>,
    ),
    #[cfg(feature = "audio-linux")]
    FilterActive(u64, bool, Completion),
    Link(super::graph::LinkCreation),
    #[cfg(any(feature = "desktop-audio-linux", feature = "video-linux"))]
    Mutate(super::control::Mutation, Completion),
    #[cfg(feature = "audio-linux")]
    CreateAudio(
        u64,
        crate::media::audio::AudioConfig,
        Arc<crate::media::audio::AudioShared>,
        crate::media::audio::RealtimeData,
    ),
    #[cfg(feature = "audio-linux")]
    Audio(u64, super::audio::AudioCommand, Completion),
}

/// Sole shutdown owner. Clone [`ConnectionHandle`] for non-owning users. The worker is
/// independent of GUI/shell entry points. All operations here are control-thread operations.
pub struct Connection {
    handle: ConnectionHandle,
    worker: Option<JoinHandle<()>>,
    config: ConnectionConfig,
}
#[derive(Clone)]
pub struct ConnectionHandle {
    pub(crate) shared: Arc<Shared>,
    pub(crate) commands: mpsc::SyncSender<Command>,
    max_subscriptions: usize,
}
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
impl Connection {
    pub fn connect(config: ConnectionConfig, remote: Remote) -> Result<Self, MediaError> {
        config.validate()?;
        if let Remote::Named(name) = &remote {
            if name.is_empty() || name.len() > 256 || name.contains('\0') {
                return Err(MediaError::InvalidArgument("remote name"));
            }
        }
        let epoch = NEXT_EPOCH
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| MediaError::ResourceLimit("connection epochs"))?;
        let shared = Arc::new(Shared {
            registry: Mutex::new(RegistryState::new(
                epoch,
                matches!(remote, Remote::Portal(_)),
                config.max_objects,
            )),
            subscribers: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
        });
        let (commands, receiver) = mpsc::sync_channel(config.command_capacity);
        let handle = ConnectionHandle {
            shared: shared.clone(),
            commands,
            max_subscriptions: config.max_subscriptions,
        };
        let settings = config.clone();
        let worker = std::thread::Builder::new()
            .name("telorgon-pipewire".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    super::worker::run(settings, remote, &shared, receiver)
                }));
                match result {
                    Ok(Ok(())) => shared.state(ConnectionState::Stopped),
                    Ok(Err(error)) => shared.state(ConnectionState::Failed(error)),
                    Err(_) => {
                        shared.state(ConnectionState::Failed(native("control worker panicked")))
                    }
                }
            })
            .map_err(native)?;
        Ok(Self {
            handle,
            worker: Some(worker),
            config,
        })
    }
    pub fn handle(&self) -> ConnectionHandle {
        self.handle.clone()
    }
    /// Nonblocking, idempotent, bypasses the bounded command queue.
    pub fn request_shutdown(&self) {
        self.handle.shared.stop.store(true, Ordering::Release);
    }
    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }
    /// Blocking join. Do not call from a subscription wake callback or UI/realtime thread.
    pub fn shutdown(&mut self) -> Result<(), MediaError> {
        if self.worker.as_ref().is_some_and(|worker| worker.thread().id() == std::thread::current().id()) {
            return Err(MediaError::InvalidArgument("cannot join the PipeWire worker from its callback"));
        }
        self.request_shutdown();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| native("control worker panicked"))?;
        }
        Ok(())
    }
    /// Only after the old worker finishes. Old handles stay disconnected. Authorization is
    /// supplied again explicitly; no streams or system mutations are replayed on recovery.
    pub fn reconnect(&mut self, remote: Remote) -> Result<(), MediaError> {
        if !self.is_finished() {
            return Err(MediaError::NotReady);
        }
        self.shutdown()?;
        *self = Self::connect(self.config.clone(), remote)?;
        Ok(())
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}
impl ConnectionHandle {
    pub fn state(&self) -> ConnectionState {
        self.shared
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .state
            .clone()
    }
    pub fn snapshot(&self) -> RegistrySnapshot {
        self.shared
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .clone()
    }
    pub fn subscribe(
        &self,
        capacity: usize,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Subscription, MediaError> {
        if !(1..=4096).contains(&capacity) {
            return Err(MediaError::InvalidArgument("event capacity"));
        }
        let mut list = self
            .shared
            .subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        list.retain(|s| s.strong_count() > 0);
        if list.len() >= self.max_subscriptions {
            return Err(MediaError::ResourceLimit("subscriptions"));
        }
        let inner = Arc::new(Subscriber {
            queue: Mutex::new(EventQueue {
                events: VecDeque::with_capacity(capacity),
                capacity,
                overflow: true,
            }),
            wake: Arc::new(wake),
        });
        list.push(Arc::downgrade(&inner));
        Ok(Subscription { inner })
    }
    /// A server round trip; confirms preceding native protocol processing, not policy or
    /// hardware completion. Commands are FIFO in channel acceptance order.
    pub fn barrier(&self) -> Result<Request, MediaError> {
        self.ensure_ready()?;
        let request = Request::new();
        self.send(Command::Barrier(Completion(request.state.clone())))?;
        Ok(request)
    }
    pub(crate) fn ensure_ready(&self) -> Result<(), MediaError> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(MediaError::Disconnected);
        }
        match self
            .shared
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .state
        {
            ConnectionState::Ready => Ok(()),
            ConnectionState::Connecting => Err(MediaError::NotReady),
            _ => Err(MediaError::Disconnected),
        }
    }
    pub(crate) fn send(&self, command: Command) -> Result<(), MediaError> {
        self.commands.try_send(command).map_err(|e| match e {
            mpsc::TrySendError::Full(_) => MediaError::QueueFull,
            mpsc::TrySendError::Disconnected(_) => MediaError::Disconnected,
        })
    }
}

#[cfg(any(feature = "desktop-audio-linux", feature = "video-linux"))]
impl ConnectionHandle {
    pub(crate) fn mutate(&self, mutation: super::control::Mutation) -> Result<Request, MediaError> {
        self.ensure_ready()?;
        if self.snapshot().restricted && !mutation.allows_restricted() {
            return Err(MediaError::PermissionDenied);
        }
        let request = Request::new();
        self.send(Command::Mutate(mutation, Completion(request.state.clone())))?;
        Ok(request)
    }
}

#[cfg(test)]
impl ConnectionHandle {
    pub(crate) fn test_channel(capacity: usize) -> (Self, mpsc::Receiver<Command>) {
        let shared = Arc::new(Shared {
            registry: Mutex::new(RegistryState::new(1, false, 8)),
            subscribers: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
        });
        shared.state(ConnectionState::Ready);
        let (commands, receiver) = mpsc::sync_channel(capacity);
        (
            Self {
                shared,
                commands,
                max_subscriptions: 2,
            },
            receiver,
        )
    }
}

#[cfg(feature = "audio-linux")]
impl ConnectionHandle {
    pub(crate) fn create_audio(
        &self,
        id: u64,
        config: crate::media::audio::AudioConfig,
        shared: Arc<crate::media::audio::AudioShared>,
        data: crate::media::audio::RealtimeData,
    ) -> Result<(), MediaError> {
        self.ensure_ready()?;
        self.send(Command::CreateAudio(id, config, shared, data))
    }
    pub(crate) fn audio_command(
        &self,
        id: u64,
        command: super::audio::AudioCommand,
    ) -> Result<Request, MediaError> {
        self.ensure_ready()?;
        let request = Request::new();
        self.send(Command::Audio(
            id,
            command,
            Completion(request.state.clone()),
        ))?;
        Ok(request)
    }
}
