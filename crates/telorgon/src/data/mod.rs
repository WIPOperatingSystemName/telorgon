//! Typed, thread-safe settings with transactional updates and optional background persistence.
//! Values remain usable through existing handles after reload. Call `shutdown` to flush and stop
//! autosave explicitly; dropping a registry stops its worker without performing implicit I/O.
mod autosave;
mod error;
mod file;
mod format;
mod persistence;
mod registry;
mod subscription;
mod value;
mod variable;

pub use autosave::{Autosave, SaveStatus};
pub use error::{DataError, DataResult};
pub use persistence::{RestoreState, SaveState};
pub use registry::{Registry, Settings, Transaction};
pub use subscription::{ChangeBatch, ChangeOrigin, Subscription};
pub use value::{DataValue, EntrySpec};
pub use variable::Variable;

#[cfg(test)]
mod tests;
