//! Host-started XDG ScreenCast backend. The frontend portal owns OpenPipeWireRemote.
//! No compositor objects cross this boundary; starting a request never grants capture authority.
#[path = "transient_grants.rs"]
mod transient_grants;

#[path = "grant_worker.rs"]
mod grant_worker;
#[path = "grants.rs"]
mod grants;
#[path = "restore.rs"]
mod restore;
pub(crate) use restore::RestoreSource;
pub(crate) fn new_restore_instance() -> Result<String, String> {
    grants::random_id().map_err(|e| e.to_string())
}
#[path = "state.rs"]
mod state;

use crate::shell::capture::CaptureOptions;
use async_channel::{Receiver, Sender};
use futures_lite::{
    StreamExt,
    future::{block_on, race},
};
use state::{Options, Sessions};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::Duration;
use zbus::{
    Connection,
    message::Header,
    zvariant::{OwnedObjectPath, OwnedValue},
};

const FRONTEND: &str = "org.freedesktop.portal.Desktop";
pub(crate) const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.telorgon";
const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
type Response = (u32, HashMap<String, OwnedValue>);

/// Closing the channel is a level-triggered cancellation signal: it cannot be dropped because
/// a command queue is full. The host checks it before consent, allocation and frame delivery.
pub(crate) struct Lease {
    pub id: u64,
    forget: AtomicBool,
    saved: Mutex<Option<restore::RestoreData>>,
    cancel: Sender<()>,
    cancelled: Receiver<()>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl Lease {
    fn new(id: u64, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (cancel, cancelled) = async_channel::bounded(1);
        Self {
            id,
            forget: AtomicBool::new(false),
            saved: Mutex::new(None),
            cancel,
            cancelled,
            wake,
        }
    }
    pub fn has_saved_grant(&self) -> bool {
        self.saved
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
    /// Explicit host user action. Ordinary session Close retains a saved permission.
    pub fn forget(&self) {
        self.forget.store(true, Ordering::Release);
        self.close();
        (self.wake)();
    }
    pub fn closed(&self) -> bool {
        self.cancel.is_closed()
    }
    pub fn close(&self) {
        if self.cancel.close() {
            (self.wake)();
        }
    }
}

// Keep validation and advertised capabilities in sync as source implementations land.
pub(super) const AVAILABLE_SOURCE_TYPES: u32 = 7;
pub(crate) const MAX_STREAMS: usize = 8;

pub(crate) struct StartRequest {
    pub requester: u64,
    pub app_id: String,
    pub lease: Arc<Lease>,
    pub options: CaptureOptions,
    pub source_types: u32,
    pub multiple: bool,
    pub persistence: u32,
    pub restore: Option<restore::RestoreData>,
    reply: Sender<Result<StartCompletion, u32>>,
}
impl StartRequest {
    pub fn allows_source(&self, source: crate::shell::capture::CaptureSource) -> bool {
        self.source_types & source_type(source) != 0
    }

    /// Readiness must come from the actual transport; the host still owns source selection.
    pub fn complete(self, result: Result<Vec<StreamInfo>, u32>) {
        self.complete_checked(result.map(|streams| StartCompletion {
            streams,
            sources: Vec::new(),
            reuse: false,
            persist: false,
        }));
    }
    /// Host-only: call after explicit persistent consent and matching each approved source
    /// to its current authoritative restoration key. Acceptance still does not mean saved.
    pub fn complete_persistent(
        self,
        streams: Vec<StreamInfo>,
        sources: Vec<restore::RestoreSource>,
    ) {
        if sources.is_empty() {
            self.lease.close();
            self.complete(Err(2));
            return;
        }
        self.complete_checked(Ok(StartCompletion {
            streams,
            sources,
            reuse: false,
            persist: true,
        }));
    }
    pub fn discard_restore(&mut self) {
        self.restore = None;
        self.lease
            .saved
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }
    pub fn complete_restored(self, streams: Vec<StreamInfo>, sources: Vec<RestoreSource>) {
        if !self
            .restore
            .as_ref()
            .is_some_and(|grant| grant.sources == sources)
            || sources.is_empty()
        {
            self.lease.close();
            self.complete(Err(2));
            return;
        }
        let persist = self.persistence != 0;
        self.complete_checked(Ok(StartCompletion {
            streams,
            sources,
            reuse: true,
            persist,
        }));
    }
    fn complete_checked(self, result: Result<StartCompletion, u32>) {
        let result = result.and_then(|completion| {
            let persistent = !completion.sources.is_empty();
            let keys_valid = !persistent
                || ((matches!(self.persistence, 1 | 2) || completion.reuse)
                    && completion.sources.len() <= MAX_STREAMS
                    && completion.sources.len() == completion.streams.len()
                    && completion
                        .sources
                        .iter()
                        .zip(&completion.streams)
                        .all(|(key, stream)| key.kind == source_type(stream.source))
                    && restore::RestoreData {
                        app_id: self.app_id.clone(),
                        grant_id: "0".repeat(64),
                        sources: completion.sources.clone(),
                    }
                    .valid_for(&self.app_id, self.source_types, self.multiple));
            if valid_streams(&completion.streams, self.multiple, self.source_types)
                && keys_valid
                && !self.lease.closed()
            {
                Ok(completion)
            } else {
                self.lease.close();
                Err(2)
            }
        });
        if self.reply.try_send(result).is_err() {
            self.lease.close();
        }
    }
}

#[derive(Debug)]
pub(super) struct StartCompletion {
    streams: Vec<StreamInfo>,
    // Nonempty for explicit remembering consent or verified restoration. Mode 0 never creates saved grants.
    sources: Vec<restore::RestoreSource>,
    reuse: bool,
    persist: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StreamInfo {
    pub node_id: u32,
    pub width: i32,
    pub height: i32,
    pub source: crate::shell::capture::CaptureSource,
}

/// Owned by the compositor. All queues are bounded, and no bus wait occurs on its owner thread.
pub(crate) type PermissionReply = Receiver<Result<Vec<(String, usize)>, String>>;
pub(crate) struct Portal {
    transient: Arc<Mutex<transient_grants::Store>>,
    grants: grant_worker::Client,
    requests: Receiver<StartRequest>,
    stop: Sender<()>,
    state: Arc<Mutex<Sessions>>,
    thread: Option<JoinHandle<()>>,
}

struct RevokeOnDrop(Arc<Mutex<Sessions>>);
impl Drop for RevokeOnDrop {
    fn drop(&mut self) {
        // A worker panic must revoke its grants too; poisoned state is still usable for closing
        // cancellation channels and is never used to admit another request here.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revoke_except(None);
    }
}
impl Portal {
    pub fn start(
        wake: Arc<dyn Fn() + Send + Sync>,
        source_types: u32,
        activate_frontend: bool,
    ) -> Result<Self, String> {
        if source_types == 0 || source_types & !AVAILABLE_SOURCE_TYPES != 0 {
            return Err("invalid portal source types".into());
        }
        let (send, requests) = async_channel::bounded(state::MAX_SESSIONS);
        let (stop, stopped) = async_channel::bounded(1);
        let state = Arc::new(Mutex::new(Sessions::with_source_types(source_types)));
        let grants = grant_worker::Worker::start(grant_worker::default_path())?;
        let grant_client = grants.client.clone();
        let transient = Arc::new(Mutex::new(transient_grants::Store::default()));
        let backend = Backend {
            transient: transient.clone(),
            state: state.clone(),
            requests: send,
            grants: Some(grants.client.clone()),
            wake,
        };
        let thread = std::thread::Builder::new()
            .name("telorgon-portal".into())
            .spawn(move || {
                let _revoke = RevokeOnDrop(backend.state.clone());
                // Keep grant I/O alive through final bus cleanup after the host drops Portal.
                let _grants = grants;
                block_on(async {
                    // Reconnect without blocking desktop startup. Every disconnect revokes the old
                    // generation before a new bus connection can accept requests.
                    while !stopped.is_closed() {
                        let outcome = race(serve(backend.clone(), activate_frontend), async {
                            let _ = stopped.recv().await;
                            Ok(())
                        })
                        .await;
                        revoke_forgotten(&backend).await;
                        backend.state.lock().unwrap().revoke_except(None);
                        if stopped.is_closed() {
                            break;
                        }
                        if let Err(error) = outcome {
                            eprintln!("telorgon-portal: {error}");
                        }
                        race(
                            async {
                                async_io::Timer::after(Duration::from_secs(2)).await;
                            },
                            async {
                                let _ = stopped.recv().await;
                            },
                        )
                        .await;
                    }
                });
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            grants: grant_client,
            transient,
            requests,
            stop,
            state,
            thread: Some(thread),
        })
    }
    pub fn manage_permissions(
        &self,
        app: Option<String>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<PermissionReply, String> {
        let reply = self.grants.manage(app.clone(), wake)?;
        if let Some(app) = app {
            self.transient
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .revoke(&app, None);
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .invalidate_application(&app);
        }
        Ok(reply)
    }
    pub fn forget(&self, lease: &Lease) {
        let saved = lease
            .saved
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(grant) = saved {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .invalidate_grant(&grant.app_id, &grant.grant_id);
        }
        lease.forget();
    }
    pub fn try_failure(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .failures
            .pop_front()
    }
    pub fn try_request(&self) -> Option<StartRequest> {
        self.requests.try_recv().ok()
    }
}
impl Drop for Portal {
    fn drop(&mut self) {
        self.stop.close();
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revoke_except(None);
        // The bus worker has no GPU/host references. Never wait for bus I/O during host teardown.
        if self.thread.as_ref().is_some_and(|t| t.is_finished()) {
            let _ = self.thread.take().unwrap().join();
        }
    }
}

#[derive(Clone)]
struct Backend {
    transient: Arc<Mutex<transient_grants::Store>>,
    grants: Option<grant_worker::Client>,
    state: Arc<Mutex<Sessions>>,
    requests: Sender<StartRequest>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

async fn trusted_owner(connection: &Connection, header: &Header<'_>) -> zbus::fdo::Result<String> {
    let sender = header
        .sender()
        .ok_or_else(|| zbus::fdo::Error::AccessDenied("missing sender".into()))?;
    let proxy = zbus::fdo::DBusProxy::new(connection).await?;
    let owner = proxy.get_name_owner(FRONTEND.try_into().unwrap()).await?;
    if sender.as_str() != owner.as_str() {
        return Err(zbus::fdo::Error::AccessDenied(
            "only the desktop portal may request capture".into(),
        ));
    }
    Ok(owner.to_string())
}

fn response(code: u32) -> Response {
    (code, HashMap::new())
}

#[zbus::interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl Backend {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }
    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        self.state.lock().unwrap().available_source_types
    }
    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        7 // Hidden | Embedded | Metadata.
    }

    async fn create_session(
        &self,
        handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        options: Options,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let owner = trusted_owner(connection, &header).await?;
        let _ = options;
        if !state::valid_path(handle.as_str(), "request") {
            return Ok(response(2));
        }
        let lease = self.state.lock().unwrap().create(
            session_handle.as_str(),
            &owner,
            &app_id,
            self.wake.clone(),
        );
        let Ok(lease) = lease else {
            return Ok(response(2));
        };
        let registered = connection
            .object_server()
            .at(
                session_handle.clone(),
                SessionObject {
                    owner,
                    lease: lease.clone(),
                },
            )
            .await;
        if !self
            .state
            .lock()
            .unwrap()
            .registered(session_handle.as_str(), lease.id)
        {
            lease.close();
            if matches!(registered, Ok(true)) {
                let _ = connection
                    .object_server()
                    .remove::<SessionObject, _>(session_handle)
                    .await;
            }
            return Ok(response(2));
        }
        if !matches!(registered, Ok(true)) || lease.closed() {
            lease.close();
            return Ok(response(2));
        }
        Ok(response(0))
    }

    async fn select_sources(
        &self,
        handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        options: Options,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let owner = trusted_owner(connection, &header).await?;
        if !state::valid_path(handle.as_str(), "request") {
            return Ok(response(2));
        }
        let selected =
            self.state
                .lock()
                .unwrap()
                .select(session_handle.as_str(), &owner, &app_id, &options);
        Ok(response(if selected.is_ok() { 0 } else { 2 }))
    }

    async fn start(
        &self,
        handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        options: Options,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let owner = trusted_owner(connection, &header).await?;
        let _ = (parent_window, options); // The trusted shell owns placement of its consent surface.
        let begun = self.state.lock().unwrap().begin(
            session_handle.as_str(),
            &owner,
            &app_id,
            handle.as_str(),
        );
        let Ok((mut request, reply)) = begun else {
            return Ok(response(2));
        };
        let lease = request.lease.clone();
        if let Some(hint) = request.restore.take() {
            let revision = self.state.lock().unwrap().grant_revision;
            let temporary = self
                .transient
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .resolve(
                    &owner,
                    &hint,
                    &request.app_id,
                    request.source_types,
                    request.multiple,
                );
            if let Some(grant) = temporary {
                request.persistence = request.persistence.min(1);
                let mut state = self.state.lock().unwrap();
                if state.attach_restore(&lease, grant.clone(), revision) {
                    request.restore = Some(grant);
                }
            } else if let Some(grants) = &self.grants {
                request.restore = race(
                    async {
                        grants
                            .resolve(
                                hint,
                                request.app_id.clone(),
                                request.source_types,
                                request.multiple,
                            )
                            .await
                            .unwrap_or(None)
                    },
                    async {
                        let _ = lease.cancelled.recv().await;
                        None
                    },
                )
                .await;
                if let Some(grant) = request.restore.as_ref() {
                    if !self
                        .state
                        .lock()
                        .unwrap()
                        .attach_restore(&lease, grant.clone(), revision)
                    {
                        request.restore = None;
                    }
                }
            }
        }
        let persistence = request.persistence;
        let restored_grant = request.restore.clone();
        let registered = connection
            .object_server()
            .at(
                handle.clone(),
                RequestObject {
                    owner: owner.clone(),
                    lease: lease.clone(),
                },
            )
            .await;
        let mut result = if !matches!(registered, Ok(true)) || lease.closed() {
            Err(2)
        } else if self.requests.try_send(request).is_err() {
            Err(2)
        } else {
            (self.wake)();
            race(async { reply.recv().await.unwrap_or(Err(2)) }, async {
                let _ = lease.cancelled.recv().await;
                Err(1)
            })
            .await
        };
        let mut saved = None;
        if let Ok(completion) = &mut result {
            if !completion.sources.is_empty() && !completion.reuse {
                let sources = std::mem::take(&mut completion.sources);
                let prepared = if persistence == 1 {
                    transient_grants::Store::prepare(
                        &self.transient,
                        owner.clone(),
                        app_id,
                        sources,
                    )
                } else {
                    prepare_grant(self.grants.as_ref(), &lease, app_id, sources)
                        .await
                        .map(|grant| transient_grants::Delivery::Durable(Some(grant)))
                };
                match prepared {
                    Ok(grant) => saved = Some(grant),
                    Err(code) => result = Err(code),
                }
            }
        }
        // Only this Start invocation removes its request; cleanup never reuses its identity.
        if matches!(registered, Ok(true)) {
            let _ = connection
                .object_server()
                .remove::<RequestObject, _>(handle)
                .await;
        }
        let accepted =
            self.state
                .lock()
                .unwrap()
                .finish(session_handle.as_str(), lease.id, result.is_ok());
        match result {
            Ok(info) if accepted => {
                let mut response = stream_response(info.streams);
                if info.reuse && info.persist {
                    let Some(encoded) = restored_grant
                        .as_ref()
                        .and_then(restore::RestoreData::encode)
                    else {
                        lease.close();
                        return Ok(crate::integrations::portals::backend::response(2));
                    };
                    response.1.insert("restore_data".into(), encoded);
                    response.1.insert("persist_mode".into(), persistence.into());
                }
                if let Some(grant) = saved {
                    let Some(encoded) = grant.record().and_then(restore::RestoreData::encode)
                    else {
                        lease.close();
                        return Ok(crate::integrations::portals::backend::response(2));
                    };
                    response.1.insert("restore_data".into(), encoded);
                    response.1.insert("persist_mode".into(), persistence.into());
                    // No await follows ownership transfer; cancellation before here drops the
                    // delivery guard and schedules durable revocation on the grant worker.
                    *lease.saved.lock().unwrap_or_else(|e| e.into_inner()) = Some(grant.claim());
                    (self.wake)();
                }
                Ok(response)
            }
            Err(code) => Ok(response(code)),
            _ => Ok(response(1)),
        }
    }
}

async fn prepare_grant(
    grants: Option<&grant_worker::Client>,
    lease: &Lease,
    app: String,
    sources: Vec<restore::RestoreSource>,
) -> Result<grant_worker::Delivery, u32> {
    let Some(grants) = grants else { return Err(2) };
    if lease.closed() {
        return Err(1);
    }
    let prepared = race(
        async { grants.prepare(app, sources).await.map_err(|_| 2u32) },
        async {
            let _ = lease.cancelled.recv().await;
            Err(1)
        },
    )
    .await?;
    if lease.closed() {
        return Err(1);
    }
    Ok(prepared)
}

fn source_type(source: crate::shell::capture::CaptureSource) -> u32 {
    match source {
        crate::shell::capture::CaptureSource::Output(_) => 1,
        crate::shell::capture::CaptureSource::Window(_) => 2,
        crate::shell::capture::CaptureSource::VirtualOutput(_) => 4,
    }
}

fn valid_streams(streams: &[StreamInfo], multiple: bool, source_types: u32) -> bool {
    !streams.is_empty()
        && streams.len() <= if multiple { MAX_STREAMS } else { 1 }
        && streams.iter().enumerate().all(|(index, info)| {
            info.node_id != u32::MAX
                && info.width > 0
                && info.height > 0
                && source_type(info.source) & source_types != 0
                && !streams[..index]
                    .iter()
                    .any(|other| other.node_id == info.node_id || other.source == info.source)
        })
}

fn stream_response(streams: Vec<StreamInfo>) -> Response {
    use zbus::zvariant::Value;
    let streams: Vec<_> = streams
        .into_iter()
        .map(|info| {
            let mut properties = HashMap::<String, OwnedValue>::from([
                (
                    "size".into(),
                    Value::from((info.width, info.height)).try_into().unwrap(),
                ),
                ("source_type".into(), source_type(info.source).into()),
            ]);
            // Window coordinates are local; only monitor streams have a desktop position.
            if matches!(info.source, crate::shell::capture::CaptureSource::Output(_)) {
                properties.insert(
                    "position".into(),
                    Value::from((0i32, 0i32)).try_into().unwrap(),
                );
            }
            (info.node_id, properties)
        })
        .collect();
    (
        0,
        HashMap::from([
            ("streams".into(), Value::from(streams).try_into().unwrap()),
            // Omitting this would implicitly grant the requested persistence mode.
            ("persist_mode".into(), 0u32.into()),
        ]),
    )
}

struct SessionObject {
    owner: String,
    lease: Arc<Lease>,
}
#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl SessionObject {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
    fn close(&self, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        check_owner(&header, &self.owner)?;
        self.lease.close();
        Ok(())
    }
    #[zbus(signal)]
    async fn closed(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
}
struct RequestObject {
    owner: String,
    lease: Arc<Lease>,
}
#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestObject {
    fn close(&self, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        check_owner(&header, &self.owner)?;
        self.lease.close();
        Ok(())
    }
}
fn check_owner(header: &Header<'_>, owner: &str) -> zbus::fdo::Result<()> {
    if header.sender().is_some_and(|s| s.as_str() == owner) {
        Ok(())
    } else {
        Err(zbus::fdo::Error::AccessDenied(
            "request belongs to another caller".into(),
        ))
    }
}

async fn revoke_forgotten(backend: &Backend) {
    let grants: Vec<_> = {
        let state = backend.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .entries
            .values()
            .filter_map(|session| {
                if !session.lease.forget.load(Ordering::Acquire) {
                    return None;
                }
                let saved = session
                    .lease
                    .saved
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()?;
                Some((session.lease.clone(), saved))
            })
            .collect()
    };
    for (lease, grant) in grants {
        backend
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .invalidate_grant(&grant.app_id, &grant.grant_id);
        // Preserve the level-triggered intent and record across await. If serve is cancelled
        // by disconnect/shutdown, the outer worker retries cleanup before losing the registry.
        let temporary = backend
            .transient
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .revoke(&grant.app_id, Some(&grant.grant_id));
        let result = if temporary {
            Ok(())
        } else {
            match &backend.grants {
                Some(client) => client
                    .revoke(grant.app_id, Some(grant.grant_id))
                    .await
                    .map(|_| ()),
                None => Err("grant worker unavailable".into()),
            }
        };
        lease.forget.store(false, Ordering::Release);
        if let Err(error) = result {
            let mut state = backend.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.failures.len() == state::MAX_SESSIONS {
                state.failures.pop_front();
            }
            state.failures.push_back(format!(
                "Sharing stopped, but its saved permission could not be removed: {error}"
            ));
            (backend.wake)();
        } else {
            lease.saved.lock().unwrap_or_else(|e| e.into_inner()).take();
        }
    }
}

async fn serve(backend: Backend, activate_frontend: bool) -> zbus::Result<()> {
    // A previous bus generation has no surviving exported objects. Drop its revoked registry.
    backend.state.lock().unwrap().entries.clear();
    backend
        .transient
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain_owner(None);
    let connection = zbus::connection::Builder::session()?
        .method_timeout(Duration::from_secs(3))
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, backend.clone())?
        .build()
        .await?;
    let proxy = zbus::fdo::DBusProxy::new(&connection).await?;
    if activate_frontend {
        // The shell has already published the explicitly owned session environment.
        // D-Bus activation is idempotent; never restart another running desktop's frontend.
        if let Err(error) = proxy
            .start_service_by_name(FRONTEND.try_into().unwrap(), 0)
            .await
        {
            eprintln!(
                "telorgon-portal: frontend activation failed: {error}; check xdg-desktop-portal installation and desktop portal configuration"
            );
        }
    }
    eprintln!(
        "telorgon-portal: ScreenCast backend registered (source types: {})",
        backend.available_source_types()
    );
    let mut owners = proxy
        .receive_name_owner_changed_with_args(&[(0, FRONTEND)])
        .await?;
    loop {
        // Signals give prompt cancellation. Periodic owner checks cover creation racing a signal
        // and bus disconnects, including a frontend that has not started yet.
        let disconnected = race(async { owners.next().await.is_none() }, async {
            async_io::Timer::after(Duration::from_millis(200)).await;
            false
        })
        .await;
        if disconnected {
            return Err(zbus::Error::Failure("portal bus disconnected".into()));
        }
        let owner = match proxy.get_name_owner(FRONTEND.try_into().unwrap()).await {
            Ok(owner) => Some(owner),
            Err(zbus::fdo::Error::NameHasNoOwner(_)) => None,
            Err(error) => return Err(error.into()),
        };
        revoke_forgotten(&backend).await;
        backend
            .transient
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain_owner(owner.as_ref().map(|owner| owner.as_str()));
        let retired = {
            let state = backend.state.lock().unwrap();
            state.revoke_except(owner.as_ref().map(|owner| owner.as_str()));
            // Keep identities reserved until registration and Start cleanup have completed.
            // Otherwise a Close racing export could orphan an object or remove a replacement.
            state
                .entries
                .iter()
                .filter(|(_, s)| s.lease.closed() && !s.registering && s.request_path.is_none())
                .map(|(path, s)| (path.clone(), s.lease.id))
                .collect::<Vec<_>>()
        };
        for (path, id) in retired {
            let emitter = zbus::object_server::SignalEmitter::new(&connection, path.as_str())?;
            let _ = SessionObject::closed(&emitter).await;
            let _ = connection
                .object_server()
                .remove::<SessionObject, _>(path.as_str())
                .await;
            let mut state = backend.state.lock().unwrap();
            if state.entries.get(&path).is_some_and(|s| s.lease.id == id) {
                state.entries.remove(&path);
            }
        }
    }
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
