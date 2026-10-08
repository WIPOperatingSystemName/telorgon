mod controller;
pub(crate) mod model;
#[cfg(test)]
mod tests;
pub(crate) mod ui;

use super::PreviewController;

use super::{
    BootResult, BootScreens, BootTarget, PreviewScenario, PreviewTheme, ReadyBootApplication,
};
use crate::app::{Application, Renderer, Window};

pub(super) fn controller(
    targets: Vec<BootTarget>,
    selected: usize,
    theme: PreviewTheme,
    scenario: PreviewScenario,
) -> PreviewController {
    PreviewController::preview(model::PreviewModel::new(targets, selected, theme, scenario))
}

pub(super) fn run<S: BootScreens>(
    app: ReadyBootApplication<S>,
    scenario: PreviewScenario,
) -> BootResult<()> {
    let scenario = app.preview_startup_scenario(scenario);
    let controller = controller(app.targets, app.selected, app.theme, scenario);
    let component = app.screens.create(controller.clone());
    let _clock = controller::PreviewClock::start(controller)?;
    Application::gui("org.telorgon.boot-preview", "Telorgon Boot Preview")
        .renderer(Renderer::Software)
        .window(
            Window::new("Telorgon Boot Preview")
                .size(1100, 760)
                .minimum_size(900, 680)
                .content(component),
        )
        .run()?;
    Ok(())
}
