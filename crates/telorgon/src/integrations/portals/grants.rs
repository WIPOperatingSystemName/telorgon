//! Durable, bounded grant ownership. All methods perform file I/O and belong on the
//! portal/control worker, never the compositor or realtime thread. A failed write faults
//! this owner closed; callers must not report durable issuance/revocation on an error.
use super::restore::{RestoreData, RestoreSource};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
const MAX_GRANTS: usize = 128;
const MAX_BYTES: u64 = 1024 * 1024;
const MAGIC: &[u8; 8] = b"TLGRNT01";

pub(super) struct GrantStore {
    directory: File,
    _lock: File,
    entries: BTreeMap<String, RestoreData>,
    faulted: bool,
}
impl GrantStore {
    /// The caller supplies an existing private directory. This never creates user/global
    /// directories. An exclusive lock rejects a second owner instead of losing revocations.
    pub fn open(directory: &Path) -> io::Result<Self> {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory)?;
        check_private(&directory, true)?;
        let root = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root.join("lock"))?;
        check_private(&lock, false)?;
        // SAFETY: lock owns this live descriptor for the lifetime of this store.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let entries = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root.join("grants"))
        {
            Ok(file) => {
                check_private(&file, false)?;
                if file.metadata()?.len() > MAX_BYTES {
                    return Err(invalid());
                }
                let mut bytes = Vec::new();
                file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
                if bytes.len() as u64 > MAX_BYTES {
                    return Err(invalid());
                }
                decode(&bytes)?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            directory,
            _lock: lock,
            entries,
            faulted: false,
        })
    }
    /// Called only after explicit persistence consent and authoritative source resolution.
    /// Mode-1 application-lifetime grants must live in a separate ephemeral owner.
    pub fn issue(&mut self, app_id: &str, sources: Vec<RestoreSource>) -> io::Result<RestoreData> {
        if self.faulted || self.entries.len() >= MAX_GRANTS {
            return Err(invalid());
        }
        let record = RestoreData {
            app_id: app_id.into(),
            grant_id: random_id()?,
            sources,
        };
        if !record.valid_for(app_id, 7, true) || self.entries.contains_key(&record.grant_id) {
            return Err(invalid());
        }
        let mut next = self.entries.clone();
        next.insert(record.grant_id.clone(), record.clone());
        self.commit(next)?;
        Ok(record)
    }
    /// The untrusted hint must exactly match the stored grant and current request. Host
    /// source existence/identity checks are additional requirements, not replaced here.
    pub fn permits(&self, hint: &RestoreData, app_id: &str, types: u32, multiple: bool) -> bool {
        !self.faulted
            && hint.valid_for(app_id, types, multiple)
            && self.entries.get(&hint.grant_id) == Some(hint)
    }
    pub fn revoke(&mut self, app_id: &str, grant_id: &str) -> io::Result<bool> {
        if self.faulted {
            return Err(invalid());
        }
        if !self
            .entries
            .get(grant_id)
            .is_some_and(|g| g.app_id == app_id)
        {
            return Ok(false);
        }
        let mut next = self.entries.clone();
        next.remove(grant_id);
        self.commit(next)?;
        Ok(true)
    }
    pub fn applications(&self) -> io::Result<Vec<(String, usize)>> {
        if self.faulted {
            return Err(invalid());
        }
        let mut counts = BTreeMap::new();
        for grant in self.entries.values() {
            *counts.entry(grant.app_id.clone()).or_insert(0) += 1;
        }
        Ok(counts.into_iter().collect())
    }
    pub fn revoke_application(&mut self, app_id: &str) -> io::Result<usize> {
        if self.faulted {
            return Err(invalid());
        }
        let mut next = self.entries.clone();
        next.retain(|_, g| g.app_id != app_id);
        let count = self.entries.len() - next.len();
        if count != 0 {
            self.commit(next)?;
        }
        Ok(count)
    }
    fn commit(&mut self, next: BTreeMap<String, RestoreData>) -> io::Result<()> {
        let outcome = self.write(&next);
        match outcome {
            Ok(()) => {
                self.entries = next;
                Ok(())
            }
            Err(error) => {
                self.faulted = true;
                self.entries.clear();
                Err(error)
            }
        }
    }
    fn write(&self, entries: &BTreeMap<String, RestoreData>) -> io::Result<()> {
        let root = PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd()));
        let temporary = root.join(format!("pending-{}", random_id()?));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&temporary)?;
            file.write_all(&encode(entries)?)?;
            file.sync_all()?;
            std::fs::rename(&temporary, root.join("grants"))?;
            self.directory.sync_all()
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
}
fn check_private(file: &File, directory: bool) -> io::Result<()> {
    let meta = file.metadata()?;
    // SAFETY: geteuid has no preconditions and does not alter process state.
    if meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || if directory {
            !meta.is_dir()
        } else {
            !meta.is_file() || meta.nlink() != 1
        }
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "grant storage must be private and owned",
        ));
    }
    Ok(())
}
pub(super) fn random_id() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    let mut offset = 0;
    while offset < bytes.len() {
        // SAFETY: the remaining mutable slice is valid for the supplied length; libc writes
        // at most that length. No descriptor or native pointer escapes this function.
        let count = unsafe {
            libc::getrandom(bytes[offset..].as_mut_ptr().cast(), bytes.len() - offset, 0)
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "random source",
            ));
        }
        offset += count as usize;
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid or unavailable capture grant store",
    )
}
fn encode(entries: &BTreeMap<String, RestoreData>) -> io::Result<Vec<u8>> {
    if entries.len() > MAX_GRANTS {
        return Err(invalid());
    }
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let text = |bytes: &mut Vec<u8>, value: &str| {
        bytes.extend_from_slice(&(value.len() as u16).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    };
    for grant in entries.values() {
        if !grant.valid_for(&grant.app_id, 7, true) {
            return Err(invalid());
        }
        text(&mut bytes, &grant.app_id);
        text(&mut bytes, &grant.grant_id);
        bytes.push(grant.sources.len() as u8);
        for source in &grant.sources {
            bytes.push(source.kind as u8);
            text(&mut bytes, &source.identity);
        }
    }
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid());
    }
    Ok(bytes)
}
fn decode(mut bytes: &[u8]) -> io::Result<BTreeMap<String, RestoreData>> {
    fn take<'a>(bytes: &mut &'a [u8], count: usize) -> io::Result<&'a [u8]> {
        if bytes.len() < count {
            return Err(invalid());
        }
        let (value, rest) = bytes.split_at(count);
        *bytes = rest;
        Ok(value)
    }
    fn number(bytes: &mut &[u8]) -> io::Result<usize> {
        Ok(u16::from_le_bytes(take(bytes, 2)?.try_into().unwrap()) as usize)
    }
    fn text(bytes: &mut &[u8], limit: usize) -> io::Result<String> {
        let count = number(bytes)?;
        if count > limit {
            return Err(invalid());
        }
        String::from_utf8(take(bytes, count)?.to_vec()).map_err(|_| invalid())
    }
    if take(&mut bytes, 8)? != MAGIC {
        return Err(invalid());
    }
    let count = number(&mut bytes)?;
    if count > MAX_GRANTS {
        return Err(invalid());
    }
    let mut entries = BTreeMap::new();
    for _ in 0..count {
        let app_id = text(&mut bytes, 512)?;
        let grant_id = text(&mut bytes, 64)?;
        let count = take(&mut bytes, 1)?[0] as usize;
        if count == 0 || count > super::MAX_STREAMS {
            return Err(invalid());
        }
        let mut sources = Vec::with_capacity(count);
        for _ in 0..count {
            sources.push(RestoreSource {
                kind: take(&mut bytes, 1)?[0] as u32,
                identity: text(&mut bytes, 512)?,
            });
        }
        let record = RestoreData {
            app_id,
            grant_id,
            sources,
        };
        if !record.valid_for(&record.app_id, 7, true)
            || entries.insert(record.grant_id.clone(), record).is_some()
        {
            return Err(invalid());
        }
    }
    if !bytes.is_empty() {
        return Err(invalid());
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("telorgon-grants-{}", random_id().unwrap()));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn sources() -> Vec<RestoreSource> {
        vec![RestoreSource {
            kind: 1,
            identity: "host-display-identity".into(),
        }]
    }
    #[test]
    fn durable_grants_bind_app_sources_and_revoke_across_reopen() {
        let dir = Directory::new();
        let mut store = GrantStore::open(&dir.0).unwrap();
        let a = store.issue("app.a", sources()).unwrap();
        let b = store.issue("app.b", sources()).unwrap();
        assert_ne!(a.grant_id, b.grant_id);
        assert!(store.permits(&a, "app.a", 1, false));
        assert!(!store.permits(&a, "app.b", 1, false));
        let mut forged = a.clone();
        forged.sources[0].identity = "different".into();
        assert!(!store.permits(&forged, "app.a", 1, false));
        assert!(!store.revoke("app.b", &a.grant_id).unwrap());
        drop(store);
        let mut store = GrantStore::open(&dir.0).unwrap();
        assert!(store.permits(&a, "app.a", 1, false));
        assert!(store.revoke("app.a", &a.grant_id).unwrap());
        drop(store);
        let mut store = GrantStore::open(&dir.0).unwrap();
        assert!(!store.permits(&a, "app.a", 1, false));
        assert!(store.permits(&b, "app.b", 1, false));
        assert_eq!(store.revoke_application("app.b").unwrap(), 1);
    }
    #[test]
    fn unsafe_storage_concurrent_owners_and_corruption_are_rejected() {
        let dir = Directory::new();
        let store = GrantStore::open(&dir.0).unwrap();
        assert!(GrantStore::open(&dir.0).is_err());
        drop(store);
        std::os::unix::fs::symlink("lock", dir.0.join("grants")).unwrap();
        assert!(GrantStore::open(&dir.0).is_err());
        std::fs::remove_file(dir.0.join("grants")).unwrap();
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(dir.0.join("grants"))
            .unwrap();
        file.set_len(MAX_BYTES + 1).unwrap();
        drop(file);
        assert!(GrantStore::open(&dir.0).is_err());
        std::fs::remove_file(dir.0.join("grants")).unwrap();
        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(GrantStore::open(&dir.0).is_err());
    }
    #[test]
    fn failed_commit_faults_the_owner_and_cannot_claim_revocation() {
        let dir = Directory::new();
        let mut store = GrantStore::open(&dir.0).unwrap();
        let grant = store.issue("app", sources()).unwrap();
        std::fs::remove_file(dir.0.join("grants")).unwrap();
        std::fs::create_dir(dir.0.join("grants")).unwrap();
        assert!(store.revoke("app", &grant.grant_id).is_err());
        assert!(!store.permits(&grant, "app", 1, false));
        assert!(store.issue("app", sources()).is_err());
    }
    #[test]
    fn directory_descriptor_pins_storage_when_original_path_is_replaced() {
        let dir = Directory::new();
        let mut store = GrantStore::open(&dir.0).unwrap();
        let moved = dir.0.with_extension("moved");
        std::fs::rename(&dir.0, &moved).unwrap();
        let moved = Directory(moved);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir.0)
            .unwrap();
        let grant = store.issue("app", sources()).unwrap();
        assert!(!dir.0.join("grants").exists());
        assert!(moved.0.join("grants").exists());
        drop(store);
        assert!(
            GrantStore::open(&moved.0)
                .unwrap()
                .permits(&grant, "app", 1, false)
        );
    }

    #[test]
    fn full_store_rejects_issuance_without_evicting_existing_grants() {
        let dir = Directory::new();
        let mut store = GrantStore::open(&dir.0).unwrap();
        let entries = (0..MAX_GRANTS)
            .map(|index| {
                let grant = RestoreData {
                    app_id: "app".into(),
                    grant_id: format!("{index:064x}"),
                    sources: sources(),
                };
                (grant.grant_id.clone(), grant)
            })
            .collect();
        store.commit(entries).unwrap();
        let old = store.entries.values().next().unwrap().clone();
        assert!(store.issue("app", sources()).is_err());
        assert!(store.permits(&old, "app", 1, false));
        drop(store);
        assert!(
            GrantStore::open(&dir.0)
                .unwrap()
                .permits(&old, "app", 1, false)
        );
    }

    #[test]
    fn binary_decoder_rejects_truncation_trailing_data_and_unbounded_counts() {
        let dir = Directory::new();
        let mut store = GrantStore::open(&dir.0).unwrap();
        store.issue("app", sources()).unwrap();
        let encoded = encode(&store.entries).unwrap();
        for length in 0..encoded.len() {
            assert!(decode(&encoded[..length]).is_err());
        }
        let mut extra = encoded;
        extra.push(0);
        assert!(decode(&extra).is_err());
        let mut huge = MAGIC.to_vec();
        huge.extend_from_slice(&u16::MAX.to_le_bytes());
        assert!(decode(&huge).is_err());
    }
}
