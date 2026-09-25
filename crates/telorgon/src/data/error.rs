use std::path::PathBuf;

pub type DataResult<T> = Result<T, DataError>;

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("invalid empty registry identity or entry key")]
    EmptyName,
    #[error("setting already registered: {0}")]
    Duplicate(String),
    #[error("setting not registered: {0}")]
    Missing(String),
    #[error("incorrect value type for setting: {0}")]
    TypeMismatch(String),
    #[error("setting {key}: {message}")]
    Value { key: String, message: String },
    #[error("invalid settings document: {0}")]
    Format(String),
    #[error("settings document belongs to a different registry or schema version")]
    SchemaMismatch,
    #[error("registry changed during this operation; retry with a fresh snapshot")]
    Conflict,
    #[error("variable belongs to a different registry")]
    WrongRegistry,
    #[error("settings lock was poisoned")]
    Poisoned,
    #[error("file operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("autosave is already enabled")]
    AutosaveEnabled,
    #[error("autosave is not enabled")]
    AutosaveDisabled,
    #[error("autosave delays must be positive and no longer than one day")]
    InvalidDelay,
    #[error("autosave worker panicked")]
    WorkerPanicked,
    #[error("global settings have not been installed")]
    NotInitialized,
    #[error("global settings are already installed")]
    AlreadyInitialized,
}
