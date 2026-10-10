//! Portable display preferences and observed state. Hardware changes belong to the host.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayMode {
    pub width: i32,
    pub height: i32,
    pub refresh_millihertz: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DisplayConfiguration {
    /// Empty selects the connected output chosen by the host.
    pub connector: String,
    /// None selects the monitor's preferred mode.
    pub mode: Option<DisplayMode>,
    /// None selects automatic density; fixed values are 1.0–4.0.
    pub scale: Option<f32>,
}
impl DisplayConfiguration {
    pub fn validate(&self) -> Result<(), String> {
        if self.connector.len() > 128 {
            return Err("Monitor name is too long".into());
        }
        if self
            .scale
            .is_some_and(|s| !s.is_finite() || !(1.0..=4.0).contains(&s))
        {
            return Err("Display scale must be between 100% and 400%".into());
        }
        if self
            .mode
            .is_some_and(|m| m.width <= 0 || m.height <= 0 || m.refresh_millihertz == 0)
        {
            return Err("Invalid display mode".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DisplaySnapshot {
    pub ready: bool,
    pub connector: String,
    #[serde(default)]
    pub connector_name: Option<String>,
    #[serde(default)]
    pub monitor_name: Option<String>,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub product_code: Option<u16>,
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default)]
    pub physical_millimeters: Option<crate::foundation::SizeI>,
    pub modes: Vec<DisplayMode>,
    pub preferred_mode: Option<DisplayMode>,
    pub current: DisplayConfiguration,
    pub resolved_scale: f32,
    pub preview: Option<u64>,
    pub seconds_remaining: u64,
    pub error: Option<String>,
}
