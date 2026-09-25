use super::{ApplicationRegistry, Error, Result, SessionConfig};
use std::{path::PathBuf, time::Duration};

/// GUI launch-service options. Identity belongs to the application declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuiSessionConfig {
    pub applications: ApplicationRegistry,
    pub terminal: Vec<String>,
    pub recovery: bool,
    /// Exact recovery destination; must be absolute when recovery is enabled.
    pub recovery_directory: Option<PathBuf>,
    pub shutdown_timeout: Duration,
}

impl Default for GuiSessionConfig {
    fn default() -> Self {
        let config = SessionConfig::default();
        Self {
            applications: config.applications,
            terminal: config.terminal,
            recovery: config.recovery,
            recovery_directory: config.recovery_directory,
            shutdown_timeout: config.shutdown_timeout,
        }
    }
}

impl GuiSessionConfig {
    pub(crate) fn into_session(self, identity: String) -> SessionConfig {
        SessionConfig {
            identity,
            applications: self.applications,
            terminal: self.terminal,
            recovery: self.recovery,
            recovery_directory: self.recovery_directory,
            shutdown_timeout: self.shutdown_timeout,
            desktop_settings: None,
            publish_user_service_environment: false,
        }
    }
}

pub(crate) fn validate_identity(identity: &str) -> Result<()> {
    let stem = identity
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if identity.is_empty()
        || identity.len() > 128
        || !identity
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || !identity.as_bytes()[0].is_ascii_alphanumeric()
        || identity.ends_with('.')
        || reserved
    {
        return Err(Error::Invalid("invalid application identity: use 1-128 ASCII letters, digits, dots, underscores or hyphens, starting with a letter or digit; trailing dots and Windows device names are forbidden".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_identity_rejects_unsafe_portable_names() {
        for invalid in [
            "", ".", "..", "../app", "a/b", "a\\b", "CON", "nul.txt", "COM1", "lpt9.log", "app.",
            "app name",
        ] {
            assert!(validate_identity(invalid).is_err(), "{invalid}");
        }
        assert!(validate_identity(&"a".repeat(129)).is_err());
        assert!(validate_identity("org.example.settings").is_ok());
    }
}
