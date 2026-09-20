//! Process launching in the active managed GUI or desktop session.
//!
//! Builders are inert. Execution resolves the current session and never mutates the process-wide
//! environment. Futures use standard Rust wakers and do not require Tokio. Dropping a child handle
//! does not kill the child; its session continues to supervise and reap it.
//!
//! Run these operations from callbacks/tasks after the managed host has started:
//!
//! ```no_run
//! use telorgon::session;
//!
//! async fn launch() -> session::Result<()> {
//!     let terminal = session::command("foot")
//!         .restart(session::RestartPolicy::OnFailure)
//!         .recover(true)
//!         .spawn()?;
//!     let output = session::command("cat").input(b"hello\n".to_vec()).output().await?;
//!     assert!(output.status.success());
//!     let apps = session::application("org.example.Editor.desktop")
//!         .open_file("/home/user/notes.txt").launch().await?;
//!     for error in apps.errors { eprintln!("{error}"); }
//!     for entry in session::pending_recovery()? {
//!         // Present entries in the shell; restore only after the user's recovery action.
//!         let _ = entry.program();
//!     }
//!     Ok(())
//! }
//! ```

#[cfg(target_os = "linux")]
mod bus;
mod command;
mod desktop;
mod environment;
mod recovery;
mod supervisor;
#[cfg(test)]
mod tests;

pub use command::{Command, ManagedChild, ProcessOutput, RestartPolicy, Stream};
pub use desktop::{ApplicationLaunch, ApplicationRequest};
pub(crate) use environment::Environment;
pub use recovery::RecoveryEntry;
pub(crate) use supervisor::SessionOwner;
pub use supervisor::{SessionHandle, SessionPhase};

use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

static SESSION: Mutex<Option<SessionHandle>> = Mutex::new(None);

/// Configuration shared by session-owned launch services.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    /// Stable ID used for desktop identity and recovery storage, independent of display names.
    pub identity: String,
    /// Explicit terminal executable and arguments before the application command (e.g. foot -e).
    pub terminal: Vec<String>,
    /// Persist recoverable launches. No environment variables or arbitrary application memory are
    /// persisted. Files/URLs supplied as launch arguments may be stored in the private journal.
    pub recovery: bool,
    /// Override the recovery directory. By default uses XDG_STATE_HOME/telorgon/<identity>.
    pub recovery_directory: Option<PathBuf>,
    /// Time allowed for normal child exit. There is no automatic SIGKILL escalation.
    pub shutdown_timeout: Duration,
    /// DE only. Publish display variables to the shared D-Bus/systemd user environment. Enable
    /// only when this desktop owns the user's graphical session; defaults off for multi-session safety.
    pub publish_user_service_environment: bool,
}

impl SessionConfig {
    pub fn new(identity: impl Into<String>) -> Self {
        Self {
            identity: identity.into(),
            ..Self::default()
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.identity.is_empty()
            || self.identity.len() > 128
            || !self
                .identity
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || self.identity == "."
            || self.identity == ".."
            || self.shutdown_timeout.is_zero()
            || self.shutdown_timeout > Duration::from_secs(300)
        {
            return Err(Error::Invalid(
                "invalid session identity or shutdown timeout".into(),
            ));
        }
        Ok(())
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            identity: "telorgon".into(),
            terminal: Vec::new(),
            recovery: true,
            recovery_directory: None,
            shutdown_timeout: Duration::from_secs(30),
            publish_user_service_environment: false,
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "no managed session is ready; launch programs after Application::run initializes the host"
    )]
    NotReady,
    #[error("the session is shutting down")]
    Closing,
    #[error("another managed session is already running in this process")]
    AlreadyRunning,
    #[error("another process owns recovery storage for this session identity")]
    RecoveryInUse,
    #[error("session process limit reached")]
    ProcessLimit,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("session I/O: {0}")]
    Io(String),
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;

pub fn current() -> Result<SessionHandle> {
    SESSION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or(Error::NotReady)
}

pub fn command(program: impl AsRef<OsStr>) -> Command {
    Command::new(program)
}

/// Explicit shell interpretation; never use with untrusted shell text.
pub fn shell(script: impl AsRef<OsStr>) -> Command {
    #[cfg(windows)]
    {
        command("cmd.exe").args([OsStr::new("/C"), script.as_ref()])
    }
    #[cfg(not(windows))]
    {
        command("/bin/sh").args([OsStr::new("-c"), script.as_ref()])
    }
}

pub fn application(desktop_id: impl Into<String>) -> ApplicationRequest {
    ApplicationRequest::new(desktop_id.into())
}

/// Previous unclean-run launches, offered to the shell/application for user-selected recovery.
pub fn pending_recovery() -> Result<Vec<RecoveryEntry>> {
    current()?.pending_recovery()
}
