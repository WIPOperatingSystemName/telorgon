use super::{Error, Result};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct Environment(pub BTreeMap<OsString, OsString>);

impl Environment {
    pub(crate) fn inherited() -> Self {
        Self(std::env::vars_os().collect())
    }

    pub(crate) fn gui() -> Result<Self> {
        let mut env = Self::inherited();
        #[cfg(target_os = "linux")]
        {
            if env.get("DISPLAY").is_none() && env.get("WAYLAND_DISPLAY").is_none() {
                return Err(Error::Invalid("GUI startup needs an existing graphical session (DISPLAY or WAYLAND_DISPLAY); a bare TTY requires Application::shell_environment".into()));
            }
            if let Some(display) = env.get("WAYLAND_DISPLAY") {
                if !Path::new(display).is_absolute() {
                    // The native GUI backend also reads XDG_RUNTIME_DIR. Do not claim that fixing
                    // only child environments repairs its own display connection.
                    let runtime = env.get("XDG_RUNTIME_DIR").ok_or_else(|| Error::Invalid(
                        "the existing Wayland GUI session is missing XDG_RUNTIME_DIR; fix its login/session launcher".into()))?;
                    validate_runtime(Path::new(runtime))?;
                }
            }
        }
        // FD-based client connections and activation tokens are single-use, not inheritable state.
        env.clear_transient();
        Ok(env)
    }

    pub(crate) fn clear_transient(&mut self) {
        for key in [
            "WAYLAND_SOCKET",
            "XDG_ACTIVATION_TOKEN",
            "DESKTOP_STARTUP_ID",
        ] {
            self.0.remove(OsStr::new(key));
        }
    }

    pub(crate) fn desktop(&self, runtime: &Path, socket: &str, identity: &str) -> Self {
        let mut env = self.clone();
        env.clear_transient();
        env.0.remove(OsStr::new("DISPLAY"));
        env.0.remove(OsStr::new("XAUTHORITY"));
        env.0
            .insert("XDG_RUNTIME_DIR".into(), runtime.as_os_str().into());
        for (key, value) in [
            ("WAYLAND_DISPLAY", socket),
            ("XDG_SESSION_TYPE", "wayland"),
            ("XDG_CURRENT_DESKTOP", identity),
            ("XDG_SESSION_DESKTOP", identity),
        ] {
            env.0.insert(key.into(), value.into());
        }
        // Use an existing user bus only. Never start a second bus behind the user's back.
        if env.get("DBUS_SESSION_BUS_ADDRESS").is_none() && runtime.join("bus").exists() {
            // Percent-escape the D-Bus address path rather than interpolating arbitrary text.
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                let path = runtime.join("bus");
                let encoded: String = path
                    .as_os_str()
                    .as_bytes()
                    .iter()
                    .map(|b| {
                        if b.is_ascii_alphanumeric() || b"/-_.".contains(b) {
                            char::from(*b).to_string()
                        } else {
                            format!("%{b:02x}")
                        }
                    })
                    .collect();
                env.0.insert(
                    "DBUS_SESSION_BUS_ADDRESS".into(),
                    format!("unix:path={encoded}").into(),
                );
            }
        }
        env
    }

    pub(crate) fn get(&self, key: &str) -> Option<&OsStr> {
        self.0
            .get(OsStr::new(key))
            .filter(|value| !value.is_empty())
            .map(OsString::as_os_str)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn runtime_directory(&self) -> Result<PathBuf> {
        let path = self
            .get("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                // SAFETY: geteuid has no preconditions and does not mutate process state.
                PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() }))
            });
        validate_runtime(&path)?;
        Ok(path)
    }
}

#[cfg(target_os = "linux")]
fn validate_runtime(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).map_err(|e| {
        Error::Invalid(format!(
            "runtime directory {} is unavailable ({e}); start from a normal local PAM login",
            path.display()
        ))
    })?;
    if !path.is_absolute()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(Error::Invalid("XDG_RUNTIME_DIR must be an absolute directory owned by the current user with permissions 0700".into()));
    }
    Ok(())
}
