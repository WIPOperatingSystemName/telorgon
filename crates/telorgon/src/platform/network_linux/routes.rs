use crate::services::network::{IpAddress, NetworkRoute};
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::{Duration, Instant},
};
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid route inventory")
}
fn u32_ne(data: &[u8]) -> Option<u32> {
    Some(u32::from_ne_bytes(data.get(..4)?.try_into().ok()?))
}
fn address(family: u8, data: &[u8]) -> Option<IpAddr> {
    match family as i32 {
        libc::AF_INET => Some(Ipv4Addr::from(<[u8; 4]>::try_from(data).ok()?).into()),
        libc::AF_INET6 => Some(Ipv6Addr::from(<[u8; 16]>::try_from(data).ok()?).into()),
        _ => None,
    }
}
fn parse(data: &[u8]) -> Option<(u32, NetworkRoute)> {
    if data.len() < 12 || !matches!(data[7], 1 | 2) {
        return None;
    }
    let family = data[0];
    let mut dest = address(
        family,
        if family as i32 == libc::AF_INET {
            &[0; 4]
        } else {
            &[0; 16]
        },
    )?;
    let (mut interface, mut gateway, mut metric) = (None, None, None);
    let mut attrs = &data[12..];
    while attrs.len() >= 4 {
        let len = u16::from_ne_bytes(attrs[..2].try_into().ok()?) as usize;
        let kind = u16::from_ne_bytes(attrs[2..4].try_into().ok()?) & 0x3fff;
        if len < 4 || len > attrs.len() {
            return None;
        }
        let value = &attrs[4..len];
        match kind {
            1 => dest = address(family, value)?,
            4 => interface = u32_ne(value),
            5 => gateway = address(family, value),
            6 => metric = u32_ne(value),
            _ => {}
        }
        let aligned = (len + 3) & !3;
        if aligned > attrs.len() {
            break;
        }
        attrs = &attrs[aligned..];
    }
    Some((
        interface?,
        NetworkRoute {
            destination: IpAddress::new(dest, data[1]).ok()?,
            gateway,
            metric,
        },
    ))
}
pub(super) fn read() -> io::Result<Vec<(u32, NetworkRoute)>> {
    // The owned fd closes on every exit; all requests are read-only routing-table dumps.
    let raw = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut target: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    target.nl_family = libc::AF_NETLINK as u16;
    let len = std::mem::size_of_val(&target) as libc::socklen_t;
    if unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&target as *const libc::sockaddr_nl).cast(),
            len,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let timeout = libc::timeval {
        tv_sec: 0,
        tv_usec: 200_000,
    };
    if unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&timeout as *const libc::timeval).cast(),
            std::mem::size_of_val(&timeout) as _,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut request = [0u8; 28];
    request[..4].copy_from_slice(&28u32.to_ne_bytes());
    request[4..6].copy_from_slice(&26u16.to_ne_bytes());
    request[6..8].copy_from_slice(&0x301u16.to_ne_bytes());
    request[8..12].copy_from_slice(&1u32.to_ne_bytes());
    if unsafe {
        libc::sendto(
            fd.as_raw_fd(),
            request.as_ptr().cast(),
            request.len(),
            0,
            (&target as *const libc::sockaddr_nl).cast(),
            len,
        )
    } != request.len() as isize
    {
        return Err(io::Error::last_os_error());
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut buffer = [0u8; 65536];
    let mut routes = Vec::new();
    for _ in 0..128 {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "route dump timed out",
            ));
        }
        let mut source: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        let mut source_len = len;
        let size = unsafe {
            libc::recvfrom(
                fd.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                libc::MSG_TRUNC,
                (&mut source as *mut libc::sockaddr_nl).cast(),
                &mut source_len,
            )
        };
        if size < 0 {
            return Err(io::Error::last_os_error());
        }
        if size as usize > buffer.len() || source.nl_pid != 0 {
            return Err(invalid());
        }
        let mut messages = &buffer[..size as usize];
        while messages.len() >= 16 {
            let len = u32_ne(messages).ok_or_else(invalid)? as usize;
            if len < 16 || len > messages.len() {
                return Err(invalid());
            }
            let kind = u16::from_ne_bytes(messages[4..6].try_into().unwrap());
            let flags = u16::from_ne_bytes(messages[6..8].try_into().unwrap());
            if u32_ne(&messages[8..]) != Some(1) || flags & 0x10 != 0 {
                return Err(invalid());
            }
            match kind {
                3 => {
                    if len >= 20 && u32_ne(&messages[16..]) != Some(0) {
                        return Err(invalid());
                    }
                    return Ok(routes);
                }
                2 => return Err(invalid()),
                24 => {
                    if let Some(route) = parse(&messages[16..len]) {
                        routes.push(route);
                    }
                }
                _ => {}
            }
            let aligned = (len + 3) & !3;
            if aligned > messages.len() {
                break;
            }
            messages = &messages[aligned..];
        }
    }
    Err(invalid())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn route_inventory_parses_default_and_rejects_truncated_attributes() {
        let mut data = vec![0u8; 12];
        data[0] = libc::AF_INET as u8;
        data[7] = 1;
        data.extend_from_slice(&8u16.to_ne_bytes());
        data.extend_from_slice(&4u16.to_ne_bytes());
        data.extend_from_slice(&7u32.to_ne_bytes());
        data.extend_from_slice(&8u16.to_ne_bytes());
        data.extend_from_slice(&5u16.to_ne_bytes());
        data.extend_from_slice(&[192, 0, 2, 1]);
        let (index, route) = parse(&data).unwrap();
        assert_eq!(index, 7);
        assert_eq!(route.destination.prefix, 0);
        assert_eq!(route.gateway, Some("192.0.2.1".parse().unwrap()));
        data.pop();
        assert!(parse(&data).is_none());
    }
}
