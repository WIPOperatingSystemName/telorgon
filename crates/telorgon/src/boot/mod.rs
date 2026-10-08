//! Shared boot selection and splash views with host-owned execution and scheduling.

extern crate alloc;

pub mod artwork;
#[cfg(feature = "boot-config")]
pub mod config;
mod controller;
pub mod handoff;
#[cfg(test)]
mod host_tests;
mod model;
#[cfg(feature = "boot-preview")]
mod preview;
mod state;
#[cfg(all(test, feature = "boot-preview"))]
mod tests;
mod ui;
mod ui_assets;

pub use controller::{BootController, PreviewController};
pub use handoff::{
    SPLASH_HANDOFF_MAGIC, SPLASH_HANDOFF_SIZE, SPLASH_THEME_DISKS, SPLASH_THEME_VOXEL,
    SplashFramebuffer, SplashHandoff, SplashHandoffError,
};
pub use model::{
    BootCommand, BootControl, BootHostEvent, BootPhase, BootProgress, BootRequest, BootRequestId,
    BootSelectionMode, BootSnapshot, BootSource, BootTarget, BootTargetKind, BootTheme, BootVolume,
    PreviewCommand, PreviewPhase, PreviewScenario, PreviewSnapshot, PreviewTheme, TargetId,
};
pub use ui::BootInterface;
#[cfg(feature = "boot-preview")]
pub type BootPreview = BootInterface;

use crate::authoring::compose::Component;
use alloc::{collections::BTreeSet, format, string::String, vec::Vec};

pub type BootResult<T> = Result<T, BootError>;

#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error("invalid boot configuration: {0}")]
    Configuration(String),
    #[error("boot host error: {0}")]
    Host(String),
    #[cfg(feature = "boot-preview")]
    #[error(transparent)]
    Application(#[from] crate::host::application::AppError),
    #[cfg(feature = "boot-preview")]
    #[error("could not start the preview clock: {0}")]
    Clock(#[from] std::io::Error),
}

/// Creates the same controller-backed component for a native boot host or a preview.
pub trait BootScreens {
    type Component: Component;
    fn create(self, controller: BootController) -> Self::Component;
}

impl<C, F> BootScreens for F
where
    C: Component,
    F: FnOnce(BootController) -> C,
{
    type Component = C;
    fn create(self, controller: BootController) -> C {
        self(controller)
    }
}

#[derive(Default)]
pub struct DefaultBootScreens;

impl BootScreens for DefaultBootScreens {
    type Component = BootInterface;
    fn create(self, controller: BootController) -> BootInterface {
        BootInterface::new(controller)
    }
}

pub struct BootApplication<S = DefaultBootScreens> {
    targets: Vec<BootTarget>,
    default_target: Option<String>,
    theme: BootTheme,
    selection_mode: BootSelectionMode,
    screens: S,
}

impl Default for BootApplication<DefaultBootScreens> {
    fn default() -> Self {
        Self::new()
    }
}

impl BootApplication<DefaultBootScreens> {
    pub fn new() -> Self {
        Self {
            targets: Vec::new(),
            default_target: None,
            theme: BootTheme::Disks,
            selection_mode: BootSelectionMode::Auto,
            screens: DefaultBootScreens,
        }
    }
}

impl<S: BootScreens> BootApplication<S> {
    pub fn target(mut self, target: BootTarget) -> Self {
        self.targets.push(target);
        self
    }

    pub fn default_target(mut self, id: &str) -> Self {
        self.default_target = Some(id.into());
        self
    }

    pub fn theme(mut self, theme: BootTheme) -> Self {
        self.theme = theme;
        self
    }

    pub fn selection_mode(mut self, selection_mode: BootSelectionMode) -> Self {
        self.selection_mode = selection_mode;
        self
    }

    pub fn screens<T: BootScreens>(self, screens: T) -> BootApplication<T> {
        BootApplication {
            targets: self.targets,
            default_target: self.default_target,
            theme: self.theme,
            selection_mode: self.selection_mode,
            screens,
        }
    }

    pub fn build(self) -> BootResult<ReadyBootApplication<S>> {
        if self.targets.is_empty() || self.targets.len() > 16 {
            return Err(BootError::Configuration(
                "choose between 1 and 16 targets".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        for target in &self.targets {
            BootTarget::new(target.id.as_str(), &target.name, target.kind)?;
            if let Some(source) = &target.source {
                source.validate()?;
            }
            if !ids.insert(target.id.clone()) {
                return Err(BootError::Configuration(format!(
                    "duplicate target {}",
                    target.id.as_str()
                )));
            }
            if target.detail.len() > 160 || target.detail.chars().any(char::is_control) {
                return Err(BootError::Configuration(
                    "target detail must be at most 160 bytes without control characters".into(),
                ));
            }
        }
        let selected = match self.default_target {
            Some(id) => self
                .targets
                .iter()
                .position(|target| target.id.as_str() == id)
                .ok_or_else(|| {
                    BootError::Configuration(format!("default target {id} does not exist"))
                })?,
            None => 0,
        };
        Ok(ReadyBootApplication {
            targets: self.targets,
            selected,
            theme: self.theme,
            selection_mode: self.selection_mode,
            screens: self.screens,
        })
    }
}

pub struct ReadyBootApplication<S = DefaultBootScreens> {
    targets: Vec<BootTarget>,
    selected: usize,
    theme: BootTheme,
    selection_mode: BootSelectionMode,
    screens: S,
}

impl<S: BootScreens> ReadyBootApplication<S> {
    pub fn auto_boots(&self) -> bool {
        self.selection_mode == BootSelectionMode::Auto && self.targets.len() == 1
    }

    /// Starts an OS-owned splash without emitting an EFI launch request.
    pub fn into_splash_session(self, target: &str) -> BootResult<BootSession<S::Component>> {
        let selected = self
            .targets
            .iter()
            .position(|entry| entry.id.as_str() == target)
            .ok_or_else(|| {
                BootError::Configuration(format!("splash target {target} does not exist"))
            })?;
        let controller = BootController::splash(self.targets, selected, self.theme);
        let component = self.screens.create(controller.clone());
        Ok(BootSession {
            controller,
            component,
        })
    }

    pub fn into_session(self) -> BootResult<BootSession<S::Component>> {
        let auto_boot = self.auto_boots();
        self.into_host_session(auto_boot)
    }

    /// Opens the selector for this session, including a single-target recovery session.
    pub fn into_selector_session(self) -> BootResult<BootSession<S::Component>> {
        self.into_host_session(false)
    }

    fn into_host_session(self, auto_boot: bool) -> BootResult<BootSession<S::Component>> {
        for target in &self.targets {
            if target.source.is_none() {
                return Err(BootError::Configuration(format!(
                    "target {} needs a boot source for a native session",
                    target.id.as_str()
                )));
            }
        }
        let controller = BootController::host(self.targets, self.selected, self.theme);
        if auto_boot {
            controller.dispatch(BootCommand::Launch);
        }
        let component = self.screens.create(controller.clone());
        Ok(BootSession {
            controller,
            component,
        })
    }

    #[cfg(feature = "boot-preview")]
    pub(crate) fn preview_startup_scenario(&self, scenario: PreviewScenario) -> PreviewScenario {
        if scenario == PreviewScenario::Startup && !self.auto_boots() {
            PreviewScenario::Selecting
        } else {
            scenario
        }
    }

    #[cfg(feature = "boot-preview")]
    pub fn preview(self, scenario: PreviewScenario) -> BootResult<()> {
        preview::run(self, scenario)
    }

    /// Renders the same component without opening a window or starting a clock thread.
    #[cfg(feature = "boot-preview")]
    pub fn render_preview(
        self,
        scenario: PreviewScenario,
        extent: crate::foundation::SizeI,
    ) -> BootResult<crate::graphics::render::ReadbackImage> {
        if extent.width <= 0 || extent.height <= 0 || extent.width > 4096 || extent.height > 4096 {
            return Err(BootError::Configuration(
                "preview dimensions must be 1..=4096".into(),
            ));
        }
        let scenario = self.preview_startup_scenario(scenario);
        let controller = preview::controller(self.targets, self.selected, self.theme, scenario);
        let component = self.screens.create(controller);
        Ok(crate::host::application::HeadlessRuntime::default()
            .run_composed_once(component, extent)?)
    }
}

pub struct BootSession<C: Component> {
    pub controller: BootController,
    pub component: C,
}

impl<C: Component> BootSession<C> {
    pub fn active_request(&self) -> Option<BootRequestId> {
        self.controller.active_request()
    }
}
