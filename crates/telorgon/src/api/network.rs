//! Network management contracts and opt-in native provider assembly.
pub use crate::services::network::*;

impl NetworkController {
    pub fn new(config: NetworkConfig) -> Result<Self, NetworkError> {
        config.validate()?;
        #[cfg(all(target_os = "linux", feature = "networkmanager-linux"))]
        {
            Self::with_provider(
                config,
                crate::integrations::networkmanager::NetworkManagerProvider::default(),
            )
        }
        #[cfg(not(all(target_os = "linux", feature = "networkmanager-linux")))]
        {
            Err(NetworkError::Unsupported)
        }
    }
}
