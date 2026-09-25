use super::{Error, Result};

#[cfg(target_os = "linux")]
mod dconf;

/// Desktop defaults for GTK clients. Higher-priority user preferences and settings services
/// still win. Linux uses the dconf compiler to prepare a session-local default database.
/// These defaults are private to the managed session, not written to ~/.config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopSettings {
    /// GTK's left:right button layout; supported names are menu, icon, minimize, maximize, close.
    pub decoration_layout: String,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            decoration_layout: "menu:minimize,maximize,close".into(),
        }
    }
}

impl DesktopSettings {
    pub(crate) fn validate(&self) -> Result<()> {
        let valid = self
            .decoration_layout
            .split_once(':')
            .is_some_and(|(left, right)| {
                [left, right].into_iter().all(|side| {
                    side.is_empty()
                        || side.split(',').all(|button| {
                            matches!(button, "menu" | "icon" | "minimize" | "maximize" | "close")
                        })
                })
            });
        if !valid || self.decoration_layout.len() > 256 {
            return Err(Error::Invalid(
                "invalid GTK decoration layout (expected left:right button lists)".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(super) use linux::{PreparedSettings, prepare};

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::services::session::{Environment, SessionConfig};
    use std::ffi::OsStr;
    use std::os::unix::fs::DirBuilderExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    pub(crate) struct PreparedSettings(PathBuf);

    pub(crate) fn prepare(
        mut env: Environment,
        config: &SessionConfig,
    ) -> Result<(Environment, Option<PreparedSettings>)> {
        let Some(settings) = &config.desktop_settings else {
            return Ok((env, None));
        };
        settings.validate()?;
        let runtime = env.runtime_directory()?;
        let directory = loop {
            let directory = runtime.join(format!(
                "telorgon-settings-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
                Ok(()) => break PreparedSettings(directory),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        };
        // Keep this guard alive across all fallible operations to clean up failed startup too.
        let gtk = directory.0.join("gtk-3.0");
        std::fs::create_dir(&gtk)?;
        std::fs::write(
            gtk.join("settings.ini"),
            format!(
                "[Settings]\ngtk-decoration-layout={}\n",
                settings.decoration_layout
            ),
        )?;
        let inherited = env.get("XDG_CONFIG_DIRS").unwrap_or(OsStr::new("/etc/xdg"));
        let paths = std::iter::once(directory.0.clone()).chain(
            // XDG config directory entries must be absolute; never introduce cwd dependencies.
            std::env::split_paths(inherited).filter(|path| path.is_absolute()),
        );
        let paths =
            std::env::join_paths(paths).map_err(|error| Error::Invalid(error.to_string()))?;
        env.0.insert("XDG_CONFIG_DIRS".into(), paths);
        dconf::prepare(&mut env, settings, &directory.0)?;
        Ok((env, Some(directory)))
    }

    impl Drop for PreparedSettings {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_dir_all(&self.0) {
                eprintln!(
                    "telorgon-session: could not remove desktop defaults {}: {error}",
                    self.0.display()
                );
            }
        }
    }
}
