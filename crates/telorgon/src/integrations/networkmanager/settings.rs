use super::*;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use zbus::zvariant::{OwnedValue, Str, Value};

pub(super) type Values = HashMap<String, OwnedValue>;
pub(super) type Settings = HashMap<String, Values>;

pub(super) fn string(value: impl Into<String>) -> OwnedValue {
    OwnedValue::from(Str::from(value.into()))
}
pub(super) fn array<T: Into<Value<'static>>>(value: T) -> Result<OwnedValue, NetworkError> {
    OwnedValue::try_from(value.into()).map_err(|_| NetworkError::InvalidData)
}
pub(super) fn get<'a>(values: &'a Values, name: &str) -> Option<&'a OwnedValue> {
    values.get(name)
}
pub(super) fn text(values: &Values, name: &str) -> Option<String> {
    get(values, name)
        .and_then(|v| <&str>::try_from(v).ok())
        .map(str::to_owned)
}
pub(super) fn uint(values: &Values, name: &str) -> Option<u32> {
    get(values, name).and_then(|v| u32::try_from(v).ok())
}
pub(super) fn boolean(values: &Values, name: &str) -> Option<bool> {
    get(values, name).and_then(|v| bool::try_from(v).ok())
}
fn list<T>(values: &Values, name: &str) -> Result<Vec<T>, NetworkError>
where
    Vec<T>: TryFrom<OwnedValue, Error = zbus::zvariant::Error>,
{
    match get(values, name) {
        None => Ok(Vec::new()),
        Some(v) => Vec::<T>::try_from(v.try_clone().map_err(|_| NetworkError::InvalidData)?)
            .map_err(|_| NetworkError::InvalidData),
    }
}

pub(super) fn encode_ip(
    settings: &IpSettings,
    family: IpFamily,
    values: &mut Values,
) -> Result<(), NetworkError> {
    settings.validate(family)?;
    // Clear the alternate legacy representation so it cannot override the new addresses/routes.
    for key in [
        "addresses",
        "address-data",
        "routes",
        "route-data",
        "dns",
        "dns-data",
        "gateway",
    ] {
        values.remove(key);
    }
    let method = match &settings.method {
        IpMethod::Automatic => "auto",
        IpMethod::Static(_) => "manual",
        IpMethod::Disabled => "disabled",
        IpMethod::Unmanaged => "ignore",
        IpMethod::Unknown => return Err(NetworkError::Unsupported),
    };
    values.insert("method".into(), string(method));
    let addresses = match &settings.method {
        IpMethod::Static(addresses) => addresses
            .iter()
            .map(|a| {
                HashMap::from([
                    ("address".into(), string(a.address.to_string())),
                    ("prefix".into(), u32::from(a.prefix).into()),
                ])
            })
            .collect::<Vec<Values>>(),
        _ => Vec::new(),
    };
    values.insert("address-data".into(), array(addresses)?);
    if let Some(gateway) = settings.gateway {
        values.insert("gateway".into(), string(gateway.to_string()));
    }
    let routes = settings
        .routes
        .iter()
        .map(|route| {
            let mut row = Values::from([
                ("dest".into(), string(route.destination.address.to_string())),
                ("prefix".into(), u32::from(route.destination.prefix).into()),
            ]);
            if let Some(gateway) = route.gateway {
                row.insert("next-hop".into(), string(gateway.to_string()));
            }
            if let Some(metric) = route.metric {
                row.insert("metric".into(), metric.into());
            }
            row
        })
        .collect::<Vec<_>>();
    values.insert("route-data".into(), array(routes)?);
    if family == IpFamily::V4 {
        let dns = settings
            .dns
            .iter()
            .map(|v| match v {
                IpAddr::V4(v) => u32::from_ne_bytes(v.octets()),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        values.insert("dns".into(), array(dns)?);
    } else {
        let dns = settings
            .dns
            .iter()
            .map(|v| match v {
                IpAddr::V6(v) => v.octets().to_vec(),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        values.insert("dns".into(), array(dns)?);
    }
    values.insert(
        "ignore-auto-dns".into(),
        settings.ignore_automatic_dns.into(),
    );
    values.insert(
        "ignore-auto-routes".into(),
        settings.ignore_automatic_routes.into(),
    );
    Ok(())
}
pub(super) fn encode_config(
    config: &NetworkIpConfig,
    settings: &mut Settings,
) -> Result<(), NetworkError> {
    encode_ip(
        &config.ipv4,
        IpFamily::V4,
        settings.entry("ipv4".into()).or_default(),
    )?;
    encode_ip(
        &config.ipv6,
        IpFamily::V6,
        settings.entry("ipv6".into()).or_default(),
    )
}
pub(super) fn decode_ip(
    values: Option<&Values>,
    family: IpFamily,
) -> Result<IpSettings, NetworkError> {
    let Some(values) = values else {
        return Ok(Default::default());
    };
    let addresses = decode_addresses(list::<Values>(values, "address-data")?)?;
    let method = match text(values, "method").as_deref() {
        Some("manual") => IpMethod::Static(addresses),
        Some("disabled") => IpMethod::Disabled,
        Some("ignore") => IpMethod::Unmanaged,
        Some("auto") | None => IpMethod::Automatic,
        _ => IpMethod::Unknown,
    };
    let dns = if family == IpFamily::V4 {
        list::<u32>(values, "dns")?
            .into_iter()
            .map(|v| IpAddr::V4(Ipv4Addr::from(v.to_ne_bytes())))
            .collect()
    } else {
        list::<Vec<u8>>(values, "dns")?
            .into_iter()
            .map(|v| {
                <[u8; 16]>::try_from(v)
                    .map(|v| IpAddr::V6(Ipv6Addr::from(v)))
                    .map_err(|_| NetworkError::InvalidData)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(IpSettings {
        method,
        gateway: parse_optional_ip(text(values, "gateway"))?,
        dns,
        routes: decode_routes(list::<Values>(values, "route-data")?)?,
        ignore_automatic_dns: boolean(values, "ignore-auto-dns").unwrap_or(false),
        ignore_automatic_routes: boolean(values, "ignore-auto-routes").unwrap_or(false),
    })
}
pub(super) fn parse_optional_ip(value: Option<String>) -> Result<Option<IpAddr>, NetworkError> {
    value
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().map_err(|_| NetworkError::InvalidData))
        .transpose()
}
pub(super) fn decode_addresses(rows: Vec<Values>) -> Result<Vec<IpAddress>, NetworkError> {
    rows.into_iter()
        .map(|row| {
            let address = text(&row, "address")
                .ok_or(NetworkError::InvalidData)?
                .parse()
                .map_err(|_| NetworkError::InvalidData)?;
            let prefix = u8::try_from(uint(&row, "prefix").ok_or(NetworkError::InvalidData)?)
                .map_err(|_| NetworkError::InvalidData)?;
            IpAddress::new(address, prefix).map_err(|_| NetworkError::InvalidData)
        })
        .collect()
}
pub(super) fn decode_routes(rows: Vec<Values>) -> Result<Vec<NetworkRoute>, NetworkError> {
    rows.into_iter()
        .map(|row| {
            let address = text(&row, "dest")
                .ok_or(NetworkError::InvalidData)?
                .parse()
                .map_err(|_| NetworkError::InvalidData)?;
            let prefix = u8::try_from(uint(&row, "prefix").ok_or(NetworkError::InvalidData)?)
                .map_err(|_| NetworkError::InvalidData)?;
            Ok(NetworkRoute {
                destination: IpAddress::new(address, prefix)
                    .map_err(|_| NetworkError::InvalidData)?,
                gateway: parse_optional_ip(text(&row, "next-hop"))?,
                metric: uint(&row, "metric"),
            })
        })
        .collect()
}
pub(super) fn profile(
    id: NetworkProfileId,
    settings: &Settings,
    saved: bool,
) -> Result<NetworkProfile, NetworkError> {
    let connection = settings
        .get("connection")
        .ok_or(NetworkError::InvalidData)?;
    let kind = match text(connection, "type").as_deref() {
        Some("802-3-ethernet") => NetworkInterfaceKind::Ethernet,
        Some("802-11-wireless") => NetworkInterfaceKind::Wifi,
        _ => NetworkInterfaceKind::Other,
    };
    let ssid = settings
        .get("802-11-wireless")
        .map(|v| list::<u8>(v, "ssid"))
        .transpose()?
        .filter(|v| !v.is_empty())
        .map(Ssid::new)
        .transpose()?;
    Ok(NetworkProfile {
        interface_name: text(connection, "interface-name").filter(|name| !name.is_empty()),
        id,
        name: text(connection, "id").unwrap_or_default(),
        kind,
        ssid,
        autoconnect: boolean(connection, "autoconnect").unwrap_or(true),
        persistence: if saved {
            NetworkPersistence::Saved
        } else {
            NetworkPersistence::Session
        },
        ip: NetworkIpConfig {
            ipv4: decode_ip(settings.get("ipv4"), IpFamily::V4)?,
            ipv6: decode_ip(settings.get("ipv6"), IpFamily::V6)?,
        },
    })
}
