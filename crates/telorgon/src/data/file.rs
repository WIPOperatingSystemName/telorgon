use super::{registry::Core, *};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(crate) fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut attempts = 0;
    let (temporary, mut file) = loop {
        let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let name = format!(".telorgon-settings-{}-{number}.tmp", std::process::id());
        let temporary = parent.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temporary) {
            Ok(file) => break (Temporary(temporary), file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempts < 100 => {
                attempts += 1;
            }
            Err(e) => return Err(e),
        }
    };
    file.write_all(bytes)?;
    if let Ok(metadata) = fs::metadata(path) {
        file.set_permissions(metadata.permissions())?;
    }
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary.0, path)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

impl Core {
    pub(crate) fn save_file(&self, path: &Path) -> DataResult<()> {
        self.save_file_inner(path, false)
    }
    pub(crate) fn save_file_inner(&self, path: &Path, pending_only: bool) -> DataResult<()> {
        // Acquire before snapshot: a delayed older save must never overwrite a newer snapshot.
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|source| DataError::Io {
                    path: path.into(),
                    source,
                })?
                .join(path)
        };
        let _io = self.io.lock().map_err(|_| DataError::Poisoned)?;
        {
            let mut state = self.lock()?;
            if pending_only && (!state.dirty() || state.stop) {
                return Ok(());
            }
            state.saving = true;
        }
        let result: DataResult<u64> = (|| {
            let (revision, text) = self.serialize()?;
            replace(path, text.as_bytes()).map_err(|source| DataError::Io {
                path: path.into(),
                source,
            })?;
            Ok(revision)
        })();
        let mut state = self.lock()?;
        state.saving = false;
        match &result {
            Ok(revision) => {
                // Exporting to another file must not acknowledge the autosave destination.
                if state.autosave_path.as_ref().is_none_or(|p| p == &absolute) {
                    state.saved_revision = state.saved_revision.max(*revision);
                }
                state.last_error = None;
                if !state.dirty() {
                    state.first_change = None;
                    state.last_change = None;
                }
            }
            Err(error) => {
                state.last_error = Some(error.to_string());
            }
        }
        drop(state);
        self.wake.notify_all();
        result.map(|_| ())
    }
}
impl Registry {
    pub fn save(&self, path: impl AsRef<Path>) -> DataResult<()> {
        self.core.save_file(path.as_ref())
    }
}
