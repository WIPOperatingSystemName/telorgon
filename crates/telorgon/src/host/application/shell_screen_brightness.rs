use super::declaration::{ReadyShellEnvironment, ShellEnvironment, ShellEnvironmentWithCompositor};
use crate::screen_brightness::ScreenBrightnessController;

macro_rules! brightness_builder {
    ($owner:ty) => {
        impl $owner {
            /// Moves the controller into the shell host and installs its widget handle.
            /// Construction starts no native work; shell startup supplies live seat authority.
            pub fn screen_brightness(mut self, controller: ScreenBrightnessController) -> Self {
                let handle = controller.handle();
                self.services.insert(handle.observer());
                self.services.insert(handle);
                self.screen_brightness = Some(controller);
                self
            }
        }
    };
}
brightness_builder!(ShellEnvironment);
brightness_builder!(ShellEnvironmentWithCompositor);
brightness_builder!(ReadyShellEnvironment);
