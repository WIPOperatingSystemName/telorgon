//! Linux kernel backlight adapter. No worker or device is started by constructing configuration.
#[cfg(feature = "shell-screen-brightness-ddc-linux")]
pub mod ddc;
mod events;
mod logind;
mod sysfs;
use crate::{platform::contracts::PermissionState, screen_brightness::*, shell::OutputId};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScreenBrightnessLinuxWriteAccess {
    #[default]
    Logind,
    DirectProvisioned,
}
#[derive(Clone, Debug)]
pub struct ScreenBrightnessLinuxConfig {
    pub access: ScreenBrightnessLinuxWriteAccess,
    /// Trusted host selection. Without it, output association remains unknown.
    pub panel_device: Option<String>,
    /// Explicit launcher relationship for a shell outside logind session membership.
    pub session_id: Option<String>,
    pub seat: String,
    pub timeout: std::time::Duration,
}
impl Default for ScreenBrightnessLinuxConfig {
    fn default() -> Self {
        Self {
            access: Default::default(),
            panel_device: None,
            session_id: None,
            seat: "seat0".into(),
            timeout: std::time::Duration::from_secs(5),
        }
    }
}
pub struct ScreenBrightnessLinuxProvider {
    config: ScreenBrightnessLinuxConfig,
    output: OutputId,
    sysfs: Option<sysfs::Sysfs>,
    devices: HashMap<String, sysfs::Device>,
    maximums: HashMap<String, u32>,
    session: Option<logind::Session>,
    quarantined: bool,
    events: Option<events::Events>,
}
impl ScreenBrightnessLinuxProvider {
    pub fn new(
        config: ScreenBrightnessLinuxConfig,
        output: OutputId,
    ) -> Result<Self, ScreenBrightnessError> {
        if config
            .panel_device
            .as_ref()
            .is_some_and(|s| !sysfs::valid_name(s))
            || config
                .session_id
                .as_ref()
                .is_some_and(|s| !sysfs::valid_name(s))
            || !sysfs::valid_name(&config.seat)
            || config.timeout.is_zero()
            || config.timeout > std::time::Duration::from_secs(60)
        {
            return Err(ScreenBrightnessError::InvalidConfig(
                "Linux device, session, seat or timeout",
            ));
        }
        Ok(Self {
            config,
            output,
            sysfs: None,
            devices: HashMap::new(),
            maximums: HashMap::new(),
            session: None,
            quarantined: false,
            events: None,
        })
    }
}
impl ScreenBrightnessProvider for ScreenBrightnessLinuxProvider {
    fn has_changes(&mut self) -> bool {
        self.events.as_ref().is_some_and(events::Events::changed)
    }
    fn discover(&mut self) -> Result<Vec<ScreenBrightnessProviderDevice>, ScreenBrightnessError> {
        if self.sysfs.is_none() {
            self.sysfs = Some(sysfs::Sysfs::open()?);
            self.events = events::Events::open();
        }
        let discovered = self.sysfs.as_ref().unwrap().devices()?;
        if self.config.access == ScreenBrightnessLinuxWriteAccess::Logind
            && self.session.is_none()
            && !self.quarantined
        {
            self.session = logind::Session::connect(
                self.config.session_id.as_deref(),
                std::process::id(),
                self.config.timeout,
            )
            .ok();
        }
        let session_permission = if self.config.access == ScreenBrightnessLinuxWriteAccess::Logind
            && !self.quarantined
        {
            let authorization = self
                .session
                .as_ref()
                .map(|session| session.authorize(unsafe { libc::geteuid() }, &self.config.seat));
            match authorization {
                Some(Ok(())) => PermissionState::Granted,
                Some(Err(
                    ScreenBrightnessError::Locked | ScreenBrightnessError::SessionInactive,
                )) => PermissionState::Restricted,
                Some(Err(ScreenBrightnessError::PermissionDenied)) | None => {
                    PermissionState::Denied
                }
                Some(Err(_)) => {
                    self.session = None;
                    PermissionState::Unknown
                }
            }
        } else {
            PermissionState::Restricted
        };
        let mut devices = HashMap::new();
        let mut maximums = HashMap::new();
        let mut descriptors = Vec::new();
        for device in discovered {
            if self
                .config
                .panel_device
                .as_ref()
                .is_some_and(|wanted| wanted != &device.name)
            {
                continue;
            }
            let maximum = device.read("max_brightness")?;
            if maximum == 0 {
                continue;
            }
            let permission = if self.quarantined {
                PermissionState::Restricted
            } else {
                match self.config.access {
                    ScreenBrightnessLinuxWriteAccess::Logind => session_permission,
                    ScreenBrightnessLinuxWriteAccess::DirectProvisioned => {
                        if device.can_write() {
                            PermissionState::Granted
                        } else {
                            PermissionState::Denied
                        }
                    }
                }
            };
            descriptors.push(ScreenBrightnessProviderDevice {
                key: device.key.clone(),
                name: device.name.clone(),
                kind: ScreenBrightnessKind::InternalBacklight,
                association: if self.config.panel_device.is_some() {
                    ScreenBrightnessAssociation::HostConfigured(vec![self.output])
                } else {
                    ScreenBrightnessAssociation::Unknown
                },
                maximum,
                permission,
                verification: ScreenBrightnessVerification::DriverSetting,
            });
            maximums.insert(device.key.clone(), maximum);
            devices.insert(device.key.clone(), device);
        }
        self.devices = devices;
        self.maximums = maximums;
        Ok(descriptors)
    }
    fn read(&mut self, key: &str) -> Result<ScreenBrightnessReading, ScreenBrightnessError> {
        let device = self
            .devices
            .get(key)
            .ok_or(ScreenBrightnessError::StaleDevice)?;
        if Some(&device.read("max_brightness")?) != self.maximums.get(key) {
            return Err(ScreenBrightnessError::StaleDevice);
        }
        Ok(ScreenBrightnessReading {
            configured: device.read("brightness")?,
            actual: device.read("actual_brightness").ok(),
        })
    }
    fn set(&mut self, key: &str, value: u32) -> Result<(), ScreenBrightnessError> {
        if self.quarantined {
            return Err(ScreenBrightnessError::Unavailable);
        }
        let device = self
            .devices
            .get(key)
            .ok_or(ScreenBrightnessError::StaleDevice)?;
        let max = device.read("max_brightness")?;
        if Some(&max) != self.maximums.get(key) {
            return Err(ScreenBrightnessError::StaleDevice);
        }
        if max == 0 || value > max {
            return Err(ScreenBrightnessError::InvalidLevel);
        }
        match self.config.access {
            ScreenBrightnessLinuxWriteAccess::DirectProvisioned => device.write(value),
            ScreenBrightnessLinuxWriteAccess::Logind => {
                let session = self
                    .session
                    .as_ref()
                    .ok_or(ScreenBrightnessError::PermissionDenied)?;
                // Logind also checks foreground/device seat at execution. No privileged fallback.
                session.authorize(unsafe { libc::geteuid() }, &self.config.seat)?;
                let result = session.set(&device.name, value);
                // A lost reply cannot establish that the server drained its write. Fail closed
                // until the owner is replaced, rather than overlap another command.
                if matches!(result, Err(ScreenBrightnessError::Transport)) {
                    self.quarantined = true;
                }
                result
            }
        }
    }
}
