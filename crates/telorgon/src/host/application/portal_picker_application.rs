use super::{AppError, AppResult, Application, GuiApplication, ReadyWindow, Renderer};
use crate::{AssetBundle, ScreenCastPortalContext};

pub struct PortalPickerApplication {
    gui: GuiApplication,
}
impl Default for PortalPickerApplication {
    fn default() -> Self {
        Self::new()
    }
}
impl PortalPickerApplication {
    pub fn new() -> Self {
        Self {
            gui: Application::gui("org.telorgon.portal-picker", "Screen sharing"),
        }
    }
    pub fn assets(mut self, assets: AssetBundle) -> Self {
        self.gui = self.gui.assets(assets);
        self
    }
    pub fn renderer(mut self, renderer: Renderer) -> Self {
        self.gui = self.gui.renderer(renderer);
        self
    }
    pub fn window<F: FnOnce(ScreenCastPortalContext) -> ReadyWindow>(
        self,
        factory: F,
    ) -> ReadyPortalPickerApplication<F> {
        ReadyPortalPickerApplication {
            gui: self.gui,
            factory,
        }
    }
}
pub struct ReadyPortalPickerApplication<F> {
    gui: GuiApplication,
    factory: F,
}
impl<F: FnOnce(ScreenCastPortalContext) -> ReadyWindow> ReadyPortalPickerApplication<F> {
    pub fn run(self) -> AppResult<()> {
        let context = ScreenCastPortalContext::from_picker_stdio()
            .map_err(|e| AppError::new(e.to_string()))?;
        self.gui.window((self.factory)(context)).run()
    }
}
