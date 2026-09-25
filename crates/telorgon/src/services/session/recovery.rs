use super::command::LaunchSpec;
use super::{Command, Environment, Error, ManagedChild, Result, SessionConfig, SessionHandle};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

/// A recoverable launch from an unclean host run. This is launch metadata, not application memory.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RecoveryEntry {
    pub(crate) id: u64,
    pub(crate) spec: LaunchSpec,
    #[serde(default)]
    pub(crate) pid: u32,
    #[serde(default)]
    pub(crate) process_identity: Option<String>,
    /// Identifies the journal generation; stale UI entries cannot restore a different later run.
    #[serde(default)]
    pub(crate) generation: u64,
}

impl RecoveryEntry {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn program(&self) -> &std::ffi::OsStr {
        &self.spec.program
    }
    pub fn arguments(&self) -> &[std::ffi::OsString] {
        &self.spec.args
    }
    pub fn application_id(&self) -> Option<&str> {
        self.spec.application.as_deref()
    }
    /// Refuse duplicate relaunch while the original direct child is still alive.
    pub fn original_process_alive(&self) -> bool {
        self.process_identity
            .as_ref()
            .is_some_and(|identity| process_identity(self.pid).as_ref() == Some(identity))
    }
    pub fn restore(&self) -> Result<ManagedChild> {
        super::current()?.restore_entry(self)
    }
}

pub(crate) fn process_identity(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let tail = stat
            .rsplit_once(')')?
            .1
            .split_whitespace()
            .collect::<Vec<_>>();
        if tail.first() == Some(&"Z") {
            return None;
        }
        let start = tail.get(19)?;
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
        Some(format!("{}:{start}", boot.trim()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct JournalData {
    version: u32,
    entries: Vec<RecoveryEntry>,
}

pub(crate) struct Journal {
    directory: PathBuf,
    _lock: File,
}

fn recovery_directory(config: &SessionConfig, env: &Environment) -> Result<PathBuf> {
    if let Some(path) = &config.recovery_directory {
        return Ok(path.clone());
    }
    let absolute = |key| env.get(key).map(PathBuf::from).filter(|p| p.is_absolute());
    #[cfg(target_os = "windows")]
    let root = absolute("LOCALAPPDATA");
    #[cfg(target_os = "macos")]
    let root = absolute("HOME").map(|p| p.join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let root = absolute("XDG_STATE_HOME")
        .or_else(|| absolute("HOME").map(|p| p.join(".local/state")));
    let root = root.ok_or_else(|| Error::Invalid(
        "recovery requires an absolute platform state directory (LOCALAPPDATA on Windows, HOME on macOS, XDG_STATE_HOME or HOME on Unix) or an explicit recovery_directory".into()
    ))?;
    Ok(root.join(&config.identity).join("recovery"))
}

#[cfg(test)]
mod path_tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn recovery_path_uses_platform_root_and_exact_override() {
        let root = std::env::temp_dir();
        let mut env = Environment(BTreeMap::new());
        let mut config = SessionConfig::new("org.example.settings");
        assert!(recovery_directory(&config, &env).is_err());
        #[cfg(target_os = "windows")]
        let (key, suffix) = ("LOCALAPPDATA", "");
        #[cfg(target_os = "macos")]
        let (key, suffix) = ("HOME", "Library/Application Support");
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let (key, suffix) = ("XDG_STATE_HOME", "");
        env.0.insert(key.into(), root.clone().into_os_string());
        assert_eq!(
            recovery_directory(&config, &env).unwrap(),
            root.join(suffix).join("org.example.settings/recovery")
        );
        env.0.insert(key.into(), "relative/path".into());
        assert!(recovery_directory(&config, &env).is_err());
        config.recovery_directory = Some(root.join("custom"));
        assert_eq!(recovery_directory(&config, &env).unwrap(), root.join("custom"));
    }

    #[test]
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    fn relative_xdg_state_falls_back_to_absolute_home() {
        let root = std::env::temp_dir();
        let env = Environment(BTreeMap::from([
            ("XDG_STATE_HOME".into(), "relative".into()),
            ("HOME".into(), root.clone().into_os_string()),
        ]));
        assert_eq!(
            recovery_directory(&SessionConfig::new("org.example.settings"), &env).unwrap(),
            root.join(".local/state/org.example.settings/recovery")
        );
    }
}

impl Journal {
    pub fn open(
        config: &SessionConfig,
        env: &Environment,
    ) -> Result<Option<(Self, Vec<RecoveryEntry>)>> {
        if !config.recovery {
            return Ok(None);
        }
        let directory = recovery_directory(config, env)?;
        if !directory.is_absolute() {
            return Err(Error::Invalid("recovery_directory must be absolute".into()));
        }
        std::fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let metadata = std::fs::symlink_metadata(&directory)?;
            if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
                return Err(Error::Invalid(
                    "recovery directory must be owned by the current user and cannot be a symlink"
                        .into(),
                ));
            }
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        }
        let lock = private_options()
            .create(true)
            .read(true)
            .write(true)
            .open(directory.join("session.lock"))?;
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => Error::RecoveryInUse,
            std::fs::TryLockError::Error(error) => error.into(),
        })?;
        let journal = Self {
            directory,
            _lock: lock,
        };
        let entries = match private_options().read(true).open(journal.path()) {
            Ok(file) => {
                let mut data = String::new();
                file.take(1024 * 1024 + 1).read_to_string(&mut data)?;
                if data.len() > 1024 * 1024 {
                    return Err(Error::Invalid("recovery journal exceeds 1 MiB".into()));
                }
                let data: JournalData = toml::from_str(&data)
                    .map_err(|e| Error::Invalid(format!("invalid recovery journal: {e}")))?;
                // Journal size is bounded in bytes; recovery offers do not consume process slots.
                if data.version != 1 {
                    return Err(Error::Invalid(
                        "unsupported recovery journal version or size".into(),
                    ));
                }
                data.entries
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Some((journal, entries)))
    }

    fn path(&self) -> PathBuf {
        self.directory.join("session.toml")
    }

    pub fn write(&self, entries: Vec<RecoveryEntry>) -> Result<()> {
        let data = toml::to_string(&JournalData {
            version: 1,
            entries,
        })
        .map_err(|e| Error::Io(e.to_string()))?;
        if data.len() > 1024 * 1024 {
            return Err(Error::Invalid("recovery journal exceeds 1 MiB".into()));
        }
        let temporary = self.directory.join("session.tmp");
        let mut file = private_options()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(data.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(temporary, self.path())?;
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options
}

pub(crate) fn restored_command(entry: &RecoveryEntry, session: &SessionHandle) -> Command {
    let mut command = Command::new(&entry.spec.program);
    command.spec = entry.spec.clone();
    command.session = Some(session.clone());
    command
}
