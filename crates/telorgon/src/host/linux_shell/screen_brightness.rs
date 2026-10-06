use super::{AppError, AppResult};
use crate::screen_brightness::{
    ScreenBrightnessController, ScreenBrightnessError, ScreenBrightnessHostState,
};

pub(super) struct Session(Option<ScreenBrightnessController>);
impl Session {
    pub fn start(
        mut controller: Option<ScreenBrightnessController>,
        active: bool,
    ) -> AppResult<Self> {
        if let Some(owner) = &mut controller {
            owner.set_host_state(ScreenBrightnessHostState {
                active,
                locked: false,
            });
            owner
                .start_for_shell(crate::shell::OutputId::MIN)
                .map_err(|e| AppError::new(e.to_string()))?;
        }
        Ok(Self(controller))
    }
    pub fn update(&mut self, active: bool, locked: bool) {
        if let Some(owner) = &self.0 {
            owner.set_host_state(ScreenBrightnessHostState { active, locked });
        }
    }
    pub fn release_key(&self, keycode: u32) {
        if let Some(owner) = &self.0 {
            owner.release_key(keycode);
        }
    }
    pub fn cancel_keys(&self) {
        if let Some(owner) = &self.0 {
            owner.cancel_keys();
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut owner) = self.0.take() {
            // Shutdown is bounded and runs after host loop termination, outside rendering/input.
            if let Err(error) = futures_lite::future::block_on(owner.shutdown()) {
                if error != ScreenBrightnessError::TimedOut {
                    eprintln!("telorgon-screen-brightness: {error}");
                }
            }
        }
    }
}
