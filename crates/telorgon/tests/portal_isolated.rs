#![cfg(all(target_os = "linux", feature = "portal-client-linux"))]
//! Mock portal on a private D-Bus daemon, using only the harness's synthetic PipeWire
//! socket. This proves protocol/lifetime behavior, not consent UI or real camera access.
use futures_lite::future::{block_on, race, zip};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, AtomicUsize, Ordering},
    },
    time::Duration,
};
use telorgon::integrations::{pipewire as pw, portals::client::*};
use zbus::{
    Connection,
    message::Header,
    zvariant::{OwnedObjectPath, OwnedValue},
};
type Dict = HashMap<String, OwnedValue>;
const NAME: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
#[derive(Default)]
struct State {
    response: AtomicU32,
    request_closes: AtomicUsize,
    session_closes: AtomicUsize,
    remote_opens: AtomicUsize,
    sessions: Mutex<Vec<OwnedObjectPath>>,
    nodes: Mutex<Vec<(u32, u64)>>,
    selected: Mutex<Option<(bool, u32, Option<String>)>>,
}
fn value(s: &str) -> OwnedValue {
    zbus::zvariant::Str::from(s.to_owned()).into()
}
fn path(header: &Header<'_>, kind: &str, token: &str) -> OwnedObjectPath {
    let sender = header
        .sender()
        .unwrap()
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    format!("{PATH}/{kind}/{sender}/{token}")
        .try_into()
        .unwrap()
}
async fn respond(
    connection: &Connection,
    header: &Header<'_>,
    options: &Dict,
    state: &Arc<State>,
    results: Dict,
) -> zbus::fdo::Result<OwnedObjectPath> {
    let token = options
        .get("handle_token")
        .and_then(|v| <&str>::try_from(v).ok())
        .ok_or_else(|| zbus::fdo::Error::InvalidArgs("token".into()))?;
    let path = path(header, "request", token);
    connection
        .object_server()
        .at(path.clone(), MockRequest(state.clone()))
        .await?;
    let code = state.response.load(Ordering::Relaxed);
    if code < 3 {
        // Intentionally before the method reply: clients must subscribe first.
        connection
            .emit_signal(
                header.sender().map(|s| s.as_str()),
                path.as_str(),
                REQUEST,
                "Response",
                &(code, results),
            )
            .await?;
    }
    Ok(path)
}
fn remote(state: &State) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
    let name = std::env::var("TELORGON_TEST_REMOTE").unwrap();
    assert!(name.starts_with("telorgon-test-"));
    let path =
        std::path::PathBuf::from(std::env::var_os("PIPEWIRE_RUNTIME_DIR").unwrap()).join(name);
    let socket = std::os::unix::net::UnixStream::connect(path)
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
    state.remote_opens.fetch_add(1, Ordering::Relaxed);
    Ok(std::os::fd::OwnedFd::from(socket).into())
}
struct MockRequest(Arc<State>);
#[zbus::interface(name = "org.freedesktop.portal.Request")]
impl MockRequest {
    fn close(&self) {
        self.0.request_closes.fetch_add(1, Ordering::Relaxed);
    }
}
struct MockSession(Arc<State>);
#[zbus::interface(name = "org.freedesktop.portal.Session")]
impl MockSession {
    async fn close(
        &self,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.0.session_closes.fetch_add(1, Ordering::Relaxed);
        Self::closed(&emitter, Dict::new()).await?;
        Ok(())
    }
    #[zbus(signal)]
    async fn closed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        details: Dict,
    ) -> zbus::Result<()>;
}
struct MockCamera(Arc<State>);
#[zbus::interface(name = "org.freedesktop.portal.Camera")]
impl MockCamera {
    #[zbus(property)]
    fn is_camera_present(&self) -> bool {
        true
    }
    async fn access_camera(
        &self,
        options: Dict,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        respond(connection, &header, &options, &self.0, Dict::new()).await
    }
    fn open_pipe_wire_remote(&self, _options: Dict) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        remote(&self.0)
    }
}
struct MockScreen(Arc<State>);
#[zbus::interface(name = "org.freedesktop.portal.ScreenCast")]
impl MockScreen {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        6
    }
    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        7
    }
    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        7
    }
    async fn create_session(
        &self,
        options: Dict,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let token = options
            .get("session_handle_token")
            .and_then(|v| <&str>::try_from(v).ok())
            .unwrap();
        let session = path(&header, "session", token);
        connection
            .object_server()
            .at(session.clone(), MockSession(self.0.clone()))
            .await?;
        self.0.sessions.lock().unwrap().push(session.clone());
        respond(
            connection,
            &header,
            &options,
            &self.0,
            Dict::from([("session_handle".into(), value(session.as_str()))]),
        )
        .await
    }
    async fn select_sources(
        &self,
        _session: OwnedObjectPath,
        options: Dict,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let multiple = bool::try_from(options.get("multiple").unwrap()).unwrap();
        let mode = u32::try_from(options.get("persist_mode").unwrap()).unwrap();
        let token = options
            .get("restore_token")
            .and_then(|v| <&str>::try_from(v).ok())
            .map(str::to_owned);
        *self.0.selected.lock().unwrap() = Some((multiple, mode, token));
        respond(connection, &header, &options, &self.0, Dict::new()).await
    }
    async fn start(
        &self,
        _session: OwnedObjectPath,
        _parent: String,
        options: Dict,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let multiple = self.0.selected.lock().unwrap().as_ref().unwrap().0;
        let streams = self
            .0
            .nodes
            .lock()
            .unwrap()
            .iter()
            .take(if multiple { 2 } else { 1 })
            .map(|(id, serial)| {
                (
                    *id,
                    Dict::from([
                        ("pipewire-serial".into(), OwnedValue::from(*serial)),
                        ("source_type".into(), OwnedValue::from(1u32)),
                        (
                            "size".into(),
                            OwnedValue::try_from(zbus::zvariant::Value::from((320i32, 240i32)))
                                .unwrap(),
                        ),
                    ]),
                )
            })
            .collect::<Vec<_>>();
        let results = Dict::from([
            (
                "streams".into(),
                OwnedValue::try_from(zbus::zvariant::Value::from(streams)).unwrap(),
            ),
            ("restore_token".into(), value("rotated-token")),
        ]);
        respond(connection, &header, &options, &self.0, results).await
    }
    fn open_pipe_wire_remote(
        &self,
        _session: OwnedObjectPath,
        _options: Dict,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        remote(&self.0)
    }
}
async fn until(mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "private portal timed out"
        );
        async_io::Timer::after(Duration::from_millis(5)).await;
    }
}
async fn connect(address: &str) -> Connection {
    zbus::connection::Builder::address(address)
        .unwrap()
        .build()
        .await
        .unwrap()
}
#[test]
#[ignore = "requires the isolated synthetic PipeWire and private D-Bus harness"]
fn portal_requests_cancel_rotate_tokens_and_revoke_native_connections() {
    block_on(async {
        let address = std::env::var("TELORGON_TEST_BUS_ADDRESS").expect("use test_pipewire.py");
        assert!(address.starts_with("unix:path=/tmp/telorgon-pw-"));
        let state = Arc::new(State::default());
        let server = connect(&address).await;
        server
            .object_server()
            .at(PATH, MockCamera(state.clone()))
            .await
            .unwrap();
        server
            .object_server()
            .at(PATH, MockScreen(state.clone()))
            .await
            .unwrap();
        server.request_name(NAME).await.unwrap();
        let client =
            PortalClient::from_connection(connect(&address).await, Duration::from_millis(500))
                .await
                .unwrap();
        assert!(client.camera_present().await.unwrap());
        assert_eq!(client.screencast_capabilities().await.unwrap().version, 6);
        let mut media = client
            .access_camera(
                pw::ConnectionConfig::default(),
                &PortalCancellation::default(),
            )
            .await
            .unwrap();
        let handle = media.handle();
        until(|| handle.state() == pw::ConnectionState::Ready).await;
        assert!(handle.snapshot().restricted);
        *state.nodes.lock().unwrap() = handle
            .snapshot()
            .objects_of_kind(pw::ObjectKind::Node)
            .take(2)
            .map(|o| {
                (
                    o.handle.id(),
                    o.properties["object.serial"].parse().unwrap(),
                )
            })
            .collect();
        assert_eq!(state.nodes.lock().unwrap().len(), 2);
        media.shutdown().unwrap();
        state.response.store(2, Ordering::Relaxed);
        assert!(matches!(
            client
                .access_camera(
                    pw::ConnectionConfig::default(),
                    &PortalCancellation::default()
                )
                .await,
            Err(PortalError::Rejected(2))
        ));
        assert_eq!(state.remote_opens.load(Ordering::Relaxed), 1);
        state.response.store(3, Ordering::Relaxed);
        let cancel = PortalCancellation::default();
        let (result, _) = zip(
            client.access_camera(pw::ConnectionConfig::default(), &cancel),
            async {
                async_io::Timer::after(Duration::from_millis(50)).await;
                cancel.cancel();
            },
        )
        .await;
        assert!(matches!(result, Err(PortalError::Cancelled)));
        until(|| state.request_closes.load(Ordering::Relaxed) >= 1).await;
        // Dropping the actual future (e.g. a GUI navigation) must also close its request.
        let cancel = PortalCancellation::default();
        race(
            async {
                let _ = client
                    .access_camera(pw::ConnectionConfig::default(), &cancel)
                    .await;
            },
            async {
                async_io::Timer::after(Duration::from_millis(50)).await;
            },
        )
        .await;
        until(|| state.request_closes.load(Ordering::Relaxed) >= 2).await;
        assert!(matches!(
            client
                .access_camera(
                    pw::ConnectionConfig::default(),
                    &PortalCancellation::default()
                )
                .await,
            Err(PortalError::Timeout)
        ));
        until(|| state.request_closes.load(Ordering::Relaxed) >= 3).await;
        state.response.store(0, Ordering::Relaxed);
        let options = ScreenCastOptions {
            multiple: true,
            persistence: Persistence::UntilRevoked,
            restore_token: Some("old-token".into()),
            cursor_mode: CursorMode::Metadata,
            ..Default::default()
        };
        let session = client
            .share_screen(
                options,
                pw::ConnectionConfig::default(),
                &PortalCancellation::default(),
            )
            .await
            .unwrap();
        assert_eq!(session.sources().len(), 2);
        assert_eq!(session.restore_token(), Some("rotated-token"));
        assert_eq!(
            *state.selected.lock().unwrap(),
            Some((true, 2, Some("old-token".into())))
        );
        let handle = session.connection();
        for source in session.sources() {
            assert!(source.resolve(&handle).is_ok());
        }
        let path = state.sessions.lock().unwrap().last().unwrap().clone();
        server
            .emit_signal(
                None::<&str>,
                path.as_str(),
                SESSION,
                "Closed",
                &(Dict::new(),),
            )
            .await
            .unwrap();
        until(|| session.is_closed()).await;
        until(|| handle.state() == pw::ConnectionState::Stopped).await;
        assert!(handle.barrier().is_err());
        session.close().await;
        let session = client
            .share_screen(
                ScreenCastOptions::default(),
                pw::ConnectionConfig::default(),
                &PortalCancellation::default(),
            )
            .await
            .unwrap();
        let handle = session.connection();
        server.release_name(NAME).await.unwrap();
        until(|| session.is_closed()).await;
        until(|| handle.state() == pw::ConnectionState::Stopped).await;
        session.close().await;
        assert!(state.session_closes.load(Ordering::Relaxed) >= 2);
    });
}
