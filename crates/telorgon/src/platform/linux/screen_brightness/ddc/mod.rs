//! Restricted DDC/CI over a private inherited Unix channel. The host/launcher owns process setup.
//! Use a separate broker process; never call the server on a compositor or UI thread.
mod channel;
mod transport;
use crate::{platform::contracts::PermissionState, screen_brightness::*, shell::OutputId};
use ScreenBrightnessError as Error;
use std::{
    os::{fd::AsRawFd, unix::net::UnixStream},
    time::Duration,
};
const MAGIC: [u8; 4] = *b"TGB1";
const LIST: u8 = 1;
const READ: u8 = 2;
const SET: u8 = 3;

#[derive(Clone, Debug)]
pub struct ScreenBrightnessDdcMonitorConfig {
    pub connector: String,
    pub label: String,
}
#[derive(Clone, Debug)]
pub struct ScreenBrightnessDdcBrokerConfig {
    pub session_pid: u32,
    pub seat: String,
    pub monitors: Vec<ScreenBrightnessDdcMonitorConfig>,
    pub timeout: Duration,
    pub minimum: ScreenBrightnessLevel,
    pub allow_zero: bool,
}
/// Runs the broker on a private channel established by the trusted session launcher.
/// There is no listener/socket pathname and clients cannot request arbitrary buses or VCP codes.
/// The launcher must retain ownership of stalled workers until exit/reap or authoritative reset,
/// sandbox this process, and prevent channel inheritance by untrusted children.
pub fn serve_screen_brightness_ddc(
    mut channel: UnixStream,
    config: ScreenBrightnessDdcBrokerConfig,
) -> Result<(), Error> {
    if config.monitors.is_empty()
        || config.monitors.len() > 64
        || !super::sysfs::valid_name(&config.seat)
        || config.timeout.is_zero()
        || config.timeout > Duration::from_secs(60)
        || config.monitors.iter().any(|m| m.label.len() > 256)
    {
        return Err(Error::InvalidConfig("DDC broker configuration"));
    }
    let session = super::logind::Session::connect(None, config.session_pid, config.timeout)?;
    let peer = peer_uid(&channel)?;
    let uid = session.uid();
    if peer != 0 && peer != uid {
        return Err(Error::PermissionDenied);
    }
    let mut connectors = std::collections::HashSet::new();
    if config
        .monitors
        .iter()
        .any(|m| !connectors.insert(m.connector.clone()))
    {
        return Err(Error::InvalidConfig("duplicate DDC connector"));
    }
    let monitors = config
        .monitors
        .iter()
        .map(|m| transport::Monitor::open(&m.connector))
        .collect::<Result<Vec<_>, _>>()?;
    channel
        .set_nonblocking(true)
        .map_err(|_| Error::Transport)?;
    loop {
        let mut request = [0_u8; 20];
        // Idle channels can remain open indefinitely; partial frames have a deadline.
        if !channel::read(&mut channel, &mut request[..1], None, true)? {
            return Ok(());
        }
        channel::read(
            &mut channel,
            &mut request[1..],
            Some(std::time::Instant::now() + config.timeout),
            false,
        )?;
        let nonce = u64::from_be_bytes(request[12..20].try_into().unwrap());
        let mut payload = Vec::new();
        let result = parse_request(&request).and_then(|(operation, index, value)| {
            session.authorize(uid, &config.seat)?;
            if operation == LIST {
                payload.extend_from_slice(&(monitors.len() as u16).to_be_bytes());
                for (index, monitor) in monitors.iter().enumerate() {
                    let (maximum, current) = monitor.read()?;
                    payload.extend_from_slice(&(index as u16).to_be_bytes());
                    payload.extend_from_slice(&maximum.to_be_bytes());
                    payload.extend_from_slice(&current.to_be_bytes());
                    let label = &config.monitors[index].label;
                    payload.extend_from_slice(&(label.len() as u16).to_be_bytes());
                    payload.extend_from_slice(label.as_bytes());
                }
            } else {
                let monitor = monitors.get(index as usize).ok_or(Error::StaleDevice)?;
                let (maximum, before) = monitor.read()?;
                if operation == SET {
                    session.authorize(uid, &config.seat)?;
                    let minimum = ((u64::from(config.minimum.as_basis_points())
                        * u64::from(maximum))
                    .div_ceil(10_000) as u32)
                        .max(u32::from(!config.allow_zero));
                    if value < minimum || value > maximum {
                        return Err(Error::InvalidLevel);
                    }
                    monitor.set(value)?;
                }
                let (maximum, current) = if operation == SET {
                    monitor.read()?
                } else {
                    (maximum, before)
                };
                payload.extend_from_slice(&maximum.to_be_bytes());
                payload.extend_from_slice(&current.to_be_bytes());
            }
            Ok(())
        });
        let status = match result {
            Ok(()) => 0,
            Err(Error::PermissionDenied | Error::Locked | Error::SessionInactive) => 1,
            Err(Error::Unsupported) => 2,
            Err(Error::StaleDevice) => 3,
            Err(_) => 4,
        };
        let mut response = Vec::from(MAGIC);
        response.push(status);
        response.extend_from_slice(&[0; 3]);
        response.extend_from_slice(&nonce.to_be_bytes());
        if status == 0 {
            response.extend_from_slice(&payload);
        }
        channel::write(
            &mut channel,
            &response,
            std::time::Instant::now() + config.timeout,
        )?;
    }
}
fn peer_uid(channel: &UnixStream) -> Result<u32, Error> {
    let mut peer = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: live socket and correctly sized writable peer-credential record.
    if unsafe {
        libc::getsockopt(
            channel.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            peer.as_mut_ptr().cast(),
            &mut length,
        )
    } != 0
        || length as usize != std::mem::size_of::<libc::ucred>()
    {
        return Err(Error::PermissionDenied);
    }
    Ok(unsafe { peer.assume_init() }.uid)
}
fn parse_request(request: &[u8; 20]) -> Result<(u8, u16, u32), Error> {
    let operation = request[4];
    let index = u16::from_be_bytes([request[6], request[7]]);
    let value = u32::from_be_bytes(request[8..12].try_into().unwrap());
    if request[..4] != MAGIC
        || request[5] != 0
        || ![LIST, READ, SET].contains(&operation)
        || (operation != SET && value != 0)
        || (operation == LIST && index != 0)
    {
        return Err(Error::InvalidData);
    }
    Ok((operation, index, value))
}
/// Client provider. The supplied channel is an existing, authenticated, private broker connection.
/// A timeout permanently quarantines this connection; constructing another does not prove an old
/// broker has exited. The trusted launcher must establish transport recovery before replacement.
pub struct ScreenBrightnessDdcProvider {
    channel: UnixStream,
    timeout: Duration,
    nonce: u64,
    failed: bool,
    deadline: std::time::Instant,
    outputs: Vec<Vec<OutputId>>,
    maximums: std::collections::HashMap<u16, u32>,
}
impl ScreenBrightnessDdcProvider {
    pub fn new(
        channel: UnixStream,
        timeout: Duration,
        outputs: Vec<Vec<OutputId>>,
    ) -> Result<Self, Error> {
        if timeout.is_zero()
            || timeout > Duration::from_secs(60)
            || outputs.len() > 64
            || outputs.iter().any(|v| v.len() > 64)
        {
            return Err(Error::InvalidConfig("DDC client bounds"));
        }
        channel
            .set_nonblocking(true)
            .map_err(|_| Error::Transport)?;
        Ok(Self {
            channel,
            timeout,
            nonce: 0,
            failed: false,
            deadline: std::time::Instant::now(),
            outputs,
            maximums: std::collections::HashMap::new(),
        })
    }
    fn read_bytes(&mut self, bytes: &mut [u8]) -> Result<(), Error> {
        channel::read(&mut self.channel, bytes, Some(self.deadline), false).map(|_| ())
    }
    fn exchange(&mut self, operation: u8, index: u16, value: u32) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Unavailable);
        }
        self.deadline = std::time::Instant::now() + self.timeout;
        self.nonce = self.nonce.checked_add(1).ok_or(Error::Unavailable)?;
        let mut request = Vec::from(MAGIC);
        request.extend_from_slice(&[operation, 0]);
        request.extend_from_slice(&index.to_be_bytes());
        request.extend_from_slice(&value.to_be_bytes());
        request.extend_from_slice(&self.nonce.to_be_bytes());
        channel::write(&mut self.channel, &request, self.deadline)?;
        let mut header = [0; 16];
        self.read_bytes(&mut header)?;
        if header[..4] != MAGIC
            || header[5..8] != [0; 3]
            || u64::from_be_bytes(header[8..16].try_into().unwrap()) != self.nonce
        {
            return Err(Error::InvalidData);
        }
        match header[4] {
            0 => Ok(()),
            1 => Err(Error::PermissionDenied),
            2 => Err(Error::Unsupported),
            3 => Err(Error::StaleDevice),
            _ => Err(Error::Transport),
        }
    }
    fn quarantine<T>(&mut self, result: Result<T, Error>) -> Result<T, Error> {
        if matches!(result, Err(Error::Transport | Error::InvalidData)) {
            self.failed = true;
            let _ = self.channel.shutdown(std::net::Shutdown::Both);
        }
        result
    }
    fn reading(&mut self, index: u16, operation: u8, value: u32) -> Result<(u32, u32), Error> {
        let result = (|| {
            self.exchange(operation, index, value)?;
            let mut data = [0; 8];
            self.read_bytes(&mut data)?;
            let maximum = u32::from_be_bytes(data[..4].try_into().unwrap());
            let current = u32::from_be_bytes(data[4..].try_into().unwrap());
            if maximum == 0 || current > maximum {
                return Err(Error::InvalidData);
            }
            if self
                .maximums
                .get(&index)
                .is_some_and(|known| *known != maximum)
            {
                return Err(Error::StaleDevice);
            }
            Ok((maximum, current))
        })();
        self.quarantine(result)
    }
}
impl ScreenBrightnessProvider for ScreenBrightnessDdcProvider {
    fn discover(&mut self) -> Result<Vec<ScreenBrightnessProviderDevice>, Error> {
        let result = (|| {
            self.exchange(LIST, 0, 0)?;
            let mut count = [0; 2];
            self.read_bytes(&mut count)?;
            let count = u16::from_be_bytes(count);
            if count > 64 {
                return Err(Error::InvalidData);
            }
            let mut devices = Vec::new();
            let mut maximums = std::collections::HashMap::new();
            for _ in 0..count {
                let mut record = [0; 12];
                self.read_bytes(&mut record)?;
                let index = u16::from_be_bytes(record[..2].try_into().unwrap());
                let maximum = u32::from_be_bytes(record[2..6].try_into().unwrap());
                let current = u32::from_be_bytes(record[6..10].try_into().unwrap());
                let length = u16::from_be_bytes(record[10..].try_into().unwrap()) as usize;
                if length > 256 || maximum == 0 || current > maximum || index >= count {
                    return Err(Error::InvalidData);
                }
                if maximums.insert(index, maximum).is_some() {
                    return Err(Error::InvalidData);
                }
                let mut label = vec![0; length];
                self.read_bytes(&mut label)?;
                devices.push(ScreenBrightnessProviderDevice {
                    key: index.to_string(),
                    name: String::from_utf8(label).map_err(|_| Error::InvalidData)?,
                    kind: ScreenBrightnessKind::ExternalMonitor,
                    association: self.outputs.get(index as usize).cloned().map_or(
                        ScreenBrightnessAssociation::Unknown,
                        ScreenBrightnessAssociation::HostConfigured,
                    ),
                    maximum,
                    permission: PermissionState::Granted,
                    verification: ScreenBrightnessVerification::MonitorVcp,
                });
            }
            self.maximums = maximums;
            Ok(devices)
        })();
        self.quarantine(result)
    }
    fn read(&mut self, key: &str) -> Result<ScreenBrightnessReading, Error> {
        let (_, configured) =
            self.reading(key.parse().map_err(|_| Error::StaleDevice)?, READ, 0)?;
        Ok(ScreenBrightnessReading {
            configured,
            actual: None,
        })
    }
    fn set(&mut self, key: &str, value: u32) -> Result<(), Error> {
        self.reading(key.parse().map_err(|_| Error::StaleDevice)?, SET, value)
            .map(|_| ())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_arbitrary_operations_and_payloads() {
        let mut request = [0; 20];
        request[..4].copy_from_slice(&MAGIC);
        request[4] = SET;
        assert!(parse_request(&request).is_ok());
        request[4] = 0x10;
        assert_eq!(parse_request(&request), Err(Error::InvalidData));
        request[4] = READ;
        request[11] = 1;
        assert_eq!(parse_request(&request), Err(Error::InvalidData));
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;
    #[test]
    fn malformed_response_quarantines_channel() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let worker = std::thread::spawn(move || {
            server.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            let mut request = [0; 20];
            channel::read(&mut server, &mut request, Some(deadline), false).unwrap();
            channel::write(&mut server, &[0; 16], deadline).unwrap();
        });
        let mut provider =
            ScreenBrightnessDdcProvider::new(client, Duration::from_millis(100), vec![]).unwrap();
        assert!(matches!(provider.discover(), Err(Error::InvalidData)));
        assert!(matches!(provider.discover(), Err(Error::Unavailable)));
        worker.join().unwrap();
    }
    #[test]
    fn missing_reply_quarantines_channel_without_retrying() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let worker = std::thread::spawn(move || {
            server.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            let mut request = [0; 20];
            channel::read(&mut server, &mut request, Some(deadline), false).unwrap();
            let mut byte = [0; 1];
            assert!(!channel::read(&mut server, &mut byte, Some(deadline), true).unwrap());
        });
        let mut provider =
            ScreenBrightnessDdcProvider::new(client, Duration::from_millis(50), vec![]).unwrap();
        assert!(matches!(provider.discover(), Err(Error::Transport)));
        assert!(matches!(provider.discover(), Err(Error::Unavailable)));
        worker.join().unwrap();
    }
}
