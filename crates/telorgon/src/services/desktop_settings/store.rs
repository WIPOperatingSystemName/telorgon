use super::*;
use crate::data::{EntrySpec, Registry};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct SettingsStore {
    identity: String,
    pub(crate) path: PathBuf,
}
impl SettingsStore {
    pub fn new(identity: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            identity: identity.into(),
            path: path.into(),
        }
    }
    pub fn for_identity(identity: &str) -> Result<Self> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .map(|p| p.join(".config"))
            })
            .ok_or("An absolute HOME or XDG_CONFIG_HOME is required")?;
        let store = Self::new(identity, config.join(identity).join("settings.toml"));
        store.validate()?;
        Ok(store)
    }
    fn validate(&self) -> Result<()> {
        if self.identity.is_empty()
            || self.identity.len() > 128
            || !self
                .identity
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || matches!(self.identity.as_str(), "." | "..")
            || self.path.file_name().is_none()
        {
            return Err("Invalid settings identity or path".into());
        }
        Ok(())
    }
    pub(crate) fn parent(&self) -> Result<&Path> {
        self.validate()?;
        Ok(self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")))
    }
    pub(crate) fn lock(&self) -> Result<File> {
        let parent = self.parent()?;
        private_directory(parent)?;
        let mut name = self.path.file_name().unwrap().to_os_string();
        name.push(".lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(parent.join(name))
            .map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err("Settings lock must be a private regular file".into());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            // flock is tied to this open file description; drop releases it, including on errors.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(file);
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EWOULDBLOCK)
                && error.kind() != std::io::ErrorKind::Interrupted
            {
                return Err(error.to_string());
            }
            if Instant::now() >= deadline {
                return Err("Desktop settings are busy".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn registry(&self) -> Result<Registry> {
        self.validate()?;
        let registry = Registry::new(&self.identity, 1);
        registry
            .register(
                EntrySpec::new("display", DisplayConfiguration::default())
                    .validate(DisplayConfiguration::validate),
            )
            .map_err(|e| e.to_string())?;
        registry
            .register(
                EntrySpec::new("sound", SoundSettings::default()).validate(SoundSettings::validate),
            )
            .map_err(|e| e.to_string())?;
        registry
            .register(
                EntrySpec::new("personalization", PersonalizationSettings::default())
                    .validate(PersonalizationSettings::validate),
            )
            .map_err(|e| e.to_string())?;
        match read_regular(&self.path, MAX_SETTINGS_BYTES) {
            Ok(bytes) => {
                let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
                registry.restore_toml(text).map_err(|e| e.to_string())?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Cannot read {}: {e}", self.path.display())),
        }
        Ok(registry)
    }
    pub fn load(&self) -> Result<Preferences> {
        // Atomic replacement permits a coherent read without creating files on first startup.
        let registry = self.registry()?;
        Ok(Preferences {
            display: registry
                .get::<DisplayConfiguration>("display")
                .and_then(|v| v.get())
                .map_err(|e| e.to_string())?,
            sound: registry
                .get::<SoundSettings>("sound")
                .and_then(|v| v.get())
                .map_err(|e| e.to_string())?,
            personalization: registry
                .get::<PersonalizationSettings>("personalization")
                .and_then(|v| v.get())
                .map_err(|e| e.to_string())?,
        })
    }
    fn update<T: crate::data::DataValue>(&self, key: &str, value: &T) -> Result<()> {
        let registry = self.registry()?;
        registry
            .get::<T>(key)
            .and_then(|v| v.set(value.clone()))
            .map_err(|e| e.to_string())?;
        registry.save(&self.path).map_err(|e| e.to_string())
    }
    pub fn save_display(&self, display: &DisplayConfiguration) -> Result<()> {
        display.validate()?;
        let _lock = self.lock()?;
        self.update("display", display)
    }
    pub fn save_sound(&self, sound: &SoundSettings) -> Result<()> {
        sound.validate()?;
        let _lock = self.lock()?;
        self.update("sound", sound)
    }
    pub fn load_personalization(&self) -> Result<PersonalizationSettings> {
        Ok(self.load()?.personalization)
    }
    pub fn save_personalization(&self, settings: &PersonalizationSettings) -> Result<()> {
        settings.validate()?;
        let _lock = self.lock()?;
        if settings.background.is_some() {
            let (bytes, _) = self.read_background(settings)?;
            super::backgrounds::decode_image(&bytes)?;
        }
        self.update("personalization", settings)
    }
    pub fn save_personalization_validated(&self, validated: &ValidatedBackground) -> Result<()> {
        let _lock = self.lock()?;
        validated.verify_locked(self)?;
        self.update("personalization", validated.settings())
    }
}

pub(crate) fn private_directory(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| e.to_string())?;
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .is_dir()
    {
        return Err("Settings directory must be a real directory".into());
    }
    Ok(())
}

pub(crate) fn read_regular(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(std::io::Error::other(
            "Expected a regular file within the size limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::other("File exceeds the size limit"));
    }
    Ok(bytes)
}
