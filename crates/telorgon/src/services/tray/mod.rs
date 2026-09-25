//! Session tray publishing and observation. Owners stop workers; handles never keep them alive.
mod model;
pub use crate::integrations::status_notifier::{TrayHandle, TrayHost, TrayIcon};
pub use model::*;
