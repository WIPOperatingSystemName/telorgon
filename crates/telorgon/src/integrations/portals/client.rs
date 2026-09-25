//! Async camera and screen-sharing clients. Requests may display portal consent UI; call
//! only in response to application intent. No call falls back to an unrestricted remote.
//! Futures run on any async executor and never block a UI thread. Dropping a request future
//! closes it; dropping a screen session revokes its local media connection and closes the
//! portal session. Restoration tokens are returned to the application, never stored here.
use super::request::*;
use crate::integrations::pipewire::{
    Connection, ConnectionConfig, ConnectionHandle, MediaError, Remote,
};
use futures_lite::{StreamExt, future::race};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PortalError {
    #[error("portal request cancelled")]
    Cancelled,
    #[error("portal rejected or failed the request (response {0})")]
    Rejected(u32),
    #[error("portal request timed out")]
    Timeout,
    #[error("portal disconnected")]
    Disconnected,
    #[error("portal capability unavailable: {0}")]
    Unsupported(&'static str),
    #[error("invalid portal response: {0}")]
    InvalidResponse(&'static str),
    #[error("invalid portal options: {0}")]
    InvalidOptions(&'static str),
    #[error("too many pending requests or sessions")]
    ResourceLimit,
    #[error("portal bus error: {0}")]
    Bus(String),
    #[error(transparent)]
    Media(#[from] MediaError),
}
/// Level-triggered cancellation shared with a running request. Clones refer to the same
/// operation; cancellation cannot be reset. Drop alone does not cancel other clones.
#[derive(Clone, Debug)]
pub struct PortalCancellation {
    sender: async_channel::Sender<()>,
    pub(super) receiver: async_channel::Receiver<()>,
}
impl Default for PortalCancellation {
    fn default() -> Self {
        let (sender, receiver) = async_channel::bounded(1);
        Self { sender, receiver }
    }
}
impl PortalCancellation {
    pub fn cancel(&self) {
        self.sender.close();
    }
    pub fn is_cancelled(&self) -> bool {
        self.sender.is_closed()
    }
}
pub(super) struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One session-bus connection pinned to the portal's unique owner. At most 32 pending
/// requests/screen sessions per client, including asynchronous cleanup. Recreate the client
/// after frontend restart; grants and in-flight work are never replayed automatically.
pub struct PortalClient {
    connection: zbus::Connection,
    owner: String,
    timeout: Duration,
    permits: Arc<AtomicUsize>,
}
impl PortalClient {
    pub async fn session_bus() -> Result<Self, PortalError> {
        let cancel = PortalCancellation::default();
        let connection = bounded(Duration::from_secs(5), &cancel, async {
            zbus::Connection::session().await.map_err(bus)
        })
        .await?;
        Self::from_connection(connection, Duration::from_secs(120)).await
    }
    /// Advanced bus injection, including a private bus for automated tests. Uses the standard
    /// frontend name and object paths. The connection must run zbus's internal executor.
    pub async fn from_connection(
        connection: zbus::Connection,
        timeout: Duration,
    ) -> Result<Self, PortalError> {
        if timeout.is_zero() || timeout > Duration::from_secs(600) {
            return Err(PortalError::InvalidOptions("request timeout"));
        }
        let cancel = PortalCancellation::default();
        let owner = bounded(Duration::from_secs(5), &cancel, async {
            let dbus = zbus::fdo::DBusProxy::new(&connection).await.map_err(bus)?;
            // Activating the frontend does not request capture or access any device.
            if dbus
                .get_name_owner(FRONTEND.try_into().map_err(bus)?)
                .await
                .is_err()
            {
                dbus.start_service_by_name(FRONTEND.try_into().map_err(bus)?, 0)
                    .await
                    .map_err(bus)?;
            }
            Ok(dbus
                .get_name_owner(FRONTEND.try_into().map_err(bus)?)
                .await
                .map_err(bus)?
                .to_string())
        })
        .await?;
        Ok(Self {
            connection,
            owner,
            timeout,
            permits: Arc::new(AtomicUsize::new(0)),
        })
    }
    fn permit(&self) -> Result<Arc<Permit>, PortalError> {
        self.permits
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 32).then_some(n + 1)
            })
            .map_err(|_| PortalError::ResourceLimit)?;
        Ok(Arc::new(Permit(self.permits.clone())))
    }
    pub async fn camera_present(&self) -> Result<bool, PortalError> {
        bounded(
            Duration::from_secs(5),
            &PortalCancellation::default(),
            async {
                proxy(&self.connection, &self.owner, PATH, CAMERA)
                    .await?
                    .get_property("IsCameraPresent")
                    .await
                    .map_err(bus)
            },
        )
        .await
    }
    /// Presents the camera access request and returns an owned restricted connection.
    /// Readiness remains asynchronous through ConnectionHandle::state. No camera starts
    /// streaming until a separate video stream is explicitly opened on this connection.
    pub async fn access_camera(
        &self,
        config: ConnectionConfig,
        cancel: &PortalCancellation,
    ) -> Result<Connection, PortalError> {
        let permit = self.permit()?;
        let token = token();
        let options = Dict::from([("handle_token".into(), text(&token))]);
        request(
            &self.connection,
            &self.owner,
            CAMERA,
            "AccessCamera",
            &token,
            &(options,),
            self.timeout,
            cancel,
            permit.clone(),
        )
        .await?;
        let fd: zbus::zvariant::OwnedFd = bounded(Duration::from_secs(5), cancel, async {
            proxy(&self.connection, &self.owner, PATH, CAMERA)
                .await?
                .call("OpenPipeWireRemote", &(Dict::new(),))
                .await
                .map_err(bus)
        })
        .await?;
        if cancel.is_cancelled() {
            return Err(PortalError::Cancelled);
        }
        Ok(Connection::connect(config, Remote::Portal(fd.into()))?)
    }
    pub async fn screencast_capabilities(&self) -> Result<ScreenCastCapabilities, PortalError> {
        bounded(
            Duration::from_secs(5),
            &PortalCancellation::default(),
            async {
                let proxy = proxy(&self.connection, &self.owner, PATH, SCREEN).await?;
                let version: u32 = proxy.get_property("version").await.map_err(bus)?;
                let source_types = proxy
                    .get_property("AvailableSourceTypes")
                    .await
                    .map_err(bus)?;
                let cursor_modes = if version >= 2 {
                    proxy
                        .get_property("AvailableCursorModes")
                        .await
                        .map_err(bus)?
                } else {
                    1
                };
                Ok(ScreenCastCapabilities {
                    version,
                    source_types,
                    cursor_modes,
                })
            },
        )
        .await
    }
    /// Selects and starts one portal session with up to 16 sources. The portal decides
    /// which requested permissions are granted. Closing or revoking the session stops all
    /// media on its restricted connection. Request acceptance alone never yields a session.
    pub async fn share_screen(
        &self,
        options: ScreenCastOptions,
        config: ConnectionConfig,
        cancel: &PortalCancellation,
    ) -> Result<ScreenCastSession, PortalError> {
        let permit = self.permit()?;
        let capabilities = bounded(
            Duration::from_secs(5),
            cancel,
            self.screencast_capabilities(),
        )
        .await?;
        options.validate(capabilities)?;
        let session_token = token();
        let path = object_path(&self.connection, "session", &session_token)?;
        let mut guard = CloseGuard {
            connection: self.connection.clone(),
            owner: self.owner.clone(),
            path,
            interface: SESSION,
            armed: true,
            permit: Some(permit.clone()),
        };
        let request_token = token();
        let create = Dict::from([
            ("handle_token".into(), text(&request_token)),
            ("session_handle_token".into(), text(&session_token)),
        ]);
        let mut result = request(
            &self.connection,
            &self.owner,
            SCREEN,
            "CreateSession",
            &request_token,
            &(create,),
            self.timeout,
            cancel,
            permit.clone(),
        )
        .await?;
        let session = result
            .remove("session_handle")
            .and_then(|v| String::try_from(v).ok())
            .ok_or(PortalError::InvalidResponse("session handle"))?;
        guard.path = OwnedObjectPath::try_from(session).map_err(bus)?;
        let mut closed = bounded(
            Duration::from_secs(5),
            cancel,
            signals(
                &self.connection,
                &self.owner,
                Some(guard.path.as_str()),
                SESSION,
                "Closed",
            ),
        )
        .await?;
        let owner_rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")
            .map_err(bus)?
            .interface("org.freedesktop.DBus")
            .map_err(bus)?
            .member("NameOwnerChanged")
            .map_err(bus)?
            .arg(0, FRONTEND)
            .map_err(bus)?
            .build();
        let mut owner_changed =
            zbus::MessageStream::for_match_rule(owner_rule, &self.connection, Some(4))
                .await
                .map_err(bus)?;
        let request_token = token();
        let mut select = Dict::from([
            ("handle_token".into(), text(&request_token)),
            ("types".into(), OwnedValue::from(options.source_types)),
            ("multiple".into(), OwnedValue::from(options.multiple)),
        ]);
        if capabilities.version >= 2 {
            select.insert(
                "cursor_mode".into(),
                OwnedValue::from(options.cursor_mode as u32),
            );
        }
        if capabilities.version >= 4 {
            select.insert(
                "persist_mode".into(),
                OwnedValue::from(options.persistence as u32),
            );
            if let Some(token) = &options.restore_token {
                select.insert("restore_token".into(), text(token));
            }
        }
        request(
            &self.connection,
            &self.owner,
            SCREEN,
            "SelectSources",
            &request_token,
            &(&guard.path, select),
            self.timeout,
            cancel,
            permit.clone(),
        )
        .await?;
        let request_token = token();
        let start = Dict::from([("handle_token".into(), text(&request_token))]);
        let mut result = request(
            &self.connection,
            &self.owner,
            SCREEN,
            "Start",
            &request_token,
            &(&guard.path, options.parent_window, start),
            self.timeout,
            cancel,
            permit.clone(),
        )
        .await?;
        let mut streams = parse_streams(
            result
                .remove("streams")
                .ok_or(PortalError::InvalidResponse("missing streams"))?,
            options.multiple,
        )?;
        let restore_token = result
            .remove("restore_token")
            .map(|v| String::try_from(v).map_err(bus))
            .transpose()?;
        if restore_token.as_ref().is_some_and(|s| s.len() > 4096) {
            return Err(PortalError::InvalidResponse("restore token length"));
        }
        let fd: zbus::zvariant::OwnedFd = bounded(Duration::from_secs(5), cancel, async {
            proxy(&self.connection, &self.owner, PATH, SCREEN)
                .await?
                .call("OpenPipeWireRemote", &(&guard.path, Dict::new()))
                .await
                .map_err(bus)
        })
        .await?;
        if cancel.is_cancelled() {
            return Err(PortalError::Cancelled);
        }
        let ready_timeout = config.connect_timeout;
        let media = Connection::connect(config, Remote::Portal(fd.into()))?;
        let handle = media.handle();
        bounded(ready_timeout, cancel, async {
            loop {
                match handle.state() {
                    crate::integrations::pipewire::ConnectionState::Ready => break,
                    crate::integrations::pipewire::ConnectionState::Failed(error) => {
                        return Err(PortalError::Media(error));
                    }
                    crate::integrations::pipewire::ConnectionState::Stopped => {
                        return Err(PortalError::Disconnected);
                    }
                    _ => {
                        async_io::Timer::after(Duration::from_millis(5)).await;
                    }
                }
            }
            let snapshot = handle.snapshot();
            for source in &mut streams {
                source.bound = Some(
                    snapshot
                        .objects_of_kind(crate::integrations::pipewire::ObjectKind::Node)
                        .find(|o| match source.pipewire_serial {
                            Some(serial) => {
                                o.properties
                                    .get("object.serial")
                                    .and_then(|s| s.parse::<u64>().ok())
                                    == Some(serial)
                            }
                            None => o.handle.id() == source.node_id,
                        })
                        .map(|o| o.handle)
                        .ok_or(MediaError::StaleHandle)?,
                );
            }
            Ok(())
        })
        .await?;
        let session_closed = Arc::new(AtomicBool::new(false));
        let stopped = session_closed.clone();
        let (close, closing) = async_channel::bounded::<()>(1);
        let (done, finished) = async_channel::bounded::<()>(1);
        self.connection
            .executor()
            .spawn(
                async move {
                    race(
                        async {
                            let _ = closing.recv().await;
                        },
                        race(
                            async {
                                let _ = closed.next().await;
                            },
                            async {
                                let _ = owner_changed.next().await;
                            },
                        ),
                    )
                    .await;
                    stopped.store(true, Ordering::Release);
                    // Portal authorization owns this restricted connection's lifetime. Handles are
                    // non-owning; no reconnect or unrestricted replacement can outlive revocation.
                    handle.shared.stop.store(true, Ordering::Release);
                    guard.close().await;
                    done.close();
                    drop(permit);
                },
                "telorgon-portal-session",
            )
            .detach();
        Ok(ScreenCastSession {
            media,
            streams,
            restore_token,
            closed: session_closed,
            close,
            finished,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ScreenCastCapabilities {
    pub version: u32,
    pub source_types: u32,
    pub cursor_modes: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CursorMode {
    Hidden = 1,
    Embedded = 2,
    Metadata = 4,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Persistence {
    None = 0,
    Application = 1,
    UntilRevoked = 2,
}
#[derive(Clone, Debug)]
pub struct ScreenCastOptions {
    /// MONITOR=1, WINDOW=2, VIRTUAL=4; a nonempty subset of advertised source types.
    pub source_types: u32,
    pub multiple: bool,
    pub cursor_mode: CursorMode,
    pub persistence: Persistence,
    /// A single-use previous token; the application must replace it with the returned token.
    pub restore_token: Option<String>,
    /// Portal window identifier (e.g. wayland exported handle); empty permits an unparented dialog.
    pub parent_window: String,
}
impl Default for ScreenCastOptions {
    fn default() -> Self {
        Self {
            source_types: 1,
            multiple: false,
            cursor_mode: CursorMode::Embedded,
            persistence: Persistence::None,
            restore_token: None,
            parent_window: String::new(),
        }
    }
}
impl ScreenCastOptions {
    fn validate(&self, caps: ScreenCastCapabilities) -> Result<(), PortalError> {
        if self.source_types == 0
            || self.source_types & !7 != 0
            || self.parent_window.len() > 4096
            || self
                .restore_token
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 4096)
        {
            return Err(PortalError::InvalidOptions(
                "sources, window identifier or restore token",
            ));
        }
        if self.source_types & !caps.source_types != 0 {
            return Err(PortalError::Unsupported("requested source types"));
        }
        if self.cursor_mode as u32 & caps.cursor_modes == 0 {
            return Err(PortalError::Unsupported("cursor mode"));
        }
        if caps.version < 4
            && (self.persistence != Persistence::None || self.restore_token.is_some())
        {
            return Err(PortalError::Unsupported(
                "persistent sessions require portal version 4",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct ScreenCastSource {
    bound: Option<crate::integrations::pipewire::ObjectHandle>,
    pub node_id: u32,
    pub pipewire_serial: Option<u64>,
    pub id: Option<String>,
    pub source_type: Option<u32>,
    pub position: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
}
impl ScreenCastSource {
    /// Resolve only within this session's restricted snapshot. Version 6 serials prevent
    /// matching a node ID reused after source removal. Retain the returned handle thereafter.
    pub fn resolve(
        &self,
        connection: &ConnectionHandle,
    ) -> Result<crate::integrations::pipewire::ObjectHandle, MediaError> {
        connection.ensure_ready()?;
        let snapshot = connection.snapshot();
        if !snapshot.restricted {
            return Err(MediaError::PermissionDenied);
        }
        let handle = self.bound.ok_or(MediaError::StaleHandle)?;
        snapshot.resolve(handle)?;
        Ok(handle)
    }
}
pub struct ScreenCastSession {
    media: Connection,
    streams: Vec<ScreenCastSource>,
    restore_token: Option<String>,
    closed: Arc<AtomicBool>,
    close: async_channel::Sender<()>,
    finished: async_channel::Receiver<()>,
}
impl ScreenCastSession {
    pub fn connection(&self) -> ConnectionHandle {
        self.media.handle()
    }
    pub fn sources(&self) -> &[ScreenCastSource] {
        &self.streams
    }
    pub fn restore_token(&self) -> Option<&str> {
        self.restore_token.as_deref()
    }
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    pub fn request_close(&self) {
        self.close.close();
        self.media.request_shutdown();
        self.closed.store(true, Ordering::Release);
    }
    /// Waits asynchronously for the bounded portal Close call. PipeWire shutdown is requested
    /// immediately; this does not synchronously join a native worker on the caller's executor.
    pub async fn close(&self) {
        self.request_close();
        let _ = self.finished.recv().await;
    }
}
impl Drop for ScreenCastSession {
    fn drop(&mut self) {
        self.request_close();
    }
}
fn parse_streams(value: OwnedValue, multiple: bool) -> Result<Vec<ScreenCastSource>, PortalError> {
    let values = Vec::<(u32, Dict)>::try_from(value).map_err(bus)?;
    if values.is_empty() || values.len() > 16 || (!multiple && values.len() != 1) {
        return Err(PortalError::InvalidResponse("stream count"));
    }
    let mut streams = Vec::with_capacity(values.len());
    for (node_id, mut properties) in values {
        if streams
            .iter()
            .any(|s: &ScreenCastSource| s.node_id == node_id)
        {
            return Err(PortalError::InvalidResponse("duplicate stream node"));
        }
        let pipewire_serial = properties
            .remove("pipewire-serial")
            .map(u64::try_from)
            .transpose()
            .map_err(bus)?;
        let id = properties
            .remove("id")
            .map(String::try_from)
            .transpose()
            .map_err(bus)?;
        if id.as_ref().is_some_and(|s| s.len() > 4096) {
            return Err(PortalError::InvalidResponse("stream ID length"));
        }
        let source_type = properties
            .remove("source_type")
            .map(u32::try_from)
            .transpose()
            .map_err(bus)?;
        let position = properties
            .remove("position")
            .map(<(i32, i32)>::try_from)
            .transpose()
            .map_err(bus)?;
        let size = properties
            .remove("size")
            .map(<(i32, i32)>::try_from)
            .transpose()
            .map_err(bus)?;
        if size.is_some_and(|(w, h)| w <= 0 || h <= 0) {
            return Err(PortalError::InvalidResponse("stream size"));
        }
        streams.push(ScreenCastSource {
            bound: None,
            node_id,
            pipewire_serial,
            id,
            source_type,
            position,
            size,
        });
    }
    Ok(streams)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::pipewire::{ConnectionState, ObjectKind};
    fn streams(nodes: &[u32]) -> OwnedValue {
        OwnedValue::try_from(zbus::zvariant::Value::from(
            nodes
                .iter()
                .map(|id| (*id, Dict::new()))
                .collect::<Vec<_>>(),
        ))
        .unwrap()
    }
    #[test]
    fn response_bounds_and_versioned_capabilities_are_enforced() {
        assert!(parse_streams(streams(&[]), true).is_err());
        assert!(parse_streams(streams(&[1, 1]), true).is_err());
        assert!(parse_streams(streams(&[1, 2]), false).is_err());
        assert!(parse_streams(streams(&(0..17).collect::<Vec<_>>()), true).is_err());
        let mut options = ScreenCastOptions::default();
        options.restore_token = Some("previous".into());
        assert!(matches!(
            options.validate(ScreenCastCapabilities {
                version: 3,
                source_types: 7,
                cursor_modes: 7
            }),
            Err(PortalError::Unsupported(_))
        ));
        options.restore_token = None;
        options.source_types = 4;
        assert!(matches!(
            options.validate(ScreenCastCapabilities {
                version: 6,
                source_types: 3,
                cursor_modes: 7
            }),
            Err(PortalError::Unsupported(_))
        ));
    }
    #[test]
    fn resolved_portal_source_cannot_rebind_after_removal_or_shutdown() {
        let (handle, _) = ConnectionHandle::test_channel(1);
        let original = {
            let mut registry = handle.shared.registry.lock().unwrap();
            registry.snapshot.restricted = true;
            registry
                .insert(7, ObjectKind::Node, 7, Default::default())
                .unwrap()
        };
        let mut source = parse_streams(streams(&[7]), false).unwrap().remove(0);
        source.bound = Some(original);
        assert_eq!(source.resolve(&handle), Ok(original));
        {
            let mut registry = handle.shared.registry.lock().unwrap();
            registry.remove(7);
            registry
                .insert(7, ObjectKind::Node, 7, Default::default())
                .unwrap();
        }
        assert_eq!(source.resolve(&handle), Err(MediaError::StaleHandle));
        handle.shared.state(ConnectionState::Stopped);
        assert!(source.resolve(&handle).is_err());
    }
}
