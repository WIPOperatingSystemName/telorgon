use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::core::{RectI, SizeI};
use crate::gpu_abi::GpuView;
use crate::render::{
    AlphaMode, ColorSpace, RenderError, RenderErrorKind, RenderResult, RenderTargetInfo,
};
use ash::vk::{self, Handle};
use gpu_allocator::MemoryLocation;

use crate::renderer_vulkan::buffer::AllocatedBuffer;
use crate::renderer_vulkan::descriptor::{
    DescriptorLayouts, FrameDescriptorSets, allocate_frame_sets,
};
use crate::renderer_vulkan::device::{DeviceInner, DeviceOwnership, NEXT_DEVICE_ID};
use crate::renderer_vulkan::driver_workarounds::DriverWorkarounds;
use crate::renderer_vulkan::error::{unsupported, vk_error};
use crate::renderer_vulkan::external_image::{
    ExternalImageInner, HostedExternalImageUse, HostedExternalSemaphoreSignal,
    HostedExternalSemaphoreWait, VulkanExternalAcquire, VulkanExternalRelease,
};
use crate::renderer_vulkan::frame::{DescriptorBindingState, FrameCore, VulkanFrameContext};
use crate::renderer_vulkan::image::AllocatedImage;
use crate::renderer_vulkan::memory::{VulkanMemory, allocator_desc};
use crate::renderer_vulkan::pipeline::PipelineCache;
use crate::renderer_vulkan::target::{VulkanImageState, VulkanTarget};
use crate::renderer_vulkan::{VulkanCapabilities, VulkanConfig, VulkanDevice, VulkanInstance};

static NEXT_COMPLETION_DOMAIN_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HostedAllocationPolicy {
    /// Telorgon may create child Vulkan objects and allocate their backing memory. It never owns the
    /// instance, physical device, logical device, queue, or host command buffer.
    TelorgonManaged,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct HostedDeviceFeatures {
    pub dynamic_rendering: bool,
    pub synchronization2: bool,
    pub shader_demote_to_helper_invocation: bool,
}

/// External-resource extensions enabled by the host when it created the borrowed logical device.
///
/// Vulkan can report physical-device support after creation, but it cannot report which extensions
/// were enabled on an existing logical device. These booleans are therefore part of the unsafe
/// hosted-device contract and are checked against physical-device support before use.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct HostedDeviceExtensions {
    pub external_memory_fd: bool,
    pub external_memory_dma_buf: bool,
    pub image_drm_format_modifier: bool,
    pub external_semaphore_fd: bool,
    pub queue_family_foreign: bool,
}

impl HostedDeviceExtensions {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) const fn linux_dma_buf_complete(self) -> bool {
        self.external_memory_fd
            && self.external_memory_dma_buf
            && self.image_drm_format_modifier
            && self.external_semaphore_fd
            && self.queue_family_foreign
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HostedCommandBufferState {
    RecordingPrimaryOutsideRenderPass,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HostedImageUse {
    Undefined,
    ColorAttachment,
    ShaderRead,
    TransferSource,
    TransferDestination,
    General,
    Present,
    /// An exact host render-graph state when the semantic presets are insufficient.
    Custom {
        layout: vk::ImageLayout,
        stage: vk::PipelineStageFlags2,
        access: vk::AccessFlags2,
    },
}

impl HostedImageUse {
    pub(crate) fn state(self) -> VulkanImageState {
        match self {
            Self::Undefined => VulkanImageState::UNDEFINED,
            Self::ColorAttachment => VulkanImageState::COLOR_ATTACHMENT,
            Self::ShaderRead => VulkanImageState {
                layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                stage: vk::PipelineStageFlags2::ALL_GRAPHICS,
                access: vk::AccessFlags2::SHADER_SAMPLED_READ,
            },
            Self::TransferSource => VulkanImageState {
                layout: vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                stage: vk::PipelineStageFlags2::TRANSFER,
                access: vk::AccessFlags2::TRANSFER_READ,
            },
            Self::TransferDestination => VulkanImageState {
                layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                stage: vk::PipelineStageFlags2::TRANSFER,
                access: vk::AccessFlags2::TRANSFER_WRITE,
            },
            Self::General => VulkanImageState {
                layout: vk::ImageLayout::GENERAL,
                stage: vk::PipelineStageFlags2::ALL_COMMANDS,
                access: vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE,
            },
            Self::Present => VulkanImageState {
                layout: vk::ImageLayout::PRESENT_SRC_KHR,
                stage: vk::PipelineStageFlags2::NONE,
                access: vk::AccessFlags2::NONE,
            },
            Self::Custom {
                layout,
                stage,
                access,
            } => VulkanImageState {
                layout,
                stage,
                access,
            },
        }
    }
}

/// Host-owned native Vulkan objects used to create a command-only Telorgon device.
///
/// The feature booleans declare features enabled on `device`, not merely supported by the adapter.
pub struct HostedVulkanDeviceDescriptor<'host> {
    pub instance: &'host VulkanInstance,
    pub physical_device: vk::PhysicalDevice,
    pub device: &'host ash::Device,
    pub graphics_queue: vk::Queue,
    pub graphics_queue_family: u32,
    pub features: HostedDeviceFeatures,
    pub extensions: HostedDeviceExtensions,
    pub allocation_policy: HostedAllocationPolicy,
    pub completion_domain: HostCompletionDomain,
}

/// A host image and the exact rendering interval Telorgon may modify.
#[derive(Copy, Clone)]
pub struct HostedTargetDescriptor<'host> {
    pub image: vk::Image,
    pub view: vk::ImageView,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    pub region: RectI,
    pub usage: vk::ImageUsageFlags,
    pub sample_count: u8,
    pub color_space: ColorSpace,
    pub alpha_mode: AlphaMode,
    pub queue_family: u32,
    pub initial_use: HostedImageUse,
    pub final_use: HostedImageUse,
    _borrow: PhantomData<&'host mut vk::Image>,
}

impl<'host> HostedTargetDescriptor<'host> {
    /// Creates a lifetime-bearing description of a host-owned image.
    ///
    /// # Safety
    ///
    /// `image` and `view` must be live, compatible, belong to the hosted device, and remain live
    /// through completion of every command recorded from this descriptor. The declared format,
    /// extent, usage, queue family, and initial use must match the host's actual image state.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn new(
        image: vk::Image,
        view: vk::ImageView,
        format: vk::Format,
        extent: vk::Extent2D,
        region: RectI,
        usage: vk::ImageUsageFlags,
        queue_family: u32,
        initial_use: HostedImageUse,
        final_use: HostedImageUse,
        color_space: ColorSpace,
        alpha_mode: AlphaMode,
    ) -> Self {
        Self {
            image,
            view,
            format,
            extent,
            region,
            usage,
            sample_count: 1,
            color_space,
            alpha_mode,
            queue_family,
            initial_use,
            final_use,
            _borrow: PhantomData,
        }
    }
}

pub struct HostedFrameDescriptor<'host> {
    pub command_buffer: vk::CommandBuffer,
    pub command_buffer_state: HostedCommandBufferState,
    pub target: HostedTargetDescriptor<'host>,
    _borrow: PhantomData<&'host mut vk::CommandBuffer>,
}

impl<'host> HostedFrameDescriptor<'host> {
    /// Binds a host recording interval to one host target.
    ///
    /// # Safety
    ///
    /// The command buffer must be a recording primary command buffer outside a legacy render pass,
    /// belong to the hosted device and graphics queue family, and remain externally synchronized.
    /// Telorgon only appends commands; it never begins, ends, resets, or submits this buffer.
    pub unsafe fn new(
        command_buffer: vk::CommandBuffer,
        target: HostedTargetDescriptor<'host>,
    ) -> Self {
        Self {
            command_buffer,
            command_buffer_state: HostedCommandBufferState::RecordingPrimaryOutsideRenderPass,
            target,
            _borrow: PhantomData,
        }
    }
}

#[derive(Clone)]
pub struct HostCompletionDomain {
    inner: Arc<HostCompletionDomainInner>,
}

struct HostCompletionDomainInner {
    id: u64,
    bound_device: AtomicU64,
    state: Mutex<HostCompletionState>,
}

#[derive(Default)]
struct HostCompletionState {
    last_submitted: u64,
    completed: u64,
    retired: Vec<RetiredHostedFrame>,
    quarantine: Vec<RetiredHostedFrame>,
    violations: u64,
}

struct RetiredHostedFrame {
    completion: u64,
    frame_id: u64,
    _device: Arc<DeviceInner>,
    _arena: HostedFrameArena,
    _buffers: Vec<Arc<AllocatedBuffer>>,
    _images: Vec<Arc<AllocatedImage>>,
    external_images: Vec<Arc<ExternalImageInner>>,
}

impl Default for HostCompletionDomain {
    fn default() -> Self {
        Self::new()
    }
}

impl HostCompletionDomain {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(HostCompletionDomainInner {
                id: NEXT_COMPLETION_DOMAIN_ID.fetch_add(1, Ordering::Relaxed),
                bound_device: AtomicU64::new(0),
                state: Mutex::new(HostCompletionState::default()),
            }),
        }
    }

    /// Declares a nonzero, monotonically nondecreasing host submission value.
    pub fn point(&self, submitted_value: u64) -> RenderResult<HostCompletionPoint> {
        if submitted_value == 0 {
            return Err(host_contract("host completion values must be nonzero"));
        }
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| host_contract("host completion domain lock poisoned"))?;
        if submitted_value < state.last_submitted {
            return Err(host_contract(
                "host submission completion values must be monotonic",
            ));
        }
        state.last_submitted = submitted_value;
        Ok(HostCompletionPoint {
            domain_id: self.inner.id,
            value: submitted_value,
        })
    }

    pub fn completed_value(&self) -> u64 {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .completed
    }

    pub fn contract_violations(&self) -> u64 {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .violations
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HostCompletionPoint {
    domain_id: u64,
    value: u64,
}

impl HostCompletionPoint {
    pub fn value(self) -> u64 {
        self.value
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct HostedMaintenanceStats {
    pub completed_value: u64,
    pub released_frames: u32,
    pub released_buffers: u32,
    pub released_images: u32,
    pub released_external_images: u32,
    pub quarantined_frames: u32,
    pub contract_violations: u64,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct HostedRecordStats {
    pub frame_id: u64,
    pub recorded: bool,
    pub target_initial_use: Option<HostedImageUse>,
    pub target_final_use: Option<HostedImageUse>,
    pub pinned_buffers: u32,
    pub pinned_images: u32,
    pub external_image_reads: u32,
    pub command_buffers_begun: u32,
    pub command_buffers_ended: u32,
    pub submissions: u32,
    pub presentations: u32,
}

pub(crate) struct HostedDeviceState {
    domain: HostCompletionDomain,
    staging_bytes: u64,
}

struct HostedFrameArena {
    device: Arc<DeviceInner>,
    descriptor_pool: vk::DescriptorPool,
    staging: Arc<AllocatedBuffer>,
}

impl HostedFrameArena {
    fn create(
        device: &Arc<DeviceInner>,
        staging_bytes: u64,
    ) -> RenderResult<(Self, FrameDescriptorSets)> {
        if staging_bytes < size_of::<GpuView>() as u64 {
            return Err(RenderError::new(
                RenderErrorKind::OutOfMemory,
                "hosted Vulkan staging budget is too small for a view uniform",
            ));
        }
        let (descriptor_pool, descriptor_sets) = allocate_frame_sets(&device.raw, &device.layouts)?;
        let staging = match AllocatedBuffer::new(
            Arc::clone(device),
            staging_bytes,
            vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::UNIFORM_BUFFER
                | vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryLocation::CpuToGpu,
            "Telorgon hosted-frame staging",
        ) {
            Ok(buffer) => Arc::new(buffer),
            Err(error) => {
                unsafe { device.raw.destroy_descriptor_pool(descriptor_pool, None) };
                return Err(error);
            }
        };
        let view = [vk::DescriptorBufferInfo {
            buffer: staging.raw(),
            offset: 0,
            range: size_of::<GpuView>() as u64,
        }];
        let dummy = [vk::DescriptorBufferInfo {
            buffer: staging.raw(),
            offset: 0,
            range: staging.size(),
        }];
        unsafe {
            device.raw.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(descriptor_sets.view)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(&view)],
                &[],
            );
            device.raw.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(descriptor_sets.scene)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&dummy)],
                &[],
            );
        }
        Ok((
            Self {
                device: Arc::clone(device),
                descriptor_pool,
                staging,
            },
            descriptor_sets,
        ))
    }
}

impl Drop for HostedFrameArena {
    fn drop(&mut self) {
        unsafe {
            self.device
                .raw
                .destroy_descriptor_pool(self.descriptor_pool, None);
        }
    }
}

pub struct VulkanHostedFrame<'host> {
    device: &'host VulkanDevice,
    target: VulkanTarget<'host>,
    initial_use: HostedImageUse,
    final_use: HostedImageUse,
    core: Option<FrameCore>,
    arena: Option<HostedFrameArena>,
    _thread_bound: PhantomData<Rc<()>>,
}

#[must_use = "commit this receipt to host completion, or discard it only when not submitted"]
pub struct HostedFrameReceipt {
    domain: HostCompletionDomain,
    device: Option<Arc<DeviceInner>>,
    arena: Option<HostedFrameArena>,
    buffers: Vec<Arc<AllocatedBuffer>>,
    images: Vec<Arc<AllocatedImage>>,
    external_images: Vec<Arc<ExternalImageInner>>,
    external_waits: Vec<HostedExternalSemaphoreWait>,
    external_signals: Vec<HostedExternalSemaphoreSignal>,
    external_uses: Vec<HostedExternalImageUse>,
    stats: HostedRecordStats,
    resolved: bool,
}

impl HostedFrameReceipt {
    pub fn stats(&self) -> HostedRecordStats {
        self.stats
    }

    pub fn external_waits(&self) -> &[HostedExternalSemaphoreWait] {
        &self.external_waits
    }

    pub fn external_signals(&self) -> &[HostedExternalSemaphoreSignal] {
        &self.external_signals
    }

    pub fn external_image_uses(&self) -> &[HostedExternalImageUse] {
        &self.external_uses
    }

    /// Exports the one-shot sync FD for one submitted DMA-BUF lease generation.
    ///
    /// # Safety
    ///
    /// The host must first submit this receipt's returned wait, command buffer, and signal using
    /// the same externally synchronized queue. The submitted signal operation must remain pending
    /// or complete, and no other thread may export the same semaphore payload concurrently.
    #[cfg(target_os = "linux")]
    pub unsafe fn export_external_release_sync_fd(
        &self,
        image: vk::Image,
        lease_generation: u64,
    ) -> RenderResult<crate::renderer_vulkan::VulkanDmaBufReleaseSyncFd> {
        let external = self
            .external_images
            .iter()
            .find(|external| {
                external.image == image && external.lease_generation == lease_generation
            })
            .ok_or_else(|| {
                host_contract(
                    "hosted receipt does not contain the requested external image lease generation",
                )
            })?;
        unsafe { external.export_release_sync_fd() }?.ok_or_else(|| {
            host_contract("requested external lease is not an owning Linux DMA-BUF import")
        })
    }
}

impl VulkanDevice {
    /// Imports a host-owned Vulkan logical device for command-only recording.
    ///
    /// # Safety
    ///
    /// All descriptor handles must be compatible and live. The host must keep the native instance,
    /// device, and queue alive until every Telorgon device clone, scene, hosted frame, receipt, and
    /// completion-domain retirement has been released. The declared features must actually be
    /// enabled. The host remains responsible for command-buffer and queue external synchronization.
    pub unsafe fn from_hosted(
        descriptor: HostedVulkanDeviceDescriptor<'_>,
        config: &VulkanConfig,
    ) -> RenderResult<Self> {
        validate_hosted_device_descriptor(&descriptor)?;
        let raw = descriptor.device.clone();
        let memory = VulkanMemory::new(&allocator_desc(
            descriptor.instance.inner.raw.clone(),
            raw.clone(),
            descriptor.physical_device,
        ))?;
        let layouts = match DescriptorLayouts::new(&raw) {
            Ok(layouts) => layouts,
            Err(error) => {
                let mut memory = memory;
                unsafe { memory.destroy() };
                return Err(error);
            }
        };
        let sampler = match unsafe {
            raw.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            )
        } {
            Ok(sampler) => sampler,
            Err(result) => {
                drop(layouts);
                let mut memory = memory;
                unsafe { memory.destroy() };
                return Err(vk_error("failed to create hosted Vulkan sampler", result));
            }
        };
        let mut driver_properties = vk::PhysicalDeviceDriverProperties::default();
        let mut properties2 =
            vk::PhysicalDeviceProperties2::default().push_next(&mut driver_properties);
        unsafe {
            descriptor
                .instance
                .inner
                .raw
                .get_physical_device_properties2(descriptor.physical_device, &mut properties2)
        };
        let properties = properties2.properties;
        let adapter_name = unsafe { std::ffi::CStr::from_ptr(properties.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let format_support = |format: vk::Format| unsafe {
            descriptor
                .instance
                .inner
                .raw
                .get_physical_device_format_properties(descriptor.physical_device, format)
                .optimal_tiling_features
                .contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT)
        };
        let id = NEXT_DEVICE_ID.fetch_add(1, Ordering::Relaxed);
        if let Err(error) = descriptor.completion_domain.bind_device(id) {
            unsafe { raw.destroy_sampler(sampler, None) };
            drop(layouts);
            let mut memory = memory;
            unsafe { memory.destroy() };
            return Err(error);
        }
        let queue_lock = Arc::new(Mutex::new(()));
        let inner = Arc::new(DeviceInner {
            id,
            instance: descriptor.instance.clone(),
            pipelines: ManuallyDrop::new(Mutex::new(PipelineCache::new(&raw))),
            layouts: ManuallyDrop::new(layouts),
            raw,
            physical_device: descriptor.physical_device,
            queue: descriptor.graphics_queue,
            queue_family: descriptor.graphics_queue_family,
            queue_lock: Arc::clone(&queue_lock),
            present_queue: descriptor.graphics_queue,
            present_queue_family: descriptor.graphics_queue_family,
            present_queue_lock: queue_lock,
            completion_timeline: None,
            memory,
            sampler,
            capabilities: VulkanCapabilities {
                adapter_name,
                vendor_id: properties.vendor_id,
                device_id: properties.device_id,
                driver_version: properties.driver_version,
                api_version: properties.api_version,
                graphics_queue_family: descriptor.graphics_queue_family,
                presentation_enabled: false,
                swapchain_maintenance1: false,
                present_wait: false,
                dxgi_interop: false,
                device_luid: [0; vk::LUID_SIZE],
                rgba8_color_target: format_support(vk::Format::R8G8B8A8_UNORM),
                bgra8_srgb_color_target: format_support(vk::Format::B8G8R8A8_SRGB),
                #[cfg(feature = "instrumentation")]
                profiler_timestamp_valid_bits: 0,
                #[cfg(feature = "instrumentation")]
                profiler_timestamp_period_ns: properties.limits.timestamp_period,
            },
            device_local_budget_bytes: config.device_local_budget_bytes,
            driver_workarounds: DriverWorkarounds::for_driver(driver_properties.driver_id),
            device_local_reserved_bytes: AtomicU64::new(0),
            next_frame_id: AtomicU64::new(1),
            next_completion_value: AtomicU64::new(1),
            uniform_buffer_offset_alignment: properties
                .limits
                .min_uniform_buffer_offset_alignment
                .max(1),
            #[cfg(feature = "instrumentation")]
            profiler_timestamp_valid_bits: 0,
            #[cfg(feature = "instrumentation")]
            profiler_timestamp_period_ns: properties.limits.timestamp_period,
            ownership: DeviceOwnership::Hosted,
            owned_dma_buf_targets: false,
            owned_dma_buf_imports: false,
            hosted_extensions: descriptor.extensions,
        });
        inner.driver_workarounds.report(
            inner.instance.diagnostics(),
            &driver_properties,
            &inner.capabilities.adapter_name,
            inner.capabilities.driver_version,
        );
        Ok(Self {
            frames: None,
            inner,
            hosted: Some(Arc::new(HostedDeviceState {
                domain: descriptor.completion_domain,
                staging_bytes: config.staging_budget_bytes.max(size_of::<GpuView>() as u64),
            })),
        })
    }

    /// Starts a command-only recording interval in a host command buffer.
    ///
    /// # Safety
    ///
    /// The descriptor's native state and lifetime declarations must be true for this call and for
    /// every command Telorgon records. The call performs no command-buffer begin/end/reset or submit.
    pub unsafe fn begin_hosted_frame<'host>(
        &'host self,
        descriptor: HostedFrameDescriptor<'host>,
    ) -> RenderResult<VulkanHostedFrame<'host>> {
        let hosted = self
            .hosted
            .as_ref()
            .ok_or_else(|| host_contract("begin_hosted_frame requires a borrowed Vulkan device"))?;
        validate_hosted_frame_descriptor(self, &descriptor)?;
        let frame_id = self.inner.next_frame_id.fetch_add(1, Ordering::Relaxed);
        let (arena, descriptor_sets) = HostedFrameArena::create(&self.inner, hosted.staging_bytes)?;
        let target = VulkanTarget {
            device_id: self.inner.id,
            image: descriptor.target.image,
            view: descriptor.target.view,
            format: descriptor.target.format,
            extent: descriptor.target.extent,
            info: RenderTargetInfo {
                extent: SizeI {
                    width: descriptor.target.extent.width as i32,
                    height: descriptor.target.extent.height as i32,
                },
                region: descriptor.target.region,
                color_space: descriptor.target.color_space,
                alpha_mode: descriptor.target.alpha_mode,
                sample_count: descriptor.target.sample_count,
            },
            initial_state: descriptor.target.initial_use.state(),
            final_state: descriptor.target.final_use.state(),
            initial_queue_family: vk::QUEUE_FAMILY_IGNORED,
            final_queue_family: vk::QUEUE_FAMILY_IGNORED,
            _borrow: PhantomData,
        };
        let core = FrameCore {
            device: self.clone(),
            frame_id,
            command_buffer: descriptor.command_buffer,
            descriptor_sets,
            descriptor_bindings: DescriptorBindingState::default(),
            #[cfg(target_os = "linux")]
            composite_descriptor_pool: None,
            staging: Arc::clone(&arena.staging),
            staging_bytes_used: 0,
            buffers: Vec::new(),
            images: Vec::new(),
            external_images: Vec::new(),
            rendered: false,
            #[cfg(feature = "instrumentation")]
            profiler_query_pool: None,
            #[cfg(feature = "instrumentation")]
            profiler_timestamp_mask: 0,
            #[cfg(feature = "instrumentation")]
            profiler_timestamps_complete: false,
            #[cfg(feature = "instrumentation")]
            gpu_scopes: Vec::new(),
        };
        Ok(VulkanHostedFrame {
            device: self,
            target,
            initial_use: descriptor.target.initial_use,
            final_use: descriptor.target.final_use,
            core: Some(core),
            arena: Some(arena),
            _thread_bound: PhantomData,
        })
    }

    pub fn commit_hosted(
        &self,
        mut receipt: HostedFrameReceipt,
        point: HostCompletionPoint,
    ) -> RenderResult<()> {
        self.validate_receipt(&receipt, point.domain_id)?;
        if point.value == 0 {
            return Err(host_contract("host completion values must be nonzero"));
        }
        if receipt
            .external_images
            .iter()
            .any(|external| !external.submitted_release_is_resolved())
        {
            return Err(host_contract(
                "submitted DMA-BUF receipt requires release sync-FD export before commit",
            ));
        }
        let domain = receipt.domain.clone();
        let mut state = domain
            .inner
            .state
            .lock()
            .map_err(|_| host_contract("host completion domain lock poisoned"))?;
        if point.value > state.last_submitted {
            return Err(host_contract(
                "host completion point was not declared by this domain",
            ));
        }
        let retired = receipt.take_retired(point.value)?;
        state.retired.push(retired);
        receipt.resolved = true;
        Ok(())
    }

    /// Releases a hosted receipt only when its command buffer will not be submitted.
    pub fn discard_hosted(&self, mut receipt: HostedFrameReceipt) -> RenderResult<()> {
        self.validate_receipt(&receipt, receipt.domain.inner.id)?;
        receipt.device.take();
        receipt.arena.take();
        receipt.buffers.clear();
        receipt.images.clear();
        for external in &receipt.external_images {
            external.cancel_use(receipt.stats.frame_id);
        }
        receipt.external_images.clear();
        receipt.resolved = true;
        Ok(())
    }

    pub fn advance_host_completion(
        &self,
        domain: &HostCompletionDomain,
        completed_value: u64,
    ) -> RenderResult<HostedMaintenanceStats> {
        let hosted = self
            .hosted
            .as_ref()
            .ok_or_else(|| host_contract("host completion is unavailable on an owned device"))?;
        if hosted.domain.inner.id != domain.inner.id {
            return Err(host_contract(
                "host completion domain belongs to another device",
            ));
        }
        let mut state = domain
            .inner
            .state
            .lock()
            .map_err(|_| host_contract("host completion domain lock poisoned"))?;
        if completed_value < state.completed || completed_value > state.last_submitted {
            return Err(host_contract(
                "host completed value must be monotonic and no greater than the last submission",
            ));
        }
        state.completed = completed_value;
        let mut stats = HostedMaintenanceStats {
            completed_value,
            quarantined_frames: state.quarantine.len() as u32,
            contract_violations: state.violations,
            ..HostedMaintenanceStats::default()
        };
        let mut index = 0;
        while index < state.retired.len() {
            if state.retired[index].completion <= completed_value {
                let retired = state.retired.swap_remove(index);
                stats.released_frames += 1;
                stats.released_buffers += retired._buffers.len() as u32;
                stats.released_images += retired._images.len() as u32;
                stats.released_external_images += retired.external_images.len() as u32;
                for external in &retired.external_images {
                    external.complete_use(retired.frame_id);
                }
                drop(retired);
            } else {
                index += 1;
            }
        }
        Ok(stats)
    }

    fn validate_receipt(&self, receipt: &HostedFrameReceipt, domain_id: u64) -> RenderResult<()> {
        if receipt.resolved {
            return Err(host_contract("hosted frame receipt was already resolved"));
        }
        if receipt
            .device
            .as_ref()
            .is_none_or(|device| device.id != self.inner.id)
        {
            return Err(host_contract(
                "hosted frame receipt belongs to another device",
            ));
        }
        let hosted = self
            .hosted
            .as_ref()
            .ok_or_else(|| host_contract("hosted receipt used with an owned Vulkan device"))?;
        if receipt.domain.inner.id != hosted.domain.inner.id || domain_id != hosted.domain.inner.id
        {
            return Err(host_contract(
                "hosted frame receipt uses another completion domain",
            ));
        }
        Ok(())
    }
}

impl HostCompletionDomain {
    fn bind_device(&self, device_id: u64) -> RenderResult<()> {
        match self.inner.bound_device.compare_exchange(
            0,
            device_id,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(()),
            Err(current) if current == device_id => Ok(()),
            Err(_) => Err(host_contract(
                "a host completion domain cannot be shared by distinct Telorgon devices",
            )),
        }
    }
}

impl VulkanHostedFrame<'_> {
    pub fn frame_id(&self) -> u64 {
        self.core.as_ref().map_or(0, |core| core.frame_id)
    }

    pub fn context_and_target(&mut self) -> (VulkanFrameContext<'_>, VulkanTarget<'_>) {
        let target = self.target;
        let context = VulkanFrameContext {
            core: self.core.as_mut().expect("hosted frame already resolved"),
        };
        (context, target)
    }

    pub fn finish(mut self) -> RenderResult<HostedFrameReceipt> {
        let core = self
            .core
            .take()
            .ok_or_else(|| host_contract("hosted frame was already resolved"))?;
        let arena = self
            .arena
            .take()
            .ok_or_else(|| host_contract("hosted frame resources were already resolved"))?;
        let stats = HostedRecordStats {
            frame_id: core.frame_id,
            recorded: core.rendered,
            target_initial_use: core.rendered.then_some(self.initial_use),
            target_final_use: core.rendered.then_some(self.final_use),
            pinned_buffers: core.buffers.len() as u32,
            pinned_images: core.images.len() as u32,
            external_image_reads: core.external_images.len() as u32,
            ..HostedRecordStats::default()
        };
        let (external_waits, external_signals, external_uses) =
            external_reports(&core.external_images);
        Ok(HostedFrameReceipt {
            domain: self
                .device
                .hosted
                .as_ref()
                .expect("hosted frame has hosted state")
                .domain
                .clone(),
            device: Some(Arc::clone(&self.device.inner)),
            arena: Some(arena),
            buffers: core.buffers,
            images: core.images,
            external_images: core.external_images,
            external_waits,
            external_signals,
            external_uses,
            stats,
            resolved: false,
        })
    }

    /// Aborts the interval. The caller guarantees that the host command buffer will not execute.
    pub fn abort(mut self) -> RenderResult<()> {
        if let Some(core) = self.core.take() {
            for external in &core.external_images {
                external.cancel_use(core.frame_id);
            }
        }
        self.arena.take();
        Ok(())
    }
}

impl Drop for VulkanHostedFrame<'_> {
    fn drop(&mut self) {
        let Some(core) = self.core.take() else {
            return;
        };
        let Some(arena) = self.arena.take() else {
            return;
        };
        if !core.rendered {
            for external in &core.external_images {
                external.cancel_use(core.frame_id);
            }
            return;
        }
        let hosted = self
            .device
            .hosted
            .as_ref()
            .expect("hosted frame has hosted state");
        let mut state = hosted
            .domain
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.violations += 1;
        state.quarantine.push(RetiredHostedFrame {
            completion: u64::MAX,
            frame_id: core.frame_id,
            _device: Arc::clone(&self.device.inner),
            _arena: arena,
            _buffers: core.buffers,
            _images: core.images,
            external_images: core.external_images,
        });
    }
}

impl HostedFrameReceipt {
    fn take_retired(&mut self, completion: u64) -> RenderResult<RetiredHostedFrame> {
        Ok(RetiredHostedFrame {
            completion,
            frame_id: self.stats.frame_id,
            _device: self
                .device
                .take()
                .ok_or_else(|| host_contract("hosted receipt device pin is missing"))?,
            _arena: self
                .arena
                .take()
                .ok_or_else(|| host_contract("hosted receipt frame resources are missing"))?,
            _buffers: std::mem::take(&mut self.buffers),
            _images: std::mem::take(&mut self.images),
            external_images: std::mem::take(&mut self.external_images),
        })
    }
}

impl Drop for HostedFrameReceipt {
    fn drop(&mut self) {
        if self.resolved {
            return;
        }
        let (Some(device), Some(arena)) = (self.device.take(), self.arena.take()) else {
            return;
        };
        let mut state = self
            .domain
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.violations += 1;
        state.quarantine.push(RetiredHostedFrame {
            completion: u64::MAX,
            frame_id: self.stats.frame_id,
            _device: device,
            _arena: arena,
            _buffers: std::mem::take(&mut self.buffers),
            _images: std::mem::take(&mut self.images),
            external_images: std::mem::take(&mut self.external_images),
        });
    }
}

fn external_reports(
    images: &[Arc<ExternalImageInner>],
) -> (
    Vec<HostedExternalSemaphoreWait>,
    Vec<HostedExternalSemaphoreSignal>,
    Vec<HostedExternalImageUse>,
) {
    let mut waits = Vec::new();
    let mut signals = Vec::new();
    let mut uses = Vec::with_capacity(images.len());
    for image in images {
        if let VulkanExternalAcquire::BinarySemaphore(semaphore) = image.acquire {
            waits.push(HostedExternalSemaphoreWait {
                semaphore,
                stage_mask: vk::PipelineStageFlags2::FRAGMENT_SHADER,
            });
        }
        if let VulkanExternalRelease::BinarySemaphore(semaphore) = image.release {
            signals.push(HostedExternalSemaphoreSignal {
                semaphore,
                stage_mask: vk::PipelineStageFlags2::ALL_COMMANDS,
            });
        }
        uses.push(image.use_report());
    }
    (waits, signals, uses)
}

fn validate_hosted_device_descriptor(
    descriptor: &HostedVulkanDeviceDescriptor<'_>,
) -> RenderResult<()> {
    if descriptor.physical_device.is_null()
        || descriptor.graphics_queue.is_null()
        || descriptor.device.handle().is_null()
    {
        return Err(host_contract(
            "hosted Vulkan device handles must be non-null",
        ));
    }
    if !descriptor.features.dynamic_rendering
        || !descriptor.features.synchronization2
        || !descriptor.features.shader_demote_to_helper_invocation
    {
        return Err(unsupported(
            "hosted Vulkan requires dynamic rendering, synchronization2, and shader demote enabled",
        ));
    }
    let queue_families = unsafe {
        descriptor
            .instance
            .inner
            .raw
            .get_physical_device_queue_family_properties(descriptor.physical_device)
    };
    if queue_families
        .get(descriptor.graphics_queue_family as usize)
        .is_none_or(|properties| !properties.queue_flags.contains(vk::QueueFlags::GRAPHICS))
    {
        return Err(host_contract(
            "hosted Vulkan queue family is missing or is not graphics-capable",
        ));
    }
    validate_declared_device_extensions(descriptor)?;
    Ok(())
}

fn validate_declared_device_extensions(
    descriptor: &HostedVulkanDeviceDescriptor<'_>,
) -> RenderResult<()> {
    let declared = [
        (
            descriptor.extensions.external_memory_fd,
            ash::khr::external_memory_fd::NAME,
        ),
        (
            descriptor.extensions.external_memory_dma_buf,
            ash::ext::external_memory_dma_buf::NAME,
        ),
        (
            descriptor.extensions.image_drm_format_modifier,
            ash::ext::image_drm_format_modifier::NAME,
        ),
        (
            descriptor.extensions.external_semaphore_fd,
            ash::khr::external_semaphore_fd::NAME,
        ),
        (
            descriptor.extensions.queue_family_foreign,
            ash::ext::queue_family_foreign::NAME,
        ),
    ];
    if !declared.iter().any(|(enabled, _)| *enabled) {
        return Ok(());
    }
    let supported = unsafe {
        descriptor
            .instance
            .inner
            .raw
            .enumerate_device_extension_properties(descriptor.physical_device)
    }
    .map_err(|result| {
        vk_error(
            "failed to enumerate hosted Vulkan device extensions",
            result,
        )
    })?;
    for (enabled, name) in declared {
        if enabled
            && !supported.iter().any(|property| unsafe {
                std::ffi::CStr::from_ptr(property.extension_name.as_ptr()) == name
            })
        {
            return Err(host_contract(format!(
                "host declared unsupported Vulkan device extension {} as enabled",
                name.to_string_lossy()
            )));
        }
    }
    Ok(())
}

fn validate_hosted_frame_descriptor(
    device: &VulkanDevice,
    descriptor: &HostedFrameDescriptor<'_>,
) -> RenderResult<()> {
    let target = descriptor.target;
    if descriptor.command_buffer.is_null() || target.image.is_null() || target.view.is_null() {
        return Err(host_contract(
            "hosted command buffer, image, and image view must be non-null",
        ));
    }
    if target.format == vk::Format::UNDEFINED
        || target.extent.width == 0
        || target.extent.height == 0
        || target.extent.width > i32::MAX as u32
        || target.extent.height > i32::MAX as u32
        || target.sample_count != 1
    {
        return Err(RenderError::new(
            RenderErrorKind::InvalidTarget,
            "hosted target format, extent, or sample count is unsupported",
        ));
    }
    if !target.usage.contains(vk::ImageUsageFlags::COLOR_ATTACHMENT) {
        return Err(RenderError::new(
            RenderErrorKind::InvalidTarget,
            "hosted target was not created for color-attachment use",
        ));
    }
    let format_features = unsafe {
        device
            .inner
            .instance
            .inner
            .raw
            .get_physical_device_format_properties(device.inner.physical_device, target.format)
            .optimal_tiling_features
    };
    if !format_features.contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT) {
        return Err(RenderError::new(
            RenderErrorKind::InvalidTarget,
            "hosted target format is not color-attachment capable",
        ));
    }
    if target.queue_family != device.inner.queue_family {
        return Err(host_contract(
            "hosted target queue ownership transfers are not performed by Telorgon",
        ));
    }
    if target.final_use.state().layout == vk::ImageLayout::UNDEFINED {
        return Err(RenderError::new(
            RenderErrorKind::InvalidTarget,
            "hosted target final use cannot be undefined",
        ));
    }
    let right = target
        .region
        .x
        .checked_add(target.region.width)
        .filter(|right| *right <= target.extent.width as i32);
    let bottom = target
        .region
        .y
        .checked_add(target.region.height)
        .filter(|bottom| *bottom <= target.extent.height as i32);
    if target.region.x < 0
        || target.region.y < 0
        || target.region.width <= 0
        || target.region.height <= 0
        || right.is_none()
        || bottom.is_none()
    {
        return Err(RenderError::new(
            RenderErrorKind::InvalidTarget,
            "hosted render area lies outside its image extent",
        ));
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
    fn completion_domain_rejects_zero_and_regression() {
        let domain = HostCompletionDomain::new();
        assert_eq!(
            domain.point(0).unwrap_err().kind(),
            RenderErrorKind::HostContract
        );
        assert_eq!(domain.point(4).unwrap().value(), 4);
        assert_eq!(domain.point(4).unwrap().value(), 4);
        assert_eq!(
            domain.point(3).unwrap_err().kind(),
            RenderErrorKind::HostContract
        );
    }

    #[test]
    fn linux_dma_buf_contract_requires_every_external_extension() {
        let complete = HostedDeviceExtensions {
            external_memory_fd: true,
            external_memory_dma_buf: true,
            image_drm_format_modifier: true,
            external_semaphore_fd: true,
            queue_family_foreign: true,
        };
        assert!(complete.linux_dma_buf_complete());
        assert!(
            !HostedDeviceExtensions {
                external_semaphore_fd: false,
                ..complete
            }
            .linux_dma_buf_complete()
        );
    }
}
