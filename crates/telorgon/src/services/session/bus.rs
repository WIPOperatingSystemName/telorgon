//! Optional integration with the existing user bus. Never starts a second bus or changes env.
use super::{Environment, Error, Result};
use std::collections::HashMap;
use std::time::Duration;
use zbus::blocking::{Connection, Proxy, connection::Builder};

fn bus_error(error: impl std::fmt::Display) -> Error {
    Error::Io(format!("D-Bus: {error}"))
}
fn connect(env: &Environment) -> Result<Connection> {
    let address = env
        .get("DBUS_SESSION_BUS_ADDRESS")
        .and_then(|s| s.to_str())
        .ok_or_else(|| {
            Error::Invalid(
                "this operation needs the existing user's DBUS_SESSION_BUS_ADDRESS".into(),
            )
        })?;
    Builder::address(address)
        .map_err(bus_error)?
        .method_timeout(Duration::from_secs(3))
        .build()
        .map_err(bus_error)
}
pub(crate) fn application_address(id: &str) -> Result<(String, String)> {
    let name = id
        .strip_suffix(".desktop")
        .ok_or_else(|| Error::Invalid("invalid desktop ID".into()))?;
    zbus::names::WellKnownName::try_from(name).map_err(bus_error)?;
    let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
    zbus::zvariant::ObjectPath::try_from(path.as_str()).map_err(bus_error)?;
    Ok((name.into(), path))
}
pub(crate) fn activate(
    env: &Environment,
    id: &str,
    uris: &[String],
    token: Option<&str>,
) -> Result<String> {
    let (name, path) = application_address(id)?;
    let connection = connect(env)?;
    let proxy = Proxy::new(
        &connection,
        name.as_str(),
        path.as_str(),
        "org.freedesktop.Application",
    )
    .map_err(bus_error)?;
    let mut data = HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    if let Some(token) = token {
        data.insert("activation-token", token.into());
    }
    if uris.is_empty() {
        proxy
            .call::<_, _, ()>("Activate", &(data,))
            .map_err(bus_error)?;
    } else {
        proxy
            .call::<_, _, ()>("Open", &(uris, data))
            .map_err(bus_error)?;
    }
    drop(proxy);
    Ok(name)
}

const KEYS: &[&str] = &[
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "DISPLAY",
    "XAUTHORITY",
];
/// Single-graphical-session ownership only. D-Bus cannot read/unset activation variables, so
/// restoration uses the systemd snapshot and restores originally missing values as empty on D-Bus.
pub(crate) struct ServiceEnvironment {
    connection: Connection,
    previous: HashMap<String, String>,
    published: HashMap<String, String>,
}
impl ServiceEnvironment {
    pub fn publish(env: &Environment) -> Result<Self> {
        let connection = connect(env)?;
        let manager = Proxy::new(
            &connection,
            "org.freedesktop.systemd1",
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
        )
        .map_err(bus_error)?;
        let previous = parse_environment(
            manager
                .get_property::<Vec<String>>("Environment")
                .map_err(bus_error)?,
        );
        let published = KEYS
            .iter()
            .map(|key| {
                (
                    key.to_string(),
                    env.get(key)
                        .map(|v| v.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
            })
            .collect::<HashMap<_, _>>();
        let guard = Self {
            connection: connection.clone(),
            previous,
            published,
        };
        let values = guard
            .published
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>();
        manager
            .call::<_, _, ()>("SetEnvironment", &(values,))
            .map_err(bus_error)?;
        let dbus = Proxy::new(
            &connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .map_err(bus_error)?;
        dbus.call::<_, _, ()>("UpdateActivationEnvironment", &(&guard.published,))
            .map_err(bus_error)?;
        Ok(guard)
    }
    fn restore(&self) -> Result<()> {
        let manager = Proxy::new(
            &self.connection,
            "org.freedesktop.systemd1",
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
        )
        .map_err(bus_error)?;
        let current = parse_environment(
            manager
                .get_property::<Vec<String>>("Environment")
                .map_err(bus_error)?,
        );
        let keys = self
            .published
            .iter()
            .filter(|(key, value)| current.get(*key) == Some(*value))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let set = keys
            .iter()
            .filter_map(|key| self.previous.get(key).map(|value| format!("{key}={value}")))
            .collect::<Vec<_>>();
        manager
            .call::<_, _, ()>("UnsetAndSetEnvironment", &(&keys, set))
            .map_err(bus_error)?;
        let restore = keys
            .iter()
            .map(|key| {
                (
                    key.clone(),
                    self.previous.get(key).cloned().unwrap_or_default(),
                )
            })
            .collect::<HashMap<_, _>>();
        let dbus = Proxy::new(
            &self.connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .map_err(bus_error)?;
        dbus.call::<_, _, ()>("UpdateActivationEnvironment", &(restore,))
            .map_err(bus_error)
    }
}
impl Drop for ServiceEnvironment {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("telorgon-session: could not restore user-service environment: {error}");
        }
    }
}
fn parse_environment(values: Vec<String>) -> HashMap<String, String> {
    values
        .into_iter()
        .filter_map(|value| {
            value
                .split_once('=')
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_id_maps_to_the_specified_dbus_object_path() {
        assert_eq!(
            application_address("org.example.My-App.desktop").unwrap(),
            ("org.example.My-App".into(), "/org/example/My_App".into())
        );
        assert!(application_address("../../bad.desktop").is_err());
        assert_eq!(
            parse_environment(vec!["A=a=b".into()]).get("A").unwrap(),
            "a=b"
        );
    }
}
