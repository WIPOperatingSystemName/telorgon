use crate::screen_brightness::ScreenBrightnessError as Error;
use std::{
    fs::{self, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::PathBuf,
    time::Duration,
};

#[repr(C)]
struct I2cMessage {
    address: u16,
    flags: u16,
    length: u16,
    buffer: *mut u8,
}
#[repr(C)]
struct I2cTransfer {
    messages: *mut I2cMessage,
    count: u32,
}
pub(super) struct Monitor {
    file: std::fs::File,
    connector: PathBuf,
    identity: (u64, u64),
    edid: Vec<u8>,
}
impl Monitor {
    /// Trusted broker configuration selects a DRM connector, never an arbitrary bus path.
    pub fn open(connector: &str) -> Result<Self, Error> {
        if !super::super::sysfs::valid_name(connector) || !connector.starts_with("card") {
            return Err(Error::InvalidConfig("DRM connector"));
        }
        let _sysfs = super::super::sysfs::Sysfs::open()?;
        let connector = PathBuf::from("/sys/class/drm").join(connector);
        let metadata = fs::metadata(&connector).map_err(|_| Error::Unavailable)?;
        let ddc = fs::canonicalize(connector.join("ddc")).map_err(|_| Error::Unsupported)?;
        if !ddc.starts_with("/sys/devices") {
            return Err(Error::InvalidData);
        }
        let bus = ddc
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| {
                s.strip_prefix("i2c-")
                    .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            })
            .ok_or(Error::InvalidData)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(PathBuf::from("/dev").join(bus))
            .map_err(|_| Error::PermissionDenied)?;
        let stat = file.metadata().map_err(|_| Error::Unavailable)?;
        if stat.mode() & libc::S_IFMT != libc::S_IFCHR || libc::major(stat.rdev()) != 89 {
            return Err(Error::InvalidData);
        }
        let mut functions: libc::c_ulong = 0;
        // SAFETY: live owned fd and correctly sized writable ioctl output.
        if unsafe { libc::ioctl(file.as_raw_fd(), 0x0705, &mut functions) } < 0
            || functions & 1 == 0
        {
            return Err(Error::Unsupported);
        }
        let edid = read_edid(&connector)?;
        Ok(Self {
            file,
            connector,
            identity: (metadata.dev(), metadata.ino()),
            edid,
        })
    }
    fn validate(&self) -> Result<(), Error> {
        let metadata = fs::metadata(&self.connector).map_err(|_| Error::StaleDevice)?;
        if (metadata.dev(), metadata.ino()) != self.identity
            || read_edid(&self.connector)? != self.edid
            || fs::read_to_string(self.connector.join("status"))
                .map_err(|_| Error::StaleDevice)?
                .trim()
                != "connected"
        {
            return Err(Error::StaleDevice);
        }
        Ok(())
    }
    fn transfer(&self, bytes: &mut [u8], read: bool) -> Result<(), Error> {
        let mut message = I2cMessage {
            address: 0x37,
            flags: u16::from(read),
            length: bytes.len() as u16,
            buffer: bytes.as_mut_ptr(),
        };
        let mut transfer = I2cTransfer {
            messages: &mut message,
            count: 1,
        };
        // SAFETY: both repr(C) records and the buffer remain valid for the synchronous ioctl.
        // The address is always DDC/CI and callers cannot choose messages or flags.
        if unsafe { libc::ioctl(self.file.as_raw_fd(), 0x0707, &mut transfer) } != 1 {
            return Err(Error::Transport);
        }
        Ok(())
    }
    pub fn read(&self) -> Result<(u32, u32), Error> {
        self.validate()?;
        let mut request = get_packet();
        self.transfer(&mut request, false)?;
        std::thread::sleep(Duration::from_millis(50));
        let mut response = [0_u8; 11];
        self.transfer(&mut response, true)?;
        std::thread::sleep(Duration::from_millis(50));
        parse_response(&response)
    }
    pub fn set(&self, value: u32) -> Result<(), Error> {
        self.validate()?;
        let mut packet = set_packet(u16::try_from(value).map_err(|_| Error::InvalidLevel)?);
        self.transfer(&mut packet, false)?;
        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    }
}
fn read_edid(connector: &std::path::Path) -> Result<Vec<u8>, Error> {
    use std::io::Read;
    let mut data = Vec::new();
    std::fs::File::open(connector.join("edid"))
        .map_err(|_| Error::StaleDevice)?
        .take(4097)
        .read_to_end(&mut data)
        .map_err(|_| Error::Transport)?;
    if data.len() < 128
        || data.len() > 4096
        || !data.len().is_multiple_of(128)
        || data[..8] != [0, 255, 255, 255, 255, 255, 255, 0]
        || data
            .chunks_exact(128)
            .any(|block| block.iter().fold(0_u8, |sum, v| sum.wrapping_add(*v)) != 0)
    {
        return Err(Error::InvalidData);
    }
    Ok(data)
}
fn get_packet() -> [u8; 5] {
    let mut p = [0x51, 0x82, 0x01, 0x10, 0];
    p[4] = p[..4].iter().fold(0x6e, |s, b| s ^ b);
    p
}
fn set_packet(value: u16) -> [u8; 7] {
    let [hi, lo] = value.to_be_bytes();
    let mut p = [0x51, 0x84, 0x03, 0x10, hi, lo, 0];
    p[6] = p[..6].iter().fold(0x6e, |s, b| s ^ b);
    p
}
fn parse_response(p: &[u8; 11]) -> Result<(u32, u32), Error> {
    if p[0] != 0x6e
        || p[1] != 0x88
        || p[2] != 0x02
        || p[4] != 0x10
        || p.iter().fold(0x50, |sum, v| sum ^ v) != 0
    {
        return Err(Error::InvalidData);
    }
    if p[3] != 0 {
        return Err(Error::Unsupported);
    }
    let maximum = u32::from(u16::from_be_bytes([p[6], p[7]]));
    let current = u32::from(u16::from_be_bytes([p[8], p[9]]));
    if maximum == 0 || current > maximum {
        return Err(Error::InvalidData);
    }
    Ok((maximum, current))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_brightness_packets_and_valid_readback_are_accepted() {
        assert_eq!(get_packet(), [0x51, 0x82, 1, 0x10, 0xac]);
        assert_eq!(set_packet(60)[3], 0x10);
        let mut p = [0x6e, 0x88, 2, 0, 0x10, 0, 0, 200, 0, 100, 0];
        p[10] = p[..10].iter().fold(0x50, |sum, v| sum ^ v);
        assert_eq!(parse_response(&p), Ok((200, 100)));
        p[4] = 0x12;
        assert_eq!(parse_response(&p), Err(Error::InvalidData));
    }
}
