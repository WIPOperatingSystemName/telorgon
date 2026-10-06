//! Host-owned, asynchronous physical screen brightness controls.
mod combined;
mod model;
mod provider;
pub use combined::ScreenBrightnessCombinedProvider;
mod controller;
mod keys;
mod request;
mod worker;
pub use keys::{ScreenBrightnessKeyConfig, ScreenBrightnessKeyHandling};
#[cfg(test)]
mod tests;
pub use controller::{
    ScreenBrightnessController, ScreenBrightnessHandle, ScreenBrightnessObserver,
};
pub use model::*;
pub use provider::*;
pub use request::{
    ScreenBrightnessCancellation, ScreenBrightnessRequest, ScreenBrightnessRequestState,
};
