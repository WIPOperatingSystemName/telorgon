use super::*;
/// Combines explicitly supplied providers without exposing their native keys or resources.
pub struct ScreenBrightnessCombinedProvider {
    providers: Vec<Box<dyn ScreenBrightnessProvider>>,
}
impl ScreenBrightnessCombinedProvider {
    pub fn new(
        providers: Vec<Box<dyn ScreenBrightnessProvider>>,
    ) -> Result<Self, ScreenBrightnessError> {
        if providers.is_empty() || providers.len() > 8 {
            return Err(ScreenBrightnessError::InvalidConfig("provider count"));
        }
        Ok(Self { providers })
    }
    fn resolve(
        &mut self,
        key: &str,
    ) -> Result<(&mut Box<dyn ScreenBrightnessProvider>, String), ScreenBrightnessError> {
        let (index, key) = key
            .split_once(':')
            .ok_or(ScreenBrightnessError::StaleDevice)?;
        let provider = self
            .providers
            .get_mut(
                index
                    .parse::<usize>()
                    .map_err(|_| ScreenBrightnessError::StaleDevice)?,
            )
            .ok_or(ScreenBrightnessError::StaleDevice)?;
        Ok((provider, key.into()))
    }
}
impl ScreenBrightnessProvider for ScreenBrightnessCombinedProvider {
    fn has_changes(&mut self) -> bool {
        self.providers
            .iter_mut()
            .fold(false, |changed, provider| provider.has_changes() || changed)
    }
    fn discover(&mut self) -> Result<Vec<ScreenBrightnessProviderDevice>, ScreenBrightnessError> {
        let mut devices = Vec::new();
        let mut error = None;
        for (index, provider) in self.providers.iter_mut().enumerate() {
            match provider.discover() {
                Ok(found) => {
                    for mut device in found {
                        device.key = format!("{index}:{}", device.key);
                        if device.key.len() > 256 || devices.len() >= 64 {
                            return Err(ScreenBrightnessError::InvalidData);
                        }
                        devices.push(device);
                    }
                }
                Err(failed) => error = Some(failed),
            }
        }
        if devices.is_empty() {
            if let Some(error) = error {
                return Err(error);
            }
        }
        Ok(devices)
    }
    fn read(&mut self, key: &str) -> Result<ScreenBrightnessReading, ScreenBrightnessError> {
        let (provider, key) = self.resolve(key)?;
        provider.read(&key)
    }
    fn set(&mut self, key: &str, value: u32) -> Result<(), ScreenBrightnessError> {
        let (provider, key) = self.resolve(key)?;
        provider.set(&key, value)
    }
}
