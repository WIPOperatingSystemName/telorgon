use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::{RectI, SizeI};
use crate::render::{
    ImageAlphaMode, ImageColorEncoding, RenderError, RenderErrorKind, RenderResult,
};
use ash::vk::{self, Handle};

use crate::renderer_vulkan::hosted::HostedImageUse;
use crate::renderer_vulkan::{VulkanDevice, error::unsupported};

pub(crate) const UNUSED: u64 = 0;
const COMPLETED: u64 = u64::MAX;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VulkanExternalAcquire {
    /// The host establishes availability earlier in the same externally synchronized command
    /// stream. Telorgon still records the declared image-state transition.
    CommandStream,
    /// The host must wait on this borrowed binary semaphore before executing Telorgon's commands.
    BinarySemaphore(vk::Semaphore),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VulkanExternalRelease {
    /// The host consumes the final image state later in the same command stream.
    CommandStream,
    /// The host must signal this borrowed binary semaphore after Telorgon's final image read.
    BinarySemaphore(vk::Semaphore),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VulkanExternalImageOrigin {
    TopLeft,
    BottomLeft,
}

/// One linear use-generation of a host-owned, same-device Vulkan image.
pub struct VulkanExternalImageDescriptor {
    pub image: vk::Image,
    pub view: vk::ImageView,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    pub usage: vk::ImageUsageFlags,
    pub queue_family: u32,
    pub content_version: u64,
    pub lease_generation: u64,
    pub color_encoding: ImageColorEncoding,
    pub alpha_mode: ImageAlphaMode,
    pub origin: VulkanExternalImageOrigin,
    pub initial_use: HostedImageUse,
    pub final_use: HostedImageUse,
    pub acquire: VulkanExternalAcquire,
    pub release: VulkanExternalRelease,
    pub damage: Vec<RectI>,
    pub protected: bool,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct VulkanExternalImageCapabilities {
    pub borrowed_same_device: bool,
    pub binary_semaphore_acquire_release: bool,
    pub linux_dma_buf: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VulkanExternalImageImport {
    BorrowedSameDevice,
    LinuxDmaBuf,
}

impl VulkanExternalImageCapabilities {
    pub fn supports(self, mechanism: VulkanExternalImageImport) -> bool {
        match mechanism {
            VulkanExternalImageImport::BorrowedSameDevice => self.borrowed_same_device,
            VulkanExternalImageImport::LinuxDmaBuf => self.linux_dma_buf,
        }
    }

    pub fn require(self, mechanism: VulkanExternalImageImport) -> RenderResult<()> {
        if self.supports(mechanism) {
            Ok(())
        } else {
            Err(unsupported(format!(
                "external Vulkan image import mechanism {mechanism:?} is unavailable"
            )))
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HostedExternalSemaphoreWait {
    pub semaphore: vk::Semaphore,
    pub stage_mask: vk::PipelineStageFlags2,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HostedExternalSemaphoreSignal {
    pub semaphore: vk::Semaphore,
    pub stage_mask: vk::PipelineStageFlags2,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HostedExternalImageUse {
    pub import: VulkanExternalImageImport,
    pub image: vk::Image,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    pub content_version: u64,
    pub lease_generation: u64,
    pub color_encoding: ImageColorEncoding,
    pub damage_rects: u32,
    pub initial_use: HostedImageUse,
    pub final_use: HostedImageUse,
    pub initial_queue_family: u32,
    pub final_queue_family: u32,
}

/// Linear wrapper consumed by `VulkanScene::bind_external_image`; it never owns the native image,
/// image view, or semaphore handles.
pub struct VulkanExternalImageLease {
    pub(crate) inner: Option<Arc<ExternalImageInner>>,
    _linear: PhantomData<Rc<()>>,
}

/// Keeps one immutable native import alive independently of its per-commit use token.
#[cfg(target_os = "linux")]
pub(crate) struct CachedDmaBufImage(Arc<ExternalImageInner>);

#[cfg(target_os = "linux")]
impl CachedDmaBufImage {
    pub(crate) fn matches_device(&self, device: &VulkanDevice) -> bool {
        self.0.device_id == device.inner.id
    }
    pub(crate) fn can_reuse(&self) -> bool {
        cache_reusable(
            Arc::strong_count(&self.0),
            self.0.state.load(Ordering::Acquire),
            self.0.submitted_release_is_resolved(),
        )
    }

    /// The caller must match the immutable allocation identity/descriptor and acquire contract.
    pub(crate) unsafe fn reuse(
        &mut self,
        content_version: u64,
        lease_generation: u64,
        acquire: Option<std::os::fd::OwnedFd>,
        damage: Vec<RectI>,
    ) -> RenderResult<VulkanExternalImageLease> {
        if !self.can_reuse() || content_version == 0 || lease_generation == 0 {
            return Err(unsupported(
                "DMA-BUF cache entry is still in use or has an invalid generation",
            ));
        }
        let inner = Arc::get_mut(&mut self.0)
            .ok_or_else(|| unsupported("DMA-BUF cache entry is shared"))?;
        let ExternalImageOwnership::DmaBuf(resources) = &mut inner.ownership else {
            return Err(unsupported("only owned DMA-BUF imports can be rearmed"));
        };
        inner.acquire = resources.rearm(acquire)?;
        inner.content_version = content_version;
        inner.lease_generation = lease_generation;
        inner.damage = damage;
        inner.state.store(UNUSED, Ordering::Release);
        Ok(VulkanExternalImageLease {
            inner: Some(Arc::clone(&self.0)),
            _linear: PhantomData,
        })
    }
}

#[cfg(any(target_os = "linux", test))]
fn cache_reusable(references: usize, state: u64, release_resolved: bool) -> bool {
    references == 1 && state == COMPLETED && release_resolved
}

pub(crate) struct ExternalImageInner {
    pub(crate) device_id: u64,
    pub(crate) image: vk::Image,
    pub(crate) view: vk::ImageView,
    pub(crate) format: vk::Format,
    pub(crate) extent: SizeI,
    pub(crate) content_version: u64,
    pub(crate) lease_generation: u64,
    pub(crate) color_encoding: ImageColorEncoding,
    pub(crate) alpha_mode: ImageAlphaMode,
    pub(crate) initial_use: HostedImageUse,
    pub(crate) final_use: HostedImageUse,
    pub(crate) initial_queue_family: u32,
    pub(crate) final_queue_family: u32,
    pub(crate) acquire: VulkanExternalAcquire,
    pub(crate) release: VulkanExternalRelease,
    pub(crate) damage: Vec<RectI>,
    pub(crate) ownership: ExternalImageOwnership,
    pub(crate) state: AtomicU64,
}

pub(crate) enum ExternalImageOwnership {
    Borrowed,
    #[cfg(target_os = "linux")]
    DmaBuf(crate::renderer_vulkan::external_dma_buf::DmaBufOwnedResources),
}

impl VulkanDevice {
    pub fn external_image_capabilities(&self) -> VulkanExternalImageCapabilities {
        VulkanExternalImageCapabilities {
            borrowed_same_device: self.hosted.is_some(),
            binary_semaphore_acquire_release: self.hosted.is_some(),
            linux_dma_buf: crate::renderer_vulkan::external_dma_buf::device_supports_linux_dma_buf(
                self,
            ),
        }
    }

    /// Imports one use-generation of a host-owned image without copying its pixels.
    ///
    /// # Safety
    ///
    /// The image, view, and semaphore handles must belong to this hosted logical device and remain
    /// live through the returned lease's recorded release. Metadata, usage, state, queue ownership,
    /// content version, and synchronization declarations must match host truth. Telorgon borrows and
    /// never destroys any supplied handle.
    pub unsafe fn import_external_image(
        &self,
        descriptor: VulkanExternalImageDescriptor,
    ) -> RenderResult<VulkanExternalImageLease> {
        if self.hosted.is_none() {
            return Err(unsupported(
                "same-device external images currently require command-only hosted Vulkan",
            ));
        }
        validate_descriptor(self, &descriptor)?;
        Ok(VulkanExternalImageLease {
            inner: Some(Arc::new(ExternalImageInner {
                device_id: self.inner.id,
                image: descriptor.image,
                view: descriptor.view,
                format: descriptor.format,
                extent: SizeI {
                    width: descriptor.extent.width as i32,
                    height: descriptor.extent.height as i32,
                },
                content_version: descriptor.content_version,
                lease_generation: descriptor.lease_generation,
                color_encoding: descriptor.color_encoding,
                alpha_mode: descriptor.alpha_mode,
                initial_use: descriptor.initial_use,
                final_use: descriptor.final_use,
                initial_queue_family: vk::QUEUE_FAMILY_IGNORED,
                final_queue_family: vk::QUEUE_FAMILY_IGNORED,
                acquire: descriptor.acquire,
                release: descriptor.release,
                damage: descriptor.damage,
                ownership: ExternalImageOwnership::Borrowed,
                state: AtomicU64::new(UNUSED),
            })),
            _linear: PhantomData,
        })
    }
}

impl VulkanExternalImageLease {
    #[cfg(target_os = "linux")]
    pub(crate) fn cache_dma_buf(&self) -> CachedDmaBufImage {
        CachedDmaBufImage(Arc::clone(
            self.inner.as_ref().expect("unconsumed DMA-BUF lease"),
        ))
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn from_inner(inner: ExternalImageInner) -> Self {
        Self {
            inner: Some(Arc::new(inner)),
            _linear: PhantomData,
        }
    }

    pub fn content_version(&self) -> u64 {
        self.inner
            .as_ref()
            .expect("external image lease was already consumed")
            .content_version
    }

    pub fn lease_generation(&self) -> u64 {
        self.inner
            .as_ref()
            .expect("external image lease was already consumed")
            .lease_generation
    }

    pub(crate) fn take_inner(&mut self) -> RenderResult<Arc<ExternalImageInner>> {
        self.inner.take().ok_or_else(|| {
            RenderError::new(
                RenderErrorKind::HostContract,
                "external image lease was already consumed",
            )
        })
    }
}

impl ExternalImageInner {
    pub(crate) fn begin_use(&self, frame_id: u64) -> RenderResult<()> {
        match self
            .state
            .compare_exchange(UNUSED, frame_id, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Ok(()),
            Err(COMPLETED) => Err(host_contract(
                "external image lease generation was already released; import a new lease",
            )),
            Err(_) => Err(host_contract(
                "external image lease generation is already recorded or in flight",
            )),
        }
    }

    pub(crate) fn cancel_use(&self, frame_id: u64) {
        let _ = self
            .state
            .compare_exchange(frame_id, UNUSED, Ordering::AcqRel, Ordering::Acquire);
    }

    pub(crate) fn complete_use(&self, frame_id: u64) {
        let _ =
            self.state
                .compare_exchange(frame_id, COMPLETED, Ordering::AcqRel, Ordering::Acquire);
    }

    pub(crate) fn use_report(&self) -> HostedExternalImageUse {
        HostedExternalImageUse {
            import: match &self.ownership {
                ExternalImageOwnership::Borrowed => VulkanExternalImageImport::BorrowedSameDevice,
                #[cfg(target_os = "linux")]
                ExternalImageOwnership::DmaBuf(_) => VulkanExternalImageImport::LinuxDmaBuf,
            },
            image: self.image,
            format: self.format,
            extent: vk::Extent2D {
                width: self.extent.width as u32,
                height: self.extent.height as u32,
            },
            content_version: self.content_version,
            lease_generation: self.lease_generation,
            color_encoding: self.color_encoding,
            damage_rects: self.damage.len() as u32,
            initial_use: self.initial_use,
            final_use: self.final_use,
            initial_queue_family: self.initial_queue_family,
            final_queue_family: self.final_queue_family,
        }
    }

    pub(crate) fn submitted_release_is_resolved(&self) -> bool {
        match &self.ownership {
            ExternalImageOwnership::Borrowed => true,
            #[cfg(target_os = "linux")]
            ExternalImageOwnership::DmaBuf(resources) => resources.release_is_resolved(),
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) unsafe fn export_release_sync_fd(
        &self,
    ) -> RenderResult<Option<crate::renderer_vulkan::external_dma_buf::VulkanDmaBufReleaseSyncFd>>
    {
        match &self.ownership {
            ExternalImageOwnership::Borrowed => Ok(None),
            ExternalImageOwnership::DmaBuf(resources) => unsafe {
                resources.export_release_sync_fd(self.content_version, self.lease_generation)
            },
        }
    }
}

fn validate_descriptor(
    device: &VulkanDevice,
    descriptor: &VulkanExternalImageDescriptor,
) -> RenderResult<()> {
    if descriptor.image.is_null() || descriptor.view.is_null() {
        return Err(host_contract("external image and view must be non-null"));
    }
    if descriptor.format == vk::Format::UNDEFINED
        || descriptor.extent.width == 0
        || descriptor.extent.height == 0
        || descriptor.extent.width > i32::MAX as u32
        || descriptor.extent.height > i32::MAX as u32
    {
        return Err(RenderError::new(
            RenderErrorKind::InvalidTarget,
            "external image format or extent is invalid",
        ));
    }
    if descriptor.content_version == 0 || descriptor.lease_generation == 0 {
        return Err(host_contract(
            "external content version and lease generation must be nonzero",
        ));
    }
    if descriptor.protected {
        return Err(unsupported(
            "protected external images are not supported by this Vulkan profile",
        ));
    }
    if !descriptor.usage.contains(vk::ImageUsageFlags::SAMPLED) {
        return Err(RenderError::new(
            RenderErrorKind::InvalidTarget,
            "external image was not created for sampled use",
        ));
    }
    if descriptor.queue_family != device.inner.queue_family {
        return Err(host_contract(
            "external image queue ownership transfers are not performed by Telorgon",
        ));
    }
    if descriptor.initial_use.state().layout == vk::ImageLayout::UNDEFINED
        || descriptor.final_use.state().layout == vk::ImageLayout::UNDEFINED
    {
        return Err(host_contract(
            "external image initial and final uses must preserve valid content",
        ));
    }
    if descriptor.origin != VulkanExternalImageOrigin::TopLeft {
        return Err(unsupported(
            "bottom-left external image origin requires an explicit normalization path",
        ));
    }
    let encoding_matches_format = match descriptor.color_encoding {
        ImageColorEncoding::Linear => matches!(
            descriptor.format,
            vk::Format::R8G8B8A8_UNORM | vk::Format::B8G8R8A8_UNORM
        ),
        ImageColorEncoding::Srgb => matches!(
            descriptor.format,
            vk::Format::R8G8B8A8_SRGB | vk::Format::B8G8R8A8_SRGB
        ),
    };
    if !encoding_matches_format {
        return Err(unsupported(
            "external image format does not implement its declared color encoding",
        ));
    }
    let valid_sync = |semaphore: vk::Semaphore| !semaphore.is_null();
    if matches!(descriptor.acquire, VulkanExternalAcquire::BinarySemaphore(value) if !valid_sync(value))
        || matches!(descriptor.release, VulkanExternalRelease::BinarySemaphore(value) if !valid_sync(value))
    {
        return Err(host_contract(
            "external acquire/release semaphore handles must be non-null",
        ));
    }
    let features = unsafe {
        device
            .inner
            .instance
            .inner
            .raw
            .get_physical_device_format_properties(device.inner.physical_device, descriptor.format)
            .optimal_tiling_features
    };
    if !features.contains(
        vk::FormatFeatureFlags::SAMPLED_IMAGE | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR,
    ) {
        return Err(unsupported(
            "external image format cannot be linearly sampled on this adapter",
        ));
    }
    for damage in &descriptor.damage {
        let right = damage.x.checked_add(damage.width);
        let bottom = damage.y.checked_add(damage.height);
        if damage.x < 0
            || damage.y < 0
            || damage.width <= 0
            || damage.height <= 0
            || right.is_none_or(|value| value > descriptor.extent.width as i32)
            || bottom.is_none_or(|value| value > descriptor.extent.height as i32)
        {
            return Err(host_contract(
                "external image damage lies outside its physical extent",
            ));
        }
    }
    Ok(())
}

fn host_contract(message: impl Into<String>) -> RenderError {
    RenderError::new(RenderErrorKind::HostContract, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_import_waits_for_gpu_release_and_all_pins() {
        assert!(cache_reusable(1, COMPLETED, true));
        for state in [UNUSED, 1, 24] {
            assert!(!cache_reusable(1, state, true));
        }
        assert!(!cache_reusable(1, COMPLETED, false));
        for references in [0, 2, 3, 10] {
            assert!(!cache_reusable(references, COMPLETED, true));
        }
    }

    #[test]
    fn unavailable_dma_buf_capability_returns_structured_unsupported() {
        let capabilities = VulkanExternalImageCapabilities {
            borrowed_same_device: true,
            binary_semaphore_acquire_release: true,
            linux_dma_buf: false,
        };
        assert!(capabilities.borrowed_same_device);
        assert!(!capabilities.linux_dma_buf);
        capabilities
            .require(VulkanExternalImageImport::BorrowedSameDevice)
            .unwrap();
        assert_eq!(
            capabilities
                .require(VulkanExternalImageImport::LinuxDmaBuf)
                .unwrap_err()
                .kind(),
            RenderErrorKind::Unsupported
        );
    }

    #[test]
    fn one_lease_generation_is_linear_until_cancel_or_completion() {
        let image = ExternalImageInner {
            device_id: 7,
            image: vk::Image::from_raw(1),
            view: vk::ImageView::from_raw(2),
            format: vk::Format::R8G8B8A8_UNORM,
            extent: SizeI {
                width: 8,
                height: 8,
            },
            content_version: 3,
            lease_generation: 4,
            color_encoding: ImageColorEncoding::Linear,
            alpha_mode: ImageAlphaMode::Opaque,
            initial_use: HostedImageUse::ColorAttachment,
            final_use: HostedImageUse::ShaderRead,
            initial_queue_family: vk::QUEUE_FAMILY_IGNORED,
            final_queue_family: vk::QUEUE_FAMILY_IGNORED,
            acquire: VulkanExternalAcquire::CommandStream,
            release: VulkanExternalRelease::CommandStream,
            damage: Vec::new(),
            ownership: ExternalImageOwnership::Borrowed,
            state: AtomicU64::new(UNUSED),
        };
        image.begin_use(11).unwrap();
        assert_eq!(
            image.begin_use(12).unwrap_err().kind(),
            RenderErrorKind::HostContract
        );
        image.cancel_use(11);
        image.begin_use(12).unwrap();
        image.complete_use(12);
        assert_eq!(
            image.begin_use(13).unwrap_err().kind(),
            RenderErrorKind::HostContract
        );
    }
}
