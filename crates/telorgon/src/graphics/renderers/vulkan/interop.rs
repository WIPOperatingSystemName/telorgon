//! Narrow native Vulkan bridge used by presenters and future hosted integrations.

use std::ffi::CStr;
use std::marker::PhantomData;

use crate::foundation::SizeI;
use crate::graphics::render::{AlphaMode, ColorSpace, RenderTargetInfo};
use ash::vk;

use crate::graphics::render::{RenderError, RenderErrorKind, RenderResult};

use crate::graphics::renderers::vulkan::target::VulkanImageState;
use crate::graphics::renderers::vulkan::{
    SubmissionReceipt, VulkanDevice, VulkanInstance, VulkanRecordedFrame, VulkanTarget,
};

#[derive(Copy, Clone)]
pub struct BorrowedVulkanSurface<'a> {
    pub(crate) loader: &'a ash::khr::surface::Instance,
    pub(crate) raw: vk::SurfaceKHR,
    _borrow: PhantomData<&'a vk::SurfaceKHR>,
}

impl<'a> BorrowedVulkanSurface<'a> {
    /// # Safety
    ///
    /// `raw` must be a live surface created from `instance`, and it must remain live for this
    /// borrow. The caller retains ownership and must not destroy it during device selection.
    pub unsafe fn new(
        instance: &VulkanInstance,
        loader: &'a ash::khr::surface::Instance,
        raw: vk::SurfaceKHR,
    ) -> Self {
        let _ = instance;
        Self {
            loader,
            raw,
            _borrow: PhantomData,
        }
    }
}

#[derive(Copy, Clone)]
pub struct PresentationRequirement<'a> {
    pub surface: BorrowedVulkanSurface<'a>,
}

/// Native instance access is confined to a presenter that creates a surface for this instance.
pub fn instance_entry(instance: &VulkanInstance) -> &ash::Entry {
    &instance.inner.entry
}

/// Native instance access is confined to a presenter that creates a surface for this instance.
pub fn raw_instance(instance: &VulkanInstance) -> &ash::Instance {
    &instance.inner.raw
}

/// Reports whether an extension was enabled on a Telorgon-owned Vulkan instance.
///
/// Borrowed instances conservatively report `false` because their enabled-extension set is not
/// part of the interop contract.
pub fn instance_extension_enabled(instance: &VulkanInstance, name: &CStr) -> bool {
    instance
        .inner
        .enabled_extensions
        .iter()
        .any(|enabled| enabled.as_c_str() == name)
}

/// Wraps host-owned Vulkan instance dispatch without taking ownership of the native instance.
///
/// # Safety
///
/// `raw` must have been loaded from `entry`, remain valid until every clone of the returned value
/// is dropped, and outlive every hosted device created from it. Telorgon never destroys `raw`.
pub unsafe fn borrowed_instance(entry: ash::Entry, raw: ash::Instance) -> VulkanInstance {
    unsafe { VulkanInstance::from_borrowed_raw(entry, raw) }
}

pub fn raw_physical_device(device: &VulkanDevice) -> vk::PhysicalDevice {
    device.inner.physical_device
}

/// Returns the opaque identity shared by every clone of one logical device.
pub fn device_id(device: &VulkanDevice) -> u64 {
    device.inner.id
}

pub fn raw_device(device: &VulkanDevice) -> &ash::Device {
    &device.inner.raw
}

#[derive(Copy, Clone)]
pub struct BorrowedVulkanTargetParts<'target> {
    pub image: vk::Image,
    pub view: vk::ImageView,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    _borrow: PhantomData<&'target mut vk::Image>,
}

/// Exposes native handles for a host that owns command recording and synchronization.
pub fn borrowed_target_parts<'target>(
    target: &VulkanTarget<'target>,
) -> BorrowedVulkanTargetParts<'target> {
    BorrowedVulkanTargetParts {
        image: target.image,
        view: target.view,
        format: target.format,
        extent: target.extent,
        _borrow: PhantomData,
    }
}

pub fn device_instance(device: &VulkanDevice) -> &ash::Instance {
    &device.inner.instance.inner.raw
}

pub fn graphics_queue(device: &VulkanDevice) -> vk::Queue {
    device.inner.queue
}

pub fn graphics_queue_family(device: &VulkanDevice) -> u32 {
    device.inner.queue_family
}

pub fn presentation_queue_family(device: &VulkanDevice) -> u32 {
    device.inner.present_queue_family
}

/// Submits a recorded frame waiting on the acquired-image semaphore and signaling presentation.
///
/// # Safety
///
/// Both semaphores must be live binary semaphores created from `device`. `wait` must correspond to
/// an acquired target used by `frame`; `signal` must not still be pending in a presentation wait.
pub unsafe fn submit_present_frame(
    device: &VulkanDevice,
    frame: VulkanRecordedFrame,
    wait: vk::Semaphore,
    signal: vk::Semaphore,
) -> RenderResult<SubmissionReceipt> {
    if frame.device_id() != device.inner.id {
        return Err(RenderError::new(
            RenderErrorKind::HostContract,
            "recorded frame belongs to another Vulkan device",
        ));
    }
    frame.submit_with_binary_semaphores(wait, signal)
}

/// Submits a frame between two values of one imported D3D shared fence.
///
/// # Safety
///
/// `semaphore` must be a timeline semaphore imported from the D3D fence that guards the target.
/// `keyed_mutex_memory` must be the imported allocation backing that target.
/// D3D must signal `wait_value` only after its last read, and must wait for `signal_value` before
/// reading the rendered target.
pub unsafe fn submit_external_timeline_frame(
    device: &VulkanDevice,
    frame: VulkanRecordedFrame,
    semaphore: vk::Semaphore,
    wait_value: u64,
    signal_value: u64,
    keyed_mutex_memory: vk::DeviceMemory,
) -> RenderResult<SubmissionReceipt> {
    if frame.device_id() != device.inner.id {
        return Err(RenderError::new(
            RenderErrorKind::HostContract,
            "recorded frame belongs to another Vulkan device",
        ));
    }
    frame.submit_with_timeline_semaphore_and_keyed_mutex(
        semaphore,
        wait_value,
        signal_value,
        keyed_mutex_memory,
    )
}

/// Creates a renderer target borrowing one acquired swapchain image.
///
/// # Safety
///
/// The image and view must belong to `device`, match `format` and `extent`, and remain acquired and
/// live for `'frame`. The caller must release the acquisition through present or explicit discard.
pub unsafe fn swapchain_target<'frame>(
    device: &VulkanDevice,
    image: vk::Image,
    view: vk::ImageView,
    format: vk::Format,
    extent: vk::Extent2D,
    initialized: bool,
    alpha_mode: AlphaMode,
) -> VulkanTarget<'frame> {
    let size = SizeI {
        width: extent.width as i32,
        height: extent.height as i32,
    };
    VulkanTarget {
        device_id: device.inner.id,
        image,
        view,
        format,
        extent,
        info: RenderTargetInfo {
            color_space: if matches!(
                format,
                vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB
            ) {
                ColorSpace::Srgb
            } else {
                ColorSpace::Linear
            },
            alpha_mode,
            ..RenderTargetInfo::full(size)
        },
        initial_state: if initialized {
            VulkanImageState {
                layout: vk::ImageLayout::PRESENT_SRC_KHR,
                stage: vk::PipelineStageFlags2::NONE,
                access: vk::AccessFlags2::NONE,
            }
        } else {
            VulkanImageState::UNDEFINED
        },
        final_state: VulkanImageState {
            layout: vk::ImageLayout::PRESENT_SRC_KHR,
            stage: vk::PipelineStageFlags2::NONE,
            access: vk::AccessFlags2::NONE,
        },
        initial_queue_family: vk::QUEUE_FAMILY_IGNORED,
        final_queue_family: vk::QUEUE_FAMILY_IGNORED,
        _borrow: std::marker::PhantomData,
    }
}

/// Creates a renderer target over a D3D11-owned image imported into Vulkan.
///
/// # Safety
///
/// The image and view must match `format` and `extent`, remain live for the returned borrow, and be
/// protected by the external timeline synchronization used for submission.
pub unsafe fn dxgi_bridge_target<'frame>(
    device: &VulkanDevice,
    image: vk::Image,
    view: vk::ImageView,
    format: vk::Format,
    extent: vk::Extent2D,
    initialized: bool,
) -> VulkanTarget<'frame> {
    let size = SizeI {
        width: extent.width as i32,
        height: extent.height as i32,
    };
    VulkanTarget {
        device_id: device.inner.id,
        image,
        view,
        format,
        extent,
        info: RenderTargetInfo {
            color_space: ColorSpace::Srgb,
            alpha_mode: AlphaMode::Opaque,
            ..RenderTargetInfo::full(size)
        },
        initial_state: if initialized {
            VulkanImageState {
                layout: vk::ImageLayout::GENERAL,
                stage: vk::PipelineStageFlags2::NONE,
                access: vk::AccessFlags2::NONE,
            }
        } else {
            VulkanImageState::UNDEFINED
        },
        final_state: VulkanImageState {
            layout: vk::ImageLayout::GENERAL,
            stage: vk::PipelineStageFlags2::ALL_GRAPHICS,
            access: vk::AccessFlags2::MEMORY_WRITE,
        },
        initial_queue_family: vk::QUEUE_FAMILY_IGNORED,
        final_queue_family: vk::QUEUE_FAMILY_IGNORED,
        _borrow: std::marker::PhantomData,
    }
}

/// Presents through the device's externally synchronized graphics/present queue.
///
/// # Safety
///
/// `loader` and `info` must describe a live swapchain created from `device`; all wait semaphores
/// must be valid and signaled by submissions on the same device.
pub unsafe fn queue_present(
    device: &VulkanDevice,
    loader: &ash::khr::swapchain::Device,
    info: &vk::PresentInfoKHR<'_>,
) -> Result<bool, vk::Result> {
    let _queue = device
        .inner
        .present_queue_lock
        .lock()
        .map_err(|_| vk::Result::ERROR_UNKNOWN)?;
    unsafe { loader.queue_present(device.inner.present_queue, info) }
}

/// Waits for the owned graphics/presentation queue only during explicit legacy generation
/// retirement or shutdown.
///
/// # Safety
///
/// The caller must ensure it is not holding another queue lock and must not use this on a normal
/// frame path.
pub unsafe fn wait_presentation_queues_idle(device: &VulkanDevice) -> Result<(), vk::Result> {
    let _graphics = device
        .inner
        .queue_lock
        .lock()
        .map_err(|_| vk::Result::ERROR_UNKNOWN)?;
    unsafe { device.inner.raw.queue_wait_idle(device.inner.queue) }?;
    if device.inner.present_queue_family != device.inner.queue_family {
        let _present = device
            .inner
            .present_queue_lock
            .lock()
            .map_err(|_| vk::Result::ERROR_UNKNOWN)?;
        unsafe { device.inner.raw.queue_wait_idle(device.inner.present_queue) }?;
    }
    Ok(())
}
