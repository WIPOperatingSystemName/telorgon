use crate::foundation::SizeI;
use crate::graphics::render::AlphaMode;
use crate::graphics::renderers::vulkan::interop;
use crate::graphics::renderers::vulkan::{SubmissionReceipt, VulkanDevice};
use ash::vk;

use crate::graphics::presentation::wsi::error::{PresentError, PresentErrorKind, PresentResult};
use crate::graphics::presentation::wsi::surface::VulkanWinitSurface;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum VulkanPresentModePreference {
    #[default]
    MailboxWithFifoFallback,
    Fifo,
}

pub(crate) struct SwapchainState {
    pub(crate) device: VulkanDevice,
    pub(crate) loader: ash::khr::swapchain::Device,
    pub(crate) raw: vk::SwapchainKHR,
    pub(crate) images: Vec<vk::Image>,
    pub(crate) views: Vec<vk::ImageView>,
    pub(crate) format: vk::Format,
    pub(crate) extent: vk::Extent2D,
    #[cfg_attr(not(feature = "instrumentation"), allow(dead_code))]
    pub(crate) present_mode: vk::PresentModeKHR,
    #[cfg_attr(not(feature = "instrumentation"), allow(dead_code))]
    pub(crate) one_to_one_present_scaling: bool,
    pub(crate) alpha_mode: AlphaMode,
    pub(crate) maintenance: Option<ash::ext::swapchain_maintenance1::Device>,
    pub(crate) present_wait: Option<ash::khr::present_wait::Device>,
    pub(crate) acquire_semaphores: Vec<vk::Semaphore>,
    pub(crate) acquire_fences: Vec<vk::Fence>,
    pub(crate) acquire_fence_pending: Vec<bool>,
    pub(crate) acquire_receipts: Vec<Option<SubmissionReceipt>>,
    pub(crate) present_semaphores: Vec<vk::Semaphore>,
    pub(crate) present_fences: Vec<vk::Fence>,
    pub(crate) present_fence_pending: Vec<bool>,
    pub(crate) initialized: Vec<bool>,
    pub(crate) acquire_cursor: usize,
    pub(crate) next_present_id: u64,
}

impl SwapchainState {
    pub(crate) fn create(
        surface: &VulkanWinitSurface,
        device: &VulkanDevice,
        requested: SizeI,
        frames_in_flight: usize,
        old: vk::SwapchainKHR,
        present_mode_preference: VulkanPresentModePreference,
    ) -> PresentResult<Self> {
        if !device.capabilities().presentation_enabled {
            return Err(PresentError::new(
                PresentErrorKind::InvalidState,
                "Vulkan device was not created for presentation",
            ));
        }
        let physical = interop::raw_physical_device(device);
        let capabilities = unsafe {
            surface
                .loader()
                .get_physical_device_surface_capabilities(physical, surface.raw())
        }
        .map_err(|result| PresentError::from_vk("failed to query surface capabilities", result))?;
        if !capabilities
            .supported_usage_flags
            .contains(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        {
            return Err(PresentError::new(
                PresentErrorKind::Unsupported,
                "surface swapchains cannot be used as color attachments",
            ));
        }
        let formats = unsafe {
            surface
                .loader()
                .get_physical_device_surface_formats(physical, surface.raw())
        }
        .map_err(|result| PresentError::from_vk("failed to query surface formats", result))?;
        let modes = unsafe {
            surface
                .loader()
                .get_physical_device_surface_present_modes(physical, surface.raw())
        }
        .map_err(|result| PresentError::from_vk("failed to query present modes", result))?;
        let surface_format = choose_surface_format(&formats).ok_or_else(|| {
            PresentError::new(
                PresentErrorKind::Unsupported,
                "surface has no supported BGRA/RGBA 8-bit sRGB format",
            )
        })?;
        let present_mode =
            choose_present_mode(&modes, present_mode_preference).ok_or_else(|| {
                PresentError::new(
                    PresentErrorKind::Unsupported,
                    "surface does not expose FIFO present",
                )
            })?;
        let present_scaling = if device.capabilities().swapchain_maintenance1 {
            surface.one_to_one_present_scaling(physical, present_mode)?
        } else {
            None
        };
        let one_to_one_present_scaling = present_scaling.is_some();
        let extent = choose_extent(capabilities, requested);
        let mut image_count = capabilities.min_image_count.saturating_add(1);
        if capabilities.max_image_count != 0 {
            image_count = image_count.min(capabilities.max_image_count);
        }
        let composite_alpha = choose_composite_alpha(capabilities.supported_composite_alpha)
            .ok_or_else(|| {
                PresentError::new(
                    PresentErrorKind::Unsupported,
                    "surface has no supported composite-alpha mode",
                )
            })?;
        let loader = ash::khr::swapchain::Device::new(
            interop::device_instance(device),
            interop::raw_device(device),
        );
        let maintenance = device.capabilities().swapchain_maintenance1.then(|| {
            ash::ext::swapchain_maintenance1::Device::new(
                interop::device_instance(device),
                interop::raw_device(device),
            )
        });
        let present_wait = device.capabilities().present_wait.then(|| {
            ash::khr::present_wait::Device::new(
                interop::device_instance(device),
                interop::raw_device(device),
            )
        });
        let graphics_family = interop::graphics_queue_family(device);
        let present_family = interop::presentation_queue_family(device);
        let queue_family_indices = [graphics_family, present_family];
        let mut create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface.raw())
            .min_image_count(image_count)
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(composite_alpha)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(old);
        create_info = if graphics_family == present_family {
            create_info.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        } else {
            create_info
                .image_sharing_mode(vk::SharingMode::CONCURRENT)
                .queue_family_indices(&queue_family_indices)
        };
        let mut present_scaling_info = present_scaling.map(|selection| {
            vk::SwapchainPresentScalingCreateInfoEXT::default()
                .scaling_behavior(vk::PresentScalingFlagsEXT::ONE_TO_ONE)
                .present_gravity_x(selection.gravity_x)
                .present_gravity_y(selection.gravity_y)
        });
        if let Some(present_scaling_info) = present_scaling_info.as_mut() {
            create_info = create_info.push_next(present_scaling_info);
        }
        let raw = unsafe { loader.create_swapchain(&create_info, None) }
            .map_err(|result| PresentError::from_vk("failed to create Vulkan swapchain", result))?;
        let images = unsafe { loader.get_swapchain_images(raw) }
            .map_err(|result| PresentError::from_vk("failed to get swapchain images", result))?;
        let mut views = Vec::with_capacity(images.len());
        for image in &images {
            let view_result = unsafe {
                interop::raw_device(device).create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(*image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(surface_format.format)
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        }),
                    None,
                )
            };
            match view_result {
                Ok(view) => views.push(view),
                Err(result) => {
                    unsafe {
                        for view in views.drain(..) {
                            interop::raw_device(device).destroy_image_view(view, None);
                        }
                        loader.destroy_swapchain(raw, None);
                    }
                    return Err(PresentError::from_vk(
                        "failed to create swapchain view",
                        result,
                    ));
                }
            }
        }
        let acquire_count = frames_in_flight.clamp(1, images.len().max(1));
        let acquire_semaphores = match create_semaphores(device, acquire_count) {
            Ok(semaphores) => semaphores,
            Err(error) => {
                destroy_partial(
                    device,
                    &loader,
                    raw,
                    &mut views,
                    &mut Vec::new(),
                    &mut Vec::new(),
                );
                return Err(error);
            }
        };
        let acquire_fences = if maintenance.is_some() {
            Vec::new()
        } else {
            match create_fences(device, acquire_count) {
                Ok(fences) => fences,
                Err(error) => {
                    let mut acquire_semaphores = acquire_semaphores;
                    destroy_partial(
                        device,
                        &loader,
                        raw,
                        &mut views,
                        &mut acquire_semaphores,
                        &mut Vec::new(),
                    );
                    return Err(error);
                }
            }
        };
        let present_count = present_sync_count(maintenance.is_some(), images.len(), acquire_count);
        let present_semaphores = match create_semaphores(device, present_count) {
            Ok(semaphores) => semaphores,
            Err(error) => {
                let mut acquire_semaphores = acquire_semaphores;
                let mut acquire_fences = acquire_fences;
                destroy_partial(
                    device,
                    &loader,
                    raw,
                    &mut views,
                    &mut acquire_semaphores,
                    &mut acquire_fences,
                );
                return Err(error);
            }
        };
        let present_fences = if maintenance.is_some() {
            match create_fences(device, acquire_count) {
                Ok(fences) => fences,
                Err(error) => {
                    let mut semaphores = acquire_semaphores;
                    semaphores.extend(present_semaphores);
                    let mut fences = acquire_fences;
                    destroy_partial(
                        device,
                        &loader,
                        raw,
                        &mut views,
                        &mut semaphores,
                        &mut fences,
                    );
                    return Err(error);
                }
            }
        } else {
            Vec::new()
        };
        let acquire_receipts = (0..acquire_count).map(|_| None).collect();
        let initialized = vec![false; images.len()];
        Ok(Self {
            device: device.clone(),
            loader,
            raw,
            images,
            views,
            format: surface_format.format,
            extent,
            present_mode,
            one_to_one_present_scaling,
            alpha_mode: if composite_alpha == vk::CompositeAlphaFlagsKHR::OPAQUE {
                AlphaMode::Opaque
            } else {
                AlphaMode::Premultiplied
            },
            maintenance,
            present_wait,
            acquire_semaphores,
            acquire_fences,
            acquire_fence_pending: vec![false; acquire_count],
            acquire_receipts,
            present_semaphores,
            present_fences,
            present_fence_pending: vec![false; acquire_count],
            initialized,
            acquire_cursor: 0,
            next_present_id: 1,
        })
    }

    pub(crate) fn acquire_slot(&mut self) -> PresentResult<Option<usize>> {
        let slot = self.acquire_cursor;
        if let Some(receipt) = &mut self.acquire_receipts[slot] {
            let complete = receipt.poll().map_err(|error| {
                PresentError::new(PresentErrorKind::DeviceLost, error.to_string())
            });
            if !complete? {
                return Ok(None);
            }
        }
        self.acquire_receipts[slot] = None;
        if self.present_fence_pending[slot] {
            let signaled = unsafe {
                interop::raw_device(&self.device).get_fence_status(self.present_fences[slot])
            }
            .map_err(|result| {
                PresentError::from_vk("failed to poll swapchain present fence", result)
            })?;
            if !signaled {
                return Ok(None);
            }
            self.present_fence_pending[slot] = false;
        }
        if self.acquire_fence_pending[slot] {
            let signaled = unsafe {
                interop::raw_device(&self.device).get_fence_status(self.acquire_fences[slot])
            };
            let signaled = signaled.map_err(|result| {
                PresentError::from_vk("failed to poll swapchain acquire fence", result)
            })?;
            if !signaled {
                return Ok(None);
            }
            self.acquire_fence_pending[slot] = false;
        }
        if let Some(fence) = self.acquire_fences.get(slot).copied() {
            unsafe { interop::raw_device(&self.device).reset_fences(&[fence]) }.map_err(
                |result| PresentError::from_vk("failed to reset swapchain acquire fence", result),
            )?;
        }
        Ok(Some(slot))
    }

    pub(crate) fn mark_acquired(&mut self, slot: usize) {
        self.acquire_fence_pending[slot] = !self.acquire_fences.is_empty();
    }

    pub(crate) fn acquire_fence(&self, slot: usize) -> vk::Fence {
        self.acquire_fences
            .get(slot)
            .copied()
            .unwrap_or(vk::Fence::null())
    }

    pub(crate) fn prepare_present_fence(
        &mut self,
        slot: usize,
    ) -> PresentResult<Option<vk::Fence>> {
        let Some(fence) = self.present_fences.get(slot).copied() else {
            return Ok(None);
        };
        debug_assert!(!self.present_fence_pending[slot]);
        unsafe { interop::raw_device(&self.device).reset_fences(&[fence]) }.map_err(|result| {
            PresentError::from_vk("failed to reset swapchain present fence", result)
        })?;
        Ok(Some(fence))
    }

    pub(crate) fn present_semaphore(
        &self,
        image_index: usize,
        acquire_slot: usize,
    ) -> vk::Semaphore {
        self.present_semaphores
            [present_sync_index(self.maintenance.is_some(), image_index, acquire_slot)]
    }

    pub(crate) fn mark_present_pending(&mut self, slot: usize) {
        if !self.present_fences.is_empty() {
            self.present_fence_pending[slot] = true;
        }
    }

    pub(crate) fn present_slot_complete(&mut self, slot: usize) -> PresentResult<bool> {
        let fence = self.present_fences.get(slot).copied().ok_or_else(|| {
            PresentError::new(
                PresentErrorKind::InvalidState,
                "swapchain present-completion slot is invalid",
            )
        })?;
        if !self.present_fence_pending[slot] {
            return Ok(true);
        }
        let signaled = unsafe { interop::raw_device(&self.device).get_fence_status(fence) }
            .map_err(|result| {
                PresentError::from_vk("failed to poll resize presentation fence", result)
            })?;
        if signaled {
            self.present_fence_pending[slot] = false;
        }
        Ok(signaled)
    }

    pub(crate) fn prepare_present_id(&mut self) -> PresentResult<Option<u64>> {
        if self.present_wait.is_none() {
            return Ok(None);
        }
        allocate_present_id(&mut self.next_present_id)
            .map(Some)
            .ok_or_else(|| {
                PresentError::new(
                    PresentErrorKind::InvalidState,
                    "Vulkan swapchain presentation ID overflow",
                )
            })
    }

    pub(crate) fn present_id_complete(&self, present_id: u64) -> PresentResult<bool> {
        let present_wait = self.present_wait.as_ref().ok_or_else(|| {
            PresentError::new(
                PresentErrorKind::InvalidState,
                "swapchain exact-present completion is unavailable",
            )
        })?;
        match unsafe { present_wait.wait_for_present(self.raw, present_id, 0) } {
            Ok(()) => Ok(true),
            Err(vk::Result::TIMEOUT) => Ok(false),
            Err(result) => Err(PresentError::from_vk(
                "failed to poll exact Vulkan presentation ID",
                result,
            )),
        }
    }

    pub(crate) fn acquisition_complete(&self, slot: usize) -> PresentResult<bool> {
        let fence = self.acquire_fences.get(slot).copied().ok_or_else(|| {
            PresentError::new(
                PresentErrorKind::InvalidState,
                "swapchain acquire-fence slot is invalid",
            )
        })?;
        unsafe { interop::raw_device(&self.device).get_fence_status(fence) }.map_err(|result| {
            PresentError::from_vk("failed to poll swapchain retirement fence", result)
        })
    }

    pub(crate) fn presentation_complete(&mut self) -> PresentResult<bool> {
        for receipt in self.acquire_receipts.iter_mut().flatten() {
            if !receipt.poll().map_err(|error| {
                PresentError::new(PresentErrorKind::DeviceLost, error.to_string())
            })? {
                return Ok(false);
            }
        }
        for (slot, pending) in self.present_fence_pending.iter().copied().enumerate() {
            if !pending {
                continue;
            }
            let signaled = unsafe {
                interop::raw_device(&self.device).get_fence_status(self.present_fences[slot])
            }
            .map_err(|result| {
                PresentError::from_vk("failed to poll retired swapchain present fence", result)
            })?;
            if !signaled {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(crate) fn release_image(&self, image_index: usize) -> PresentResult<()> {
        let Some(maintenance) = &self.maintenance else {
            return Ok(());
        };
        let indices = [image_index as u32];
        let info = vk::ReleaseSwapchainImagesInfoEXT::default()
            .swapchain(self.raw)
            .image_indices(&indices);
        unsafe { maintenance.release_swapchain_images(&info) }.map_err(|result| {
            PresentError::from_vk("failed to release an abandoned swapchain image", result)
        })
    }

    pub(crate) fn advance_acquire_slot(&mut self) {
        self.acquire_cursor = (self.acquire_cursor + 1) % self.acquire_semaphores.len();
    }
}

impl Drop for SwapchainState {
    fn drop(&mut self) {
        unsafe {
            let device = interop::raw_device(&self.device);
            for semaphore in self.acquire_semaphores.drain(..) {
                device.destroy_semaphore(semaphore, None);
            }
            for fence in self.acquire_fences.drain(..) {
                device.destroy_fence(fence, None);
            }
            for fence in self.present_fences.drain(..) {
                device.destroy_fence(fence, None);
            }
            for semaphore in self.present_semaphores.drain(..) {
                device.destroy_semaphore(semaphore, None);
            }
            for view in self.views.drain(..) {
                device.destroy_image_view(view, None);
            }
            self.loader.destroy_swapchain(self.raw, None);
        }
    }
}

fn create_semaphores(device: &VulkanDevice, count: usize) -> PresentResult<Vec<vk::Semaphore>> {
    let mut semaphores = Vec::with_capacity(count);
    for _ in 0..count {
        match unsafe {
            interop::raw_device(device).create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
        } {
            Ok(semaphore) => semaphores.push(semaphore),
            Err(result) => {
                unsafe {
                    for semaphore in semaphores.drain(..) {
                        interop::raw_device(device).destroy_semaphore(semaphore, None);
                    }
                }
                return Err(PresentError::from_vk(
                    "failed to create presentation semaphore",
                    result,
                ));
            }
        }
    }
    Ok(semaphores)
}

fn create_fences(device: &VulkanDevice, count: usize) -> PresentResult<Vec<vk::Fence>> {
    let mut fences = Vec::with_capacity(count);
    for _ in 0..count {
        match unsafe {
            interop::raw_device(device).create_fence(&vk::FenceCreateInfo::default(), None)
        } {
            Ok(fence) => fences.push(fence),
            Err(result) => {
                unsafe {
                    for fence in fences.drain(..) {
                        interop::raw_device(device).destroy_fence(fence, None);
                    }
                }
                return Err(PresentError::from_vk(
                    "failed to create swapchain acquire fence",
                    result,
                ));
            }
        }
    }
    Ok(fences)
}

fn destroy_partial(
    device: &VulkanDevice,
    loader: &ash::khr::swapchain::Device,
    raw: vk::SwapchainKHR,
    views: &mut Vec<vk::ImageView>,
    semaphores: &mut Vec<vk::Semaphore>,
    fences: &mut Vec<vk::Fence>,
) {
    unsafe {
        for semaphore in semaphores.drain(..) {
            interop::raw_device(device).destroy_semaphore(semaphore, None);
        }
        for fence in fences.drain(..) {
            interop::raw_device(device).destroy_fence(fence, None);
        }
        for view in views.drain(..) {
            interop::raw_device(device).destroy_image_view(view, None);
        }
        loader.destroy_swapchain(raw, None);
    }
}

fn choose_surface_format(formats: &[vk::SurfaceFormatKHR]) -> Option<vk::SurfaceFormatKHR> {
    [vk::Format::B8G8R8A8_SRGB, vk::Format::R8G8B8A8_SRGB]
        .into_iter()
        .find_map(|preferred| {
            formats
                .iter()
                .copied()
                .find(|item| item.format == preferred)
        })
}

const fn present_sync_count(maintenance: bool, image_count: usize, acquire_count: usize) -> usize {
    if maintenance {
        acquire_count
    } else {
        image_count
    }
}

const fn present_sync_index(maintenance: bool, image_index: usize, acquire_slot: usize) -> usize {
    if maintenance {
        acquire_slot
    } else {
        image_index
    }
}

fn choose_present_mode(
    modes: &[vk::PresentModeKHR],
    preference: VulkanPresentModePreference,
) -> Option<vk::PresentModeKHR> {
    match preference {
        VulkanPresentModePreference::MailboxWithFifoFallback => {
            [vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::FIFO]
                .into_iter()
                .find(|mode| modes.contains(mode))
        }
        VulkanPresentModePreference::Fifo => modes
            .contains(&vk::PresentModeKHR::FIFO)
            .then_some(vk::PresentModeKHR::FIFO),
    }
}

fn allocate_present_id(next_present_id: &mut u64) -> Option<u64> {
    let present_id = *next_present_id;
    *next_present_id = present_id.checked_add(1)?;
    Some(present_id)
}

fn choose_extent(capabilities: vk::SurfaceCapabilitiesKHR, requested: SizeI) -> vk::Extent2D {
    if capabilities.current_extent.width != u32::MAX {
        return capabilities.current_extent;
    }
    vk::Extent2D {
        width: (requested.width.max(1) as u32).clamp(
            capabilities.min_image_extent.width,
            capabilities.max_image_extent.width,
        ),
        height: (requested.height.max(1) as u32).clamp(
            capabilities.min_image_extent.height,
            capabilities.max_image_extent.height,
        ),
    }
}

fn choose_composite_alpha(
    supported: vk::CompositeAlphaFlagsKHR,
) -> Option<vk::CompositeAlphaFlagsKHR> {
    [
        vk::CompositeAlphaFlagsKHR::OPAQUE,
        vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::INHERIT,
    ]
    .into_iter()
    .find(|mode| supported.contains(*mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_prefers_srgb_mailbox_and_clamps_extent() {
        let formats = [
            vk::SurfaceFormatKHR {
                format: vk::Format::R8G8B8A8_UNORM,
                color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            },
            vk::SurfaceFormatKHR {
                format: vk::Format::B8G8R8A8_SRGB,
                color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            },
        ];
        assert_eq!(
            choose_surface_format(&formats).unwrap().format,
            vk::Format::B8G8R8A8_SRGB
        );
        assert_eq!(
            choose_present_mode(
                &[vk::PresentModeKHR::FIFO],
                VulkanPresentModePreference::MailboxWithFifoFallback
            ),
            Some(vk::PresentModeKHR::FIFO)
        );
        assert_eq!(
            choose_present_mode(
                &[vk::PresentModeKHR::FIFO, vk::PresentModeKHR::MAILBOX],
                VulkanPresentModePreference::MailboxWithFifoFallback
            ),
            Some(vk::PresentModeKHR::MAILBOX)
        );
        assert_eq!(
            choose_present_mode(
                &[vk::PresentModeKHR::FIFO, vk::PresentModeKHR::MAILBOX],
                VulkanPresentModePreference::Fifo
            ),
            Some(vk::PresentModeKHR::FIFO)
        );
        let capabilities = vk::SurfaceCapabilitiesKHR {
            current_extent: vk::Extent2D {
                width: u32::MAX,
                height: u32::MAX,
            },
            min_image_extent: vk::Extent2D {
                width: 10,
                height: 20,
            },
            max_image_extent: vk::Extent2D {
                width: 100,
                height: 200,
            },
            ..Default::default()
        };
        assert_eq!(
            choose_extent(
                capabilities,
                SizeI {
                    width: 5,
                    height: 999
                }
            ),
            vk::Extent2D {
                width: 10,
                height: 200
            }
        );
    }

    #[test]
    fn presentation_ids_are_monotonic_and_do_not_wrap() {
        let mut next = 1;
        assert_eq!(allocate_present_id(&mut next), Some(1));
        assert_eq!(allocate_present_id(&mut next), Some(2));
        assert_eq!(next, 3);

        let mut exhausted = u64::MAX;
        assert_eq!(allocate_present_id(&mut exhausted), None);
        assert_eq!(exhausted, u64::MAX);
    }

    #[test]
    fn maintenance_reuses_present_sync_by_completed_acquire_slot() {
        assert_eq!(present_sync_count(true, 4, 3), 3);
        assert_eq!(present_sync_index(true, 3, 1), 1);
        assert_eq!(present_sync_count(false, 4, 3), 4);
        assert_eq!(present_sync_index(false, 3, 1), 3);
    }
}
