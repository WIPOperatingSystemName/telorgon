use super::{
    settings::{self, Values},
    *,
};
use std::collections::HashSet;

impl NetworkManagerProvider {
    pub(super) fn read_snapshot(&mut self) -> Result<NetworkSnapshot, NetworkError> {
        let root = self.proxy(ROOT, DESTINATION)?;
        let networking_enabled = root.get_property("NetworkingEnabled").map_err(classify)?;
        let wifi_enabled = root.get_property("WirelessEnabled").map_err(classify)?;
        let wifi_hardware_enabled = root
            .get_property("WirelessHardwareEnabled")
            .map_err(classify)?;
        let connectivity = match root.get_property::<u32>("Connectivity").map_err(classify)? {
            1 => NetworkConnectivity::Offline,
            2 => NetworkConnectivity::CaptivePortal,
            3 => NetworkConnectivity::Local,
            4 => NetworkConnectivity::Internet,
            _ => NetworkConnectivity::Unknown,
        };
        let device_paths: Vec<OwnedObjectPath> = root.call("GetDevices", &()).map_err(classify)?;
        let active_paths: Vec<OwnedObjectPath> =
            root.get_property("ActiveConnections").map_err(classify)?;
        let raw_permissions: HashMap<String, String> =
            root.call("GetPermissions", &()).map_err(classify)?;
        let permissions = [
            (NetworkPermissionKind::ControlConnections, "network-control"),
            (NetworkPermissionKind::ScanWifi, "wifi.scan"),
            (
                NetworkPermissionKind::EditOwnProfiles,
                "settings.modify.own",
            ),
            (
                NetworkPermissionKind::EditSystemProfiles,
                "settings.modify.system",
            ),
            (NetworkPermissionKind::EnableWifi, "enable-disable-wifi"),
            (
                NetworkPermissionKind::EnableNetworking,
                "enable-disable-network",
            ),
        ]
        .into_iter()
        .map(|(kind, key)| NetworkPermission {
            kind,
            authorization: match raw_permissions
                .get(&format!("org.freedesktop.NetworkManager.{key}"))
                .map(String::as_str)
            {
                Some("yes") => NetworkAuthorization::Granted,
                Some("auth") => NetworkAuthorization::AuthenticationRequired,
                Some("no") => NetworkAuthorization::Denied,
                _ => NetworkAuthorization::Unknown,
            },
        })
        .collect();
        drop(root);
        let profile_paths: Vec<OwnedObjectPath> = self
            .proxy(SETTINGS, "org.freedesktop.NetworkManager.Settings")?
            .call("ListConnections", &())
            .map_err(classify)?;
        retain(&mut self.interfaces, &device_paths);
        retain(&mut self.profiles, &profile_paths);
        retain(&mut self.connections, &active_paths);
        for path in &device_paths {
            self.interfaces.entry(path.to_string()).or_default();
        }
        for path in &profile_paths {
            self.profiles.entry(path.to_string()).or_default();
        }
        for path in &active_paths {
            self.connections.entry(path.to_string()).or_default();
        }
        let mut profiles = Vec::new();
        for path in profile_paths {
            let proxy = self.proxy(
                path.as_str(),
                "org.freedesktop.NetworkManager.Settings.Connection",
            )?;
            let values: settings::Settings = proxy.call("GetSettings", &()).map_err(classify)?;
            let flags: u32 = proxy.get_property("Flags").map_err(classify)?;
            // UNSAVED also covers edits in memory that have not been persisted.
            profiles.push(settings::profile(
                self.profiles[path.as_str()],
                &values,
                flags & 1 == 0,
            )?);
        }
        let mut interfaces = Vec::new();
        let mut live_aps = HashSet::new();
        for path in device_paths {
            let proxy = self.proxy(path.as_str(), "org.freedesktop.NetworkManager.Device")?;
            let id = self.interfaces[path.as_str()];
            let kind = device_kind(proxy.get_property("DeviceType").map_err(classify)?);
            let managed: bool = proxy.get_property("Managed").map_err(classify)?;
            let (raw_state, state_reason): (u32, u32) =
                proxy.get_property("StateReason").map_err(classify)?;
            let active: OwnedObjectPath =
                proxy.get_property("ActiveConnection").map_err(classify)?;
            let ip4: OwnedObjectPath = proxy.get_property("Ip4Config").map_err(classify)?;
            let ip6: OwnedObjectPath = proxy.get_property("Ip6Config").map_err(classify)?;
            let mut device = NetworkInterface {
                device: Default::default(),
                id,
                name: proxy.get_property("Interface").map_err(classify)?,
                kind,
                managed,
                state: device_state(raw_state),
                failure: (raw_state == 120).then(|| failure_reason(state_reason)),
                capabilities: NetworkCapabilities {
                    scan_wifi: kind == NetworkInterfaceKind::Wifi && managed,
                    connect_wifi: kind == NetworkInterfaceKind::Wifi && managed,
                    wpa3_personal: kind == NetworkInterfaceKind::Wifi && managed,
                    activate_profile: managed
                        && matches!(
                            kind,
                            NetworkInterfaceKind::Wifi | NetworkInterfaceKind::Ethernet
                        ),
                    disconnect: managed,
                    edit_profiles: true,
                    configure_ip: managed,
                    reapply: managed,
                    ..Default::default()
                },
                active_connection: self.connections.get(active.as_str()).copied(),
                ipv4: self.read_ip(ip4.as_str(), IpFamily::V4)?,
                ipv6: self.read_ip(ip6.as_str(), IpFamily::V6)?,
                access_points: Vec::new(),
                active_access_point: None,
            };
            device.device.display_name = proxy
                .get_property::<String>("Description")
                .ok()
                .filter(|value| !value.is_empty());
            drop(proxy);
            if kind == NetworkInterfaceKind::Ethernet {
                if let Ok(wired) =
                    self.proxy(path.as_str(), "org.freedesktop.NetworkManager.Device.Wired")
                {
                    device.device.speed_mbps = wired
                        .get_property::<u32>("Speed")
                        .ok()
                        .filter(|value| *value > 0)
                        .map(u64::from);
                }
            }
            if kind == NetworkInterfaceKind::Wifi {
                let wireless = self.proxy(
                    path.as_str(),
                    "org.freedesktop.NetworkManager.Device.Wireless",
                )?;
                device.device.speed_mbps = wireless
                    .get_property::<u32>("Bitrate")
                    .ok()
                    .filter(|value| *value > 0)
                    .map(|value| u64::from(value) / 1000);
                let paths: Vec<OwnedObjectPath> =
                    wireless.call("GetAllAccessPoints", &()).map_err(classify)?;
                let active: OwnedObjectPath = wireless
                    .get_property("ActiveAccessPoint")
                    .map_err(classify)?;
                drop(wireless);
                for ap in paths {
                    live_aps.insert(ap.to_string());
                    let id = *self.aps.entry(ap.to_string()).or_default();
                    let proxy =
                        self.proxy(ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint")?;
                    let ssid: Vec<u8> = proxy.get_property("Ssid").map_err(classify)?;
                    let flags = proxy.get_property("Flags").map_err(classify)?;
                    let wpa = proxy.get_property("WpaFlags").map_err(classify)?;
                    let rsn = proxy.get_property("RsnFlags").map_err(classify)?;
                    device.access_points.push(NetworkAccessPoint {
                        id,
                        ssid: if ssid.is_empty() {
                            None
                        } else {
                            Some(Ssid::new(ssid).map_err(|_| NetworkError::InvalidData)?)
                        },
                        bssid: proxy.get_property("HwAddress").map_err(classify)?,
                        strength: proxy
                            .get_property::<u8>("Strength")
                            .map_err(classify)?
                            .min(100),
                        frequency_mhz: proxy.get_property("Frequency").map_err(classify)?,
                        security: wifi_security(flags, wpa, rsn),
                    });
                }
                device.active_access_point = self.aps.get(active.as_str()).copied();
            }
            interfaces.push(device);
        }
        self.aps.retain(|k, _| live_aps.contains(k));
        let mut connections = Vec::new();
        for path in active_paths {
            let proxy = self.proxy(
                path.as_str(),
                "org.freedesktop.NetworkManager.Connection.Active",
            )?;
            let profile: OwnedObjectPath = proxy.get_property("Connection").map_err(classify)?;
            let devices: Vec<OwnedObjectPath> = proxy.get_property("Devices").map_err(classify)?;
            let state = match proxy.get_property::<u32>("State").map_err(classify)? {
                1 => NetworkConnectionState::Preparing,
                2 => NetworkConnectionState::Connected,
                3 => NetworkConnectionState::Disconnecting,
                4 => NetworkConnectionState::Disconnected,
                _ => NetworkConnectionState::Unknown,
            };
            connections.push(NetworkConnection {
                id: self.connections[path.as_str()],
                profile: self.profiles.get(profile.as_str()).copied(),
                interfaces: devices
                    .iter()
                    .filter_map(|p| self.interfaces.get(p.as_str()).copied())
                    .collect(),
                state,
            });
        }
        Ok(NetworkSnapshot {
            state: NetworkServiceState::Ready,
            capabilities: NetworkCapabilities {
                set_wifi_enabled: true,
                set_networking_enabled: true,
                edit_profiles: true,
                configure_ip: true,
                ..Default::default()
            },
            permissions,
            backend: Some("NetworkManager".into()),
            networking_enabled,
            wifi_enabled,
            wifi_hardware_enabled,
            connectivity,
            interfaces,
            profiles,
            connections,
            ..Default::default()
        })
    }
    fn read_ip(&self, path: &str, family: IpFamily) -> Result<NetworkIpState, NetworkError> {
        if path == "/" {
            return Ok(Default::default());
        }
        let interface = if family == IpFamily::V4 {
            "org.freedesktop.NetworkManager.IP4Config"
        } else {
            "org.freedesktop.NetworkManager.IP6Config"
        };
        let proxy = self.proxy(path, interface)?;
        let addresses: Vec<Values> = proxy.get_property("AddressData").map_err(classify)?;
        let routes: Vec<Values> = proxy.get_property("RouteData").map_err(classify)?;
        let dns = if family == IpFamily::V4 {
            let data: Vec<Values> = proxy.get_property("NameserverData").map_err(classify)?;
            data.into_iter()
                .map(|row| {
                    settings::text(&row, "address")
                        .ok_or(NetworkError::InvalidData)?
                        .parse()
                        .map_err(|_| NetworkError::InvalidData)
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let data: Vec<Vec<u8>> = proxy.get_property("Nameservers").map_err(classify)?;
            data.into_iter()
                .map(|bytes| {
                    <[u8; 16]>::try_from(bytes)
                        .map(|bytes| std::net::IpAddr::V6(bytes.into()))
                        .map_err(|_| NetworkError::InvalidData)
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(NetworkIpState {
            addresses: settings::decode_addresses(addresses)?,
            gateway: settings::parse_optional_ip(Some(
                proxy.get_property("Gateway").map_err(classify)?,
            ))?,
            dns,
            routes: settings::decode_routes(routes)?,
        })
    }
}
fn retain<T>(map: &mut HashMap<String, T>, paths: &[OwnedObjectPath]) {
    map.retain(|k, _| paths.iter().any(|p| p.as_str() == k));
}
pub(super) fn device_kind(raw: u32) -> NetworkInterfaceKind {
    match raw {
        1 => NetworkInterfaceKind::Ethernet,
        2 => NetworkInterfaceKind::Wifi,
        _ => NetworkInterfaceKind::Other,
    }
}
pub(super) fn device_state(raw: u32) -> NetworkConnectionState {
    match raw {
        10 | 20 => NetworkConnectionState::Unavailable,
        30 => NetworkConnectionState::Disconnected,
        40 => NetworkConnectionState::Preparing,
        50 | 60 => NetworkConnectionState::Authenticating,
        70 | 80 | 90 => NetworkConnectionState::ConfiguringIp,
        100 => NetworkConnectionState::Connected,
        110 => NetworkConnectionState::Disconnecting,
        120 => NetworkConnectionState::Failed,
        _ => NetworkConnectionState::Unknown,
    }
}
pub(super) fn wifi_security(flags: u32, wpa: u32, rsn: u32) -> WifiSecurity {
    let security = wpa | rsn;
    if security & 0x100 != 0 {
        WifiSecurity::WpaPersonal
    } else if security & 0x400 != 0 {
        WifiSecurity::Wpa3Personal
    } else if security & (0x200 | 0x2000) != 0 {
        WifiSecurity::Enterprise
    } else if security != 0 {
        WifiSecurity::Unknown
    } else if flags & 1 != 0 {
        WifiSecurity::Wep
    } else {
        WifiSecurity::Open
    }
}

pub(super) fn failure_reason(reason: u32) -> NetworkError {
    match reason {
        7 => NetworkError::CredentialsRequired,
        8..=11 => NetworkError::AuthenticationFailed,
        5..=6 | 15..=17 => NetworkError::IpConfigurationFailed,
        _ => NetworkError::BackendFailure,
    }
}
