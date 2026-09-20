//! Host-started XDG ScreenCast backend. The frontend portal owns OpenPipeWireRemote.
//! No compositor objects cross this boundary; starting a request never grants capture authority.

mod state;

use crate::shell::capture::CaptureOptions;
use async_channel::{Receiver, Sender};
use futures_lite::{
    StreamExt,
    future::{block_on, race},
};
use state::{Options, Sessions};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
    cancel: Sender<()>,
    cancelled: Receiver<()>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl Lease {
    fn new(id: u64, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (cancel, cancelled) = async_channel::bounded(1);
        Self {
            id,
            cancel,
            cancelled,
            wake,
        }
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
pub(super) const AVAILABLE_SOURCE_TYPES: u32 = 3;

pub(crate) struct StartRequest {
    pub requester: u64,
    pub app_id: String,
    pub lease: Arc<Lease>,
    pub options: CaptureOptions,
    pub source_types: u32,
    reply: Sender<Result<StreamInfo, u32>>,
}
impl StartRequest {
    pub fn allows_source(&self, source: crate::shell::capture::CaptureSource) -> bool {
        self.source_types & source_type(source) != 0
    }

    /// Readiness must come from the actual transport; the host still owns source selection.
    pub fn complete(self, result: Result<StreamInfo, u32>) {
        if self.reply.try_send(result).is_err() {
            self.lease.close();
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StreamInfo {
    pub node_id: u32,
    pub width: i32,
    pub height: i32,
    pub source: crate::shell::capture::CaptureSource,
}

/// Owned by the compositor. All queues are bounded, and no bus wait occurs on its owner thread.
pub(crate) struct Portal {
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
        let backend = Backend {
            state: state.clone(),
            requests: send,
            wake,
        };
        let thread = std::thread::Builder::new()
            .name("telorgon-portal".into())
            .spawn(move || {
                let _revoke = RevokeOnDrop(backend.state.clone());
                block_on(async {
                    // Reconnect without blocking desktop startup. Every disconnect revokes the old
                    // generation before a new bus connection can accept requests.
                    while !stopped.is_closed() {
                        let outcome = race(serve(backend.clone(), activate_frontend), async {
                            let _ = stopped.recv().await;
                            Ok(())
                        })
                        .await;
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
            requests,
            stop,
            state,
            thread: Some(thread),
        })
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
    // Version 3 deliberately omits persistence and restore-token semantics.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        self.state.lock().unwrap().available_source_types
    }
    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        3 // Hidden | Embedded. Metadata cursors remain unsupported.
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
        let Ok((request, reply)) = begun else {
            return Ok(response(2));
        };
        let lease = request.lease.clone();
        let registered = connection
            .object_server()
            .at(
                handle.clone(),
                RequestObject {
                    owner,
                    lease: lease.clone(),
                },
            )
            .await;
        let result = if !matches!(registered, Ok(true)) || lease.closed() {
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
            Ok(info) if accepted => Ok(stream_response(info)),
            Err(code) => Ok(response(code)),
            _ => Ok(response(1)),
        }
    }
}

fn source_type(source: crate::shell::capture::CaptureSource) -> u32 {
    match source {
        crate::shell::capture::CaptureSource::Output(_) => 1,
        crate::shell::capture::CaptureSource::Window(_) => 2,
    }
}

fn stream_response(info: StreamInfo) -> Response {
    use zbus::zvariant::Value;
    let mut properties = HashMap::<String, OwnedValue>::from([
        (
            "size".into(),
            Value::from((info.width, info.height)).try_into().unwrap(),
        ),
        ("source_type".into(), source_type(info.source).into()),
    ]);
    // A window stream has local capture coordinates, not the monitor's desktop position.
    if matches!(info.source, crate::shell::capture::CaptureSource::Output(_)) {
        properties.insert(
            "position".into(),
            Value::from((0i32, 0i32)).try_into().unwrap(),
        );
    }
    (
        0,
        HashMap::from([(
            "streams".into(),
            Value::from(vec![(info.node_id, properties)])
                .try_into()
                .unwrap(),
        )]),
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

async fn serve(backend: Backend, activate_frontend: bool) -> zbus::Result<()> {
    // A previous bus generation has no surviving exported objects. Drop its revoked registry.
    backend.state.lock().unwrap().entries.clear();
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
mod tests {
    use super::*;

    #[test]
    fn capture_config_advertises_only_configured_sources_without_starting_services() {
        for mask in [1u32, 2, 3] {
            let (requests, _) = async_channel::bounded(8);
            let backend = Backend {
                state: Arc::new(Mutex::new(Sessions::with_source_types(mask))),
                requests,
                wake: Arc::new(|| {}),
            };
            assert_eq!(backend.available_source_types(), mask);
        }
    }

    #[test]
    fn generated_backend_wire_contract_matches_the_portal_methods() {
        use zbus::object_server::Interface;
        let (requests, _) = async_channel::bounded(8);
        let backend = Backend {
            state: Arc::new(Mutex::new(Sessions::default())),
            requests,
            wake: Arc::new(|| {}),
        };
        // Advertise only source/cursor combinations accepted by this backend. These values
        // are part of the consumer contract, independently of the generated property types.
        assert_eq!(backend.version(), 3);
        assert_eq!(backend.available_source_types(), 3);
        assert_eq!(backend.available_cursor_modes(), 3);
        let mut xml = String::new();
        backend.introspect_to_writer(&mut xml, 0);
        let document = roxmltree::Document::parse(&xml).unwrap();
        let interface = document.root_element();
        assert_eq!(
            interface.attribute("name"),
            Some("org.freedesktop.impl.portal.ScreenCast")
        );
        for (name, input) in [
            ("CreateSession", vec!["o", "o", "s", "a{sv}"]),
            ("SelectSources", vec!["o", "o", "s", "a{sv}"]),
            ("Start", vec!["o", "o", "s", "s", "a{sv}"]),
        ] {
            let method = interface
                .children()
                .find(|n| n.has_tag_name("method") && n.attribute("name") == Some(name))
                .unwrap();
            let signature = |direction| {
                method
                    .children()
                    .filter(|n| {
                        n.has_tag_name("arg") && n.attribute("direction") == Some(direction)
                    })
                    .map(|n| n.attribute("type").unwrap())
                    .collect::<Vec<_>>()
            };
            assert_eq!(signature("in"), input, "{name}");
            assert_eq!(signature("out"), ["u", "a{sv}"], "{name}");
        }
        for name in ["version", "AvailableSourceTypes", "AvailableCursorModes"] {
            let property = interface
                .children()
                .find(|n| n.has_tag_name("property") && n.attribute("name") == Some(name))
                .unwrap();
            assert_eq!(property.attribute("type"), Some("u"));
            assert_eq!(property.attribute("access"), Some("read"));
        }
    }

    #[test]
    fn close_rejects_a_different_unique_bus_owner() {
        let message = zbus::Message::method_call(OBJECT_PATH, "Close")
            .unwrap()
            .sender(":1.2")
            .unwrap()
            .build(&())
            .unwrap();
        assert!(check_owner(&message.header(), ":1.2").is_ok());
        assert!(check_owner(&message.header(), ":1.3").is_err());
    }

    #[test]
    fn start_response_uses_portal_stream_tuple_signature() {
        let (code, result) = stream_response(StreamInfo {
            node_id: 42,
            width: 800,
            height: 600,
            source: crate::shell::capture::CaptureSource::Output(crate::shell::OutputId::MIN),
        });
        assert_eq!(code, 0);
        assert_eq!(result["streams"].value_signature().to_string(), "a(ua{sv})");
    }

    #[test]
    fn window_stream_metadata_does_not_claim_a_monitor_position() {
        use crate::shell::{WindowId, capture::CaptureSource};
        use std::num::NonZeroU32;
        let (_, mut result) = stream_response(StreamInfo {
            node_id: 42,
            width: 800,
            height: 600,
            source: CaptureSource::Window(WindowId::new(
                NonZeroU32::new(1).unwrap(),
                NonZeroU32::new(2).unwrap(),
            )),
        });
        let streams: Vec<(u32, HashMap<String, OwnedValue>)> =
            result.remove("streams").unwrap().try_into().unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].0, 42);
        assert_eq!(u32::try_from(&streams[0].1["source_type"]).unwrap(), 2);
        assert!(!streams[0].1.contains_key("position"));
        let size: (i32, i32) = streams[0].1["size"]
            .try_clone()
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(size, (800, 600));
    }
}
