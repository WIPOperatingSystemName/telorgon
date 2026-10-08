use super::state::BootModel;
use super::{
    BootControl, BootHostEvent, BootRequest, BootRequestId, BootResult, BootSnapshot, BootTarget,
    BootTheme,
};
use crate::authoring::compose::{Signal, SignalWriter};
use core::time::Duration;
use std::sync::{Arc, Mutex};

/// Shared UI control and host event channel. The owner supplies scheduling and execution.
#[derive(Clone)]
pub struct BootController {
    inner: Arc<ControllerState>,
}

struct ControllerState {
    model: Mutex<ControllerModel>,
    signal: Signal<BootSnapshot>,
    writer: SignalWriter<BootSnapshot>,
}

enum ControllerModel {
    Host(BootModel),
    #[cfg(feature = "boot-preview")]
    Preview(super::preview::model::PreviewModel),
}

impl ControllerModel {
    fn snapshot(&self) -> &BootSnapshot {
        match self {
            Self::Host(model) => &model.snapshot,
            #[cfg(feature = "boot-preview")]
            Self::Preview(model) => &model.snapshot,
        }
    }
}

impl BootController {
    pub(crate) fn host(targets: Vec<BootTarget>, selected: usize, theme: BootTheme) -> Self {
        Self::new(ControllerModel::Host(BootModel::new(
            targets, selected, theme,
        )))
    }

    pub(crate) fn splash(targets: Vec<BootTarget>, selected: usize, theme: BootTheme) -> Self {
        Self::new(ControllerModel::Host(BootModel::splash(
            targets, selected, theme,
        )))
    }

    #[cfg(feature = "boot-preview")]
    pub(crate) fn preview(model: super::preview::model::PreviewModel) -> Self {
        Self::new(ControllerModel::Preview(model))
    }

    fn new(model: ControllerModel) -> Self {
        let (signal, writer) = Signal::new(model.snapshot().clone());
        Self {
            inner: Arc::new(ControllerState {
                model: Mutex::new(model),
                signal,
                writer,
            }),
        }
    }

    pub fn signal(&self) -> Signal<BootSnapshot> {
        self.inner.signal.clone()
    }
    pub fn snapshot(&self) -> BootSnapshot {
        self.inner.signal.snapshot().value.as_ref().clone()
    }

    pub fn dispatch(&self, command: impl Into<BootControl>) {
        let command = command.into();
        self.update(|model| match (model, command) {
            (ControllerModel::Host(model), BootControl::Boot(command)) => model.dispatch(command),
            #[cfg(feature = "boot-preview")]
            (ControllerModel::Preview(model), BootControl::Boot(command)) => {
                model.dispatch(command.into())
            }
            #[cfg(feature = "boot-preview")]
            (ControllerModel::Preview(model), BootControl::Preview(command)) => {
                model.dispatch(command)
            }
            #[cfg(feature = "boot-preview")]
            (ControllerModel::Host(_), BootControl::Preview(_)) => {}
        });
    }

    /// Advances animation only in a real session. Preview mode also advances its simulation.
    pub fn advance(&self, delta: Duration) {
        self.update(|model| match model {
            ControllerModel::Host(model) => model.advance(delta),
            #[cfg(feature = "boot-preview")]
            ControllerModel::Preview(model) => model.tick(delta),
        });
    }

    /// Takes one execution request. Simulation controllers never emit these requests.
    pub fn take_request(&self) -> Option<BootRequest> {
        self.update(|model| match model {
            ControllerModel::Host(model) => model.take_request(),
            #[cfg(feature = "boot-preview")]
            ControllerModel::Preview(_) => None,
        })
    }

    pub fn active_request(&self) -> Option<BootRequestId> {
        self.update(|model| match model {
            ControllerModel::Host(model) => model.active_request(),
            #[cfg(feature = "boot-preview")]
            ControllerModel::Preview(_) => None,
        })
    }

    pub fn report(&self, event: BootHostEvent) -> BootResult<()> {
        self.update(|model| match model {
            ControllerModel::Host(model) => model.report(event),
            #[cfg(feature = "boot-preview")]
            ControllerModel::Preview(_) => Err(super::BootError::Host(
                "preview sessions do not accept real boot host events".into(),
            )),
        })
    }

    fn update<R>(&self, update: impl FnOnce(&mut ControllerModel) -> R) -> R {
        let (result, notification) = {
            let mut model = self.inner.model.lock().expect("boot model lock poisoned");
            let previous = model.snapshot().clone();
            let result = update(&mut model);
            let notification = if model.snapshot() != &previous {
                let (_, notify) = self.inner.writer.publish_deferred(model.snapshot().clone());
                Some(notify)
            } else {
                None
            };
            (result, notification)
        };
        if let Some(notify) = notification {
            notify();
        }
        result
    }
}

impl PartialEq for BootController {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}
impl Eq for BootController {}

pub type PreviewController = BootController;
