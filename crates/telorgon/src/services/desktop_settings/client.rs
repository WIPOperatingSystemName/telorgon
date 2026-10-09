use super::*;
use std::time::Duration;
use zbus::blocking::{Connection, Proxy, connection::Builder};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsEndpoint {
    bus_name: String,
    object_path: String,
    interface: String,
}
impl SettingsEndpoint {
    pub fn new(
        bus_name: impl Into<String>,
        object_path: impl Into<String>,
        interface: impl Into<String>,
    ) -> Result<Self> {
        let endpoint = Self {
            bus_name: bus_name.into(),
            object_path: object_path.into(),
            interface: interface.into(),
        };
        zbus::names::WellKnownName::try_from(endpoint.bus_name.as_str())
            .map_err(|e| e.to_string())?;
        zbus::zvariant::ObjectPath::try_from(endpoint.object_path.as_str())
            .map_err(|e| e.to_string())?;
        zbus::names::InterfaceName::try_from(endpoint.interface.as_str())
            .map_err(|e| e.to_string())?;
        Ok(endpoint)
    }
    pub fn bus_name(&self) -> &str {
        &self.bus_name
    }
    pub fn object_path(&self) -> &str {
        &self.object_path
    }
    pub fn interface(&self) -> &str {
        &self.interface
    }
}

/// A connection to the existing session bus. Calls are bounded; reconnect explicitly after failure.
pub struct SettingsClient {
    connection: Connection,
    endpoint: SettingsEndpoint,
}
impl SettingsClient {
    pub fn connect(endpoint: SettingsEndpoint) -> Result<Self> {
        let address = std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .map_err(|_| "Desktop settings need an existing session bus")?;
        let connection = Builder::address(address.as_str())
            .map_err(|e| e.to_string())?
            // DisplayControl may spend twelve seconds awaiting its KMS owner.
            .method_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            connection,
            endpoint,
        })
    }
    #[cfg(test)]
    pub(super) fn from_connection(connection: Connection, endpoint: SettingsEndpoint) -> Self {
        Self {
            connection,
            endpoint,
        }
    }
    fn proxy(&self) -> Result<Proxy<'_>> {
        Proxy::new(
            &self.connection,
            self.endpoint.bus_name(),
            self.endpoint.object_path(),
            self.endpoint.interface(),
        )
        .map_err(|e| e.to_string())
    }
    pub fn snapshot(&self) -> Result<ShellSnapshot> {
        let text: String = self
            .proxy()?
            .call("Snapshot", &())
            .map_err(|e| e.to_string())?;
        protocol::decode(&text)
    }
    pub fn reload(&self) -> Result<()> {
        self.proxy()?
            .call("ReloadSettings", &())
            .map_err(|e| e.to_string())
    }
    pub fn apply_personalization(&self, settings: &PersonalizationSettings) -> Result<()> {
        let text = protocol::encode(settings)?;
        self.proxy()?
            .call("ApplyPersonalization", &(text,))
            .map_err(|e| e.to_string())
    }
    pub fn delete_background(&self, settings: &PersonalizationSettings) -> Result<()> {
        let text = protocol::encode(settings)?;
        self.proxy()?
            .call("DeleteBackground", &(text,))
            .map_err(|e| e.to_string())
    }
    pub fn preview_display(&self, settings: &DisplayConfiguration) -> Result<u64> {
        let text = protocol::encode(settings)?;
        self.proxy()?
            .call("PreviewDisplay", &(text,))
            .map_err(|e| e.to_string())
    }
    pub fn confirm_display(&self, token: u64) -> Result<()> {
        self.proxy()?
            .call("ConfirmDisplay", &(token,))
            .map_err(|e| e.to_string())
    }
    pub fn revert_display(&self, token: u64) -> Result<()> {
        self.proxy()?
            .call("RevertDisplay", &(token,))
            .map_err(|e| e.to_string())
    }
}
