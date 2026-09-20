//! Offline payload verification and private, content-addressed extraction.
//!
//! Extraction performs filesystem I/O and belongs on a preparation worker, not
//! the compositor dispatch thread. Keep the returned lease alive while helpers
//! use the generation. No executable is launched by this module.
use super::{Error, Result, payload_format as format};
use std::{
    collections::BTreeSet,
    ffi::{CStr, CString, OsStr},
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
    path::{Component, Path, PathBuf},
};

pub use super::payload_format::{
    Component as PayloadComponent, Entry as PayloadEntry, Manifest as PayloadManifest,
};

/// An immutable verified generation with a held shared usage lock.
pub struct PayloadLease {
    path: PathBuf,
    _usage: File,
    manifest: PayloadManifest,
}
impl PayloadLease {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn manifest(&self) -> &PayloadManifest {
        &self.manifest
    }
}

/// Validate every compressed entry without writing any files.
pub fn verify(bytes: &[u8]) -> Result<PayloadManifest> {
    format::validate(bytes, "x86_64-unknown-linux-gnu").map_err(Error)
}

#[cfg(feature = "shell-xwayland-embedded")]
pub fn embedded() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/xwayland.payload"))
}

fn name(value: &OsStr) -> Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| Error("NUL in payload path".into()))
}
fn stat(file: &File) -> Result<libc::stat> {
    let mut stat = std::mem::MaybeUninit::uninit();
    // SAFETY: stat points to sufficient writable storage; the descriptor is live.
    if unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { stat.assume_init() })
}
fn open_at(dir: &File, part: &OsStr, flags: i32, mode: u32) -> Result<File> {
    let part = name(part)?;
    // SAFETY: the NUL-terminated name and descriptor remain live for this call.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            part.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            mode,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn directory(dir: &File, part: &OsStr, create: bool) -> Result<File> {
    if create {
        let part = name(part)?;
        let rc = unsafe { libc::mkdirat(dir.as_raw_fd(), part.as_ptr(), 0o700) };
        if rc != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    open_at(dir, part, libc::O_RDONLY | libc::O_DIRECTORY, 0)
}
fn private_directory(file: &File) -> Result<()> {
    let s = stat(file)?;
    if s.st_uid != unsafe { libc::geteuid() } || s.st_mode & 0o7777 != 0o700 {
        return Err(Error(
            "payload directory must be owned by this user with mode 0700".into(),
        ));
    }
    Ok(())
}
fn root(path: &Path) -> Result<File> {
    if !path.is_absolute() {
        return Err(Error("payload cache root must be absolute".into()));
    }
    let mut dir = File::open("/")?;
    let parts: Vec<_> = path.components().collect();
    if parts.len() < 2 {
        return Err(Error("payload cache root cannot be /".into()));
    }
    for (i, part) in parts.iter().enumerate().skip(1) {
        let Component::Normal(part) = part else {
            return Err(Error("ambiguous cache path".into()));
        };
        dir = directory(&dir, part, true)?;
        let s = stat(&dir)?;
        let uid = unsafe { libc::geteuid() };
        // Root-owned sticky ancestors such as /tmp are permitted. The actual
        // cache root must always be private. Never chmod an existing directory.
        if (s.st_uid != 0 && s.st_uid != uid)
            || (s.st_mode & 0o022 != 0 && !(s.st_uid == 0 && s.st_mode & libc::S_ISVTX != 0))
        {
            return Err(Error(
                "unsafe payload cache ancestor ownership or permissions".into(),
            ));
        }
        if i == parts.len() - 1 {
            private_directory(&dir)?;
        }
    }
    Ok(dir)
}
fn checked_file(dir: &File, path: &str, mode: u32, create: bool) -> Result<File> {
    let mut parent = dir.try_clone()?;
    let parts: Vec<_> = path.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        parent = directory(&parent, OsStr::new(part), create)?;
        private_directory(&parent)?;
    }
    let flags = if create {
        libc::O_RDWR | libc::O_CREAT | libc::O_EXCL
    } else {
        libc::O_RDONLY
    };
    let file = open_at(&parent, OsStr::new(parts[parts.len() - 1]), flags, mode)?;
    let s = stat(&file)?;
    if s.st_uid != unsafe { libc::geteuid() }
        || s.st_nlink != 1
        || s.st_mode & libc::S_IFMT != libc::S_IFREG
        || s.st_mode & 0o7777 != mode
    {
        return Err(Error(format!("unsafe payload file: {path}")));
    }
    Ok(file)
}
fn lock(file: &File, operation: i32) -> Result<()> {
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::Interrupted {
            return Err(e.into());
        }
    }
}
fn lock_file(dir: &File, path: &str) -> Result<File> {
    let file = open_at(dir, OsStr::new(path), libc::O_RDWR | libc::O_CREAT, 0o600)?;
    let s = stat(&file)?;
    if s.st_uid != unsafe { libc::geteuid() }
        || s.st_nlink != 1
        || s.st_mode & libc::S_IFMT != libc::S_IFREG
        || s.st_mode & 0o7777 != 0o600
    {
        return Err(Error("unsafe payload lock file".into()));
    }
    Ok(file)
}
fn entries(dir: &File, prefix: &str, out: &mut BTreeSet<String>) -> Result<()> {
    // openat(".") gives a separate directory offset, unlike dup().
    let scan = open_at(dir, OsStr::new("."), libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
    use std::os::fd::IntoRawFd;
    let fd = scan.into_raw_fd();
    let raw = unsafe { libc::fdopendir(fd) };
    if raw.is_null() {
        unsafe {
            libc::close(fd);
        }
        return Err(std::io::Error::last_os_error().into());
    }
    struct Dir(*mut libc::DIR);
    impl Drop for Dir {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    let guard = Dir(raw);
    loop {
        unsafe {
            *libc::__errno_location() = 0;
        }
        let entry = unsafe { libc::readdir(guard.0) };
        if entry.is_null() {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(0) {
                return Err(e.into());
            }
            break;
        }
        let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let part =
            std::str::from_utf8(bytes).map_err(|_| Error("non-UTF8 payload entry".into()))?;
        let path = format!("{prefix}{part}");
        if path.len() > 1024 || out.len() >= format::MAX_ENTRIES * 16 {
            return Err(Error("excessive cached payload entries".into()));
        }
        let child = open_at(dir, OsStr::new(part), libc::O_RDONLY, 0)?;
        let s = stat(&child)?;
        if s.st_mode & libc::S_IFMT == libc::S_IFDIR {
            private_directory(&child)?;
            out.insert(format!("{path}/"));
            entries(&child, &format!("{path}/"), out)?;
        } else {
            out.insert(path);
        }
    }
    Ok(())
}

/// Extract a validated archive or verify and lease an existing generation.
/// Cache corruption is an error; it is never repaired underneath running helpers.
pub fn extract(bytes: &[u8], cache_root: &Path) -> Result<PayloadLease> {
    let manifest = verify(bytes)?;
    let (_, data) = format::parse(bytes, "x86_64-unknown-linux-gnu").map_err(Error)?;
    let cache = root(cache_root)?;
    let publish = lock_file(&cache, ".publish.lock")?;
    lock(&publish, libc::LOCK_EX)?;
    let generation = format::hash(bytes);
    let existing = directory(&cache, OsStr::new(&generation), false);
    let dir = match existing {
        Ok(dir) => dir,
        Err(e) => {
            // Only ENOENT allows creation. Inspect the failed lookup without
            // following links; all other failures are fail-closed.
            let c = name(OsStr::new(&generation))?;
            let mut s = std::mem::MaybeUninit::<libc::stat>::uninit();
            let rc = unsafe {
                libc::fstatat(
                    cache.as_raw_fd(),
                    c.as_ptr(),
                    s.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if rc == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOENT) {
                return Err(e);
            }
            let mut random = [0u8; 16];
            File::open("/dev/urandom")?.read_exact(&mut random)?;
            let temporary = format!(".preparing-{}", format::hash(&random));
            // mkdir must be exclusive: never reuse someone else's partial tree.
            let temp = name(OsStr::new(&temporary))?;
            if unsafe { libc::mkdirat(cache.as_raw_fd(), temp.as_ptr(), 0o700) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let dir = directory(&cache, OsStr::new(&temporary), false)?;
            private_directory(&dir)?;
            for entry in &manifest.entries {
                let mut file = checked_file(&dir, &entry.path, entry.mode, true)?;
                file.write_all(&format::decode(entry, data).map_err(Error)?)?;
                file.sync_all()?;
            }
            lock_file(&dir, ".lease")?.sync_all()?;
            dir.sync_all()?;
            if unsafe {
                libc::renameat2(
                    cache.as_raw_fd(),
                    temp.as_ptr(),
                    cache.as_raw_fd(),
                    c.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            cache.sync_all()?;
            dir
        }
    };
    private_directory(&dir)?;
    let usage = lock_file(&dir, ".lease")?;
    lock(&usage, libc::LOCK_SH)?;
    let mut expected = BTreeSet::from([".lease".to_owned()]);
    for entry in &manifest.entries {
        expected.insert(entry.path.clone());
        for (i, _) in entry.path.match_indices('/') {
            expected.insert(entry.path[..=i].to_owned());
        }
        let mut file = checked_file(&dir, &entry.path, entry.mode, false)?;
        if file.metadata()?.len() != entry.size {
            return Err(Error(format!(
                "cached payload size mismatch: {}",
                entry.path
            )));
        }
        let mut bytes = Vec::new();
        (&mut file).take(entry.size + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != entry.size || format::hash(&bytes) != entry.sha256 {
            return Err(Error(format!(
                "cached payload digest mismatch: {}",
                entry.path
            )));
        }
    }
    let mut actual = BTreeSet::new();
    entries(&dir, "", &mut actual)?;
    if actual != expected {
        return Err(Error("unexpected cached payload entries".into()));
    }
    Ok(PayloadLease {
        path: cache_root.join(generation),
        _usage: usage,
        manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let mut random = [0; 16];
            File::open("/dev/urandom")
                .unwrap()
                .read_exact(&mut random)
                .unwrap();
            let path = PathBuf::from("/tmp")
                .join(format!("telorgon-payload-test-{}", format::hash(&random)));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn cold_and_warm_cache_share_immutable_generation() {
        let temp = Temp::new();
        let bytes = format::fixture();
        let a = extract(&bytes, &temp.0).unwrap();
        let b = extract(&bytes, &temp.0).unwrap();
        assert_eq!(a.path(), b.path());
        assert_eq!(
            std::fs::metadata(a.path().join("bin/Xwayland"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o500
        );
        let lease = File::open(a.path().join(".lease")).unwrap();
        assert!(lock(&lease, libc::LOCK_EX | libc::LOCK_NB).is_err());
        drop(a);
        drop(b);
        assert!(lock(&lease, libc::LOCK_EX | libc::LOCK_NB).is_ok());
    }
    #[test]
    fn rejects_symlink_cache_roots_and_generation_files() {
        let temp = Temp::new();
        let bytes = format::fixture();
        let alias = temp.0.join("alias");
        symlink(&temp.0, &alias).unwrap();
        assert!(extract(&bytes, &alias).is_err());
        let lease = extract(&bytes, &temp.0).unwrap();
        let binary = lease.path().join("bin/Xwayland");
        std::fs::remove_file(&binary).unwrap();
        symlink("/bin/true", &binary).unwrap();
        assert!(extract(&bytes, &temp.0).is_err());
    }
    #[test]
    fn rejects_corruption_hardlinks_and_unexpected_libraries() {
        for kind in [0, 1, 2] {
            let temp = Temp::new();
            let bytes = format::fixture();
            let lease = extract(&bytes, &temp.0).unwrap();
            let file = lease.path().join("licenses/test.txt");
            match kind {
                0 => {
                    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))
                        .unwrap();
                    std::fs::write(&file, b"unexpected cached bytes").unwrap();
                    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o400))
                        .unwrap();
                }
                1 => std::fs::hard_link(&file, temp.0.join("hardlink")).unwrap(),
                _ => std::fs::write(lease.path().join("injected.so"), b"unexpected").unwrap(),
            }
            assert!(extract(&bytes, &temp.0).is_err());
        }
    }
    #[test]
    fn concurrent_publication_has_one_generation() {
        let temp = Temp::new();
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| extract(&format::fixture(), &temp.0).unwrap()))
                .collect();
            let leases: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
            assert!(leases.iter().all(|lease| lease.path() == leases[0].path()));
        });
        assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 2);
    }

    #[test]
    #[cfg(feature = "shell-xwayland-embedded")]
    fn supplied_embedded_payload_extracts_and_revalidates() {
        let temp = Temp::new();
        let a = extract(embedded(), &temp.0).unwrap();
        let b = extract(embedded(), &temp.0).unwrap();
        assert_eq!(a.path(), b.path());
        assert_eq!(a.manifest().xwayland_version, "24.1.13");
        assert!(a.path().join("bin/Xwayland").is_file());
    }
}
