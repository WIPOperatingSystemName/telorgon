//! Linux DMA-BUF/DRM-modifier import and sync-FD release for hosted Vulkan.

#[cfg(any(target_os = "linux", test))]
use crate::foundation::RectI;
#[cfg(any(target_os = "linux", test))]
use crate::graphics::render::{
    ImageAlphaMode, ImageColorEncoding, RenderError, RenderErrorKind, RenderResult,
};
#[cfg(any(target_os = "linux", test))]
use ash::vk;
#[cfg(any(target_os = "linux", test))]
use std::sync::atomic::{AtomicU8, Ordering};

use crate::graphics::renderers::vulkan::VulkanDevice;

pub(crate) fn device_supports_linux_dma_buf(device: &VulkanDevice) -> bool {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device;
        false
    }
    #[cfg(target_os = "linux")]
    {
        if !(device.inner.owned_dma_buf_imports
            || device.inner.hosted_extensions.linux_dma_buf_complete())
        {
            return false;
        }
        let info = vk::PhysicalDeviceExternalSemaphoreInfo::default()
            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        let mut properties = vk::ExternalSemaphoreProperties::default();
        unsafe {
            device
                .inner
                .instance
                .inner
                .raw
                .get_physical_device_external_semaphore_properties(
                    device.inner.physical_device,
                    &info,
                    &mut properties,
                );
        }
        properties.external_semaphore_features.contains(
            vk::ExternalSemaphoreFeatureFlags::IMPORTABLE
                | vk::ExternalSemaphoreFeatureFlags::EXPORTABLE,
        ) && properties
            .compatible_handle_types
            .contains(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
    }
}

const fn drm_fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    a as u32 | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

pub const DRM_FORMAT_ABGR8888: u32 = drm_fourcc(b'A', b'B', b'2', b'4');
pub const DRM_FORMAT_XBGR8888: u32 = drm_fourcc(b'X', b'B', b'2', b'4');
pub const DRM_FORMAT_ARGB8888: u32 = drm_fourcc(b'A', b'R', b'2', b'4');
pub const DRM_FORMAT_XRGB8888: u32 = drm_fourcc(b'X', b'R', b'2', b'4');
pub const DRM_FORMAT_MOD_LINEAR: u64 = 0;
pub const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

#[cfg(any(target_os = "linux", test))]
const RELEASE_NOT_EXPORTED: u8 = 0;
#[cfg(any(target_os = "linux", test))]
const RELEASE_EXPORTING: u8 = 1;
#[cfg(any(target_os = "linux", test))]
const RELEASE_EXPORTED: u8 = 2;

#[cfg(any(target_os = "linux", test))]
struct ReleaseExportState(AtomicU8);

#[cfg(any(target_os = "linux", test))]
impl ReleaseExportState {
    fn new() -> Self {
        Self(AtomicU8::new(RELEASE_NOT_EXPORTED))
    }

    fn begin(&self) -> RenderResult<()> {
        match self.0.compare_exchange(
            RELEASE_NOT_EXPORTED,
            RELEASE_EXPORTING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(()),
            Err(RELEASE_EXPORTED) => Err(host_contract(
                "DMA-BUF release sync FD was already exported for this generation",
            )),
            Err(_) => Err(host_contract(
                "DMA-BUF release sync FD export is already in progress",
            )),
        }
    }

    fn fail(&self) {
        self.0.store(RELEASE_NOT_EXPORTED, Ordering::Release);
    }

    fn complete(&self) {
        self.0.store(RELEASE_EXPORTED, Ordering::Release);
    }

    fn is_resolved(&self) -> bool {
        self.0.load(Ordering::Acquire) == RELEASE_EXPORTED
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg(any(target_os = "linux", test))]
struct DmaBufMetadata {
    drm_fourcc: u32,
    drm_modifier: u64,
    format: vk::Format,
    extent: vk::Extent2D,
    usage: vk::ImageUsageFlags,
    plane_count: usize,
    memory_index: u32,
    offset: u64,
    size: u64,
    row_pitch: u32,
    allocation_size: u64,
    content_version: u64,
    lease_generation: u64,
    color_encoding: ImageColorEncoding,
    alpha_mode: ImageAlphaMode,
    damage_count: usize,
}

#[cfg(any(target_os = "linux", test))]
fn validate_metadata(metadata: DmaBufMetadata, damage: &[RectI]) -> RenderResult<()> {
    if metadata.format == vk::Format::UNDEFINED
        || metadata.extent.width == 0
        || metadata.extent.height == 0
        || metadata.extent.width > i32::MAX as u32
        || metadata.extent.height > i32::MAX as u32
    {
        return Err(invalid_target("DMA-BUF format or extent is invalid"));
    }
    if metadata.drm_modifier == DRM_FORMAT_MOD_INVALID {
        return Err(host_contract("DMA-BUF DRM modifier must be explicit"));
    }
    if metadata.plane_count != 1 || metadata.memory_index != 0 {
        return Err(unsupported(
            "the initial Vulkan DMA-BUF profile supports exactly one RGBA plane",
        ));
    }
    if metadata.offset >= metadata.allocation_size || metadata.size == 0 || metadata.row_pitch == 0
    {
        return Err(host_contract(
            "DMA-BUF plane offset, size, row pitch, or allocation size is invalid",
        ));
    }
    if metadata
        .offset
        .checked_add(metadata.size)
        .is_none_or(|end| end > metadata.allocation_size)
    {
        return Err(host_contract(
            "DMA-BUF plane range exceeds its declared allocation size",
        ));
    }
    let minimum_row_pitch = u64::from(metadata.extent.width)
        .checked_mul(4)
        .ok_or_else(|| invalid_target("DMA-BUF row-pitch calculation overflowed"))?;
    if u64::from(metadata.row_pitch) < minimum_row_pitch {
        return Err(host_contract(
            "DMA-BUF RGBA row pitch is smaller than one pixel row",
        ));
    }
    let minimum_size = u64::from(metadata.row_pitch)
        .checked_mul(u64::from(metadata.extent.height.saturating_sub(1)))
        .and_then(|bytes| bytes.checked_add(minimum_row_pitch))
        .ok_or_else(|| invalid_target("DMA-BUF allocation-size calculation overflowed"))?;
    if minimum_size > metadata.size {
        return Err(host_contract(
            "DMA-BUF row layout exceeds its declared plane size",
        ));
    }
    if metadata.content_version == 0 || metadata.lease_generation == 0 {
        return Err(host_contract(
            "DMA-BUF content version and lease generation must be nonzero",
        ));
    }
    if !metadata.usage.contains(vk::ImageUsageFlags::SAMPLED) {
        return Err(invalid_target("DMA-BUF image must allow sampled use"));
    }
    if !drm_format_matches(
        metadata.drm_fourcc,
        metadata.format,
        metadata.color_encoding,
        metadata.alpha_mode,
    ) {
        return Err(unsupported(
            "DMA-BUF DRM fourcc, Vulkan format, color encoding, and alpha mode disagree",
        ));
    }
    if metadata.damage_count != damage.len() {
        return Err(host_contract("DMA-BUF damage metadata is inconsistent"));
    }
    for rect in damage {
        let right = rect.x.checked_add(rect.width);
        let bottom = rect.y.checked_add(rect.height);
        if rect.x < 0
            || rect.y < 0
            || rect.width <= 0
            || rect.height <= 0
            || right.is_none_or(|value| value > metadata.extent.width as i32)
            || bottom.is_none_or(|value| value > metadata.extent.height as i32)
        {
            return Err(host_contract(
                "DMA-BUF damage lies outside its physical extent",
            ));
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn drm_format_matches(
    fourcc: u32,
    format: vk::Format,
    encoding: ImageColorEncoding,
    alpha: ImageAlphaMode,
) -> bool {
    let rgba = match encoding {
        ImageColorEncoding::Linear => vk::Format::R8G8B8A8_UNORM,
        ImageColorEncoding::Srgb => vk::Format::R8G8B8A8_SRGB,
    };
    let bgra = match encoding {
        ImageColorEncoding::Linear => vk::Format::B8G8R8A8_UNORM,
        ImageColorEncoding::Srgb => vk::Format::B8G8R8A8_SRGB,
    };
    matches!(
        (fourcc, format, alpha),
        (DRM_FORMAT_ABGR8888, value, ImageAlphaMode::Premultiplied) if value == rgba
    ) || matches!(
        (fourcc, format, alpha),
        (DRM_FORMAT_XBGR8888, value, ImageAlphaMode::Opaque) if value == rgba
    ) || matches!(
        (fourcc, format, alpha),
        (DRM_FORMAT_ARGB8888, value, ImageAlphaMode::Premultiplied) if value == bgra
    ) || matches!(
        (fourcc, format, alpha),
        (DRM_FORMAT_XRGB8888, value, ImageAlphaMode::Opaque) if value == bgra
    )
}

#[cfg(any(target_os = "linux", test))]
fn host_contract(message: impl Into<String>) -> RenderError {
    RenderError::new(RenderErrorKind::HostContract, message)
}

#[cfg(any(target_os = "linux", test))]
fn invalid_target(message: impl Into<String>) -> RenderError {
    RenderError::new(RenderErrorKind::InvalidTarget, message)
}

#[cfg(any(target_os = "linux", test))]
fn unsupported(message: impl Into<String>) -> RenderError {
    RenderError::new(RenderErrorKind::Unsupported, message)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::marker::PhantomData;
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
    use std::sync::Arc;

    use crate::foundation::{RectI, SizeI};
    use crate::graphics::render::{
        ImageAlphaMode, ImageColorEncoding, RenderError, RenderErrorKind, RenderResult,
    };
    use ash::vk::{self, Handle};

    use super::{
        DRM_FORMAT_ABGR8888, DRM_FORMAT_ARGB8888, DRM_FORMAT_XBGR8888, DRM_FORMAT_XRGB8888,
        DmaBufMetadata, ReleaseExportState, validate_metadata,
    };
    use crate::graphics::renderers::vulkan::device::DeviceInner;
    use crate::graphics::renderers::vulkan::error::{unsupported, vk_error};
    use crate::graphics::renderers::vulkan::external_image::{
        ExternalImageInner, ExternalImageOwnership, UNUSED, VulkanExternalAcquire,
        VulkanExternalImageLease, VulkanExternalImageOrigin, VulkanExternalRelease,
    };
    use crate::graphics::renderers::vulkan::target::{VulkanImageState, VulkanTarget};
    use crate::graphics::renderers::vulkan::{HostedImageUse, VulkanDevice};

    pub(super) fn scanout_initial_layout(initialized: bool) -> vk::ImageLayout {
        if initialized {
            vk::ImageLayout::GENERAL
        } else {
            vk::ImageLayout::UNDEFINED
        }
    }

    #[derive(Debug)]
    pub struct VulkanDmaBufPlane {
        pub memory: OwnedFd,
        pub memory_index: u32,
        pub offset: u64,
        pub size: u64,
        pub row_pitch: u32,
        pub allocation_size: u64,
    }

    /// One exact Linux DMA-BUF tuple supported for importing with the requested image usage.
    ///
    /// Hosts should intersect these records with the producer or protocol capabilities and pass
    /// the selected tuple back unchanged in [`VulkanDmaBufImport`]. This initial profile exposes
    /// only single-memory-plane, 8-bit RGBA/BGRA formats; multi-plane and YCbCr negotiation remain
    /// unsupported.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub struct VulkanDmaBufFormatCapability {
        pub drm_fourcc: u32,
        pub drm_modifier: u64,
        pub format: vk::Format,
        pub usage: vk::ImageUsageFlags,
        pub color_encoding: ImageColorEncoding,
        pub alpha_mode: ImageAlphaMode,
        pub plane_count: u32,
        pub tiling_features: vk::FormatFeatureFlags,
        pub external_memory_features: vk::ExternalMemoryFeatureFlags,
        pub max_extent: vk::Extent3D,
    }

    impl VulkanDmaBufFormatCapability {
        pub fn importable(self) -> bool {
            self.external_memory_features
                .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
        }

        pub fn exportable(self) -> bool {
            self.external_memory_features
                .contains(vk::ExternalMemoryFeatureFlags::EXPORTABLE)
        }

        pub fn dedicated_only(self) -> bool {
            self.external_memory_features
                .contains(vk::ExternalMemoryFeatureFlags::DEDICATED_ONLY)
        }
    }

    /// One owning Linux DMA-BUF generation. Every FD is closed on rejection, transferred to Vulkan
    /// on successful import, or returned as an owning release sync FD.
    pub struct VulkanDmaBufImport {
        pub planes: Vec<VulkanDmaBufPlane>,
        pub drm_fourcc: u32,
        pub drm_modifier: u64,
        pub format: vk::Format,
        pub extent: vk::Extent2D,
        pub usage: vk::ImageUsageFlags,
        pub content_version: u64,
        pub lease_generation: u64,
        pub color_encoding: ImageColorEncoding,
        pub alpha_mode: ImageAlphaMode,
        pub origin: VulkanExternalImageOrigin,
        pub initial_use: HostedImageUse,
        pub final_use: HostedImageUse,
        pub acquire: Option<OwnedFd>,
        pub damage: Vec<RectI>,
        pub protected: bool,
    }

    #[derive(Debug)]
    pub struct VulkanDmaBufReleaseSyncFd {
        pub content_version: u64,
        pub lease_generation: u64,
        pub sync_fd: OwnedFd,
    }

    /// One GBM/KMS-owned single-plane DMA-BUF imported as an owned Telorgon Vulkan color target.
    ///
    /// The target retains only the imported Vulkan handles. The GBM buffer and KMS framebuffer
    /// that own the allocation must outlive this value.
    pub struct VulkanDmaBufScanoutTarget {
        device: Arc<DeviceInner>,
        image: vk::Image,
        view: vk::ImageView,
        memory: vk::DeviceMemory,
        format: vk::Format,
        extent: vk::Extent2D,
        initialized: bool,
    }

    impl VulkanDmaBufScanoutTarget {
        /// Exact render-target candidates, queried before GBM allocation.
        pub(crate) fn supported_modifiers(
            device: &VulkanDevice,
            fourcc: u32,
            extent: SizeI,
        ) -> RenderResult<Vec<u64>> {
            if !device.inner.owned_dma_buf_targets {
                return Ok(Vec::new());
            }
            let mut result = Vec::new();
            for candidate in format_candidates()
                .into_iter()
                .filter(|c| c.drm_fourcc == fourcc)
            {
                for properties in query_modifiers(device, candidate.format) {
                    if properties.drm_format_modifier == super::DRM_FORMAT_MOD_INVALID
                        || properties.drm_format_modifier_plane_count != 1
                        || !properties
                            .drm_format_modifier_tiling_features
                            .contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT)
                    {
                        continue;
                    }
                    let Some(support) = try_query_external_image_support(
                        device,
                        candidate.format,
                        properties.drm_format_modifier,
                        vk::ImageUsageFlags::COLOR_ATTACHMENT,
                    )?
                    else {
                        continue;
                    };
                    if support
                        .external
                        .external_memory_features
                        .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
                        && support
                            .external
                            .compatible_handle_types
                            .contains(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
                        && extent.width > 0
                        && extent.height > 0
                        && extent.width as u32 <= support.max_extent.width
                        && extent.height as u32 <= support.max_extent.height
                    {
                        result.push(properties.drm_format_modifier);
                    }
                }
            }
            result.sort_unstable();
            result.dedup();
            Ok(result)
        }

        /// Imports a single-plane GBM allocation for Vulkan rendering and DRM/KMS scanout.
        ///
        /// # Safety
        ///
        /// `memory` must be a DMA-BUF FD for the declared fourcc, modifier, extent, offset, and
        /// row pitch. The originating GBM allocation must stay alive until this target is dropped,
        /// and KMS/Vulkan access must be externally ordered. Telorgon's managed KMS runner satisfies
        /// that ordering by waiting for Vulkan completion before each blocking atomic commit.
        pub unsafe fn import(
            device: &VulkanDevice,
            memory: OwnedFd,
            drm_fourcc: u32,
            drm_modifier: u64,
            extent: SizeI,
            offset: u64,
            row_pitch: u32,
        ) -> RenderResult<Self> {
            if !device.inner.owned_dma_buf_targets {
                return Err(unsupported(
                    "owned Vulkan device lacks DMA-BUF render-target extensions",
                ));
            }
            if extent.width <= 0
                || extent.height <= 0
                || row_pitch < extent.width as u32 * 4
                || drm_modifier == super::DRM_FORMAT_MOD_INVALID
            {
                return Err(RenderError::new(
                    RenderErrorKind::InvalidTarget,
                    "DMA-BUF scanout target metadata is invalid",
                ));
            }
            let extent_vk = vk::Extent2D {
                width: extent.width as u32,
                height: extent.height as u32,
            };
            let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT;
            let required = vk::FormatFeatureFlags::COLOR_ATTACHMENT;
            let mut selected = None;
            for candidate in format_candidates().into_iter().rev() {
                if candidate.drm_fourcc != drm_fourcc
                    || !query_modifiers(device, candidate.format)
                        .into_iter()
                        .any(|properties| {
                            properties.drm_format_modifier == drm_modifier
                                && properties.drm_format_modifier_plane_count == 1
                                && properties
                                    .drm_format_modifier_tiling_features
                                    .contains(required)
                        })
                {
                    continue;
                }
                let Some(support) = try_query_external_image_support(
                    device,
                    candidate.format,
                    drm_modifier,
                    usage,
                )?
                else {
                    continue;
                };
                if support
                    .external
                    .external_memory_features
                    .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
                    && support
                        .external
                        .compatible_handle_types
                        .contains(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
                    && extent_vk.width <= support.max_extent.width
                    && extent_vk.height <= support.max_extent.height
                {
                    selected = Some(candidate);
                    break;
                }
            }
            let candidate = selected.ok_or_else(|| {
                unsupported("adapter cannot render to the GBM DMA-BUF fourcc/modifier tuple")
            })?;

            let layouts = [vk::SubresourceLayout {
                offset,
                size: 0,
                row_pitch: u64::from(row_pitch),
                array_pitch: 0,
                depth_pitch: 0,
            }];
            let mut external_image = vk::ExternalMemoryImageCreateInfo::default()
                .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let mut drm_layout = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
                .drm_format_modifier(drm_modifier)
                .plane_layouts(&layouts);
            let create_info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(candidate.format)
                .extent(vk::Extent3D {
                    width: extent_vk.width,
                    height: extent_vk.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .push_next(&mut external_image)
                .push_next(&mut drm_layout);
            let mut guard = ScanoutImportGuard::new(Arc::clone(&device.inner));
            guard.image = unsafe { device.inner.raw.create_image(&create_info, None) }
                .map_err(|result| vk_error("failed to create DMA-BUF scanout image", result))?;

            let requirements_info = vk::ImageMemoryRequirementsInfo2::default().image(guard.image);
            let mut dedicated_requirements = vk::MemoryDedicatedRequirements::default();
            let mut requirements =
                vk::MemoryRequirements2::default().push_next(&mut dedicated_requirements);
            unsafe {
                device
                    .inner
                    .raw
                    .get_image_memory_requirements2(&requirements_info, &mut requirements);
            }
            let memory_fd_loader = ash::khr::external_memory_fd::Device::new(
                &device.inner.instance.inner.raw,
                &device.inner.raw,
            );
            let mut fd_properties = vk::MemoryFdPropertiesKHR::default();
            unsafe {
                memory_fd_loader.get_memory_fd_properties(
                    vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                    memory.as_raw_fd(),
                    &mut fd_properties,
                )
            }
            .map_err(|result| {
                vk_error(
                    "failed to query scanout DMA-BUF memory compatibility",
                    result,
                )
            })?;
            let compatible =
                requirements.memory_requirements.memory_type_bits & fd_properties.memory_type_bits;
            let memory_type_index = choose_memory_type(device, compatible)?;
            let raw_memory_fd = memory.into_raw_fd();
            let mut import_memory = vk::ImportMemoryFdInfoKHR::default()
                .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
                .fd(raw_memory_fd);
            let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(guard.image);
            let allocation = vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.memory_requirements.size)
                .memory_type_index(memory_type_index)
                .push_next(&mut import_memory)
                .push_next(&mut dedicated);
            guard.memory = match unsafe { device.inner.raw.allocate_memory(&allocation, None) } {
                Ok(memory) => memory,
                Err(result) => {
                    drop(unsafe { OwnedFd::from_raw_fd(raw_memory_fd) });
                    return Err(vk_error("failed to import scanout DMA-BUF memory", result));
                }
            };
            unsafe {
                device
                    .inner
                    .raw
                    .bind_image_memory2(&[vk::BindImageMemoryInfo::default()
                        .image(guard.image)
                        .memory(guard.memory)
                        .memory_offset(0)])
            }
            .map_err(|result| vk_error("failed to bind scanout DMA-BUF memory", result))?;
            guard.view = unsafe {
                device.inner.raw.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(guard.image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(candidate.format)
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        }),
                    None,
                )
            }
            .map_err(|result| vk_error("failed to create scanout image view", result))?;
            let result = Self {
                device: Arc::clone(&device.inner),
                image: guard.image,
                view: guard.view,
                memory: guard.memory,
                format: candidate.format,
                extent: extent_vk,
                initialized: false,
            };
            guard.armed = false;
            Ok(result)
        }

        pub fn target(&mut self) -> VulkanTarget<'_> {
            let extent = SizeI {
                width: self.extent.width as i32,
                height: self.extent.height as i32,
            };
            VulkanTarget {
                device_id: self.device.id,
                image: self.image,
                view: self.view,
                format: self.format,
                extent: self.extent,
                info: crate::graphics::render::RenderTargetInfo {
                    color_space: if matches!(
                        self.format,
                        vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB
                    ) {
                        crate::graphics::render::ColorSpace::Srgb
                    } else {
                        crate::graphics::render::ColorSpace::Linear
                    },
                    alpha_mode: crate::graphics::render::AlphaMode::Opaque,
                    ..crate::graphics::render::RenderTargetInfo::full(extent)
                },
                initial_state: VulkanImageState {
                    layout: scanout_initial_layout(self.initialized),
                    stage: vk::PipelineStageFlags2::NONE,
                    access: vk::AccessFlags2::NONE,
                },
                final_state: VulkanImageState {
                    layout: vk::ImageLayout::GENERAL,
                    stage: vk::PipelineStageFlags2::NONE,
                    access: vk::AccessFlags2::NONE,
                },
                initial_queue_family: vk::QUEUE_FAMILY_FOREIGN_EXT,
                final_queue_family: vk::QUEUE_FAMILY_FOREIGN_EXT,
                _borrow: PhantomData,
            }
        }

        /// Records that a submitted frame transitioned this imported image out of its creation
        /// layout. Subsequent acquisitions must preserve the KMS-owned contents in `GENERAL`.
        pub(crate) fn mark_initialized(&mut self) {
            self.initialized = true;
        }
    }

    impl Drop for VulkanDmaBufScanoutTarget {
        fn drop(&mut self) {
            unsafe {
                self.device.raw.destroy_image_view(self.view, None);
                self.device.raw.destroy_image(self.image, None);
                self.device.raw.free_memory(self.memory, None);
            }
        }
    }

    struct ScanoutImportGuard {
        device: Arc<DeviceInner>,
        image: vk::Image,
        view: vk::ImageView,
        memory: vk::DeviceMemory,
        armed: bool,
    }

    impl ScanoutImportGuard {
        fn new(device: Arc<DeviceInner>) -> Self {
            Self {
                device,
                image: vk::Image::null(),
                view: vk::ImageView::null(),
                memory: vk::DeviceMemory::null(),
                armed: true,
            }
        }
    }

    impl Drop for ScanoutImportGuard {
        fn drop(&mut self) {
            if !self.armed {
                return;
            }
            unsafe {
                if !self.view.is_null() {
                    self.device.raw.destroy_image_view(self.view, None);
                }
                if !self.image.is_null() {
                    self.device.raw.destroy_image(self.image, None);
                }
                if !self.memory.is_null() {
                    self.device.raw.free_memory(self.memory, None);
                }
            }
        }
    }

    pub(crate) struct DmaBufOwnedResources {
        device: Arc<DeviceInner>,
        image: vk::Image,
        view: vk::ImageView,
        memory: vk::DeviceMemory,
        acquire: vk::Semaphore,
        release: vk::Semaphore,
        release_state: ReleaseExportState,
    }

    impl DmaBufOwnedResources {
        // Called only with exclusive ownership after completion and release export. The old
        // temporary acquire payload was consumed by its wait; export consumed the release payload.
        pub(crate) fn rearm(
            &mut self,
            acquire: Option<OwnedFd>,
        ) -> RenderResult<VulkanExternalAcquire> {
            let result = if let Some(fd) = acquire {
                if self.acquire.is_null() {
                    self.acquire = unsafe {
                        self.device
                            .raw
                            .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                    }
                    .map_err(|e| {
                        vk_error("failed to create cached DMA-BUF acquire semaphore", e)
                    })?;
                }
                let loader = ash::khr::external_semaphore_fd::Device::new(
                    &self.device.instance.inner.raw,
                    &self.device.raw,
                );
                let raw_fd = fd.into_raw_fd();
                let info = vk::ImportSemaphoreFdInfoKHR::default()
                    .semaphore(self.acquire)
                    .flags(vk::SemaphoreImportFlags::TEMPORARY)
                    .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
                    .fd(raw_fd);
                if let Err(e) = unsafe { loader.import_semaphore_fd(&info) } {
                    drop(unsafe { OwnedFd::from_raw_fd(raw_fd) });
                    return Err(vk_error(
                        "failed to rearm cached DMA-BUF acquire payload",
                        e,
                    ));
                }
                VulkanExternalAcquire::BinarySemaphore(self.acquire)
            } else {
                VulkanExternalAcquire::CommandStream
            };
            self.release_state = ReleaseExportState::new();
            Ok(result)
        }

        pub(crate) fn release_is_resolved(&self) -> bool {
            self.release_state.is_resolved()
        }

        /// # Safety
        ///
        /// The host must have submitted a wait/signal operation containing the receipt's returned
        /// semaphores before export and must externally synchronize access to that submission and
        /// this call. A sync-FD export consumes the semaphore's temporary signal payload once.
        pub(crate) unsafe fn export_release_sync_fd(
            &self,
            content_version: u64,
            lease_generation: u64,
        ) -> RenderResult<Option<VulkanDmaBufReleaseSyncFd>> {
            self.release_state.begin()?;
            let loader = ash::khr::external_semaphore_fd::Device::new(
                &self.device.instance.inner.raw,
                &self.device.raw,
            );
            let info = vk::SemaphoreGetFdInfoKHR::default()
                .semaphore(self.release)
                .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
            let fd = match unsafe { loader.get_semaphore_fd(&info) } {
                Ok(fd) => fd,
                Err(result) => {
                    self.release_state.fail();
                    return Err(vk_error(
                        "failed to export DMA-BUF release sync FD after host submission",
                        result,
                    ));
                }
            };
            self.release_state.complete();
            Ok(Some(VulkanDmaBufReleaseSyncFd {
                content_version,
                lease_generation,
                sync_fd: unsafe { OwnedFd::from_raw_fd(fd) },
            }))
        }
    }

    impl Drop for DmaBufOwnedResources {
        fn drop(&mut self) {
            unsafe {
                self.device.raw.destroy_semaphore(self.release, None);
                if !self.acquire.is_null() {
                    self.device.raw.destroy_semaphore(self.acquire, None);
                }
                self.device.raw.destroy_image_view(self.view, None);
                self.device.raw.destroy_image(self.image, None);
                self.device.raw.free_memory(self.memory, None);
            }
        }
    }

    struct ImportGuard {
        device: Arc<DeviceInner>,
        image: vk::Image,
        view: vk::ImageView,
        memory: vk::DeviceMemory,
        acquire: vk::Semaphore,
        release: vk::Semaphore,
        armed: bool,
    }

    impl ImportGuard {
        fn new(device: Arc<DeviceInner>) -> Self {
            Self {
                device,
                image: vk::Image::null(),
                view: vk::ImageView::null(),
                memory: vk::DeviceMemory::null(),
                acquire: vk::Semaphore::null(),
                release: vk::Semaphore::null(),
                armed: true,
            }
        }

        fn finish(mut self) -> DmaBufOwnedResources {
            self.armed = false;
            DmaBufOwnedResources {
                device: Arc::clone(&self.device),
                image: self.image,
                view: self.view,
                memory: self.memory,
                acquire: self.acquire,
                release: self.release,
                release_state: ReleaseExportState::new(),
            }
        }
    }

    impl Drop for ImportGuard {
        fn drop(&mut self) {
            if !self.armed {
                return;
            }
            unsafe {
                if !self.release.is_null() {
                    self.device.raw.destroy_semaphore(self.release, None);
                }
                if !self.acquire.is_null() {
                    self.device.raw.destroy_semaphore(self.acquire, None);
                }
                if !self.view.is_null() {
                    self.device.raw.destroy_image_view(self.view, None);
                }
                if !self.image.is_null() {
                    self.device.raw.destroy_image(self.image, None);
                }
                if !self.memory.is_null() {
                    self.device.raw.free_memory(self.memory, None);
                }
            }
        }
    }

    #[derive(Copy, Clone)]
    struct FormatCandidate {
        drm_fourcc: u32,
        format: vk::Format,
        color_encoding: ImageColorEncoding,
        alpha_mode: ImageAlphaMode,
    }

    fn format_candidates() -> [FormatCandidate; 8] {
        [
            FormatCandidate {
                drm_fourcc: DRM_FORMAT_ABGR8888,
                format: vk::Format::R8G8B8A8_UNORM,
                color_encoding: ImageColorEncoding::Linear,
                alpha_mode: ImageAlphaMode::Premultiplied,
            },
            FormatCandidate {
                drm_fourcc: DRM_FORMAT_ABGR8888,
                format: vk::Format::R8G8B8A8_SRGB,
                color_encoding: ImageColorEncoding::Srgb,
                alpha_mode: ImageAlphaMode::Premultiplied,
            },
            FormatCandidate {
                drm_fourcc: DRM_FORMAT_XBGR8888,
                format: vk::Format::R8G8B8A8_UNORM,
                color_encoding: ImageColorEncoding::Linear,
                alpha_mode: ImageAlphaMode::Opaque,
            },
            FormatCandidate {
                drm_fourcc: DRM_FORMAT_XBGR8888,
                format: vk::Format::R8G8B8A8_SRGB,
                color_encoding: ImageColorEncoding::Srgb,
                alpha_mode: ImageAlphaMode::Opaque,
            },
            FormatCandidate {
                drm_fourcc: DRM_FORMAT_ARGB8888,
                format: vk::Format::B8G8R8A8_UNORM,
                color_encoding: ImageColorEncoding::Linear,
                alpha_mode: ImageAlphaMode::Premultiplied,
            },
            FormatCandidate {
                drm_fourcc: DRM_FORMAT_ARGB8888,
                format: vk::Format::B8G8R8A8_SRGB,
                color_encoding: ImageColorEncoding::Srgb,
                alpha_mode: ImageAlphaMode::Premultiplied,
            },
            FormatCandidate {
                drm_fourcc: DRM_FORMAT_XRGB8888,
                format: vk::Format::B8G8R8A8_UNORM,
                color_encoding: ImageColorEncoding::Linear,
                alpha_mode: ImageAlphaMode::Opaque,
            },
            FormatCandidate {
                drm_fourcc: DRM_FORMAT_XRGB8888,
                format: vk::Format::B8G8R8A8_SRGB,
                color_encoding: ImageColorEncoding::Srgb,
                alpha_mode: ImageAlphaMode::Opaque,
            },
        ]
    }

    impl VulkanDevice {
        /// Returns the exact single-plane DMA-BUF tuples this device can import for
        /// `usage`. An empty list is a valid result when the complete Linux interop extension set
        /// exists but no supported RGBA/BGRA tuple satisfies the requested usage.
        pub fn dma_buf_import_capabilities(
            &self,
            usage: vk::ImageUsageFlags,
        ) -> RenderResult<Vec<VulkanDmaBufFormatCapability>> {
            if !super::device_supports_linux_dma_buf(self) {
                return Err(unsupported(
                    "Vulkan device lacks the complete DMA-BUF/modifier/sync-FD contract",
                ));
            }
            if !usage.contains(vk::ImageUsageFlags::SAMPLED) {
                return Err(RenderError::new(
                    RenderErrorKind::InvalidTarget,
                    "DMA-BUF negotiation usage must include sampled access",
                ));
            }

            let required_features = vk::FormatFeatureFlags::SAMPLED_IMAGE
                | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
            let mut capabilities = Vec::new();
            for candidate in format_candidates() {
                for modifier in query_modifiers(self, candidate.format) {
                    if modifier.drm_format_modifier_plane_count != 1
                        || !modifier
                            .drm_format_modifier_tiling_features
                            .contains(required_features)
                    {
                        continue;
                    }
                    let Some(support) = try_query_external_image_support(
                        self,
                        candidate.format,
                        modifier.drm_format_modifier,
                        usage,
                    )?
                    else {
                        continue;
                    };
                    if !support
                        .external
                        .compatible_handle_types
                        .contains(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
                        || !support
                            .external
                            .external_memory_features
                            .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
                    {
                        continue;
                    }
                    capabilities.push(VulkanDmaBufFormatCapability {
                        drm_fourcc: candidate.drm_fourcc,
                        drm_modifier: modifier.drm_format_modifier,
                        format: candidate.format,
                        usage,
                        color_encoding: candidate.color_encoding,
                        alpha_mode: candidate.alpha_mode,
                        plane_count: modifier.drm_format_modifier_plane_count,
                        tiling_features: modifier.drm_format_modifier_tiling_features,
                        external_memory_features: support.external.external_memory_features,
                        max_extent: support.max_extent,
                    });
                }
            }
            capabilities.sort_by_key(|capability| {
                (
                    capability.drm_fourcc,
                    capability.format.as_raw(),
                    capability.drm_modifier,
                )
            });
            capabilities.dedup_by_key(|capability| {
                (
                    capability.drm_fourcc,
                    capability.format.as_raw(),
                    capability.drm_modifier,
                )
            });
            Ok(capabilities)
        }

        /// Imports one Linux DMA-BUF generation into a Vulkan device without a CPU copy.
        ///
        /// # Safety
        ///
        /// The host declarations must match the producer's DRM fourcc/modifier/plane layout and
        /// synchronization contract. The FDs must refer to the declared allocation and acquire
        /// fence. The host must submit returned waits/signals, export release synchronization from
        /// the receipt after submission, and commit the receipt to real completion.
        pub unsafe fn import_dma_buf(
            &self,
            mut import: VulkanDmaBufImport,
        ) -> RenderResult<VulkanExternalImageLease> {
            if !super::device_supports_linux_dma_buf(self) {
                return Err(unsupported(
                    "Vulkan device lacks the complete DMA-BUF/modifier/sync-FD contract",
                ));
            }
            if import.protected {
                return Err(unsupported(
                    "protected DMA-BUF content is not supported by this Vulkan profile",
                ));
            }
            if import.origin != VulkanExternalImageOrigin::TopLeft {
                return Err(unsupported(
                    "bottom-left DMA-BUF origin requires an explicit normalization path",
                ));
            }
            if import.initial_use != HostedImageUse::General
                || import.final_use != HostedImageUse::General
            {
                return Err(host_contract(
                    "the initial DMA-BUF profile requires GENERAL layout at foreign acquire/release",
                ));
            }
            let plane = import
                .planes
                .first()
                .ok_or_else(|| host_contract("DMA-BUF import has no planes"))?;
            validate_metadata(
                DmaBufMetadata {
                    drm_fourcc: import.drm_fourcc,
                    drm_modifier: import.drm_modifier,
                    format: import.format,
                    extent: import.extent,
                    usage: import.usage,
                    plane_count: import.planes.len(),
                    memory_index: plane.memory_index,
                    offset: plane.offset,
                    size: plane.size,
                    row_pitch: plane.row_pitch,
                    allocation_size: plane.allocation_size,
                    content_version: import.content_version,
                    lease_generation: import.lease_generation,
                    color_encoding: import.color_encoding,
                    alpha_mode: import.alpha_mode,
                    damage_count: import.damage.len(),
                },
                &import.damage,
            )?;
            let modifier = query_modifiers(self, import.format)
                .into_iter()
                .find(|properties| properties.drm_format_modifier == import.drm_modifier)
                .ok_or_else(|| {
                    unsupported("adapter does not support the requested DMA-BUF DRM modifier")
                })?;
            if modifier.drm_format_modifier_plane_count != import.planes.len() as u32 {
                return Err(unsupported(
                    "DMA-BUF plane count does not match the adapter's DRM modifier properties",
                ));
            }
            if !modifier.drm_format_modifier_tiling_features.contains(
                vk::FormatFeatureFlags::SAMPLED_IMAGE
                    | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR,
            ) {
                return Err(unsupported(
                    "DMA-BUF modifier cannot be linearly sampled by this adapter",
                ));
            }
            query_external_image_support(
                self,
                import.format,
                import.drm_modifier,
                import.usage,
                import.extent,
            )?;

            let plane = import.planes.pop().expect("one plane validated");
            let layouts = [vk::SubresourceLayout {
                offset: plane.offset,
                // VK_EXT_image_drm_format_modifier requires `size` to be zero on explicit
                // import; the implementation derives the plane size from format, extent, offset,
                // and row pitch. `plane.size` remains host validation metadata above.
                size: 0,
                row_pitch: u64::from(plane.row_pitch),
                array_pitch: 0,
                depth_pitch: 0,
            }];
            let mut external_image = vk::ExternalMemoryImageCreateInfo::default()
                .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let mut drm_layout = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
                .drm_format_modifier(import.drm_modifier)
                .plane_layouts(&layouts);
            let create_info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(import.format)
                .extent(vk::Extent3D {
                    width: import.extent.width,
                    height: import.extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                .usage(import.usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .push_next(&mut external_image)
                .push_next(&mut drm_layout);
            let mut guard = ImportGuard::new(Arc::clone(&self.inner));
            guard.image = unsafe { self.inner.raw.create_image(&create_info, None) }
                .map_err(|result| vk_error("failed to create DMA-BUF Vulkan image", result))?;

            let requirements_info = vk::ImageMemoryRequirementsInfo2::default().image(guard.image);
            let mut dedicated_requirements = vk::MemoryDedicatedRequirements::default();
            let mut requirements =
                vk::MemoryRequirements2::default().push_next(&mut dedicated_requirements);
            unsafe {
                self.inner
                    .raw
                    .get_image_memory_requirements2(&requirements_info, &mut requirements);
            }
            if plane.allocation_size < requirements.memory_requirements.size {
                return Err(host_contract(
                    "DMA-BUF allocation is smaller than Vulkan image memory requirements",
                ));
            }
            let memory_fd_loader = ash::khr::external_memory_fd::Device::new(
                &self.inner.instance.inner.raw,
                &self.inner.raw,
            );
            let mut fd_properties = vk::MemoryFdPropertiesKHR::default();
            unsafe {
                memory_fd_loader.get_memory_fd_properties(
                    vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                    plane.memory.as_raw_fd(),
                    &mut fd_properties,
                )
            }
            .map_err(|result| {
                vk_error(
                    "failed to query DMA-BUF Vulkan memory type compatibility",
                    result,
                )
            })?;
            let compatible =
                requirements.memory_requirements.memory_type_bits & fd_properties.memory_type_bits;
            let memory_type_index = choose_memory_type(self, compatible)?;
            let raw_memory_fd = plane.memory.into_raw_fd();
            let mut import_memory = vk::ImportMemoryFdInfoKHR::default()
                .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
                .fd(raw_memory_fd);
            let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(guard.image);
            let allocation = vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.memory_requirements.size)
                .memory_type_index(memory_type_index)
                .push_next(&mut import_memory)
                .push_next(&mut dedicated);
            guard.memory = match unsafe { self.inner.raw.allocate_memory(&allocation, None) } {
                Ok(memory) => memory,
                Err(result) => {
                    drop(unsafe { OwnedFd::from_raw_fd(raw_memory_fd) });
                    return Err(vk_error("failed to import DMA-BUF Vulkan memory", result));
                }
            };
            unsafe {
                self.inner
                    .raw
                    .bind_image_memory2(&[vk::BindImageMemoryInfo::default()
                        .image(guard.image)
                        .memory(guard.memory)
                        .memory_offset(0)])
            }
            .map_err(|result| vk_error("failed to bind imported DMA-BUF memory", result))?;
            guard.view = unsafe {
                self.inner.raw.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(guard.image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(import.format)
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        }),
                    None,
                )
            }
            .map_err(|result| vk_error("failed to create DMA-BUF Vulkan image view", result))?;

            if let Some(acquire_fd) = import.acquire.take() {
                guard.acquire = unsafe {
                    self.inner
                        .raw
                        .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                }
                .map_err(|result| vk_error("failed to create DMA-BUF acquire semaphore", result))?;
                let raw_acquire_fd = acquire_fd.into_raw_fd();
                let semaphore_fd_loader = ash::khr::external_semaphore_fd::Device::new(
                    &self.inner.instance.inner.raw,
                    &self.inner.raw,
                );
                let acquire_info = vk::ImportSemaphoreFdInfoKHR::default()
                    .semaphore(guard.acquire)
                    .flags(vk::SemaphoreImportFlags::TEMPORARY)
                    .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
                    .fd(raw_acquire_fd);
                if let Err(result) =
                    unsafe { semaphore_fd_loader.import_semaphore_fd(&acquire_info) }
                {
                    drop(unsafe { OwnedFd::from_raw_fd(raw_acquire_fd) });
                    return Err(vk_error("failed to import DMA-BUF acquire sync FD", result));
                }
            }
            let mut export = vk::ExportSemaphoreCreateInfo::default()
                .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
            guard.release = unsafe {
                self.inner.raw.create_semaphore(
                    &vk::SemaphoreCreateInfo::default().push_next(&mut export),
                    None,
                )
            }
            .map_err(|result| vk_error("failed to create DMA-BUF release semaphore", result))?;

            let image = guard.image;
            let view = guard.view;
            let acquire = guard.acquire;
            let release = guard.release;
            let resources = guard.finish();
            Ok(VulkanExternalImageLease::from_inner(ExternalImageInner {
                device_id: self.inner.id,
                image,
                view,
                format: import.format,
                extent: SizeI {
                    width: import.extent.width as i32,
                    height: import.extent.height as i32,
                },
                content_version: import.content_version,
                lease_generation: import.lease_generation,
                color_encoding: import.color_encoding,
                alpha_mode: import.alpha_mode,
                initial_use: import.initial_use,
                final_use: import.final_use,
                initial_queue_family: vk::QUEUE_FAMILY_FOREIGN_EXT,
                final_queue_family: vk::QUEUE_FAMILY_FOREIGN_EXT,
                acquire: if acquire.is_null() {
                    VulkanExternalAcquire::CommandStream
                } else {
                    VulkanExternalAcquire::BinarySemaphore(acquire)
                },
                release: VulkanExternalRelease::BinarySemaphore(release),
                damage: import.damage,
                ownership: ExternalImageOwnership::DmaBuf(resources),
                state: std::sync::atomic::AtomicU64::new(UNUSED),
            }))
        }
    }

    fn query_modifiers(
        device: &VulkanDevice,
        format: vk::Format,
    ) -> Vec<vk::DrmFormatModifierPropertiesEXT> {
        let mut count_list = vk::DrmFormatModifierPropertiesListEXT::default();
        let mut properties = vk::FormatProperties2::default().push_next(&mut count_list);
        unsafe {
            device
                .inner
                .instance
                .inner
                .raw
                .get_physical_device_format_properties2(
                    device.inner.physical_device,
                    format,
                    &mut properties,
                );
        }
        let mut modifiers = vec![
            vk::DrmFormatModifierPropertiesEXT::default();
            count_list.drm_format_modifier_count as usize
        ];
        let returned_count = {
            let mut value_list = vk::DrmFormatModifierPropertiesListEXT::default()
                .drm_format_modifier_properties(&mut modifiers);
            let mut properties = vk::FormatProperties2::default().push_next(&mut value_list);
            unsafe {
                device
                    .inner
                    .instance
                    .inner
                    .raw
                    .get_physical_device_format_properties2(
                        device.inner.physical_device,
                        format,
                        &mut properties,
                    );
            }
            value_list.drm_format_modifier_count as usize
        };
        modifiers.truncate(returned_count.min(modifiers.len()));
        modifiers
    }

    #[derive(Copy, Clone)]
    struct ExternalImageSupport {
        external: vk::ExternalMemoryProperties,
        max_extent: vk::Extent3D,
    }

    fn try_query_external_image_support(
        device: &VulkanDevice,
        format: vk::Format,
        modifier: u64,
        usage: vk::ImageUsageFlags,
    ) -> RenderResult<Option<ExternalImageSupport>> {
        let mut drm_info = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
            .drm_format_modifier(modifier)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let mut external_info = vk::PhysicalDeviceExternalImageFormatInfo::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let info = vk::PhysicalDeviceImageFormatInfo2::default()
            .format(format)
            .ty(vk::ImageType::TYPE_2D)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(usage)
            .flags(vk::ImageCreateFlags::empty())
            .push_next(&mut drm_info)
            .push_next(&mut external_info);
        let mut external_properties = vk::ExternalImageFormatProperties::default();
        let mut properties =
            vk::ImageFormatProperties2::default().push_next(&mut external_properties);
        match unsafe {
            device
                .inner
                .instance
                .inner
                .raw
                .get_physical_device_image_format_properties2(
                    device.inner.physical_device,
                    &info,
                    &mut properties,
                )
        } {
            Ok(()) => {}
            Err(vk::Result::ERROR_FORMAT_NOT_SUPPORTED) => return Ok(None),
            Err(result) => {
                return Err(vk_error("failed to query DMA-BUF image support", result));
            }
        }
        let max_extent = properties.image_format_properties.max_extent;
        Ok(Some(ExternalImageSupport {
            external: external_properties.external_memory_properties,
            max_extent,
        }))
    }

    fn query_external_image_support(
        device: &VulkanDevice,
        format: vk::Format,
        modifier: u64,
        usage: vk::ImageUsageFlags,
        extent: vk::Extent2D,
    ) -> RenderResult<()> {
        let support = try_query_external_image_support(device, format, modifier, usage)?
            .ok_or_else(|| {
                unsupported("adapter rejects the requested DMA-BUF image format/modifier/usage")
            })?;
        let external = support.external;
        if !external
            .external_memory_features
            .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
            || !external
                .compatible_handle_types
                .contains(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
        {
            return Err(unsupported(
                "adapter cannot import DMA-BUF memory for the requested image",
            ));
        }
        if extent.width > support.max_extent.width || extent.height > support.max_extent.height {
            return Err(RenderError::new(
                RenderErrorKind::InvalidTarget,
                "DMA-BUF extent exceeds adapter image-format limits",
            ));
        }
        Ok(())
    }

    fn choose_memory_type(device: &VulkanDevice, compatible: u32) -> RenderResult<u32> {
        if compatible == 0 {
            return Err(unsupported(
                "DMA-BUF memory has no Vulkan-compatible memory type",
            ));
        }
        let properties = unsafe {
            device
                .inner
                .instance
                .inner
                .raw
                .get_physical_device_memory_properties(device.inner.physical_device)
        };
        let candidates = (0..properties.memory_type_count)
            .filter(|index| compatible & (1 << index) != 0)
            .collect::<Vec<_>>();
        candidates
            .iter()
            .copied()
            .find(|index| {
                properties.memory_types[*index as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .or_else(|| candidates.first().copied())
            .ok_or_else(|| unsupported("DMA-BUF memory type selection failed"))
    }

    fn host_contract(message: impl Into<String>) -> RenderError {
        RenderError::new(RenderErrorKind::HostContract, message)
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::DmaBufOwnedResources;
#[cfg(target_os = "linux")]
pub use linux::{
    VulkanDmaBufFormatCapability, VulkanDmaBufImport, VulkanDmaBufPlane, VulkanDmaBufReleaseSyncFd,
    VulkanDmaBufScanoutTarget,
};

#[cfg(test)]
mod tests;
