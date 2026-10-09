//! Shared desktop preferences and bounded IPC. Enabling this module starts no services.
//! Blocking storage, image decoding, and bus calls must run outside the UI/renderer thread.
mod audio;
mod backgrounds;
mod client;
pub mod protocol;
mod store;
mod types;

pub use super::display::{DisplayConfiguration, DisplayMode, DisplaySnapshot};
pub use audio::*;
pub use backgrounds::{BackgroundEntry, PreparedBackgroundThumbnail, ValidatedBackground};
pub use client::{SettingsClient, SettingsEndpoint};
pub use store::SettingsStore;
pub use types::*;

// Callers report operation failures directly in their existing status models.
pub type Result<T> = std::result::Result<T, String>;

#[cfg(test)]
mod tests;
