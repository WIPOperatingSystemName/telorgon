use super::*;

/// Trusted embedding boundary. Keys identify incarnations, not persistent preferences.
/// Providers must not redirect a key to a replacement device. Calls run on one I/O worker.
pub trait ScreenBrightnessProvider: Send + 'static {
    /// Nonblocking refresh hint. Native paths remain private to the provider.
    fn has_changes(&mut self) -> bool {
        false
    }
    fn discover(&mut self) -> Result<Vec<ScreenBrightnessProviderDevice>, ScreenBrightnessError>;
    fn read(&mut self, key: &str) -> Result<ScreenBrightnessReading, ScreenBrightnessError>;
    /// An error can follow a partial operation and therefore never proves no mutation.
    fn set(&mut self, key: &str, value: u32) -> Result<(), ScreenBrightnessError>;
}
#[derive(Clone, Debug)]
pub struct ScreenBrightnessProviderDevice {
    pub key: String,
    pub name: String,
    pub kind: ScreenBrightnessKind,
    pub association: ScreenBrightnessAssociation,
    pub maximum: u32,
    pub permission: crate::platform::contracts::PermissionState,
    pub verification: ScreenBrightnessVerification,
}
#[derive(Clone, Copy, Debug)]
pub struct ScreenBrightnessReading {
    pub configured: u32,
    pub actual: Option<u32>,
}
impl ScreenBrightnessReading {
    pub(crate) fn validate(self, maximum: u32) -> Result<Self, ScreenBrightnessError> {
        if maximum == 0 || self.configured > maximum || self.actual.is_some_and(|v| v > maximum) {
            Err(ScreenBrightnessError::InvalidData)
        } else {
            Ok(self)
        }
    }
}
