//! Renderer- and platform-free ownership for one mounted application view.

mod binding;
mod component;
mod component_arena;
mod component_driver;
mod composition;
mod context;
mod diagnostics;
mod error;
mod input_route;
mod observer;
mod read;
mod read_arena;
mod routed_action;
mod scheduler;
mod state;
mod state_arena;
mod structure;
mod task;
mod task_host;
mod transaction;
mod view;

pub use component::{Component, ComponentId, LifecycleState, NoAction};
pub use component_driver::{ComponentDriver, ComponentRuntimeDriver};
pub use composition::{CompositionDiagnostics, CompositionDriver};
pub use context::{Command, CreateContext, Ui, UnmountContext, UpdateContext};
pub use diagnostics::ComponentDiagnostics;
pub use error::{RuntimeError, RuntimeResult};
pub use read::Read;
pub use scheduler::{FrameScheduler, MonotonicInstant, TimerHandle};
pub use state::State;
pub use structure::SwitchBranch;
pub use task::{LocalTaskSender, TaskHandle, TaskSendError, TaskSender};
pub use task_host::{LocalTask, SendTask, TaskCancellation, TaskHost, UnsupportedTaskHost};
pub use view::ViewRuntime;

#[cfg(feature = "instrumentation")]
pub mod instrumentation;
