//! Linux DRM/KMS atomic presentation and GBM scanout allocation for Telorgon's Vulkan renderer.
//!
//! This package is intentionally independent of Winit, X11, wlroots, and Rust compositor stacks.

mod model;

#[cfg(target_os = "linux")]
pub mod ffi;
#[cfg(target_os = "linux")]
mod gbm;
#[cfg(target_os = "linux")]
mod kms;
#[cfg(target_os = "linux")]
mod plane_formats;
#[cfg(target_os = "linux")]
mod topology;
#[cfg(target_os = "linux")]
pub use plane_formats::plane_formats;

#[cfg(target_os = "linux")]
pub use gbm::{GbmBuffer, GbmDevice, GbmPlane, GbmWriteMapping};
#[cfg(target_os = "linux")]
pub use kms::{AtomicRequest, KmsDevice, KmsError, KmsErrorKind, KmsFramebuffer, PropertyBlob};
pub use model::{
    AtomicProperty, DRM_FORMAT_ARGB8888, DRM_FORMAT_MOD_INVALID, DRM_FORMAT_MOD_LINEAR,
    DRM_FORMAT_XRGB8888, DRM_PLANE_TYPE_CURSOR, DRM_PLANE_TYPE_PRIMARY, FrameSlot, FrameSlotError,
    FrameSlotState, KmsConnectorId, KmsCrtcId, KmsFramebufferId, KmsMode, KmsPlaneId,
    KmsPropertyId, ScanoutFormat,
};
#[cfg(target_os = "linux")]
pub use topology::{
    ConnectorStatus, KmsConnector, KmsConnectorMode, KmsObjectProperties, KmsPlane, KmsProperty,
    KmsPropertyObject, KmsTopology,
};

pub const NATIVE_KMS_AVAILABLE: bool = cfg!(target_os = "linux");

#[cfg(target_os = "linux")]
mod scanout;
#[cfg(target_os = "linux")]
pub use scanout::{DumbBuffer, ScanoutBuffer};
