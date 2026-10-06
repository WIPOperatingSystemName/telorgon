//! Read-only Linux interface inventory; configuration remains with the management provider.
mod routes;
use crate::services::network::*;
use std::{
    collections::{BTreeMap, HashMap},
    ffi::CStr,
    fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::Path,
};

#[derive(Default)]
pub(crate) struct Inventory {
    ids: BTreeMap<u32, NetworkInterfaceId>,
}
struct Link {
    name: String,
    index: u32,
    info: NetworkDeviceInfo,
    v4: NetworkIpState,
    v6: NetworkIpState,
    addresses_known: bool,
    parent: Option<u32>,
    master: Option<u32>,
}
impl Inventory {
    pub fn enrich(&mut self, snapshot: &mut NetworkSnapshot) {
        let Ok(links) = read_links() else {
            return;
        };
        self.merge(snapshot, links, routes::read().ok());
    }
    fn merge(
        &mut self,
        snapshot: &mut NetworkSnapshot,
        mut links: Vec<Link>,
        route_list: Option<Vec<(u32, NetworkRoute)>>,
    ) {
        let routes_known = route_list.is_some();
        self.ids
            .retain(|index, _| links.iter().any(|link| link.index == *index));
        for link in &links {
            self.ids.entry(link.index).or_default();
        }
        let id_by_index: HashMap<_, _> = links
            .iter()
            .map(|link| {
                let id = snapshot
                    .interfaces
                    .iter()
                    .find(|device| device.name == link.name)
                    .map(|device| device.id)
                    .unwrap_or(self.ids[&link.index]);
                (link.index, id)
            })
            .collect();
        if let Some(routes) = route_list {
            for (index, route) in routes {
                if let Some(link) = links.iter_mut().find(|link| link.index == index) {
                    let ip = if route.destination.address.is_ipv4() {
                        &mut link.v4
                    } else {
                        &mut link.v6
                    };
                    if route.destination.prefix == 0 && ip.gateway.is_none() {
                        ip.gateway = route.gateway;
                    }
                    if !ip.routes.contains(&route) {
                        ip.routes.push(route);
                    }
                }
            }
        }
        for mut link in links {
            link.info.parent = link
                .parent
                .and_then(|index| id_by_index.get(&index).copied());
            link.info.master = link
                .master
                .and_then(|index| id_by_index.get(&index).copied());
            if let Some(device) = snapshot
                .interfaces
                .iter_mut()
                .find(|device| device.name == link.name)
            {
                if link.info.display_name.is_none() {
                    link.info.display_name = device.device.display_name.clone();
                }
                if link.info.speed_mbps.is_none() {
                    link.info.speed_mbps = device.device.speed_mbps;
                }
                device.device = link.info;
                // Kernel state is useful for unmanaged devices too; do not erase backend DNS.
                for (observed, kernel) in [(&mut device.ipv4, link.v4), (&mut device.ipv6, link.v6)]
                {
                    if link.addresses_known {
                        observed.addresses = kernel.addresses;
                    }
                    if routes_known {
                        observed.routes = kernel.routes;
                        observed.gateway = kernel.gateway;
                    }
                }
            } else {
                let kind = match link.info.device_type {
                    NetworkDeviceType::Wifi => NetworkInterfaceKind::Wifi,
                    NetworkDeviceType::Ethernet => NetworkInterfaceKind::Ethernet,
                    _ => NetworkInterfaceKind::Other,
                };
                snapshot.interfaces.push(NetworkInterface {
                    id: id_by_index[&link.index],
                    name: link.name,
                    kind,
                    device: link.info,
                    managed: false,
                    state: NetworkConnectionState::Unknown,
                    failure: None,
                    capabilities: Default::default(),
                    active_connection: None,
                    ipv4: link.v4,
                    ipv6: link.v6,
                    access_points: Vec::new(),
                    active_access_point: None,
                });
            }
        }
        snapshot.interfaces.sort_by(|a, b| a.name.cmp(&b.name));
    }
}
fn value(path: &Path, name: &str) -> Option<String> {
    fs::read_to_string(path.join(name))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}
fn number<T: std::str::FromStr>(path: &Path, name: &str) -> Option<T> {
    value(path, name)?.parse().ok()
}
fn read_links() -> std::io::Result<Vec<Link>> {
    let mut links = Vec::new();
    for entry in fs::read_dir("/sys/class/net")? {
        let entry = entry?;
        let path = entry.path();
        let Some(index) = number::<u32>(&path, "ifindex") else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let virtual_device = fs::canonicalize(&path)
            .ok()
            .is_some_and(|p| p.starts_with("/sys/devices/virtual/net"));
        let device_type = if number::<u32>(&path, "type") == Some(772) {
            NetworkDeviceType::Loopback
        } else if path.join("wireless").exists() || path.join("phy80211").exists() {
            NetworkDeviceType::Wifi
        } else if path.join("bridge").exists() {
            NetworkDeviceType::Bridge
        } else if path.join("bonding").exists() {
            NetworkDeviceType::Bond
        } else if Path::new("/proc/net/vlan").join(&name).exists() {
            NetworkDeviceType::Vlan
        } else if path.join("tun_flags").exists()
            || matches!(
                number::<u32>(&path, "type"),
                Some(768 | 769 | 776 | 778 | 823)
            )
        {
            NetworkDeviceType::Tunnel
        } else if virtual_device {
            NetworkDeviceType::Virtual
        } else {
            NetworkDeviceType::Ethernet
        };
        let stat = |name| number::<u64>(&path, &format!("statistics/{name}"));
        let traffic = (|| {
            Some(NetworkTraffic {
                received_bytes: stat("rx_bytes")?,
                transmitted_bytes: stat("tx_bytes")?,
                received_packets: stat("rx_packets")?,
                transmitted_packets: stat("tx_packets")?,
                receive_errors: stat("rx_errors")?,
                transmit_errors: stat("tx_errors")?,
                receive_drops: stat("rx_dropped")?,
                transmit_drops: stat("tx_dropped")?,
            })
        })();
        let flags = value(&path, "flags")
            .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok());
        let lowers: Vec<_> = fs::read_dir(&path)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("lower_"))
            .filter_map(|entry| number::<u32>(&entry.path(), "ifindex"))
            .collect();
        let parent = if lowers.len() == 1
            && !matches!(
                device_type,
                NetworkDeviceType::Bridge | NetworkDeviceType::Bond
            ) {
            lowers.first().copied()
        } else {
            None
        };
        let master = number::<u32>(&path, "master/ifindex");
        let driver = fs::read_link(path.join("device/driver"))
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()));
        links.push(Link {
            name,
            index,
            info: NetworkDeviceInfo {
                device_type,
                index: Some(index),
                mac_address: value(&path, "address"),
                mtu: number(&path, "mtu"),
                speed_mbps: number::<u64>(&path, "speed").filter(|v| *v > 0),
                carrier: number::<u8>(&path, "carrier").map(|v| v != 0),
                administrative_up: flags.map(|v| v & 1 != 0),
                operational_state: value(&path, "operstate"),
                virtual_device,
                driver,
                traffic,
                ..Default::default()
            },
            v4: Default::default(),
            v6: Default::default(),
            addresses_known: false,
            parent,
            master,
        });
    }
    let mut list = std::ptr::null_mut();
    // getifaddrs owns a linked list with valid sockaddr/name storage until freeifaddrs.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return Ok(links);
    }
    struct Addresses(*mut libc::ifaddrs);
    impl Drop for Addresses {
        fn drop(&mut self) {
            unsafe { libc::freeifaddrs(self.0) };
        }
    }
    let _owner = Addresses(list);
    for link in &mut links {
        link.addresses_known = true;
    }
    let mut current = list;
    while let Some(row) = unsafe { current.as_ref() } {
        current = row.ifa_next;
        if row.ifa_name.is_null() || row.ifa_addr.is_null() || row.ifa_netmask.is_null() {
            continue;
        }
        let name = unsafe { CStr::from_ptr(row.ifa_name) }.to_string_lossy();
        let Some(link) = links.iter_mut().find(|link| link.name == name) else {
            continue;
        };
        // The address family determines the native sockaddr layout and length.
        let pair = unsafe {
            match (*row.ifa_addr).sa_family as i32 {
                libc::AF_INET => {
                    let addr = &*row.ifa_addr.cast::<libc::sockaddr_in>();
                    let mask = &*row.ifa_netmask.cast::<libc::sockaddr_in>();
                    Some((
                        IpAddr::V4(Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes())),
                        u32::from_be(mask.sin_addr.s_addr).count_ones() as u8,
                    ))
                }
                libc::AF_INET6 => {
                    let addr = &*row.ifa_addr.cast::<libc::sockaddr_in6>();
                    let mask = &*row.ifa_netmask.cast::<libc::sockaddr_in6>();
                    Some((
                        IpAddr::V6(Ipv6Addr::from(addr.sin6_addr.s6_addr)),
                        mask.sin6_addr
                            .s6_addr
                            .iter()
                            .map(|b| b.count_ones())
                            .sum::<u32>() as u8,
                    ))
                }
                _ => None,
            }
        };
        if let Some((address, prefix)) = pair {
            if let Ok(address) = IpAddress::new(address, prefix) {
                let ip = if address.address.is_ipv4() {
                    &mut link.v4
                } else {
                    &mut link.v6
                };
                if !ip.addresses.contains(&address) {
                    ip.addresses.push(address);
                }
            }
        }
    }
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn link(index: u32, name: &str) -> Link {
        Link {
            index,
            name: name.into(),
            info: NetworkDeviceInfo {
                device_type: NetworkDeviceType::Loopback,
                mtu: Some(65536),
                ..Default::default()
            },
            v4: Default::default(),
            v6: Default::default(),
            addresses_known: true,
            parent: None,
            master: None,
        }
    }
    #[test]
    fn network_inventory_keeps_unmanaged_devices_and_invalidates_removed_identities() {
        let mut inventory = Inventory::default();
        let mut snapshot = NetworkSnapshot {
            state: NetworkServiceState::Unavailable,
            ..Default::default()
        };
        inventory.merge(&mut snapshot, vec![link(1, "lo")], Some(vec![]));
        let original = snapshot.interfaces[0].id;
        assert!(!snapshot.interfaces[0].managed);
        assert!(!snapshot.interfaces[0].capabilities.disconnect);
        snapshot.interfaces.clear();
        inventory.merge(&mut snapshot, vec![link(1, "renamed")], None);
        assert_eq!(snapshot.interfaces[0].id, original);
        snapshot.interfaces.clear();
        inventory.merge(&mut snapshot, vec![], None);
        inventory.merge(&mut snapshot, vec![link(1, "lo")], None);
        assert_ne!(snapshot.interfaces[0].id, original);
    }
    #[test]
    fn network_inventory_retains_backend_identity_and_dns_when_routes_are_unavailable() {
        let mut inventory = Inventory::default();
        let mut snapshot = NetworkSnapshot::default();
        inventory.merge(&mut snapshot, vec![link(2, "eth0")], None);
        let id = snapshot.interfaces[0].id;
        snapshot.interfaces[0].managed = true;
        snapshot.interfaces[0].ipv4.dns = vec!["192.0.2.53".parse().unwrap()];
        snapshot.interfaces[0].ipv4.routes = vec![NetworkRoute {
            destination: IpAddress::new("0.0.0.0".parse().unwrap(), 0).unwrap(),
            gateway: None,
            metric: Some(5),
        }];
        inventory.merge(&mut snapshot, vec![link(2, "eth0")], None);
        assert_eq!(snapshot.interfaces[0].id, id);
        assert!(snapshot.interfaces[0].managed);
        assert_eq!(snapshot.interfaces[0].ipv4.dns.len(), 1);
        assert_eq!(snapshot.interfaces[0].ipv4.routes.len(), 1);
    }
}
