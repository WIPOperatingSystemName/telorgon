use crate::screen_brightness::ScreenBrightnessError as Error;
use std::{
    ffi::CString,
    fs::File,
    io::Read,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
};

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}
const BENEATH: u64 = 0x08;
const NO_MAGICLINKS: u64 = 0x02;
const NO_SYMLINKS: u64 = 0x04;
const NO_XDEV: u64 = 0x01;
fn open_at(parent: RawFd, path: &str, flags: i32, no_links: bool) -> Result<OwnedFd, Error> {
    let path = CString::new(path).map_err(|_| Error::InvalidData)?;
    let how = OpenHow {
        flags: (flags | libc::O_CLOEXEC) as u64,
        mode: 0,
        resolve: BENEATH | NO_MAGICLINKS | NO_XDEV | if no_links { NO_SYMLINKS } else { 0 },
    };
    // SAFETY: CString and OpenHow remain valid for the syscall; success creates an owned fd.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            parent,
            path.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if fd < 0 {
        return Err(map_io(std::io::Error::last_os_error()));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
}
fn map_io(error: std::io::Error) -> Error {
    match error.raw_os_error() {
        Some(libc::EACCES | libc::EPERM) => Error::PermissionDenied,
        Some(libc::ENOENT | libc::ENODEV) => Error::StaleDevice,
        Some(libc::ENOSYS) => Error::Unsupported,
        _ => Error::Transport,
    }
}
pub(super) struct Sysfs {
    root: OwnedFd,
}
pub(super) struct Device {
    directory: OwnedFd,
    pub name: String,
    pub key: String,
}
impl Sysfs {
    pub fn open() -> Result<Self, Error> {
        // SAFETY: fixed NUL-terminated path and scalar flags; success creates an owned fd.
        let fd = unsafe {
            libc::open(
                c"/sys".as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(map_io(std::io::Error::last_os_error()));
        }
        let root = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: root stays open; stat is a writable output buffer.
        if unsafe { libc::fstatfs(root.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
            return Err(Error::Unavailable);
        }
        if unsafe { stat.assume_init() }.f_type != libc::SYSFS_MAGIC as libc::c_long {
            return Err(Error::Unavailable);
        }
        Ok(Self { root })
    }
    pub fn devices(&self) -> Result<Vec<Device>, Error> {
        use std::os::fd::IntoRawFd;
        let directory = open_at(
            self.root.as_raw_fd(),
            "class/backlight",
            libc::O_RDONLY | libc::O_DIRECTORY,
            false,
        )?;
        let fd = directory.into_raw_fd();
        // SAFETY: fdopendir takes ownership on success; readdir's pointer is used before next call.
        let raw = unsafe { libc::fdopendir(fd) };
        if raw.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err(Error::Unavailable);
        }
        struct Directory(*mut libc::DIR);
        impl Drop for Directory {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let directory = Directory(raw);
        let mut devices = Vec::new();
        loop {
            // SAFETY: errno is thread local; distinguish end-of-directory from a failed scan.
            unsafe {
                *libc::__errno_location() = 0;
            }
            let entry = unsafe { libc::readdir(directory.0) };
            if entry.is_null() {
                if unsafe { *libc::__errno_location() } != 0 {
                    return Err(map_io(std::io::Error::last_os_error()));
                }
                break;
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()
                .map_err(|_| Error::InvalidData)?;
            if name == "." || name == ".." {
                continue;
            }
            if !valid_name(name) || devices.len() >= 64 {
                return Err(Error::InvalidData);
            }
            // Class symlinks are legitimate. Resolution stays under the pinned sysfs root.
            let directory = match open_at(
                self.root.as_raw_fd(),
                &format!("class/backlight/{name}"),
                libc::O_PATH | libc::O_DIRECTORY,
                false,
            ) {
                Ok(fd) => fd,
                Err(Error::StaleDevice) => continue,
                Err(e) => return Err(e),
            };
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe { libc::fstat(directory.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
                continue;
            }
            let stat = unsafe { stat.assume_init() };
            devices.push(Device {
                directory,
                name: name.into(),
                key: format!("{name}:{}:{}", stat.st_dev, stat.st_ino),
            });
        }
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(devices)
    }
}
pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"_-.:".contains(&v))
}
impl Device {
    fn attribute(&self, name: &str, flags: i32) -> Result<OwnedFd, Error> {
        open_at(
            self.directory.as_raw_fd(),
            name,
            flags | libc::O_NOFOLLOW,
            true,
        )
    }
    pub fn read(&self, name: &str) -> Result<u32, Error> {
        let mut text = String::new();
        File::from(self.attribute(name, libc::O_RDONLY)?)
            .take(64)
            .read_to_string(&mut text)
            .map_err(map_io)?;
        parse(&text)
    }
    pub fn can_write(&self) -> bool {
        self.attribute("brightness", libc::O_WRONLY).is_ok()
    }
    pub fn write(&self, value: u32) -> Result<(), Error> {
        let fd = self.attribute("brightness", libc::O_WRONLY)?;
        let text = format!("{value}\n");
        // SAFETY: owned fd and complete ASCII buffer live for this one syscall. A suffix retry
        // would issue another independent sysfs command, so short writes are uncertain failures.
        let count = unsafe { libc::write(fd.as_raw_fd(), text.as_ptr().cast(), text.len()) };
        exact_write(count, text.len())
    }
}
pub(super) fn parse(text: &str) -> Result<u32, Error> {
    let value = text.trim_ascii();
    if text.len() >= 64 || value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::InvalidData);
    }
    value.parse().map_err(|_| Error::InvalidData)
}
pub(super) fn exact_write(count: isize, expected: usize) -> Result<(), Error> {
    if count >= 0 && count as usize == expected {
        Ok(())
    } else {
        Err(Error::Transport)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_partial_commands_and_malformed_attributes() {
        assert_eq!(exact_write(1, 4), Err(Error::Transport));
        assert_eq!(parse("12\n"), Ok(12));
        for value in ["-1", "12 13", "4294967296", "", "1\0"] {
            assert_eq!(parse(value), Err(Error::InvalidData));
        }
        for value in ["../x", "x/y", ".", ""] {
            assert!(!valid_name(value));
        }
    }
}
