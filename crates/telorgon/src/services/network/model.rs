use std::{
    fmt,
    net::IpAddr,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
macro_rules! identity {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);
        impl $name {
            /// Allocates an opaque runtime identity. Providers must not reuse it for replacements.
            pub fn new() -> Self {
                Self(
                    NEXT_ID
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
                        .expect("network identity exhausted"),
                )
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}
identity!(NetworkInterfaceId);
identity!(NetworkProfileId);
identity!(NetworkAccessPointId);
identity!(NetworkConnectionId);
identity!(NetworkRequestId);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ssid(Vec<u8>);
impl Ssid {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, NetworkError> {
        let bytes = bytes.into();
        if bytes.is_empty() || bytes.len() > 32 {
            return Err(NetworkError::InvalidConfig("SSID must contain 1–32 bytes"));
        }
        Ok(Self(bytes))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn display_name(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

/// Deliberately redacted in diagnostics and excluded from observable state.
pub struct NetworkSecret(pub(crate) String);
impl NetworkSecret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// Explicit access for trusted providers sending credentials to their backend.
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for NetworkSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NetworkSecret([redacted])")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetworkServiceState {
    #[default]
    Unstarted,
    Ready,
    Unavailable,
    Restricted,
    Stopped,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkInterfaceKind {
    Ethernet,
    Wifi,
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetworkDeviceType {
    #[default]
    Unknown,
    Ethernet,
    Wifi,
    Loopback,
    Bridge,
    Bond,
    Vlan,
    Tunnel,
    Virtual,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkTraffic {
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
    pub received_packets: u64,
    pub transmitted_packets: u64,
    pub receive_errors: u64,
    pub transmit_errors: u64,
    pub receive_drops: u64,
    pub transmit_drops: u64,
}
/// Optional observed device information. Missing values are unknown, never zero/off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkDeviceInfo {
    pub device_type: NetworkDeviceType,
    pub index: Option<u32>,
    pub display_name: Option<String>,
    pub mac_address: Option<String>,
    pub mtu: Option<u32>,
    pub speed_mbps: Option<u64>,
    pub carrier: Option<bool>,
    pub administrative_up: Option<bool>,
    pub operational_state: Option<String>,
    pub virtual_device: bool,
    pub parent: Option<NetworkInterfaceId>,
    pub master: Option<NetworkInterfaceId>,
    pub driver: Option<String>,
    pub traffic: Option<NetworkTraffic>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkConnectionState {
    Unavailable,
    Disconnected,
    Preparing,
    Authenticating,
    ConfiguringIp,
    Connected,
    Disconnecting,
    Failed,
    Unknown,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetworkConnectivity {
    #[default]
    Unknown,
    Offline,
    Local,
    CaptivePortal,
    Internet,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiSecurity {
    Open,
    WpaPersonal,
    Wpa3Personal,
    Enterprise,
    Wep,
    Unknown,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetworkPersistence {
    #[default]
    Session,
    Saved,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisconnectPolicy {
    AllowAutoconnect,
    BlockAutoconnect,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpFamily {
    V4,
    V6,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpAddress {
    pub address: IpAddr,
    pub prefix: u8,
}
impl IpAddress {
    pub fn new(address: IpAddr, prefix: u8) -> Result<Self, NetworkError> {
        let value = Self { address, prefix };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn validate(&self) -> Result<(), NetworkError> {
        if self.prefix > if self.address.is_ipv4() { 32 } else { 128 } {
            Err(NetworkError::InvalidConfig("invalid IP prefix"))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkRoute {
    pub destination: IpAddress,
    pub gateway: Option<IpAddr>,
    pub metric: Option<u32>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum IpMethod {
    #[default]
    Automatic,
    Static(Vec<IpAddress>),
    Disabled,
    Unmanaged,
    Unknown,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IpSettings {
    pub method: IpMethod,
    pub gateway: Option<IpAddr>,
    pub dns: Vec<IpAddr>,
    pub routes: Vec<NetworkRoute>,
    pub ignore_automatic_dns: bool,
    pub ignore_automatic_routes: bool,
}
impl IpSettings {
    pub(crate) fn validate(&self, family: IpFamily) -> Result<(), NetworkError> {
        if self.method == IpMethod::Unknown
            || (self.method == IpMethod::Unmanaged && family == IpFamily::V4)
        {
            return Err(NetworkError::Unsupported);
        }
        let correct = |ip: IpAddr| ip.is_ipv4() == (family == IpFamily::V4);
        if let IpMethod::Static(addresses) = &self.method {
            if addresses.is_empty() {
                return Err(NetworkError::InvalidConfig(
                    "static configuration requires an address",
                ));
            }
            for a in addresses {
                a.validate()?;
                if !correct(a.address) {
                    return Err(NetworkError::InvalidConfig("address family mismatch"));
                }
            }
        }
        if self.gateway.is_some_and(|v| !correct(v)) || self.dns.iter().any(|v| !correct(*v)) {
            return Err(NetworkError::InvalidConfig("address family mismatch"));
        }
        for route in &self.routes {
            route.destination.validate()?;
            if !correct(route.destination.address) || route.gateway.is_some_and(|v| !correct(v)) {
                return Err(NetworkError::InvalidConfig("route family mismatch"));
            }
        }
        if matches!(self.method, IpMethod::Disabled)
            && (self.gateway.is_some() || !self.dns.is_empty() || !self.routes.is_empty())
        {
            return Err(NetworkError::InvalidConfig(
                "disabled IP family cannot have routes or DNS",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkIpConfig {
    pub ipv4: IpSettings,
    pub ipv6: IpSettings,
}
impl NetworkIpConfig {
    pub(crate) fn validate(&self) -> Result<(), NetworkError> {
        self.ipv4.validate(IpFamily::V4)?;
        self.ipv6.validate(IpFamily::V6)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkIpState {
    pub addresses: Vec<IpAddress>,
    pub gateway: Option<IpAddr>,
    pub dns: Vec<IpAddr>,
    pub routes: Vec<NetworkRoute>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkCapabilities {
    pub set_wifi_enabled: bool,
    pub set_networking_enabled: bool,
    pub scan_wifi: bool,
    pub connect_wifi: bool,
    pub wpa3_personal: bool,
    pub activate_profile: bool,
    pub disconnect: bool,
    pub edit_profiles: bool,
    pub configure_ip: bool,
    pub reapply: bool,
    pub renew_dhcp: bool,
    pub flush_dns_cache: bool,
    pub clear_addresses: bool,
    pub clear_routes: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkInterface {
    pub device: NetworkDeviceInfo,
    pub id: NetworkInterfaceId,
    pub name: String,
    pub kind: NetworkInterfaceKind,
    pub managed: bool,
    pub state: NetworkConnectionState,
    pub failure: Option<NetworkError>,
    pub capabilities: NetworkCapabilities,
    pub active_connection: Option<NetworkConnectionId>,
    pub ipv4: NetworkIpState,
    pub ipv6: NetworkIpState,
    pub access_points: Vec<NetworkAccessPoint>,
    pub active_access_point: Option<NetworkAccessPointId>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkAccessPoint {
    pub id: NetworkAccessPointId,
    pub ssid: Option<Ssid>,
    pub bssid: String,
    pub strength: u8,
    pub frequency_mhz: u32,
    pub security: WifiSecurity,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkProfile {
    pub interface_name: Option<String>,
    pub id: NetworkProfileId,
    pub name: String,
    pub kind: NetworkInterfaceKind,
    pub ssid: Option<Ssid>,
    pub autoconnect: bool,
    pub persistence: NetworkPersistence,
    pub ip: NetworkIpConfig,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkConnection {
    pub id: NetworkConnectionId,
    pub profile: Option<NetworkProfileId>,
    pub interfaces: Vec<NetworkInterfaceId>,
    pub state: NetworkConnectionState,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkSnapshot {
    pub revision: u64,
    pub capabilities: NetworkCapabilities,
    pub permissions: Vec<NetworkPermission>,
    pub state: NetworkServiceState,
    pub backend: Option<String>,
    pub networking_enabled: bool,
    pub wifi_enabled: bool,
    pub wifi_hardware_enabled: bool,
    pub connectivity: NetworkConnectivity,
    pub interfaces: Vec<NetworkInterface>,
    pub profiles: Vec<NetworkProfile>,
    pub connections: Vec<NetworkConnection>,
    pub last_error: Option<NetworkError>,
}

#[derive(Debug)]
pub struct WifiConnectOptions {
    pub ssid: Ssid,
    pub security: WifiSecurity,
    pub credentials: Option<NetworkSecret>,
    pub access_point: Option<NetworkAccessPointId>,
    pub hidden: bool,
    pub persistence: NetworkPersistence,
    pub autoconnect: bool,
    pub ip: NetworkIpConfig,
}
impl WifiConnectOptions {
    pub fn new(ssid: Ssid, security: WifiSecurity) -> Self {
        Self {
            ssid,
            security,
            credentials: None,
            access_point: None,
            hidden: false,
            persistence: NetworkPersistence::Session,
            autoconnect: false,
            ip: Default::default(),
        }
    }
    pub(crate) fn validate(&self) -> Result<(), NetworkError> {
        self.ip.validate()?;
        if let Some(secret) = &self.credentials {
            let bytes = secret.0.as_bytes();
            let valid = match self.security {
                WifiSecurity::WpaPersonal => {
                    (8..=63).contains(&bytes.len())
                        || (bytes.len() == 64 && bytes.iter().all(u8::is_ascii_hexdigit))
                }
                WifiSecurity::Wpa3Personal => (1..=63).contains(&bytes.len()),
                _ => false,
            };
            if !valid || bytes.contains(&0) {
                return Err(NetworkError::InvalidConfig("invalid Wi-Fi credentials"));
            }
        }
        Ok(())
    }
}
#[derive(Debug)]
pub enum NetworkCommand {
    ScanWifi(NetworkInterfaceId),
    ConnectWifi {
        interface: NetworkInterfaceId,
        options: WifiConnectOptions,
    },
    ActivateProfile {
        interface: NetworkInterfaceId,
        profile: NetworkProfileId,
    },
    Disconnect {
        interface: NetworkInterfaceId,
        policy: DisconnectPolicy,
    },
    UpdateProfile {
        profile: NetworkProfileId,
        ip: NetworkIpConfig,
        persistence: NetworkPersistence,
    },
    SetAutoconnect {
        profile: NetworkProfileId,
        enabled: bool,
        persistence: NetworkPersistence,
    },
    ForgetProfile(NetworkProfileId),
    Reapply(NetworkInterfaceId),
    SetWifiEnabled(bool),
    SetNetworkingEnabled(bool),
    RenewDhcp {
        interface: NetworkInterfaceId,
        family: IpFamily,
    },
    FlushDnsCache,
    ClearAddresses {
        interface: NetworkInterfaceId,
        family: IpFamily,
    },
    ClearRoutes {
        interface: NetworkInterfaceId,
        family: IpFamily,
    },
}
impl NetworkCommand {
    pub(crate) fn needs_credentials(&self) -> bool {
        matches!(self, Self::ConnectWifi { options, .. } if options.credentials.is_none() && matches!(options.security, WifiSecurity::WpaPersonal | WifiSecurity::Wpa3Personal))
    }
    pub(crate) fn validate(&self) -> Result<(), NetworkError> {
        match self {
            Self::ConnectWifi { options, .. } => options.validate(),
            Self::UpdateProfile { ip, .. } => ip.validate(),
            _ => Ok(()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkCredentialChallenge {
    pub request: NetworkRequestId,
    pub interface: NetworkInterfaceId,
    pub ssid: Ssid,
    pub security: WifiSecurity,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkResult {
    ScanComplete(NetworkInterfaceId),
    Connected(NetworkConnectionId),
    Disconnected(NetworkInterfaceId),
    ProfileUpdated(NetworkProfileId),
    ProfileRemoved(NetworkProfileId),
    Reapplied(NetworkInterfaceId),
    WifiEnabled(bool),
    NetworkingEnabled(bool),
    DhcpRenewed(NetworkInterfaceId),
    DnsCacheFlushed,
    AddressesCleared(NetworkInterfaceId),
    RoutesCleared(NetworkInterfaceId),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkOutcome {
    Applied(NetworkResult),
    Unconfirmed(NetworkError),
    Failed(NetworkError),
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkError {
    Unsupported,
    Unavailable,
    PermissionDenied,
    AuthorizationRequired,
    CredentialsRequired,
    AuthenticationFailed,
    IpConfigurationFailed,
    Stale,
    Busy,
    Stopped,
    TimedOut,
    Transport,
    InvalidData,
    InvalidConfig(&'static str),
    BackendFailure,
}
impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "network: {self:?}")
    }
}
impl std::error::Error for NetworkError {}
#[derive(Clone, Debug)]
pub struct NetworkConfig {
    pub poll_interval: Duration,
    pub request_timeout: Duration,
    pub queue_capacity: usize,
}
impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(2),
            request_timeout: Duration::from_secs(45),
            queue_capacity: 32,
        }
    }
}
impl NetworkConfig {
    pub(crate) fn validate(&self) -> Result<(), NetworkError> {
        if self.poll_interval.is_zero()
            || self.request_timeout.is_zero()
            || self.poll_interval > Duration::from_secs(60)
            || self.request_timeout > Duration::from_secs(600)
            || !(1..=256).contains(&self.queue_capacity)
        {
            Err(NetworkError::InvalidConfig("invalid network worker limits"))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkPermissionKind {
    ControlConnections,
    ScanWifi,
    EditOwnProfiles,
    EditSystemProfiles,
    EnableWifi,
    EnableNetworking,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkAuthorization {
    Granted,
    AuthenticationRequired,
    Denied,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkPermission {
    pub kind: NetworkPermissionKind,
    pub authorization: NetworkAuthorization,
}
