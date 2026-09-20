//! Direct Vulkan rendering, explicit owned submission, and hosted interop boundaries.

mod adapter;
mod buffer;
#[cfg(target_os = "linux")]
mod composite;
mod config;
mod descriptor;
mod device;
mod diagnostics;
mod driver_workarounds;
mod entry;
mod error;
mod executor;
mod external_dma_buf;
mod external_image;
mod frame;
#[path = "shaders/generated/metadata.rs"]
mod generated_shader_bundle;
mod hosted;
mod image;
pub mod interop;
mod memory;
mod pipeline;
mod readback;
mod scene;
mod shader;
mod sync;
mod target;
mod upload;

pub use adapter::{AdapterReport, DeviceSelection};
#[cfg(target_os = "linux")]
pub use composite::{VulkanCompositePlacement, VulkanCompositeScene};
pub use config::{VulkanConfig, VulkanLiveResizeMode};
pub use device::{VulkanCapabilities, VulkanDevice, VulkanMemoryMetrics};
pub use diagnostics::{VulkanDebugMessage, VulkanDiagnostics};
pub use entry::{InstanceExtensionRequest, VulkanInstance};
pub use external_dma_buf::{
    DRM_FORMAT_ABGR8888, DRM_FORMAT_ARGB8888, DRM_FORMAT_MOD_INVALID, DRM_FORMAT_MOD_LINEAR,
    DRM_FORMAT_XBGR8888, DRM_FORMAT_XRGB8888,
};
#[cfg(target_os = "linux")]
pub use external_dma_buf::{
    VulkanDmaBufFormatCapability, VulkanDmaBufImport, VulkanDmaBufPlane, VulkanDmaBufReleaseSyncFd,
    VulkanDmaBufScanoutTarget,
};
#[cfg(target_os = "linux")]
pub(crate) use external_image::CachedDmaBufImage;
pub use external_image::{
    HostedExternalImageUse, HostedExternalSemaphoreSignal, HostedExternalSemaphoreWait,
    VulkanExternalAcquire, VulkanExternalImageCapabilities, VulkanExternalImageDescriptor,
    VulkanExternalImageImport, VulkanExternalImageLease, VulkanExternalImageOrigin,
    VulkanExternalRelease,
};
pub use frame::{
    CompletionPoint, SubmissionReceipt, VulkanFrameContext, VulkanRecordedFrame,
    VulkanRecordingFrame,
};
pub use hosted::{
    HostCompletionDomain, HostCompletionPoint, HostedAllocationPolicy, HostedCommandBufferState,
    HostedDeviceExtensions, HostedDeviceFeatures, HostedFrameDescriptor, HostedFrameReceipt,
    HostedImageUse, HostedMaintenanceStats, HostedRecordStats, HostedTargetDescriptor,
    HostedVulkanDeviceDescriptor, VulkanHostedFrame,
};
pub use readback::{PendingVulkanReadback, VulkanReadback};
pub(crate) use readback::CaptureReadbackBuffer;
pub use scene::{VulkanScene, VulkanSceneMetrics};
pub(crate) use target::VulkanMaterializationTarget;
pub(crate) use target::VulkanCaptureTarget;
pub use target::{OffscreenVulkanTarget, VulkanTarget};
