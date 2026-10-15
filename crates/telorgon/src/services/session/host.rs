use super::{Environment, Result, SessionConfig, SessionHandle, SessionOwner};

/// A launch supervisor for a session helper that owns no graphics or input devices.
/// Uses the calling user's existing display, runtime directory and bus.
pub struct SessionHost { owner: SessionOwner }
impl SessionHost {
    pub fn start(config: SessionConfig) -> Result<Self> {
        config.validate()?;
        let publish = config.publish_user_service_environment;
        let mut owner = SessionOwner::start(Environment::gui()?, config)?;
        #[cfg(target_os = "linux")]
        if publish { owner.publish_services()?; }
        #[cfg(not(target_os = "linux"))]
        let _ = publish;
        Ok(Self { owner })
    }
    pub fn handle(&self) -> SessionHandle { self.owner.handle() }
    pub fn close(self) { self.owner.close(); }
}
