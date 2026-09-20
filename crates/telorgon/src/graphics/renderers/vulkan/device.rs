use std::ffi::CStr;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::graphics::render::{RenderError, RenderErrorKind, RenderResult};
use ash::vk;

use crate::graphics::renderers::vulkan::descriptor::DescriptorLayouts;
use crate::graphics::renderers::vulkan::driver_workarounds::DriverWorkarounds;
use crate::graphics::renderers::vulkan::error::{internal, unsupported, vk_error};
use crate::graphics::renderers::vulkan::frame::{FrameSlots, VulkanRecordingFrame};
use crate::graphics::renderers::vulkan::interop::PresentationRequirement;
use crate::graphics::renderers::vulkan::memory::{VulkanMemory, allocator_desc};
use crate::graphics::renderers::vulkan::pipeline::PipelineCache;
use crate::graphics::renderers::vulkan::{DeviceSelection, VulkanConfig, VulkanDiagnostics, VulkanInstance};

pub(crate) static NEXT_DEVICE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct VulkanCapabilities {
    pub adapter_name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    /// Raw, vendor-defined Vulkan driver version from `VkPhysicalDeviceProperties`.
    pub driver_version: u32,
    pub api_version: u32,
    pub graphics_queue_family: u32,
    pub presentation_enabled: bool,
    pub swapchain_maintenance1: bool,
    pub present_wait: bool,
    /// The owned device can import D3D11 textures and a shared D3D fence on the same adapter.
    pub dxgi_interop: bool,
    /// DXGI adapter LUID reported by Vulkan. Meaningful only when `dxgi_interop` is true.
    pub device_luid: [u8; vk::LUID_SIZE],
    pub rgba8_color_target: bool,
    pub bgra8_srgb_color_target: bool,
    #[cfg(feature = "instrumentation")]
    pub profiler_timestamp_valid_bits: u32,
    #[cfg(feature = "instrumentation")]
    pub profiler_timestamp_period_ns: f32,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct VulkanMemoryMetrics {
    pub device_local_budget_bytes: Option<u64>,
    pub device_local_reserved_bytes: u64,
}

#[derive(Clone)]
pub struct VulkanDevice {
    pub(crate) frames: Option<Arc<FrameSlots>>,
    pub(crate) inner: Arc<DeviceInner>,
    pub(crate) hosted: Option<Arc<crate::graphics::renderers::vulkan::hosted::HostedDeviceState>>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeviceOwnership {
    Owned,
    Hosted,
}

pub(crate) struct DeviceInner {
    pub id: u64,
    pub instance: VulkanInstance,
    pub raw: ash::Device,
    pub physical_device: vk::PhysicalDevice,
    pub queue: vk::Queue,
    pub queue_family: u32,
    pub queue_lock: Arc<Mutex<()>>,
    pub present_queue: vk::Queue,
    pub present_queue_family: u32,
    pub present_queue_lock: Arc<Mutex<()>>,
    pub completion_timeline: Option<vk::Semaphore>,
    pub memory: VulkanMemory,
    pub sampler: vk::Sampler,
    pub layouts: ManuallyDrop<DescriptorLayouts>,
    pub pipelines: ManuallyDrop<Mutex<PipelineCache>>,
    pub capabilities: VulkanCapabilities,
    pub(crate) driver_workarounds: DriverWorkarounds,
    pub(crate) device_local_budget_bytes: Option<u64>,
    pub(crate) device_local_reserved_bytes: AtomicU64,
    pub(crate) next_frame_id: AtomicU64,
    pub next_completion_value: AtomicU64,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) uniform_buffer_offset_alignment: u64,
    #[cfg(feature = "instrumentation")]
    pub(crate) profiler_timestamp_valid_bits: u32,
    #[cfg(feature = "instrumentation")]
    pub(crate) profiler_timestamp_period_ns: f32,
    pub(crate) ownership: DeviceOwnership,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) owned_dma_buf_targets: bool,
    pub(crate) owned_dma_buf_imports: bool,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) hosted_extensions: crate::graphics::renderers::vulkan::hosted::HostedDeviceExtensions,
}

impl VulkanDevice {
    pub fn create_owned(
        instance: VulkanInstance,
        config: &VulkanConfig,
        selection: &DeviceSelection,
        presentation: Option<PresentationRequirement<'_>>,
    ) -> RenderResult<Self> {
        let physical_devices = unsafe { instance.inner.raw.enumerate_physical_devices() }
            .map_err(|result| vk_error("failed to enumerate Vulkan physical devices", result))?;
        let physical_device = physical_devices
            .get(selection.adapter_index)
            .copied()
            .ok_or_else(|| {
                RenderError::new(
                    RenderErrorKind::Unsupported,
                    format!(
                        "Vulkan adapter index {} does not exist",
                        selection.adapter_index
                    ),
                )
            })?;
        let mut id_properties = vk::PhysicalDeviceIDProperties::default();
        let mut driver_properties = vk::PhysicalDeviceDriverProperties::default();
        let mut properties2 = vk::PhysicalDeviceProperties2::default()
            .push_next(&mut id_properties)
            .push_next(&mut driver_properties);
        unsafe {
            instance
                .inner
                .raw
                .get_physical_device_properties2(physical_device, &mut properties2)
        };
        let properties = properties2.properties;
        if properties.api_version < vk::API_VERSION_1_3 {
            return Err(unsupported(
                "selected Vulkan adapter does not expose Vulkan 1.3",
            ));
        }
        if properties.device_type == vk::PhysicalDeviceType::CPU {
            return Err(unsupported(
                "CPU Vulkan adapters are not eligible for Telorgon's hardware renderer profile",
            ));
        }
        let queue_families = unsafe {
            instance
                .inner
                .raw
                .get_physical_device_queue_family_properties(physical_device)
        };
        let graphics_families = queue_families
            .iter()
            .enumerate()
            .filter_map(|(index, properties)| {
                properties
                    .queue_flags
                    .contains(vk::QueueFlags::GRAPHICS)
                    .then_some(index as u32)
            })
            .collect::<Vec<_>>();
        let queue_family = graphics_families
            .first()
            .copied()
            .ok_or_else(|| unsupported("selected Vulkan adapter has no graphics queue"))?;
        let present_families = if let Some(requirement) = presentation {
            let mut supported = Vec::new();
            for family in 0..queue_families.len() as u32 {
                if unsafe {
                    requirement
                        .surface
                        .loader
                        .get_physical_device_surface_support(
                            physical_device,
                            family,
                            requirement.surface.raw,
                        )
                }
                .map_err(|result| {
                    vk_error("failed to query Vulkan presentation queue support", result)
                })? {
                    supported.push(family);
                }
            }
            supported
        } else {
            vec![queue_family]
        };
        let common_family = graphics_families
            .iter()
            .copied()
            .find(|family| present_families.contains(family));
        let (queue_family, present_queue_family) = if let Some(common) = common_family {
            (common, common)
        } else {
            (
                queue_family,
                present_families.first().copied().ok_or_else(|| {
                    unsupported("selected Vulkan adapter has no presentation queue")
                })?,
            )
        };

        let device_extensions = unsafe {
            instance
                .inner
                .raw
                .enumerate_device_extension_properties(physical_device)
        }
        .map_err(|result| vk_error("failed to enumerate Vulkan device extensions", result))?;
        let swapchain_maintenance_available = presentation.is_some()
            && config.enable_swapchain_maintenance1
            && device_extensions.iter().any(|extension| unsafe {
                CStr::from_ptr(extension.extension_name.as_ptr())
                    == ash::ext::swapchain_maintenance1::NAME
            });
        let present_wait_extensions_available = presentation.is_some()
            && config.enable_present_wait
            && [ash::khr::present_id::NAME, ash::khr::present_wait::NAME]
                .into_iter()
                .all(|required| {
                    device_extensions.iter().any(|extension| unsafe {
                        CStr::from_ptr(extension.extension_name.as_ptr()) == required
                    })
                });
        let dxgi_extensions_available = cfg!(target_os = "windows")
            && config.enable_dxgi_presenter
            && [
                ash::khr::external_memory_win32::NAME,
                ash::khr::external_semaphore_win32::NAME,
                ash::khr::win32_keyed_mutex::NAME,
            ]
            .into_iter()
            .all(|required| {
                device_extensions.iter().any(|extension| unsafe {
                    CStr::from_ptr(extension.extension_name.as_ptr()) == required
                })
            });
        let mut external_semaphore_properties = vk::ExternalSemaphoreProperties::default();
        if dxgi_extensions_available {
            unsafe {
                instance
                    .inner
                    .raw
                    .get_physical_device_external_semaphore_properties(
                        physical_device,
                        &vk::PhysicalDeviceExternalSemaphoreInfo::default()
                            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::D3D12_FENCE),
                        &mut external_semaphore_properties,
                    )
            };
        }
        let dxgi_interop = dxgi_extensions_available
            && id_properties.device_luid_valid == vk::TRUE
            && external_semaphore_properties
                .external_semaphore_features
                .contains(vk::ExternalSemaphoreFeatureFlags::IMPORTABLE);
        #[cfg(target_os = "linux")]
        let owned_dma_buf_targets = [
            ash::khr::external_memory_fd::NAME,
            ash::ext::external_memory_dma_buf::NAME,
            ash::ext::image_drm_format_modifier::NAME,
            ash::ext::queue_family_foreign::NAME,
        ]
        .into_iter()
        .all(|required| {
            device_extensions.iter().any(|extension| unsafe {
                CStr::from_ptr(extension.extension_name.as_ptr()) == required
            })
        });
        #[cfg(target_os = "linux")]
        let owned_dma_buf_imports = owned_dma_buf_targets
            && device_extensions.iter().any(|extension| unsafe {
                CStr::from_ptr(extension.extension_name.as_ptr())
                    == ash::khr::external_semaphore_fd::NAME
            });
        #[cfg(not(target_os = "linux"))]
        let owned_dma_buf_targets = false;
        #[cfg(not(target_os = "linux"))]
        let owned_dma_buf_imports = false;
        let mut features12 = vk::PhysicalDeviceVulkan12Features::default();
        let mut features13 = vk::PhysicalDeviceVulkan13Features::default();
        let mut maintenance_features =
            vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default();
        let mut present_id_features = vk::PhysicalDevicePresentIdFeaturesKHR::default();
        let mut present_wait_features = vk::PhysicalDevicePresentWaitFeaturesKHR::default();
        let mut queried = vk::PhysicalDeviceFeatures2::default()
            .push_next(&mut features12)
            .push_next(&mut features13);
        if swapchain_maintenance_available {
            queried = queried.push_next(&mut maintenance_features);
        }
        if present_wait_extensions_available {
            queried = queried
                .push_next(&mut present_id_features)
                .push_next(&mut present_wait_features);
        }
        unsafe {
            instance
                .inner
                .raw
                .get_physical_device_features2(physical_device, &mut queried)
        };
        if features13.dynamic_rendering == vk::FALSE
            || features13.synchronization2 == vk::FALSE
            || features13.shader_demote_to_helper_invocation == vk::FALSE
        {
            return Err(unsupported(
                "selected Vulkan adapter lacks dynamic rendering, synchronization2, or shader demote-to-helper-invocation support",
            ));
        }
        if features12.timeline_semaphore == vk::FALSE {
            return Err(unsupported(
                "selected Vulkan adapter lacks timeline semaphore support",
            ));
        }
        let queue_priorities = [1.0_f32];
        let queue_families_to_create = if present_queue_family == queue_family {
            vec![queue_family]
        } else {
            vec![queue_family, present_queue_family]
        };
        let queue_info = queue_families_to_create
            .iter()
            .map(|family| {
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(*family)
                    .queue_priorities(&queue_priorities)
            })
            .collect::<Vec<_>>();
        let swapchain_maintenance1 = select_swapchain_maintenance1(
            presentation.is_some(),
            config.enable_swapchain_maintenance1,
            swapchain_maintenance_available,
            maintenance_features.swapchain_maintenance1 == vk::TRUE,
        );
        let present_wait = select_present_wait(
            presentation.is_some(),
            config.enable_present_wait,
            present_wait_extensions_available,
            present_id_features.present_id == vk::TRUE,
            present_wait_features.present_wait == vk::TRUE,
        );
        let mut extension_names = Vec::with_capacity(12);
        if presentation.is_some() {
            extension_names.push(ash::khr::swapchain::NAME.as_ptr());
        }
        if swapchain_maintenance1 {
            extension_names.push(ash::ext::swapchain_maintenance1::NAME.as_ptr());
        }
        if present_wait {
            extension_names.push(ash::khr::present_id::NAME.as_ptr());
            extension_names.push(ash::khr::present_wait::NAME.as_ptr());
        }
        if dxgi_interop {
            extension_names.push(ash::khr::external_memory_win32::NAME.as_ptr());
            extension_names.push(ash::khr::external_semaphore_win32::NAME.as_ptr());
            extension_names.push(ash::khr::win32_keyed_mutex::NAME.as_ptr());
        }
        #[cfg(target_os = "linux")]
        if owned_dma_buf_targets {
            extension_names.push(ash::khr::external_memory_fd::NAME.as_ptr());
            extension_names.push(ash::ext::external_memory_dma_buf::NAME.as_ptr());
            extension_names.push(ash::ext::image_drm_format_modifier::NAME.as_ptr());
            extension_names.push(ash::ext::queue_family_foreign::NAME.as_ptr());
        }
        #[cfg(target_os = "linux")]
        if owned_dma_buf_imports {
            extension_names.push(ash::khr::external_semaphore_fd::NAME.as_ptr());
        }
        let mut enabled13 = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(true)
            .synchronization2(true)
            .shader_demote_to_helper_invocation(true);
        let mut enabled12 = vk::PhysicalDeviceVulkan12Features::default().timeline_semaphore(true);
        let mut enabled_maintenance = vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default()
            .swapchain_maintenance1(true);
        let mut enabled_present_id =
            vk::PhysicalDevicePresentIdFeaturesKHR::default().present_id(true);
        let mut enabled_present_wait =
            vk::PhysicalDevicePresentWaitFeaturesKHR::default().present_wait(true);
        let mut create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_info)
            .enabled_extension_names(&extension_names)
            .push_next(&mut enabled12)
            .push_next(&mut enabled13);
        if swapchain_maintenance1 {
            create_info = create_info.push_next(&mut enabled_maintenance);
        }
        if present_wait {
            create_info = create_info
                .push_next(&mut enabled_present_id)
                .push_next(&mut enabled_present_wait);
        }
        let raw = unsafe {
            instance
                .inner
                .raw
                .create_device(physical_device, &create_info, None)
        }
        .map_err(|result| vk_error("failed to create Vulkan device", result))?;
        let queue = unsafe { raw.get_device_queue(queue_family, 0) };
        let present_queue = unsafe { raw.get_device_queue(present_queue_family, 0) };
        let queue_lock = Arc::new(Mutex::new(()));
        let present_queue_lock = if present_queue_family == queue_family {
            queue_lock.clone()
        } else {
            Arc::new(Mutex::new(()))
        };
        let mut timeline_info = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let completion_timeline = match unsafe {
            raw.create_semaphore(
                &vk::SemaphoreCreateInfo::default().push_next(&mut timeline_info),
                None,
            )
        } {
            Ok(semaphore) => semaphore,
            Err(result) => {
                unsafe { raw.destroy_device(None) };
                return Err(vk_error(
                    "failed to create Vulkan completion timeline",
                    result,
                ));
            }
        };
        let memory = match VulkanMemory::new(&allocator_desc(
            instance.inner.raw.clone(),
            raw.clone(),
            physical_device,
        )) {
            Ok(memory) => memory,
            Err(error) => {
                unsafe {
                    raw.destroy_semaphore(completion_timeline, None);
                    raw.destroy_device(None);
                }
                return Err(error);
            }
        };
        let layouts = match DescriptorLayouts::new(&raw) {
            Ok(layouts) => layouts,
            Err(error) => {
                let mut memory = memory;
                unsafe { memory.destroy() };
                unsafe {
                    raw.destroy_semaphore(completion_timeline, None);
                    raw.destroy_device(None);
                }
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
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .min_lod(0.0)
                    .max_lod(0.0),
                None,
            )
        } {
            Ok(sampler) => sampler,
            Err(result) => {
                drop(layouts);
                let mut memory = memory;
                unsafe { memory.destroy() };
                unsafe {
                    raw.destroy_semaphore(completion_timeline, None);
                    raw.destroy_device(None);
                }
                return Err(vk_error("failed to create Vulkan image sampler", result));
            }
        };
        let format_support = |format: vk::Format| unsafe {
            instance
                .inner
                .raw
                .get_physical_device_format_properties(physical_device, format)
                .optimal_tiling_features
                .contains(
                    vk::FormatFeatureFlags::COLOR_ATTACHMENT | vk::FormatFeatureFlags::TRANSFER_SRC,
                )
        };
        let adapter_name = unsafe { std::ffi::CStr::from_ptr(properties.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        #[cfg(feature = "instrumentation")]
        let profiler_timestamp_valid_bits =
            queue_families[queue_family as usize].timestamp_valid_bits;
        #[cfg(feature = "instrumentation")]
        let profiler_timestamp_period_ns = properties.limits.timestamp_period;
        let capabilities = VulkanCapabilities {
            adapter_name,
            vendor_id: properties.vendor_id,
            device_id: properties.device_id,
            driver_version: properties.driver_version,
            api_version: properties.api_version,
            graphics_queue_family: queue_family,
            presentation_enabled: presentation.is_some(),
            swapchain_maintenance1,
            present_wait,
            dxgi_interop,
            device_luid: id_properties.device_luid,
            rgba8_color_target: format_support(vk::Format::R8G8B8A8_UNORM),
            bgra8_srgb_color_target: format_support(vk::Format::B8G8R8A8_SRGB),
            #[cfg(feature = "instrumentation")]
            profiler_timestamp_valid_bits,
            #[cfg(feature = "instrumentation")]
            profiler_timestamp_period_ns,
        };
        if !capabilities.rgba8_color_target {
            unsafe { raw.destroy_sampler(sampler, None) };
            drop(layouts);
            let mut memory = memory;
            unsafe { memory.destroy() };
            unsafe {
                raw.destroy_semaphore(completion_timeline, None);
                raw.destroy_device(None);
            }
            return Err(unsupported(
                "selected Vulkan adapter cannot render and copy R8G8B8A8_UNORM",
            ));
        }
        let inner = Arc::new(DeviceInner {
            id: NEXT_DEVICE_ID.fetch_add(1, Ordering::Relaxed),
            instance,
            pipelines: ManuallyDrop::new(Mutex::new(PipelineCache::new(&raw))),
            layouts: ManuallyDrop::new(layouts),
            raw,
            physical_device,
            queue,
            queue_family,
            queue_lock,
            present_queue,
            present_queue_family,
            present_queue_lock,
            completion_timeline: Some(completion_timeline),
            memory,
            sampler,
            capabilities,
            driver_workarounds: DriverWorkarounds::for_driver(driver_properties.driver_id),
            device_local_budget_bytes: config.device_local_budget_bytes,
            device_local_reserved_bytes: AtomicU64::new(0),
            next_frame_id: AtomicU64::new(1),
            next_completion_value: AtomicU64::new(1),
            uniform_buffer_offset_alignment: properties
                .limits
                .min_uniform_buffer_offset_alignment
                .max(1),
            #[cfg(feature = "instrumentation")]
            profiler_timestamp_valid_bits,
            #[cfg(feature = "instrumentation")]
            profiler_timestamp_period_ns,
            ownership: DeviceOwnership::Owned,
            owned_dma_buf_targets,
            owned_dma_buf_imports,
            hosted_extensions: crate::graphics::renderers::vulkan::hosted::HostedDeviceExtensions::default(),
        });
        let frames = FrameSlots::create(
            Arc::clone(&inner),
            config.frames_in_flight,
            config.staging_budget_bytes,
        )?;
        #[cfg(feature = "instrumentation")]
        if inner.profiler_timestamp_valid_bits > 0 && crate::runtime::instrumentation::is_active() {
            crate::runtime::instrumentation::instant!("gpu.timestamps.available");
            crate::runtime::instrumentation::counter!(
                "gpu.timestamp.valid_bits",
                inner.profiler_timestamp_valid_bits
            );
            crate::runtime::instrumentation::counter!(
                "gpu.timestamp.period_ns",
                inner.profiler_timestamp_period_ns
            );
        } else if crate::runtime::instrumentation::is_active() {
            crate::runtime::instrumentation::instant!("gpu.timestamps.unavailable");
        }
        #[cfg(feature = "instrumentation")]
        if crate::runtime::instrumentation::is_active() {
            crate::runtime::instrumentation::counter!("gpu.adapter.vendor_id", inner.capabilities.vendor_id);
            crate::runtime::instrumentation::counter!("gpu.adapter.device_id", inner.capabilities.device_id);
            crate::runtime::instrumentation::counter!(
                "gpu.adapter.driver_version",
                inner.capabilities.driver_version
            );
            crate::runtime::instrumentation::counter!("gpu.adapter.api_version", inner.capabilities.api_version);
            crate::runtime::instrumentation::counter!(
                "gpu.swapchain_maintenance1",
                u8::from(inner.capabilities.swapchain_maintenance1)
            );
            crate::runtime::instrumentation::counter!(
                "gpu.present_wait",
                u8::from(inner.capabilities.present_wait)
            );
        }
        inner.driver_workarounds.report(
            inner.instance.diagnostics(),
            &driver_properties,
            &inner.capabilities.adapter_name,
            inner.capabilities.driver_version,
        );
        Ok(Self {
            frames: Some(frames),
            inner,
            hosted: None,
        })
    }

    pub fn capabilities(&self) -> &VulkanCapabilities {
        &self.inner.capabilities
    }

    pub fn diagnostics(&self) -> &VulkanDiagnostics {
        self.inner.instance.diagnostics()
    }

    pub fn memory_metrics(&self) -> VulkanMemoryMetrics {
        VulkanMemoryMetrics {
            device_local_budget_bytes: self.inner.device_local_budget_bytes,
            device_local_reserved_bytes: self
                .inner
                .device_local_reserved_bytes
                .load(Ordering::Acquire),
        }
    }

    pub fn begin_owned_frame(&self) -> RenderResult<VulkanRecordingFrame<'_>> {
        if self.inner.ownership != DeviceOwnership::Owned {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "begin_owned_frame is unavailable for a borrowed Vulkan device",
            ));
        }
        VulkanRecordingFrame::begin(
            self,
            self.inner.next_frame_id.fetch_add(1, Ordering::Relaxed),
        )
    }

    /// Attempts to reserve an owned recording frame without waiting for GPU work.
    ///
    /// `Ok(None)` is normal back-pressure: every configured frame slot is still recording or in
    /// flight. Managed hosts should retry on a later redraw instead of treating it as a failure.
    pub fn try_begin_owned_frame(&self) -> RenderResult<Option<VulkanRecordingFrame<'_>>> {
        if self.inner.ownership != DeviceOwnership::Owned {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "try_begin_owned_frame is unavailable for a borrowed Vulkan device",
            ));
        }
        VulkanRecordingFrame::try_begin(
            self,
            self.inner.next_frame_id.fetch_add(1, Ordering::Relaxed),
        )
    }

    pub(crate) fn prewarm_compositor_pipelines(&self, formats: &[vk::Format]) -> RenderResult<()> {
        use crate::graphics::render::{BlendMode, PipelineKind};
        let kinds = [
            PipelineKind::AnalyticBox,
            PipelineKind::Glyph,
            PipelineKind::Image,
            PipelineKind::Material,
            PipelineKind::GaussianBlur,
            PipelineKind::LiquidGlass,
        ];
        for &format in formats {
            for kind in kinds {
                for blend in [BlendMode::Opaque, BlendMode::Alpha, BlendMode::Add] {
                    self.pipeline(format, kind, blend)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn pipeline(
        &self,
        format: vk::Format,
        kind: crate::graphics::render::PipelineKind,
        blend_mode: crate::graphics::render::BlendMode,
    ) -> RenderResult<vk::Pipeline> {
        self.inner
            .pipelines
            .lock()
            .map_err(|_| internal("Vulkan pipeline cache lock poisoned"))?
            .get_or_create(format, kind, blend_mode, &self.inner.layouts)
    }
}

pub(crate) struct DeviceLocalReservation<'device> {
    device: &'device DeviceInner,
    bytes: u64,
    committed: bool,
}

impl DeviceInner {
    pub(crate) fn reserve_device_local(
        &self,
        bytes: u64,
    ) -> RenderResult<DeviceLocalReservation<'_>> {
        let mut current = self.device_local_reserved_bytes.load(Ordering::Acquire);
        loop {
            let next = reserved_after(current, bytes, self.device_local_budget_bytes)?;
            match self.device_local_reserved_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(DeviceLocalReservation {
                        device: self,
                        bytes,
                        committed: false,
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    pub(crate) fn release_device_local(&self, bytes: u64) {
        if bytes != 0 {
            self.device_local_reserved_bytes
                .fetch_sub(bytes, Ordering::AcqRel);
        }
    }
}

fn reserved_after(current: u64, requested: u64, budget: Option<u64>) -> RenderResult<u64> {
    let next = current.checked_add(requested).ok_or_else(|| {
        RenderError::new(
            RenderErrorKind::OutOfMemory,
            "Vulkan device-local reservation overflow",
        )
    })?;
    if budget.is_some_and(|limit| next > limit) {
        return Err(RenderError::new(
            RenderErrorKind::OutOfMemory,
            format!(
                "Vulkan device-local budget exceeded: {next} bytes requested with a {} byte limit",
                budget.unwrap_or_default()
            ),
        ));
    }
    Ok(next)
}

const fn select_swapchain_maintenance1(
    presentation_enabled: bool,
    config_enabled: bool,
    extension_available: bool,
    feature_available: bool,
) -> bool {
    presentation_enabled && config_enabled && extension_available && feature_available
}

const fn select_present_wait(
    presentation_enabled: bool,
    config_enabled: bool,
    extensions_available: bool,
    present_id_feature_available: bool,
    present_wait_feature_available: bool,
) -> bool {
    presentation_enabled
        && config_enabled
        && extensions_available
        && present_id_feature_available
        && present_wait_feature_available
}

impl DeviceLocalReservation<'_> {
    pub(crate) fn commit(mut self) -> u64 {
        self.committed = true;
        self.bytes
    }
}

impl Drop for DeviceLocalReservation<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.device.release_device_local(self.bytes);
        }
    }
}

impl Drop for DeviceInner {
    fn drop(&mut self) {
        unsafe {
            if self.ownership == DeviceOwnership::Owned {
                let _ = self.raw.device_wait_idle();
            }
            ManuallyDrop::drop(&mut self.pipelines);
            ManuallyDrop::drop(&mut self.layouts);
            self.raw.destroy_sampler(self.sampler, None);
            self.memory.destroy();
            if let Some(completion_timeline) = self.completion_timeline {
                self.raw.destroy_semaphore(completion_timeline, None);
            }
            if self.ownership == DeviceOwnership::Owned {
                self.raw.destroy_device(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{reserved_after, select_present_wait, select_swapchain_maintenance1};
    use crate::graphics::render::RenderErrorKind;

    #[test]
    fn device_local_budget_accepts_the_limit_and_rejects_exhaustion() {
        assert_eq!(reserved_after(64, 32, Some(96)).unwrap(), 96);
        assert_eq!(
            reserved_after(64, 33, Some(96)).unwrap_err().kind(),
            RenderErrorKind::OutOfMemory
        );
        assert_eq!(
            reserved_after(u64::MAX, 1, None).unwrap_err().kind(),
            RenderErrorKind::OutOfMemory
        );
    }

    #[test]
    fn swapchain_maintenance_requires_every_capability_and_policy_gate() {
        assert!(select_swapchain_maintenance1(true, true, true, true));
        assert!(!select_swapchain_maintenance1(false, true, true, true));
        assert!(!select_swapchain_maintenance1(true, false, true, true));
        assert!(!select_swapchain_maintenance1(true, true, false, true));
        assert!(!select_swapchain_maintenance1(true, true, true, false));
    }

    #[test]
    fn present_wait_requires_both_features_extensions_and_policy() {
        assert!(select_present_wait(true, true, true, true, true));
        assert!(!select_present_wait(false, true, true, true, true));
        assert!(!select_present_wait(true, false, true, true, true));
        assert!(!select_present_wait(true, true, false, true, true));
        assert!(!select_present_wait(true, true, true, false, true));
        assert!(!select_present_wait(true, true, true, true, false));
    }
}
