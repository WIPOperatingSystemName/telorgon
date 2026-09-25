//! Portal request protocol: subscribe before the method call, close on cancellation/drop,
//! pin the frontend's unique bus owner, and bound response queues and waits.
use super::client::{Permit, PortalCancellation, PortalError};
use futures_lite::{StreamExt, future::race};
use std::sync::Arc;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use zbus::{
    Connection, MatchRule, MessageStream, Proxy,
    message::Type,
    zvariant::{OwnedObjectPath, OwnedValue},
};
pub(super) const FRONTEND: &str = "org.freedesktop.portal.Desktop";
pub(super) const PATH: &str = "/org/freedesktop/portal/desktop";
pub(super) const SCREEN: &str = "org.freedesktop.portal.ScreenCast";
pub(super) const CAMERA: &str = "org.freedesktop.portal.Camera";
pub(super) const SESSION: &str = "org.freedesktop.portal.Session";
pub(super) const REQUEST: &str = "org.freedesktop.portal.Request";
pub(super) type Dict = HashMap<String, OwnedValue>;
static TOKEN: AtomicU64 = AtomicU64::new(1);
pub(super) fn token() -> String {
    format!(
        "telorgon_{}_{}",
        std::process::id(),
        TOKEN.fetch_add(1, Ordering::Relaxed)
    )
}
pub(super) fn bus(error: impl std::fmt::Display) -> PortalError {
    PortalError::Bus(error.to_string().chars().take(512).collect())
}
pub(super) fn text(value: &str) -> OwnedValue {
    OwnedValue::from(zbus::zvariant::Str::from(value.to_owned()))
}
pub(super) fn object_path(
    connection: &Connection,
    kind: &str,
    token: &str,
) -> Result<OwnedObjectPath, PortalError> {
    let unique = connection
        .unique_name()
        .ok_or(PortalError::InvalidResponse("bus unique name"))?;
    let sender = unique.as_str().trim_start_matches(':').replace('.', "_");
    OwnedObjectPath::try_from(format!("{PATH}/{kind}/{sender}/{token}")).map_err(bus)
}
pub(super) async fn proxy(
    connection: &Connection,
    owner: &str,
    path: &str,
    interface: &str,
) -> Result<Proxy<'static>, PortalError> {
    Proxy::new_owned(
        connection.clone(),
        owner.to_owned(),
        path.to_owned(),
        interface.to_owned(),
    )
    .await
    .map_err(bus)
}
pub(super) async fn signals(
    connection: &Connection,
    owner: &str,
    path: Option<&str>,
    interface: &str,
    member: &str,
) -> Result<MessageStream, PortalError> {
    let mut rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(owner)
        .map_err(bus)?
        .interface(interface)
        .map_err(bus)?
        .member(member)
        .map_err(bus)?;
    if let Some(path) = path {
        rule = rule.path(path).map_err(bus)?;
    }
    MessageStream::for_match_rule(rule.build(), connection, Some(32))
        .await
        .map_err(bus)
}
pub(super) async fn bounded<T>(
    timeout: Duration,
    cancel: &PortalCancellation,
    future: impl std::future::Future<Output = Result<T, PortalError>>,
) -> Result<T, PortalError> {
    if cancel.is_cancelled() {
        return Err(PortalError::Cancelled);
    }
    race(
        future,
        race(
            async {
                let _ = cancel.receiver.recv().await;
                Err(PortalError::Cancelled)
            },
            async {
                async_io::Timer::after(timeout).await;
                Err(PortalError::Timeout)
            },
        ),
    )
    .await
}
/// Dropping a pending async future must still issue Close. Cleanup runs on the existing
/// zbus executor with a two-second deadline; it never blocks the GUI or spawns a thread.
pub(super) struct CloseGuard {
    pub connection: Connection,
    pub owner: String,
    pub path: OwnedObjectPath,
    pub interface: &'static str,
    pub armed: bool,
    pub permit: Option<Arc<Permit>>,
}
impl CloseGuard {
    pub async fn close(&mut self) {
        if !self.armed {
            return;
        }
        let close = async {
            if let Ok(proxy) = proxy(
                &self.connection,
                &self.owner,
                self.path.as_str(),
                self.interface,
            )
            .await
            {
                let _ = proxy.call::<_, _, ()>("Close", &()).await;
            }
        };
        race(close, async {
            async_io::Timer::after(Duration::from_secs(2)).await;
        })
        .await;
        self.armed = false;
    }
}
impl Drop for CloseGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.armed = false;
        let mut guard = Self {
            connection: self.connection.clone(),
            owner: self.owner.clone(),
            path: self.path.clone(),
            interface: self.interface,
            armed: true,
            permit: self.permit.take(),
        };
        self.connection
            .executor()
            .spawn(
                async move {
                    guard.close().await;
                },
                "telorgon-portal-close",
            )
            .detach();
    }
}
pub(super) async fn request<B>(
    connection: &Connection,
    owner: &str,
    interface: &str,
    method: &str,
    token: &str,
    body: &B,
    timeout: Duration,
    cancel: &PortalCancellation,
    permit: Arc<Permit>,
) -> Result<Dict, PortalError>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let predicted = object_path(connection, "request", token)?;
    let mut guard = CloseGuard {
        connection: connection.clone(),
        owner: owner.into(),
        path: predicted,
        interface: REQUEST,
        armed: true,
        permit: Some(permit),
    };
    let result = bounded(timeout, cancel, async {
        // No path filter until the reply: older frontends may return a nonstandard path.
        // Subscribing first also captures a Response emitted before the method reply.
        let mut responses = signals(connection, owner, None, REQUEST, "Response").await?;
        let portal = proxy(connection, owner, PATH, interface).await?;
        let path: OwnedObjectPath = portal.call(method, body).await.map_err(bus)?;
        guard.path = path;
        while let Some(message) = responses.next().await {
            let message = message.map_err(bus)?;
            if message.header().path().map(|p| p.as_str()) != Some(guard.path.as_str()) {
                continue;
            }
            if message.body().len() > 256 * 1024 {
                return Err(PortalError::InvalidResponse("oversized response"));
            }
            let (status, results): (u32, Dict) = message.body().deserialize().map_err(bus)?;
            guard.armed = false;
            return match status {
                0 => Ok(results),
                1 => Err(PortalError::Cancelled),
                code => Err(PortalError::Rejected(code)),
            };
        }
        Err(PortalError::Disconnected)
    })
    .await;
    if result.is_err() {
        guard.close().await;
    }
    result
}
