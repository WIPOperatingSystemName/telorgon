//! Preparation-worker resources. No application connections are accepted here.
//! Keep reservations alive across helper restarts, and runtime files alive until
//! all owned helpers are reaped. Shared-server X11 is not same-user isolation.
use super::{Error, Result};
use std::{
    ffi::{CString, OsStr},
    fs::File,
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        linux::net::SocketAddrExt,
        unix::{
            ffi::OsStrExt,
            fs::MetadataExt,
            net::{SocketAddr, UnixListener},
        },
    },
    path::{Component, Path, PathBuf},
};

fn name(value: &OsStr) -> Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| Error("NUL in Xwayland resource path".into()))
}
fn open(dir: &File, part: &OsStr, flags: i32, mode: u32) -> Result<File> {
    let part = name(part)?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            part.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            mode,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn directory(path: &Path, private: bool) -> Result<File> {
    if !path.is_absolute() || path == Path::new("/") {
        return Err(Error(
            "Xwayland resource directory must be an absolute non-root path".into(),
        ));
    }
    let mut dir = File::open("/")?;
    for part in path.components().skip(1) {
        let Component::Normal(part) = part else {
            return Err(Error("ambiguous Xwayland resource path".into()));
        };
        dir = open(&dir, part, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        let m = dir.metadata()?;
        if (m.uid() != 0 && m.uid() != unsafe { libc::geteuid() })
            || (m.mode() & 0o022 != 0 && !(m.uid() == 0 && m.mode() & 0o1000 != 0))
        {
            return Err(Error(
                "unsafe Xwayland resource directory ownership or permissions".into(),
            ));
        }
    }
    let m = dir.metadata()?;
    if private && (m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o7777 != 0o700) {
        return Err(Error(
            "Xwayland runtime directory must be owned by this user with mode 0700".into(),
        ));
    }
    Ok(dir)
}
fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    let mut offset = 0;
    while offset < N {
        let n = unsafe { libc::getrandom(bytes[offset..].as_mut_ptr().cast(), N - offset, 0) };
        if n < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if n == 0 {
            return Err(Error("kernel random source returned no data".into()));
        }
        offset += n as usize;
    }
    Ok(bytes)
}

/// An entry created by this instance. Cleanup never recursively follows paths
/// or removes a replacement entry. Sticky shared directories protect entries
/// against other users; processes of the same user share authority.
struct Entry {
    parent: File,
    name: CString,
    dev: u64,
    ino: u64,
    directory: bool,
    cleanup: bool,
    _identity: File,
}
impl Entry {
    fn record(parent: &File, part: &str, directory: bool) -> Result<Self> {
        // Retain the inode, even if the name is removed, so a replacement cannot
        // recycle its inode number and fool cleanup's identity comparison.
        let identity = open(parent, OsStr::new(part), libc::O_PATH, 0)?;
        let metadata = identity.metadata()?;
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(Error("Xwayland resource ownership changed".into()));
        }
        Ok(Self {
            parent: parent.try_clone()?,
            name: name(OsStr::new(part))?,
            dev: metadata.dev(),
            ino: metadata.ino(),
            directory,
            cleanup: true,
            _identity: identity,
        })
    }
}
impl Drop for Entry {
    fn drop(&mut self) {
        if !self.cleanup {
            return;
        }
        let mut stat = std::mem::MaybeUninit::uninit();
        if unsafe {
            libc::fstatat(
                self.parent.as_raw_fd(),
                self.name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return;
        }
        let stat = unsafe { stat.assume_init() };
        if stat.st_dev == self.dev
            && stat.st_ino == self.ino
            && stat.st_uid == unsafe { libc::geteuid() }
        {
            unsafe {
                libc::unlinkat(
                    self.parent.as_raw_fd(),
                    self.name.as_ptr(),
                    if self.directory {
                        libc::AT_REMOVEDIR
                    } else {
                        0
                    },
                );
            }
        }
    }
}

/// Conventional X display lock and both Linux listener namespaces. The parent
/// retains these listeners; duplicate descriptors for each server generation.
pub struct DisplayReservation {
    number: u16,
    filesystem: UnixListener,
    abstract_socket: UnixListener,
    // Drop filesystem socket name before releasing the display lock.
    _socket: Entry,
    _lock: Entry,
}
impl DisplayReservation {
    /// Reserve the first free number in the inclusive range. Existing locks and
    /// sockets are skipped, including stale/unparseable locks. The conventional
    /// /tmp/.X11-unix directory must already exist; never repair shared state.
    pub fn reserve(first: u16, last: u16) -> Result<Self> {
        let locks = directory(Path::new("/tmp"), false)?;
        let sockets = directory(Path::new("/tmp/.X11-unix"), false)?;
        Self::in_directories(&locks, &sockets, b"/tmp/.X11-unix/", first, last)
    }
    fn in_directories(
        locks: &File,
        sockets: &File,
        abstract_prefix: &[u8],
        first: u16,
        last: u16,
    ) -> Result<Self> {
        for number in first..=last {
            let lock_name = format!(".X{number}-lock");
            // O_EXCL never opens a pre-existing file, symlink, FIFO or stale lock.
            let fd = unsafe {
                libc::openat(
                    locks.as_raw_fd(),
                    name(OsStr::new(&lock_name))?.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW,
                    0o600,
                )
            };
            if fd < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EEXIST) {
                    continue;
                }
                return Err(error.into());
            }
            let mut file = unsafe { File::from_raw_fd(fd) };
            let lock = Entry::record(locks, &lock_name, false)?;
            writeln!(file, "{:>10}", std::process::id())?;
            let socket_name = format!("X{number}");
            let mut abstract_name = abstract_prefix.to_vec();
            abstract_name.extend_from_slice(socket_name.as_bytes());
            let address = SocketAddr::from_abstract_name(&abstract_name)?;
            let abstract_socket = match UnixListener::bind_addr(&address) {
                Ok(socket) => socket,
                Err(error) if error.raw_os_error() == Some(libc::EADDRINUSE) => continue,
                Err(error) => return Err(error.into()),
            };
            // Anchor pathname binding to the validated directory, avoiding
            // path traversal after validation. Never unlink before binding.
            let path = format!("/proc/self/fd/{}/{socket_name}", sockets.as_raw_fd());
            let filesystem = match UnixListener::bind(&path) {
                Ok(socket) => socket,
                Err(error) if error.raw_os_error() == Some(libc::EADDRINUSE) => continue,
                Err(error) => return Err(error.into()),
            };
            let socket = Entry::record(sockets, &socket_name, false)?;
            // No process-global umask mutation. Authentication, not a world-
            // writable listener pathname, determines admission to the server.
            if unsafe {
                libc::fchmodat(
                    sockets.as_raw_fd(),
                    socket.name.as_ptr(),
                    0o666,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(io::Error::last_os_error().into());
            }
            return Ok(Self {
                number,
                filesystem,
                abstract_socket,
                _socket: socket,
                _lock: lock,
            });
        }
        Err(Error("no free X display in the configured range".into()))
    }
    pub fn number(&self) -> u16 {
        self.number
    }
    pub fn display(&self) -> String {
        format!(":{}", self.number)
    }
    /// Child duplicates remain blocking, as expected by the X server's listener
    /// setup. The parent never accepts connections on its retained copies.
    pub fn duplicate_listeners(&self) -> Result<[OwnedFd; 2]> {
        Ok([
            self.filesystem.try_clone()?.into(),
            self.abstract_socket.try_clone()?.into(),
        ])
    }
}

/// Private authority and keymap scratch directory. No cookie bytes are exposed
/// in Debug/status/environment; applications receive only the authority path.
pub struct RuntimeFiles {
    display: u16,
    path: PathBuf,
    _authority: Entry,
    _scratch: Entry,
    _instance: Entry,
}
impl RuntimeFiles {
    pub fn create(runtime_root: &Path, display: u16) -> Result<Self> {
        let root = directory(runtime_root, true)?;
        let suffix = random::<16>()?
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let part = format!("telorgon-x11-{suffix}");
        let cpart = name(OsStr::new(&part))?;
        if unsafe { libc::mkdirat(root.as_raw_fd(), cpart.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let instance = Entry::record(&root, &part, true)?;
        let dir = open(
            &root,
            OsStr::new(&part),
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        // Set exact modes on new descriptors. If umask makes a directory
        // inaccessible before it can be opened, preparation fails closed.
        if unsafe { libc::fchmod(dir.as_raw_fd(), 0o700) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        if unsafe { libc::mkdirat(dir.as_raw_fd(), c"keymaps".as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let scratch = Entry::record(&dir, "keymaps", true)?;
        let scratch_fd = open(
            &dir,
            OsStr::new("keymaps"),
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        if unsafe { libc::fchmod(scratch_fd.as_raw_fd(), 0o700) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut authority = open(
            &dir,
            OsStr::new("authority"),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?;
        let authority_entry = Entry::record(&dir, "authority", false)?;
        if unsafe { libc::fchmod(authority.as_raw_fd(), 0o600) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut hostname = [0u8; 256];
        if unsafe { libc::gethostname(hostname.as_mut_ptr().cast(), hostname.len()) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let end = hostname
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| Error("Xauthority hostname too long".into()))?;
        let cookie = random::<16>()?;
        // Xauthority wire records: FamilyLocal=256, then four counted byte
        // strings in network order. No xauth subprocess or TCP address records.
        authority.write_all(&256u16.to_be_bytes())?;
        for bytes in [
            &hostname[..end],
            display.to_string().as_bytes(),
            b"MIT-MAGIC-COOKIE-1",
            &cookie,
        ] {
            authority.write_all(&(bytes.len() as u16).to_be_bytes())?;
            authority.write_all(bytes)?;
        }
        authority.sync_all()?;
        Ok(Self {
            display,
            path: runtime_root.join(part),
            _authority: authority_entry,
            _scratch: scratch,
            _instance: instance,
        })
    }
    pub fn authority(&self) -> PathBuf {
        self.path.join("authority")
    }
    pub fn display_number(&self) -> u16 {
        self.display
    }
    pub fn keymap_directory(&self) -> PathBuf {
        self.path.join("keymaps")
    }
}

impl Drop for RuntimeFiles {
    fn drop(&mut self) {
        // The pinned helper writes exactly this filename. Reclaim an owned,
        // regular single-link crash remnant; never recurse or follow links.
        // Unknown entries leave the directory intact for explicit inspection.
        let part = format!("server-{}.xkm", self.display);
        if let Ok(entry) = Entry::record(&self._scratch._identity, &part, false) {
            if let Ok(metadata) = entry._identity.metadata() {
                if metadata.is_file() && metadata.nlink() == 1 {
                    drop(entry);
                    return;
                }
            }
            // Entry's destructor removes a name, so disarm it for a type or
            // link-count mismatch without retaining any descriptor indefinitely.
            let mut entry = entry;
            entry.cleanup = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let suffix = random::<16>()
                .unwrap()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let path = std::env::temp_dir().join(format!("telorgon-resources-{suffix}"));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
        fn fd(&self) -> File {
            directory(&self.0, true).unwrap()
        }
        fn reserve(&self, first: u16, last: u16) -> Result<DisplayReservation> {
            let prefix = format!("{}/", self.0.display());
            DisplayReservation::in_directories(
                &self.fd(),
                &self.fd(),
                prefix.as_bytes(),
                first,
                last,
            )
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn reservations_skip_owned_displays_and_clean_only_their_entries() {
        let temp = Temp::new();
        let first = temp.reserve(10, 12).unwrap();
        let second = temp.reserve(10, 12).unwrap();
        assert_eq!((first.number(), second.number()), (10, 11));
        assert_eq!(first.display(), ":10");
        let duplicates = first.duplicate_listeners().unwrap();
        drop(duplicates);
        assert!(temp.0.join("X10").exists());
        drop(first);
        assert!(!temp.0.join("X10").exists());
        assert!(!temp.0.join(".X10-lock").exists());
        assert_eq!(temp.reserve(10, 10).unwrap().number(), 10);
        drop(second);
        assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 0);
    }
    #[test]
    fn stale_locks_symlinks_and_both_socket_conflicts_are_preserved() {
        let temp = Temp::new();
        std::fs::write(temp.0.join(".X1-lock"), b"stale").unwrap();
        symlink("missing", temp.0.join(".X2-lock")).unwrap();
        let _socket = UnixListener::bind(temp.0.join("X3")).unwrap();
        let address = SocketAddr::from_abstract_name(format!("{}/X4", temp.0.display())).unwrap();
        let _abstract = UnixListener::bind_addr(&address).unwrap();
        let reservation = temp.reserve(1, 5).unwrap();
        assert_eq!(reservation.number(), 5);
        assert_eq!(std::fs::read(temp.0.join(".X1-lock")).unwrap(), b"stale");
        assert!(temp.0.join(".X2-lock").is_symlink());
        assert!(temp.0.join("X3").exists());
        assert!(!temp.0.join(".X3-lock").exists());
        assert!(!temp.0.join(".X4-lock").exists());
    }
    #[test]
    fn cleanup_preserves_replacement_lock() {
        let temp = Temp::new();
        let reservation = temp.reserve(1, 1).unwrap();
        // Keep the old inode alive, preventing incidental inode reuse.
        std::fs::rename(temp.0.join(".X1-lock"), temp.0.join("original")).unwrap();
        std::fs::write(temp.0.join(".X1-lock"), b"replacement").unwrap();
        drop(reservation);
        assert_eq!(
            std::fs::read(temp.0.join(".X1-lock")).unwrap(),
            b"replacement"
        );
    }
    #[test]
    fn authority_has_exact_record_permissions_and_fresh_cookie() {
        let temp = Temp::new();
        let first = RuntimeFiles::create(&temp.0, 42).unwrap();
        let second = RuntimeFiles::create(&temp.0, 42).unwrap();
        let a = std::fs::read(first.authority()).unwrap();
        let b = std::fs::read(second.authority()).unwrap();
        assert_eq!(&a[..2], &256u16.to_be_bytes());
        let mut pos = 2;
        let mut fields = vec![];
        for _ in 0..4 {
            let len = u16::from_be_bytes([a[pos], a[pos + 1]]) as usize;
            pos += 2;
            fields.push(&a[pos..pos + len]);
            pos += len;
        }
        assert_eq!(pos, a.len());
        assert!(!fields[0].is_empty());
        assert_eq!(fields[1], b"42");
        assert_eq!(fields[2], b"MIT-MAGIC-COOKIE-1");
        assert_eq!(fields[3].len(), 16);
        assert!(a[a.len() - 16..] != b[b.len() - 16..]);
        assert_eq!(
            std::fs::metadata(first.authority()).unwrap().mode() & 0o7777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(first.keymap_directory()).unwrap().mode() & 0o7777,
            0o700
        );
        let path = first.path.clone();
        drop(first);
        assert!(!path.exists());
    }
    #[test]
    fn runtime_rejects_symlinks_and_nonprivate_roots() {
        let temp = Temp::new();
        symlink(&temp.0, temp.0.join("alias")).unwrap();
        assert!(RuntimeFiles::create(&temp.0.join("alias"), 0).is_err());
        std::fs::set_permissions(&temp.0, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(RuntimeFiles::create(&temp.0, 0).is_err());
    }

    #[test]
    fn scratch_cleanup_removes_only_the_owned_regular_keymap() {
        let temp = Temp::new();
        let files = RuntimeFiles::create(&temp.0, 7).unwrap();
        std::fs::write(files.keymap_directory().join("server-7.xkm"), b"compiled").unwrap();
        let path = files.path.clone();
        drop(files);
        assert!(!path.exists());

        let files = RuntimeFiles::create(&temp.0, 7).unwrap();
        let outside = temp.0.join("preserved");
        std::fs::write(&outside, b"owned elsewhere").unwrap();
        let link = files.keymap_directory().join("server-7.xkm");
        symlink(&outside, &link).unwrap();
        drop(files);
        assert!(link.is_symlink());
        assert_eq!(std::fs::read(outside).unwrap(), b"owned elsewhere");
    }

    #[test]
    fn concurrent_reservations_have_distinct_numbers() {
        let temp = Temp::new();
        std::thread::scope(|scope| {
            let a = scope.spawn(|| temp.reserve(1, 2).unwrap());
            let b = scope.spawn(|| temp.reserve(1, 2).unwrap());
            let a = a.join().unwrap();
            let b = b.join().unwrap();
            assert_ne!(a.number(), b.number());
            assert!(temp.reserve(1, 2).is_err());
        });
        assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 0);
    }

    #[cfg(target_env = "gnu")]
    #[test]
    fn command_retains_resources_and_rejects_mismatched_authority() {
        use crate::integrations::x11::process::Command;
        use std::sync::Arc;
        let temp = Temp::new();
        let display = Arc::new(temp.reserve(3, 3).unwrap());
        let runtime = Arc::new(RuntimeFiles::create(&temp.0, 3).unwrap());
        let path = runtime.path.clone();
        let mismatch = Arc::new(RuntimeFiles::create(&temp.0, 4).unwrap());
        let mut command = Command::new(Path::new("/not-executed"), &[], &[], vec![]).unwrap();
        assert!(command.retain_resources(display.clone(), mismatch).is_err());
        command
            .retain_resources(display.clone(), runtime.clone())
            .unwrap();
        drop(display);
        drop(runtime);
        assert!(path.join("authority").exists());
        assert!(temp.0.join("X3").exists());
        drop(command);
        assert!(!path.exists());
        assert!(!temp.0.join("X3").exists());
    }

    #[cfg(target_env = "gnu")]
    #[test]
    fn embedded_preparation_owns_connections_without_executing_fixture() {
        use crate::integrations::x11::{payload, payload_format, process::Command};
        use std::{collections::BTreeMap, io::Read, sync::Arc};
        let temp = Temp::new();
        let payload = Arc::new(
            payload::extract(&payload_format::fixture(), &temp.0.join("payload")).unwrap(),
        );
        let display = Arc::new(temp.reserve(9, 9).unwrap());
        let runtime = Arc::new(RuntimeFiles::create(&temp.0, 9).unwrap());
        let authority = runtime.authority();
        // The synthetic ELF fixture is only validated/extracted, never spawned.
        let mut prepared =
            Command::embedded(payload, display.clone(), runtime, BTreeMap::new()).unwrap();
        assert!(!prepared.readiness.dispatch().unwrap());
        assert!(authority.exists());
        drop(prepared.command);
        assert!(!authority.exists());
        assert!(temp.0.join("X9").exists()); // Host retains restart reservation.
        assert!(prepared.readiness.dispatch().is_err());
        prepared
            .wayland
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        prepared
            .xwm
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        assert_eq!(prepared.wayland.read(&mut [0]).unwrap(), 0);
        assert_eq!(prepared.xwm.read(&mut [0]).unwrap(), 0);
    }
}
