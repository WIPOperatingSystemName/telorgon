use super::DesktopSettings;
use crate::services::session::{Environment, Error, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) fn prepare(
    env: &mut Environment,
    settings: &DesktopSettings,
    directory: &Path,
) -> Result<()> {
    let sources = directory.join("dconf-defaults.d");
    std::fs::create_dir(&sources)?;
    // GTK translates GSettings' appmenu token to its menu token. Validation limits the
    // remaining characters to button names and separators, so no GVariant escaping is needed.
    let layout = settings.decoration_layout.replace("menu", "appmenu");
    std::fs::write(
        sources.join("desktop"),
        format!("[org/gnome/desktop/wm/preferences]\nbutton-layout='{layout}'\n"),
    )?;
    let database = directory.join("dconf-defaults");
    let database_text = database
        .to_str()
        .filter(|path| !path.contains(['\n', '\r']))
        .ok_or_else(|| {
            Error::Invalid("desktop settings need a UTF-8 runtime path without newlines".into())
        })?;
    // Compile only a private database. Never run dconf update/write or alter the user's DB.
    let output = Command::new("dconf")
        .env_clear()
        .envs(&env.0)
        .arg("compile")
        .arg(&database)
        .arg(&sources)
        .output()
        .map_err(|error| {
            Error::Io(format!(
                "desktop settings require the dconf compiler: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(Error::Io(format!(
            "compiling desktop defaults failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let mut profile = inherited_profile(env)?;
    if !profile.ends_with('\n') {
        profile.push('\n');
    }
    // Preserve writable user DB, administrator defaults and locks with their existing
    // precedence. Our read-only session defaults are consulted only if none supplies a value.
    profile.push_str(&format!("file-db:{database_text}\n"));
    let path = directory.join("dconf-profile");
    std::fs::write(&path, profile)?;
    env.0.insert("DCONF_PROFILE".into(), path.into_os_string());
    Ok(())
}

fn inherited_profile(env: &Environment) -> Result<String> {
    let explicit = env.get("DCONF_PROFILE");
    let name = explicit
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("user"));
    let path = if name.is_absolute() {
        name
    } else {
        Path::new("/etc/dconf/profile").join(name)
    };
    match std::fs::read_to_string(&path) {
        Ok(profile) => Ok(profile),
        Err(error) if explicit.is_none() && error.kind() == std::io::ErrorKind::NotFound => {
            Ok("user-db:user\n".into())
        }
        Err(error) => Err(Error::Io(format!(
            "reading dconf profile {}: {error}",
            path.display()
        ))),
    }
}
