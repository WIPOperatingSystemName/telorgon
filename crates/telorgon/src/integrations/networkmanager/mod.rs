//! Opt-in NetworkManager D-Bus adapter. Construction neither starts a daemon nor changes networking.
mod operations;
mod settings;
mod snapshot;
mod subscription;

use crate::services::network::*;
use std::{collections::HashMap, time::Duration};
use subscription::Subscription;
use zbus::{
    blocking::{Connection, Proxy, connection::Builder},
    zvariant::OwnedObjectPath,
};

const DESTINATION: &str = "org.freedesktop.NetworkManager";
const ROOT: &str = "/org/freedesktop/NetworkManager";
const SETTINGS: &str = "/org/freedesktop/NetworkManager/Settings";

#[derive(Clone, Debug)]
pub struct NetworkManagerConfig {
    pub method_timeout: Duration,
}
impl Default for NetworkManagerConfig {
    fn default() -> Self {
        Self {
            method_timeout: Duration::from_secs(5),
        }
    }
}

pub struct NetworkManagerProvider {
    inventory: crate::platform::network_linux::Inventory,
    config: NetworkManagerConfig,
    session: Option<Session>,
    interfaces: HashMap<String, NetworkInterfaceId>,
    profiles: HashMap<String, NetworkProfileId>,
    aps: HashMap<String, NetworkAccessPointId>,
    connections: HashMap<String, NetworkConnectionId>,
    pending: HashMap<u64, operations::Pending>,
    next_token: u64,
}
struct Session {
    connection: Connection,
    owner: String,
    subscription: Subscription,
}
impl Default for NetworkManagerProvider {
    fn default() -> Self {
        Self::new(Default::default())
    }
}
impl NetworkManagerProvider {
    pub fn new(config: NetworkManagerConfig) -> Self {
        Self {
            inventory: Default::default(),
            config,
            session: None,
            interfaces: HashMap::new(),
            profiles: HashMap::new(),
            aps: HashMap::new(),
            connections: HashMap::new(),
            pending: HashMap::new(),
            next_token: 1,
        }
    }
    fn reset_ids(&mut self) {
        self.interfaces.clear();
        self.profiles.clear();
        self.aps.clear();
        self.connections.clear();
    }
    fn ensure_session(&mut self) -> Result<(), NetworkError> {
        if self.config.method_timeout.is_zero() {
            return Err(NetworkError::InvalidConfig("D-Bus timeout cannot be zero"));
        }
        let connection = match &self.session {
            Some(session) => session.connection.clone(),
            None => Builder::system()
                .map_err(classify)?
                .method_timeout(self.config.method_timeout)
                .build()
                .map_err(classify)?,
        };
        let bus = Proxy::new(
            &connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .map_err(classify)?;
        let owner: String = bus
            .call("GetNameOwner", &(DESTINATION,))
            .map_err(classify)?;
        if self.session.as_ref().is_none_or(|s| s.owner != owner) {
            let subscription = Subscription::start(&connection, &owner)?;
            self.reset_ids();
            self.session = Some(Session {
                connection,
                owner,
                subscription,
            });
        }
        Ok(())
    }
    fn proxy<'a>(&'a self, path: &'a str, interface: &'a str) -> Result<Proxy<'a>, NetworkError> {
        let session = self.session.as_ref().ok_or(NetworkError::Unavailable)?;
        zbus::blocking::proxy::Builder::new(&session.connection)
            .destination(session.owner.as_str())
            .map_err(classify)?
            .path(path)
            .map_err(classify)?
            .interface(interface)
            .map_err(classify)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .map_err(classify)
    }
    fn path<T: PartialEq>(map: &HashMap<String, T>, id: T) -> Result<String, NetworkError> {
        map.iter()
            .find(|(_, v)| **v == id)
            .map(|(k, _)| k.clone())
            .ok_or(NetworkError::Stale)
    }
    fn drain_changes(&mut self) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        let (dirty, removed, overflow) = session.subscription.drain();
        if overflow {
            self.reset_ids();
        } else {
            for path in removed {
                self.interfaces.remove(&path);
                self.profiles.remove(&path);
                self.aps.remove(&path);
                self.connections.remove(&path);
            }
        }
        dirty
    }
}
impl NetworkProvider for NetworkManagerProvider {
    fn snapshot(&mut self) -> Result<NetworkSnapshot, NetworkError> {
        self.drain_changes();
        let result = self.ensure_session().and_then(|()| self.read_snapshot());
        let mut snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.session = None;
                self.reset_ids();
                NetworkSnapshot {
                    state: if error == NetworkError::PermissionDenied {
                        NetworkServiceState::Restricted
                    } else {
                        NetworkServiceState::Unavailable
                    },
                    backend: Some("NetworkManager".into()),
                    last_error: Some(error),
                    ..Default::default()
                }
            }
        };
        self.inventory.enrich(&mut snapshot);
        Ok(snapshot)
    }
    fn execute(&mut self, command: NetworkCommand) -> Result<NetworkDispatch, NetworkError> {
        self.ensure_session()?;
        self.drain_changes();
        self.execute_command(command)
    }
    fn poll(
        &mut self,
        token: u64,
        snapshot: &NetworkSnapshot,
    ) -> Result<Option<NetworkResult>, NetworkError> {
        self.poll_command(token, snapshot)
    }
    fn abandon(&mut self, token: u64) {
        self.pending.remove(&token);
    }
    fn has_changes(&mut self) -> bool {
        self.drain_changes()
    }
}

fn object(path: &str) -> Result<OwnedObjectPath, NetworkError> {
    OwnedObjectPath::try_from(path.to_owned()).map_err(|_| NetworkError::InvalidData)
}
fn classify(error: zbus::Error) -> NetworkError {
    match error {
        zbus::Error::MethodError(name, _, _) => {
            let name = name.as_str();
            if name.ends_with("AccessDenied")
                || name.ends_with("PermissionDenied")
                || name.ends_with("NotAuthorized")
            {
                NetworkError::PermissionDenied
            } else if name.ends_with("NoSecrets") {
                NetworkError::CredentialsRequired
            } else if name.ends_with("UnknownMethod") || name.ends_with("NotSupported") {
                NetworkError::Unsupported
            } else if name.ends_with("UnknownObject")
                || name.ends_with("UnknownDevice")
                || name.ends_with("UnknownConnection")
            {
                NetworkError::Stale
            } else if name.ends_with("NameHasNoOwner") || name.ends_with("ServiceUnknown") {
                NetworkError::Unavailable
            } else if name.ends_with("NoReply") || name.ends_with("Timeout") {
                NetworkError::TimedOut
            } else {
                NetworkError::BackendFailure
            }
        }
        zbus::Error::InputOutput(_) => NetworkError::Transport,
        _ => NetworkError::Transport,
    }
}

#[cfg(test)]
mod tests;
