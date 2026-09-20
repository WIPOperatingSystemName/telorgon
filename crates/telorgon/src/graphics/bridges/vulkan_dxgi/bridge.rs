use crate::foundation::SizeI;
use crate::graphics::presentation::{
    CompletionProof, CompletionStage, PresentDisposition,
    PresentationRecovery as PresenterRecovery, PresentationState as PresenterState,
    is_zero_extent as is_zero,
};
use crate::graphics::presentation::dxgi::DxgiPresenter;
use crate::graphics::renderers::vulkan::interop;
use crate::graphics::renderers::vulkan::{
    VulkanDevice, VulkanRecordedFrame, VulkanRecordingFrame, VulkanTarget,
};
use ash::vk;
use raw_window_handle::HasWindowHandle;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device,
    ID3D11Fence, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIKeyedMutex, IDXGIResource1,
};
use windows::core::Interface;

use crate::graphics::bridges::vulkan_dxgi::{
    DxgiPresentOutcome as PresentOutcome, PresentError, PresentErrorKind, PresentResult,
};

const BRIDGE_FORMAT: vk::Format = vk::Format::B8G8R8A8_SRGB;
const BUFFER_COUNT: u32 = 2;
const VULKAN_RELEASE_KEY: u64 = 1;
const D3D_RELEASE_KEY: u64 = 0;
const KEYED_MUTEX_TIMEOUT_MS: u32 = 100;

pub enum VulkanDxgiAcquireOutcome<'presenter> {
    Ready(AcquiredVulkanDxgiFrame<'presenter>),
    Suspended,
    NotReady,
    NeedsReconfigure,
}

pub struct AcquiredVulkanDxgiFrame<'presenter> {
    presenter: &'presenter mut VulkanDxgiBridge,
    state: AcquiredVulkanDxgiState,
    consumed: bool,
}

#[derive(Copy, Clone)]
struct AcquiredVulkanDxgiState {
    device_id: u64,
    frame_id: u64,
    slot: usize,
}

struct BridgeImage {
    d3d_texture: ID3D11Texture2D,
    keyed_mutex: IDXGIKeyedMutex,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    initialized: bool,
}

pub struct VulkanDxgiBridge {
    device: VulkanDevice,
    dxgi: DxgiPresenter,
    fence: ID3D11Fence,
    semaphore: vk::Semaphore,
    images: Vec<BridgeImage>,
    retired_images: Vec<(Vec<BridgeImage>, u64)>,
    recovery: PresenterRecovery,
    slot_cursor: usize,
    next_fence_value: u64,
    last_d3d_done: u64,
    frames_in_flight: usize,
    bridge_lifetime_unproven: bool,
    shutdown: bool,
}

impl VulkanDxgiBridge {
    pub fn new(
        window: &impl HasWindowHandle,
        device: &VulkanDevice,
        extent: SizeI,
        frames_in_flight: usize,
    ) -> PresentResult<Self> {
        if !device.capabilities().dxgi_interop {
            return Err(PresentError::new(
                PresentErrorKind::Unsupported,
                "Vulkan adapter does not expose the required Windows external-memory bridge",
            ));
        }
        let dxgi = DxgiPresenter::new(window, device.capabilities().device_luid, extent)
            .map_err(map_dxgi_error)?;
        let (fence, semaphore) = create_bridge_sync(device, &dxgi)?;
        let mut presenter = Self {
            device: device.clone(),
            dxgi,
            fence,
            semaphore,
            images: Vec::new(),
            retired_images: Vec::new(),
            recovery: PresenterRecovery::new(extent),
            slot_cursor: 0,
            next_fence_value: 1,
            last_d3d_done: 0,
            frames_in_flight: frames_in_flight.max(1),
            bridge_lifetime_unproven: false,
            shutdown: false,
        };
        if !is_zero(extent)
            && let Err(error) = presenter.reconfigure()
        {
            presenter.dxgi.shutdown();
            unsafe { interop::raw_device(device).destroy_semaphore(presenter.semaphore, None) };
            presenter.semaphore = vk::Semaphore::null();
            presenter.shutdown = true;
            return Err(error);
        }
        Ok(presenter)
    }

    pub fn recovery(&self) -> PresenterRecovery {
        self.recovery
    }

    pub fn resize(&mut self, extent: SizeI) -> bool {
        self.recovery.resize(extent)
    }

    pub fn acquire<'a>(
        &'a mut self,
        device: &VulkanDevice,
        frame: &VulkanRecordingFrame<'_>,
    ) -> PresentResult<VulkanDxgiAcquireOutcome<'a>> {
        self.validate_device(device, frame.device_id())?;
        self.collect_retired();
        match self.recovery.state {
            PresenterState::Suspended => return Ok(VulkanDxgiAcquireOutcome::Suspended),
            PresenterState::NeedsReconfigure | PresenterState::Unconfigured => {
                self.reconfigure()?
            }
            PresenterState::SurfaceLost => return Ok(VulkanDxgiAcquireOutcome::NeedsReconfigure),
            PresenterState::DeviceLost | PresenterState::Shutdown => {
                return Err(PresentError::new(
                    PresentErrorKind::InvalidState,
                    format!("DXGI presenter is {:?}", self.recovery.state),
                ));
            }
            PresenterState::Ready => {}
        }
        if self.images.is_empty() {
            return Ok(VulkanDxgiAcquireOutcome::NotReady);
        }
        let slot = self.slot_cursor % self.images.len();
        Ok(VulkanDxgiAcquireOutcome::Ready(AcquiredVulkanDxgiFrame {
            presenter: self,
            state: AcquiredVulkanDxgiState {
                device_id: interop::device_id(device),
                frame_id: frame.frame_id(),
                slot,
            },
            consumed: false,
        }))
    }

    pub fn poll_present_completion(&mut self, _completion: impl Copy) -> PresentResult<bool> {
        Ok(false)
    }

    pub fn has_pending_retirement(&self) -> bool {
        !self.retired_images.is_empty()
    }

    pub fn enforce_retirement_limit(
        &mut self,
        _device: &VulkanDevice,
        maximum: usize,
    ) -> PresentResult<()> {
        if self.retired_images.len() > maximum {
            self.wait_for_d3d(self.last_d3d_done)?;
            self.destroy_retired();
        }
        Ok(())
    }

    pub fn suspend(&mut self) -> PresentResult<()> {
        if matches!(
            self.recovery.state,
            PresenterState::DeviceLost | PresenterState::Shutdown
        ) {
            return Err(PresentError::new(
                PresentErrorKind::InvalidState,
                format!(
                    "cannot suspend Vulkan/DXGI bridge in {:?} state",
                    self.recovery.state
                ),
            ));
        }
        self.wait_for_d3d(self.last_d3d_done)?;
        self.retire_current_images();
        self.destroy_retired();
        self.dxgi.suspend();
        self.recovery.state = PresenterState::Suspended;
        Ok(())
    }

    pub fn resume(&mut self, device: &VulkanDevice, extent: SizeI) -> PresentResult<()> {
        self.validate_device_id(device)?;
        self.recovery.resize(extent);
        if !is_zero(extent) {
            self.reconfigure()?;
        }
        Ok(())
    }

    pub fn shutdown(&mut self, device: &VulkanDevice) -> PresentResult<()> {
        self.validate_device_id(device)?;
        if !self.bridge_lifetime_unproven {
            self.wait_for_d3d(self.last_d3d_done)?;
        }
        unsafe { interop::wait_presentation_queues_idle(device) }.map_err(|result| {
            PresentError::from_vk("failed to stop Vulkan bridge queue", result)
        })?;
        if self.bridge_lifetime_unproven {
            // A failed D3D fence signal leaves no cross-API proof for manual Vulkan destruction.
            // Keep the raw Vulkan children alive until device destruction; dropping the COM
            // resources first lets the native devices perform their own lost-device cleanup.
            self.dxgi.shutdown();
            self.recovery.state = PresenterState::Shutdown;
            self.shutdown = true;
            return Ok(());
        }
        self.retire_current_images();
        self.destroy_retired();
        self.dxgi.shutdown();
        unsafe { interop::raw_device(device).destroy_semaphore(self.semaphore, None) };
        self.semaphore = vk::Semaphore::null();
        self.recovery.state = PresenterState::Shutdown;
        self.shutdown = true;
        Ok(())
    }

    fn reconfigure(&mut self) -> PresentResult<()> {
        if matches!(
            self.recovery.state,
            PresenterState::DeviceLost | PresenterState::Shutdown
        ) {
            return Err(PresentError::new(
                PresentErrorKind::InvalidState,
                format!(
                    "cannot reconfigure Vulkan/DXGI bridge in {:?} state",
                    self.recovery.state
                ),
            ));
        }
        let extent = self.recovery.requested_extent;
        if is_zero(extent) {
            return self.suspend();
        }
        self.wait_for_d3d(self.last_d3d_done)?;
        self.retire_current_images();
        self.destroy_retired();
        let width = extent.width as u32;
        let height = extent.height as u32;
        self.dxgi.resize(extent).map_err(map_dxgi_error)?;
        let count = self.frames_in_flight.clamp(1, BUFFER_COUNT as usize + 1);
        let mut images = Vec::with_capacity(count);
        for _ in 0..count {
            match create_bridge_image(&self.device, self.dxgi.device(), width, height) {
                Ok(image) => images.push(image),
                Err(error) => {
                    destroy_images(&self.device, &mut images);
                    return Err(error);
                }
            }
        }
        self.images = images;
        self.slot_cursor = 0;
        self.recovery.mark_reconfigured()?;
        #[cfg(feature = "instrumentation")]
        {
            crate::runtime::instrumentation::counter!("presenter.dxgi.scaling_none", 1_u8);
            crate::runtime::instrumentation::counter!("presenter.dxgi.extent_width", width);
            crate::runtime::instrumentation::counter!("presenter.dxgi.extent_height", height);
        }
        Ok(())
    }

    fn submit_acquired(
        &mut self,
        device: &VulkanDevice,
        frame: VulkanRecordedFrame,
        acquired: AcquiredVulkanDxgiState,
    ) -> PresentResult<PresentOutcome> {
        self.validate_device(device, frame.device_id())?;
        if acquired.device_id != interop::device_id(device) || acquired.frame_id != frame.frame_id()
        {
            return Err(PresentError::new(
                PresentErrorKind::InvalidState,
                "DXGI acquired image and recorded frame identity do not match",
            ));
        }
        let (render_done, d3d_done) = reserve_fence_pair(&mut self.next_fence_value)?;
        let receipt = unsafe {
            interop::submit_external_timeline_frame(
                device,
                frame,
                self.semaphore,
                self.last_d3d_done,
                render_done,
                self.images[acquired.slot].memory,
            )
        }
        .map_err(|error| PresentError::new(PresentErrorKind::DeviceLost, error.to_string()))?;
        let completion = receipt.completion();
        let source = &self.images[acquired.slot].d3d_texture;
        let keyed_mutex = &self.images[acquired.slot].keyed_mutex;
        unsafe { acquire_keyed_mutex(keyed_mutex, VULKAN_RELEASE_KEY, KEYED_MUTEX_TIMEOUT_MS)? };
        let presented =
            match self
                .dxgi
                .present_shared_texture(source, &self.fence, render_done, d3d_done)
            {
                Ok(()) => Ok(()),
                Err(failure) => {
                    if !failure.has_completion_proof() {
                        self.bridge_lifetime_unproven = true;
                        self.recovery.state = PresenterState::DeviceLost;
                    }
                    Err(map_dxgi_error(failure.into_error()))
                }
            };
        if let Err(error) = unsafe { keyed_mutex.ReleaseSync(D3D_RELEASE_KEY) } {
            self.bridge_lifetime_unproven = true;
            self.recovery.state = PresenterState::DeviceLost;
            return Err(win_error("failed to release the D3D bridge texture", error));
        }
        self.last_d3d_done = d3d_done;
        self.images[acquired.slot].initialized = true;
        self.slot_cursor = (acquired.slot + 1) % self.images.len();
        drop(receipt);
        match presented {
            Ok(()) => {
                self.recovery.state = PresenterState::Ready;
                Ok(PresentOutcome {
                    completion,
                    transport_completion: CompletionProof::new(
                        CompletionStage::Transport,
                        d3d_done,
                    ),
                    present_completion: CompletionProof::new(CompletionStage::Present, ()),
                    disposition: PresentDisposition::Presented,
                    reconfigure_pending: false,
                    maintenance_pending: false,
                })
            }
            Err(error) => {
                if !self.bridge_lifetime_unproven {
                    self.recovery.state = PresenterState::NeedsReconfigure;
                }
                Err(error)
            }
        }
    }

    fn validate_device(&self, device: &VulkanDevice, frame_device_id: u64) -> PresentResult<()> {
        self.validate_device_id(device)?;
        if frame_device_id != interop::device_id(device) {
            return Err(PresentError::new(
                PresentErrorKind::InvalidState,
                "recording frame belongs to another Vulkan device",
            ));
        }
        Ok(())
    }

    fn validate_device_id(&self, device: &VulkanDevice) -> PresentResult<()> {
        if interop::device_id(&self.device) != interop::device_id(device) {
            return Err(PresentError::new(
                PresentErrorKind::InvalidState,
                "DXGI presenter belongs to another Vulkan device",
            ));
        }
        Ok(())
    }

    fn wait_for_d3d(&self, value: u64) -> PresentResult<()> {
        self.dxgi
            .wait_for_fence(&self.fence, value)
            .map_err(map_dxgi_error)
    }

    fn retire_current_images(&mut self) {
        if !self.images.is_empty() {
            self.retired_images
                .push((std::mem::take(&mut self.images), self.last_d3d_done));
            self.recovery.mark_retired();
        }
    }

    fn collect_retired(&mut self) {
        let completed = unsafe { self.fence.GetCompletedValue() };
        let mut pending = Vec::new();
        for (mut images, value) in self.retired_images.drain(..) {
            if completed >= value {
                destroy_images(&self.device, &mut images);
            } else {
                pending.push((images, value));
            }
        }
        self.retired_images = pending;
    }

    fn destroy_retired(&mut self) {
        for (mut images, _) in self.retired_images.drain(..) {
            destroy_images(&self.device, &mut images);
        }
    }
}

impl AcquiredVulkanDxgiFrame<'_> {
    pub fn target(&self) -> VulkanTarget<'_> {
        let image = &self.presenter.images[self.state.slot];
        unsafe {
            interop::dxgi_bridge_target(
                &self.presenter.device,
                image.image,
                image.view,
                BRIDGE_FORMAT,
                vk::Extent2D {
                    width: self.presenter.recovery.requested_extent.width as u32,
                    height: self.presenter.recovery.requested_extent.height as u32,
                },
                image.initialized,
            )
        }
    }

    pub fn submit_and_present(
        mut self,
        device: &VulkanDevice,
        frame: VulkanRecordedFrame,
    ) -> PresentResult<PresentOutcome> {
        self.consumed = true;
        self.presenter.submit_acquired(device, frame, self.state)
    }

    pub fn discard(mut self, _device: &VulkanDevice) -> PresentResult<()> {
        self.consumed = true;
        Ok(())
    }
}

impl Drop for AcquiredVulkanDxgiFrame<'_> {
    fn drop(&mut self) {
        if !self.consumed {
            self.presenter.recovery.state = PresenterState::NeedsReconfigure;
        }
    }
}

impl Drop for VulkanDxgiBridge {
    fn drop(&mut self) {
        if !self.shutdown {
            // Vulkan objects are intentionally leaked rather than destroyed while either API may
            // still reference them. Managed hosts call `shutdown` on the presentation worker.
            self.images.clear();
            self.retired_images.clear();
        }
    }
}

fn create_bridge_sync(
    device: &VulkanDevice,
    dxgi: &DxgiPresenter,
) -> PresentResult<(ID3D11Fence, vk::Semaphore)> {
    let (fence, fence_handle) = dxgi.create_shared_fence().map_err(map_dxgi_error)?;
    let mut timeline = vk::SemaphoreTypeCreateInfo::default()
        .semaphore_type(vk::SemaphoreType::TIMELINE)
        .initial_value(0);
    let semaphore = unsafe {
        interop::raw_device(device).create_semaphore(
            &vk::SemaphoreCreateInfo::default().push_next(&mut timeline),
            None,
        )
    }
    .map_err(|result| PresentError::from_vk("failed to create bridge semaphore", result))?;
    let loader = ash::khr::external_semaphore_win32::Device::new(
        interop::device_instance(device),
        interop::raw_device(device),
    );
    let imported = unsafe {
        loader.import_semaphore_win32_handle(
            &vk::ImportSemaphoreWin32HandleInfoKHR::default()
                .semaphore(semaphore)
                .handle_type(vk::ExternalSemaphoreHandleTypeFlags::D3D12_FENCE)
                .handle(fence_handle.0 as vk::HANDLE),
        )
    };
    let _ = unsafe { CloseHandle(fence_handle) };
    if let Err(result) = imported {
        unsafe { interop::raw_device(device).destroy_semaphore(semaphore, None) };
        return Err(PresentError::from_vk(
            "failed to import shared D3D fence",
            result,
        ));
    }
    Ok((fence, semaphore))
}

fn reserve_fence_pair(next: &mut u64) -> PresentResult<(u64, u64)> {
    let render_done = *next;
    let d3d_done = render_done.checked_add(1).ok_or_else(|| {
        PresentError::new(PresentErrorKind::InvalidState, "DXGI fence value exhausted")
    })?;
    let following = d3d_done.checked_add(1).ok_or_else(|| {
        PresentError::new(PresentErrorKind::InvalidState, "DXGI fence value exhausted")
    })?;
    *next = following;
    Ok((render_done, d3d_done))
}

fn create_bridge_image(
    device: &VulkanDevice,
    d3d_device: &ID3D11Device,
    width: u32,
    height: u32,
) -> PresentResult<BridgeImage> {
    let desc = bridge_texture_desc(width, height);
    let mut texture = None;
    unsafe { d3d_device.CreateTexture2D(&desc, None, Some(&mut texture)) }
        .map_err(|error| win_error("failed to create shared D3D texture", error))?;
    let texture = texture.expect("D3D11 returned a texture on success");
    let keyed_mutex: IDXGIKeyedMutex = texture
        .cast()
        .map_err(|error| win_error("shared texture has no keyed mutex", error))?;
    let resource: IDXGIResource1 = texture
        .cast()
        .map_err(|error| win_error("shared texture has no IDXGIResource1", error))?;
    let access = DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0;
    let handle = unsafe { resource.CreateSharedHandle(None, access, None) }
        .map_err(|error| win_error("failed to share D3D texture", error))?;
    let result = import_bridge_image(device, texture, keyed_mutex, handle, width, height);
    let _ = unsafe { CloseHandle(handle) };
    result
}

fn bridge_texture_desc(width: u32, height: u32) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0)
            as u32,
    }
}

fn import_bridge_image(
    device: &VulkanDevice,
    d3d_texture: ID3D11Texture2D,
    keyed_mutex: IDXGIKeyedMutex,
    handle: HANDLE,
    width: u32,
    height: u32,
) -> PresentResult<BridgeImage> {
    let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC;
    let mut external_format = vk::PhysicalDeviceExternalImageFormatInfo::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE);
    let format_info = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(BRIDGE_FORMAT)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(usage)
        .push_next(&mut external_format);
    let mut external_properties = vk::ExternalImageFormatProperties::default();
    let mut image_properties =
        vk::ImageFormatProperties2::default().push_next(&mut external_properties);
    unsafe {
        interop::device_instance(device).get_physical_device_image_format_properties2(
            interop::raw_physical_device(device),
            &format_info,
            &mut image_properties,
        )
    }
    .map_err(|result| PresentError::from_vk("D3D texture format cannot be imported", result))?;
    if !external_properties
        .external_memory_properties
        .external_memory_features
        .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
    {
        return Err(PresentError::new(
            PresentErrorKind::Unsupported,
            "Vulkan adapter cannot import BGRA8 sRGB D3D11 render targets",
        ));
    }
    let mut external_image = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE);
    let image = unsafe {
        interop::raw_device(device).create_image(
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(BRIDGE_FORMAT)
                .extent(vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .push_next(&mut external_image),
            None,
        )
    }
    .map_err(|result| PresentError::from_vk("failed to create imported Vulkan image", result))?;
    let requirements = unsafe { interop::raw_device(device).get_image_memory_requirements(image) };
    let memory_properties = unsafe {
        interop::device_instance(device)
            .get_physical_device_memory_properties(interop::raw_physical_device(device))
    };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            requirements.memory_type_bits & (1 << index) != 0
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        })
        .ok_or_else(|| {
            PresentError::new(
                PresentErrorKind::Unsupported,
                "shared D3D texture has no device-local Vulkan memory type",
            )
        })?;
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let mut import = vk::ImportMemoryWin32HandleInfoKHR::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE)
        .handle(handle.0 as vk::HANDLE);
    import.p_next = (&mut dedicated as *mut vk::MemoryDedicatedAllocateInfo<'_>).cast();
    let memory = match unsafe {
        interop::raw_device(device).allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(memory_type_index)
                .push_next(&mut import),
            None,
        )
    } {
        Ok(memory) => memory,
        Err(result) => {
            unsafe { interop::raw_device(device).destroy_image(image, None) };
            return Err(PresentError::from_vk(
                "failed to import D3D texture memory",
                result,
            ));
        }
    };
    if let Err(result) = unsafe { interop::raw_device(device).bind_image_memory(image, memory, 0) }
    {
        unsafe {
            interop::raw_device(device).free_memory(memory, None);
            interop::raw_device(device).destroy_image(image, None);
        }
        return Err(PresentError::from_vk(
            "failed to bind imported D3D texture memory",
            result,
        ));
    }
    let view = match unsafe {
        interop::raw_device(device).create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(BRIDGE_FORMAT)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                }),
            None,
        )
    } {
        Ok(view) => view,
        Err(result) => {
            unsafe {
                interop::raw_device(device).free_memory(memory, None);
                interop::raw_device(device).destroy_image(image, None);
            }
            return Err(PresentError::from_vk(
                "failed to create imported D3D texture view",
                result,
            ));
        }
    };
    Ok(BridgeImage {
        d3d_texture,
        keyed_mutex,
        image,
        memory,
        view,
        initialized: false,
    })
}

fn destroy_images(device: &VulkanDevice, images: &mut Vec<BridgeImage>) {
    unsafe {
        for image in images.drain(..) {
            interop::raw_device(device).destroy_image_view(image.view, None);
            interop::raw_device(device).destroy_image(image.image, None);
            interop::raw_device(device).free_memory(image.memory, None);
            drop(image.d3d_texture);
        }
    }
}

fn win_error(context: &str, error: windows::core::Error) -> PresentError {
    PresentError::new(PresentErrorKind::Native, format!("{context}: {error}"))
}

fn map_dxgi_error(error: crate::graphics::presentation::PresentationError) -> PresentError {
    let kind = match error.kind() {
        crate::graphics::presentation::PresentationErrorKind::Unsupported => PresentErrorKind::Unsupported,
        crate::graphics::presentation::PresentationErrorKind::SurfaceLost => PresentErrorKind::SurfaceLost,
        crate::graphics::presentation::PresentationErrorKind::DeviceLost => PresentErrorKind::DeviceLost,
        crate::graphics::presentation::PresentationErrorKind::OutOfMemory => PresentErrorKind::OutOfMemory,
        crate::graphics::presentation::PresentationErrorKind::InvalidState => PresentErrorKind::InvalidState,
        crate::graphics::presentation::PresentationErrorKind::Native => PresentErrorKind::Native,
    };
    PresentError::new(kind, error.to_string())
}

unsafe fn acquire_keyed_mutex(
    mutex: &IDXGIKeyedMutex,
    key: u64,
    timeout_ms: u32,
) -> PresentResult<()> {
    let result = unsafe {
        (Interface::vtable(mutex).AcquireSync)(Interface::as_raw(mutex), key, timeout_ms)
    };
    if result.0 == 0 {
        Ok(())
    } else {
        Err(PresentError::new(
            PresentErrorKind::Native,
            format!(
                "failed to acquire the D3D bridge texture: keyed mutex returned 0x{:08X}",
                result.0 as u32
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{bridge_texture_desc, reserve_fence_pair};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX, D3D11_RESOURCE_MISC_SHARED_NTHANDLE,
    };
    #[test]
    fn bridge_texture_uses_the_nt_handle_sharing_pair() {
        let desc = bridge_texture_desc(1_280, 720);
        let required = (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0
            | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0) as u32;
        assert_eq!(desc.Width, 1_280);
        assert_eq!(desc.Height, 720);
        assert_eq!(desc.MiscFlags & required, required);
    }

    #[test]
    fn fence_values_advance_in_render_then_d3d_pairs() {
        let mut next = 1_u64;
        for expected in [(1, 2), (3, 4), (5, 6)] {
            assert_eq!(reserve_fence_pair(&mut next).unwrap(), expected);
        }
    }

    #[test]
    fn fence_exhaustion_does_not_advance_the_sequence() {
        for initial in [u64::MAX - 1, u64::MAX] {
            let mut next = initial;
            assert!(reserve_fence_pair(&mut next).is_err());
            assert_eq!(next, initial);
        }
    }
}
