use super::{
    settings::{self, Settings, Values},
    *,
};

pub(super) struct Pending {
    owner: String,
    expected: Expected,
}
enum Expected {
    Scan {
        interface: NetworkInterfaceId,
        previous: i64,
    },
    Connected {
        interface: NetworkInterfaceId,
        active: NetworkConnectionId,
    },
    Disconnected {
        interface: NetworkInterfaceId,
        active: Option<NetworkConnectionId>,
        block: bool,
    },
    ProfileIp {
        profile: NetworkProfileId,
        ip: NetworkIpConfig,
        persistence: NetworkPersistence,
    },
    Autoconnect {
        profile: NetworkProfileId,
        enabled: bool,
        persistence: NetworkPersistence,
    },
    Removed(NetworkProfileId),
    Reapplied(NetworkInterfaceId),
    Wifi(bool),
    Networking(bool),
}
impl NetworkManagerProvider {
    fn pending(&mut self, expected: Expected) -> Result<NetworkDispatch, NetworkError> {
        let token = self.next_token;
        self.next_token = token.checked_add(1).ok_or(NetworkError::Busy)?;
        let owner = self
            .session
            .as_ref()
            .ok_or(NetworkError::Unavailable)?
            .owner
            .clone();
        self.pending.insert(token, Pending { owner, expected });
        Ok(NetworkDispatch::Pending(token))
    }
    pub(super) fn execute_command(
        &mut self,
        command: NetworkCommand,
    ) -> Result<NetworkDispatch, NetworkError> {
        command.validate()?;
        match command {
            NetworkCommand::ScanWifi(interface) => {
                let path = Self::path(&self.interfaces, interface)?;
                let proxy = self.proxy(&path, "org.freedesktop.NetworkManager.Device.Wireless")?;
                let previous = proxy.get_property("LastScan").map_err(classify)?;
                proxy
                    .call::<_, _, ()>("RequestScan", &(Values::new(),))
                    .map_err(classify)?;
                drop(proxy);
                self.pending(Expected::Scan {
                    interface,
                    previous,
                })
            }
            NetworkCommand::ConnectWifi { interface, options } => {
                let path = Self::path(&self.interfaces, interface)?;
                let specific = options
                    .access_point
                    .map(|id| Self::path(&self.aps, id))
                    .transpose()?
                    .unwrap_or_else(|| "/".into());
                let settings = wifi_settings(&options)?;
                let args = Values::from([(
                    "persist".into(),
                    settings::string(if options.persistence == NetworkPersistence::Saved {
                        "disk"
                    } else {
                        "volatile"
                    }),
                )]);
                let (_, active, _): (OwnedObjectPath, OwnedObjectPath, Values) = self
                    .proxy(ROOT, DESTINATION)?
                    .call(
                        "AddAndActivateConnection2",
                        &(settings, object(&path)?, object(&specific)?, args),
                    )
                    .map_err(classify)?;
                let active = *self.connections.entry(active.to_string()).or_default();
                self.pending(Expected::Connected { interface, active })
            }
            NetworkCommand::ActivateProfile { interface, profile } => {
                let path = Self::path(&self.interfaces, interface)?;
                let profile = Self::path(&self.profiles, profile)?;
                let active: OwnedObjectPath = self
                    .proxy(ROOT, DESTINATION)?
                    .call(
                        "ActivateConnection",
                        &(object(&profile)?, object(&path)?, object("/")?),
                    )
                    .map_err(classify)?;
                let active = *self.connections.entry(active.to_string()).or_default();
                self.pending(Expected::Connected { interface, active })
            }
            NetworkCommand::Disconnect { interface, policy } => {
                let path = Self::path(&self.interfaces, interface)?;
                let device = self.proxy(&path, "org.freedesktop.NetworkManager.Device")?;
                let active: OwnedObjectPath =
                    device.get_property("ActiveConnection").map_err(classify)?;
                let active_id = self.connections.get(active.as_str()).copied();
                if policy == DisconnectPolicy::BlockAutoconnect {
                    device
                        .call::<_, _, ()>("Disconnect", &())
                        .map_err(classify)?;
                } else if active.as_str() != "/" {
                    self.proxy(ROOT, DESTINATION)?
                        .call::<_, _, ()>("DeactivateConnection", &(active,))
                        .map_err(classify)?;
                }
                drop(device);
                self.pending(Expected::Disconnected {
                    interface,
                    active: active_id,
                    block: policy == DisconnectPolicy::BlockAutoconnect,
                })
            }
            NetworkCommand::UpdateProfile {
                profile,
                ip,
                persistence,
            } => {
                let path = Self::path(&self.profiles, profile)?;
                let proxy =
                    self.proxy(&path, "org.freedesktop.NetworkManager.Settings.Connection")?;
                let mut settings: Settings = proxy.call("GetSettings", &()).map_err(classify)?;
                settings::encode_config(&ip, &mut settings)?;
                update(&proxy, settings, persistence)?;
                drop(proxy);
                self.pending(Expected::ProfileIp {
                    profile,
                    ip,
                    persistence,
                })
            }
            NetworkCommand::SetAutoconnect {
                profile,
                enabled,
                persistence,
            } => {
                let path = Self::path(&self.profiles, profile)?;
                let proxy =
                    self.proxy(&path, "org.freedesktop.NetworkManager.Settings.Connection")?;
                let mut settings: Settings = proxy.call("GetSettings", &()).map_err(classify)?;
                settings
                    .entry("connection".into())
                    .or_default()
                    .insert("autoconnect".into(), enabled.into());
                update(&proxy, settings, persistence)?;
                drop(proxy);
                self.pending(Expected::Autoconnect {
                    profile,
                    enabled,
                    persistence,
                })
            }
            NetworkCommand::ForgetProfile(profile) => {
                let path = Self::path(&self.profiles, profile)?;
                self.proxy(&path, "org.freedesktop.NetworkManager.Settings.Connection")?
                    .call::<_, _, ()>("Delete", &())
                    .map_err(classify)?;
                self.pending(Expected::Removed(profile))
            }
            NetworkCommand::Reapply(interface) => {
                let path = Self::path(&self.interfaces, interface)?;
                let proxy = self.proxy(&path, "org.freedesktop.NetworkManager.Device")?;
                let (_, version): (Settings, u64) = proxy
                    .call("GetAppliedConnection", &(0u32,))
                    .map_err(classify)?;
                proxy
                    .call::<_, _, ()>("Reapply", &(Settings::new(), version, 0u32))
                    .map_err(classify)?;
                drop(proxy);
                self.pending(Expected::Reapplied(interface))
            }
            NetworkCommand::SetWifiEnabled(enabled) => {
                self.proxy(ROOT, DESTINATION)?
                    .set_property("WirelessEnabled", enabled)
                    .map_err(|e| classify(e.into()))?;
                self.pending(Expected::Wifi(enabled))
            }
            NetworkCommand::SetNetworkingEnabled(enabled) => {
                self.proxy(ROOT, DESTINATION)?
                    .call::<_, _, ()>("Enable", &(enabled,))
                    .map_err(classify)?;
                self.pending(Expected::Networking(enabled))
            }
            // These operations have no equivalent guaranteed semantics in this adapter.
            NetworkCommand::RenewDhcp { .. }
            | NetworkCommand::FlushDnsCache
            | NetworkCommand::ClearAddresses { .. }
            | NetworkCommand::ClearRoutes { .. } => Err(NetworkError::Unsupported),
        }
    }
    pub(super) fn poll_command(
        &mut self,
        token: u64,
        snapshot: &NetworkSnapshot,
    ) -> Result<Option<NetworkResult>, NetworkError> {
        let pending = self.pending.get(&token).ok_or(NetworkError::Stale)?;
        if self
            .session
            .as_ref()
            .is_none_or(|s| s.owner != pending.owner)
        {
            return Err(NetworkError::Stale);
        }
        if snapshot.state != NetworkServiceState::Ready {
            return Err(NetworkError::Unavailable);
        }
        let device = |id| {
            snapshot
                .interfaces
                .iter()
                .find(|d| d.id == id)
                .ok_or(NetworkError::Stale)
        };
        let profile = |id| {
            snapshot
                .profiles
                .iter()
                .find(|p| p.id == id)
                .ok_or(NetworkError::Stale)
        };
        let result = match &pending.expected {
            Expected::Scan {
                interface,
                previous,
            } => {
                device(*interface)?;
                let path = Self::path(&self.interfaces, *interface)?;
                let scan: i64 = self
                    .proxy(&path, "org.freedesktop.NetworkManager.Device.Wireless")?
                    .get_property("LastScan")
                    .map_err(classify)?;
                (scan > *previous).then_some(NetworkResult::ScanComplete(*interface))
            }
            Expected::Connected { interface, active } => {
                let device = device(*interface)?;
                let connection = snapshot.connections.iter().find(|c| c.id == *active);
                if !self.connections.values().any(|id| id == active)
                    || connection.is_some_and(|c| c.state == NetworkConnectionState::Disconnected)
                    || (device.active_connection == Some(*active)
                        && device.state == NetworkConnectionState::Failed)
                {
                    return Err(device
                        .failure
                        .clone()
                        .unwrap_or(NetworkError::BackendFailure));
                }
                (connection.is_some_and(|c| c.state == NetworkConnectionState::Connected)
                    && device.active_connection == Some(*active)
                    && device.state == NetworkConnectionState::Connected)
                    .then_some(NetworkResult::Connected(*active))
            }
            Expected::Disconnected {
                interface,
                active,
                block,
            } => {
                let device = device(*interface)?;
                let inactive = if *block {
                    device.active_connection.is_none()
                        && matches!(
                            device.state,
                            NetworkConnectionState::Disconnected
                                | NetworkConnectionState::Unavailable
                        )
                } else {
                    active.is_none()
                        || !snapshot.connections.iter().any(|c| {
                            Some(c.id) == *active && c.state != NetworkConnectionState::Disconnected
                        })
                };
                inactive.then_some(NetworkResult::Disconnected(*interface))
            }
            Expected::ProfileIp {
                profile: id,
                ip,
                persistence,
            } => {
                let p = profile(*id)?;
                (&p.ip == ip && p.persistence == *persistence)
                    .then_some(NetworkResult::ProfileUpdated(*id))
            }
            Expected::Autoconnect {
                profile: id,
                enabled,
                persistence,
            } => {
                let p = profile(*id)?;
                (p.autoconnect == *enabled && p.persistence == *persistence)
                    .then_some(NetworkResult::ProfileUpdated(*id))
            }
            Expected::Removed(id) => (!snapshot.profiles.iter().any(|p| p.id == *id))
                .then_some(NetworkResult::ProfileRemoved(*id)),
            Expected::Reapplied(interface) => {
                let device = device(*interface)?;
                let Some(active) = device.active_connection else {
                    return Err(NetworkError::Unavailable);
                };
                let connection = snapshot
                    .connections
                    .iter()
                    .find(|c| c.id == active)
                    .ok_or(NetworkError::Stale)?;
                let p = profile(connection.profile.ok_or(NetworkError::Stale)?)?;
                let path = Self::path(&self.interfaces, *interface)?;
                let (settings, _): (Settings, u64) = self
                    .proxy(&path, "org.freedesktop.NetworkManager.Device")?
                    .call("GetAppliedConnection", &(0u32,))
                    .map_err(classify)?;
                let applied = NetworkIpConfig {
                    ipv4: settings::decode_ip(settings.get("ipv4"), IpFamily::V4)?,
                    ipv6: settings::decode_ip(settings.get("ipv6"), IpFamily::V6)?,
                };
                (applied == p.ip
                    && device.state == NetworkConnectionState::Connected
                    && static_observed(&p.ip.ipv4, &device.ipv4)
                    && static_observed(&p.ip.ipv6, &device.ipv6))
                .then_some(NetworkResult::Reapplied(*interface))
            }
            Expected::Wifi(enabled) => {
                (snapshot.wifi_enabled == *enabled).then_some(NetworkResult::WifiEnabled(*enabled))
            }
            Expected::Networking(enabled) => (snapshot.networking_enabled == *enabled)
                .then_some(NetworkResult::NetworkingEnabled(*enabled)),
        };
        Ok(result)
    }
}
fn static_observed(settings: &IpSettings, observed: &NetworkIpState) -> bool {
    match &settings.method {
        IpMethod::Static(addresses) => addresses.iter().all(|a| observed.addresses.contains(a)),
        IpMethod::Disabled => observed.addresses.is_empty(),
        IpMethod::Automatic | IpMethod::Unmanaged => true,
        IpMethod::Unknown => false,
    }
}
fn update(
    proxy: &Proxy<'_>,
    settings: Settings,
    persistence: NetworkPersistence,
) -> Result<(), NetworkError> {
    let flags = if persistence == NetworkPersistence::Saved {
        1u32
    } else {
        2u32
    };
    let _: Values = proxy
        .call("Update2", &(settings, flags, Values::new()))
        .map_err(classify)?;
    Ok(())
}
pub(super) fn wifi_settings(options: &WifiConnectOptions) -> Result<Settings, NetworkError> {
    options.validate()?;
    let mut settings = Settings::from([
        (
            "connection".into(),
            Values::from([
                ("id".into(), settings::string(options.ssid.display_name())),
                ("type".into(), settings::string("802-11-wireless")),
                ("autoconnect".into(), options.autoconnect.into()),
            ]),
        ),
        (
            "802-11-wireless".into(),
            Values::from([
                (
                    "ssid".into(),
                    settings::array(options.ssid.as_bytes().to_vec())?,
                ),
                ("mode".into(), settings::string("infrastructure")),
                ("hidden".into(), options.hidden.into()),
            ]),
        ),
    ]);
    let key = match options.security {
        WifiSecurity::Open => None,
        WifiSecurity::WpaPersonal => Some("wpa-psk"),
        WifiSecurity::Wpa3Personal => Some("sae"),
        _ => return Err(NetworkError::Unsupported),
    };
    if let Some(key) = key {
        let password = options
            .credentials
            .as_ref()
            .ok_or(NetworkError::CredentialsRequired)?
            .expose();
        settings.insert(
            "802-11-wireless-security".into(),
            Values::from([
                ("key-mgmt".into(), settings::string(key)),
                ("psk".into(), settings::string(password)),
            ]),
        );
    }
    settings::encode_config(&options.ip, &mut settings)?;
    Ok(settings)
}
