use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::graphics::gpu_abi::GpuView;
use crate::graphics::render::{RenderError, RenderErrorKind, RenderResult};
use ash::vk;
use gpu_allocator::MemoryLocation;

use crate::graphics::renderers::vulkan::VulkanDevice;
use crate::graphics::renderers::vulkan::buffer::AllocatedBuffer;
use crate::graphics::renderers::vulkan::descriptor::allocate_frame_sets;
#[cfg(target_os = "linux")]
use crate::graphics::renderers::vulkan::descriptor::create_composite_descriptor_pool;
use crate::graphics::renderers::vulkan::descriptor::{
    FrameDescriptorSets, MAX_TEXTURE_SETS, PRIMITIVE_SET_COUNT,
};
use crate::graphics::renderers::vulkan::device::DeviceInner;
use crate::graphics::renderers::vulkan::error::{internal, vk_error};
use crate::graphics::renderers::vulkan::external_image::ExternalImageInner;
use crate::graphics::renderers::vulkan::image::AllocatedImage;

#[cfg(feature = "instrumentation")]
pub(crate) const PROFILER_TIMESTAMP_UPLOAD_END: u32 = 1;
#[cfg(feature = "instrumentation")]
pub(crate) const PROFILER_TIMESTAMP_RENDER_BEGIN: u32 = 2;
#[cfg(feature = "instrumentation")]
pub(crate) const PROFILER_TIMESTAMP_RENDER_END: u32 = 3;
#[cfg(feature = "instrumentation")]
pub(crate) const PROFILER_TIMESTAMP_TOTAL_END: u32 = 4;
#[cfg(feature = "instrumentation")]
const PROFILER_TIMESTAMP_QUERY_COUNT: u32 = 517;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PrimitiveBindingState {
    pub(crate) instances: vk::Buffer,
    pub(crate) instances_generation: u64,
    pub(crate) parameters: vk::Buffer,
    pub(crate) parameters_generation: u64,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextureBindingState {
    pub(crate) views: [vk::ImageView; 2],
    pub(crate) generations: [u64; 2],
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct DescriptorBindingState {
    pub(crate) scene_id: u64,
    pub(crate) spatial: vk::Buffer,
    pub(crate) spatial_generation: u64,
    pub(crate) draw_indices: vk::Buffer,
    pub(crate) draw_indices_generation: u64,
    pub(crate) boxes: vk::Buffer,
    pub(crate) boxes_generation: u64,
    pub(crate) clips: vk::Buffer,
    pub(crate) clips_generation: u64,
    pub(crate) primitives: [PrimitiveBindingState; PRIMITIVE_SET_COUNT],
    pub(crate) textures: [TextureBindingState; MAX_TEXTURE_SETS],
}

impl Default for DescriptorBindingState {
    fn default() -> Self {
        Self {
            scene_id: 0,
            spatial: vk::Buffer::null(),
            spatial_generation: 0,
            draw_indices: vk::Buffer::null(),
            draw_indices_generation: 0,
            boxes: vk::Buffer::null(),
            boxes_generation: 0,
            clips: vk::Buffer::null(),
            clips_generation: 0,
            primitives: [PrimitiveBindingState::default(); PRIMITIVE_SET_COUNT],
            textures: [TextureBindingState::default(); MAX_TEXTURE_SETS],
        }
    }
}

pub struct VulkanFrameContext<'frame> {
    pub(crate) core: &'frame mut FrameCore,
}

pub(crate) struct FrameCore {
    pub(crate) device: VulkanDevice,
    pub(crate) frame_id: u64,
    pub(crate) command_buffer: vk::CommandBuffer,
    pub(crate) descriptor_sets: FrameDescriptorSets,
    pub(crate) descriptor_bindings: DescriptorBindingState,
    #[cfg(target_os = "linux")]
    pub(crate) composite_descriptor_pool:
        Option<Arc<Mutex<super::descriptor::CompositeDescriptorArena>>>,
    pub(crate) staging: Arc<AllocatedBuffer>,
    /// Bytes already assigned in the reusable staging stream by earlier render passes.
    pub(crate) staging_bytes_used: usize,
    pub(crate) buffers: Vec<Arc<AllocatedBuffer>>,
    pub(crate) images: Vec<Arc<AllocatedImage>>,
    // Additional owned targets follow the same submission/retirement lifetime as images.
    pub(crate) resources: Vec<Arc<dyn Send + Sync>>,
    pub(crate) external_images: Vec<Arc<ExternalImageInner>>,
    pub(crate) rendered: bool,
    #[cfg(feature = "instrumentation")]
    pub(crate) profiler_query_pool: Option<vk::QueryPool>,
    #[cfg(feature = "instrumentation")]
    pub(crate) profiler_timestamp_mask: u32,
    #[cfg(feature = "instrumentation")]
    pub(crate) profiler_timestamps_complete: bool,
    #[cfg(feature = "instrumentation")]
    pub(crate) gpu_scopes: Vec<&'static str>,
}

#[cfg(feature = "instrumentation")]
impl FrameCore {
    pub(crate) fn write_profiler_timestamp(&mut self, query: u32, stage: vk::PipelineStageFlags2) {
        let Some(pool) = self.profiler_query_pool else {
            return;
        };
        let bit = 1_u32.checked_shl(query).unwrap_or(0);
        if bit == 0 || self.profiler_timestamp_mask & bit != 0 {
            return;
        }
        self.profiler_timestamp_mask |= bit;
        unsafe {
            self.device
                .inner
                .raw
                .cmd_write_timestamp2(self.command_buffer, stage, pool, query);
        }
        if query == PROFILER_TIMESTAMP_TOTAL_END {
            self.profiler_timestamps_complete = true;
        }
    }
}

impl FrameCore {
    pub(crate) fn begin_gpu_scope(&mut self, label: &'static str) -> Option<u32> {
        #[cfg(feature = "instrumentation")]
        {
            let pool = self.profiler_query_pool?;
            let query = 5 + self.gpu_scopes.len() as u32 * 2;
            if query + 1 >= PROFILER_TIMESTAMP_QUERY_COUNT {
                return None;
            }
            self.gpu_scopes.push(label);
            unsafe {
                self.device.inner.raw.cmd_write_timestamp2(
                    self.command_buffer,
                    vk::PipelineStageFlags2::ALL_COMMANDS,
                    pool,
                    query,
                );
            }
            Some(query)
        }
        #[cfg(not(feature = "instrumentation"))]
        {
            let _ = label;
            None
        }
    }
    pub(crate) fn end_gpu_scope(&mut self, query: Option<u32>) {
        #[cfg(feature = "instrumentation")]
        if let (Some(query), Some(pool)) = (query, self.profiler_query_pool) {
            unsafe {
                self.device.inner.raw.cmd_write_timestamp2(
                    self.command_buffer,
                    vk::PipelineStageFlags2::ALL_COMMANDS,
                    pool,
                    query + 1,
                );
            }
        }
        #[cfg(not(feature = "instrumentation"))]
        let _ = query;
    }
}

pub struct VulkanRecordingFrame<'device> {
    device: &'device VulkanDevice,
    frames: Arc<FrameSlots>,
    frame_id: u64,
    slot_index: Option<usize>,
    core: Option<FrameCore>,
    _thread_bound: PhantomData<Rc<()>>,
}

#[must_use = "a recorded frame must be submitted or explicitly dropped"]
pub struct VulkanRecordedFrame {
    pub(crate) device: Arc<DeviceInner>,
    frames: Arc<FrameSlots>,
    pub(crate) device_id: u64,
    pub(crate) frame_id: u64,
    pub(crate) command_buffer: vk::CommandBuffer,
    slot_index: Option<usize>,
    pub(crate) buffers: Vec<Arc<AllocatedBuffer>>,
    pub(crate) images: Vec<Arc<AllocatedImage>>,
    // Additional owned targets follow the same submission/retirement lifetime as images.
    pub(crate) resources: Vec<Arc<dyn Send + Sync>>,
    pub(crate) external_images: Vec<Arc<ExternalImageInner>>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CompletionPoint {
    device_id: u64,
    frame_id: u64,
    value: u64,
}

impl CompletionPoint {
    pub fn device_id(self) -> u64 {
        self.device_id
    }

    pub fn frame_id(self) -> u64 {
        self.frame_id
    }

    pub fn value(self) -> u64 {
        self.value
    }
}

#[must_use = "retain the receipt until completion or allow device-owned deferred retirement"]
pub struct SubmissionReceipt {
    timing: Option<super::submission_timing::TimingResult>,
    submit_started: std::time::Instant,
    submitted: std::time::Instant,
    device: Arc<DeviceInner>,
    frames: Arc<FrameSlots>,
    completion: CompletionPoint,
    buffers: Vec<Arc<AllocatedBuffer>>,
    images: Vec<Arc<AllocatedImage>>,
    resources: Vec<Arc<dyn Send + Sync>>,
    external_images: Vec<Arc<ExternalImageInner>>,
    completed: bool,
}

struct RetiredSubmission {
    completion_value: u64,
    frame_id: u64,
    _buffers: Vec<Arc<AllocatedBuffer>>,
    _images: Vec<Arc<AllocatedImage>>,
    _resources: Vec<Arc<dyn Send + Sync>>,
    external_images: Vec<Arc<ExternalImageInner>>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SlotState {
    Available,
    Recording { frame_id: u64 },
    Recorded { frame_id: u64 },
    InFlight { frame_id: u64, completion: u64 },
}

struct FrameSlot {
    device: ash::Device,
    state: SlotState,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    descriptor_pool: vk::DescriptorPool,
    descriptor_sets: FrameDescriptorSets,
    descriptor_bindings: DescriptorBindingState,
    #[cfg(target_os = "linux")]
    composite_descriptor_pool: Arc<Mutex<super::descriptor::CompositeDescriptorArena>>,
    staging: Arc<AllocatedBuffer>,
    #[cfg(feature = "instrumentation")]
    profiler_timestamps: Option<ProfilerTimestampQueries>,
}

#[cfg(feature = "instrumentation")]
struct ProfilerTimestampQueries {
    timing: super::submission_timing::TimingResult,
    pool: vk::QueryPool,
    valid_bits: u32,
    period_ns: f32,
    pending: bool,
    frame: Option<crate::runtime::instrumentation::ProfileFrameId>,
    scopes: Vec<&'static str>,
}

impl FrameSlot {
    fn create(device: &Arc<DeviceInner>, staging_bytes: u64) -> RenderResult<Self> {
        let command_pool = unsafe {
            device.raw.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(device.queue_family)
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                None,
            )
        }
        .map_err(|result| vk_error("failed to create Vulkan frame-slot command pool", result))?;
        let command_buffer = match unsafe {
            device.raw.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        } {
            Ok(buffers) => buffers[0],
            Err(result) => {
                unsafe { device.raw.destroy_command_pool(command_pool, None) };
                return Err(vk_error(
                    "failed to allocate Vulkan frame-slot command buffer",
                    result,
                ));
            }
        };
        let fence = match unsafe {
            device
                .raw
                .create_fence(&vk::FenceCreateInfo::default(), None)
        } {
            Ok(fence) => fence,
            Err(result) => {
                unsafe { device.raw.destroy_command_pool(command_pool, None) };
                return Err(vk_error("failed to create Vulkan frame-slot fence", result));
            }
        };
        let (descriptor_pool, descriptor_sets) =
            match allocate_frame_sets(&device.raw, &device.layouts) {
                Ok(value) => value,
                Err(error) => {
                    unsafe {
                        device.raw.destroy_fence(fence, None);
                        device.raw.destroy_command_pool(command_pool, None);
                    }
                    return Err(error);
                }
            };
        #[cfg(target_os = "linux")]
        let composite_descriptor_pool = match create_composite_descriptor_pool(&device.raw) {
            Ok(pool) => pool,
            Err(error) => {
                unsafe {
                    device.raw.destroy_descriptor_pool(descriptor_pool, None);
                    device.raw.destroy_fence(fence, None);
                    device.raw.destroy_command_pool(command_pool, None);
                }
                return Err(error);
            }
        };
        let staging = match AllocatedBuffer::new(
            Arc::clone(device),
            staging_bytes,
            vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::UNIFORM_BUFFER
                | vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryLocation::CpuToGpu,
            "Telorgon reusable frame staging",
        ) {
            Ok(buffer) => Arc::new(buffer),
            Err(error) => {
                unsafe {
                    #[cfg(target_os = "linux")]
                    drop(composite_descriptor_pool);
                    device.raw.destroy_descriptor_pool(descriptor_pool, None);
                    device.raw.destroy_fence(fence, None);
                    device.raw.destroy_command_pool(command_pool, None);
                }
                return Err(error);
            }
        };
        let view = [vk::DescriptorBufferInfo {
            buffer: staging.raw(),
            offset: 0,
            range: size_of::<GpuView>() as u64,
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
            let dummy = [vk::DescriptorBufferInfo {
                buffer: staging.raw(),
                offset: 0,
                range: staging.size(),
            }];
            device.raw.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(descriptor_sets.scene)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&dummy)],
                &[],
            );
        }
        #[cfg(feature = "instrumentation")]
        let profiler_timestamps = if (crate::runtime::instrumentation::is_active() || frame_stats_enabled())
            && device.profiler_timestamp_valid_bits > 0
        {
            match unsafe {
                device.raw.create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count(PROFILER_TIMESTAMP_QUERY_COUNT),
                    None,
                )
            } {
                Ok(pool) => Some(ProfilerTimestampQueries {
                    timing: Arc::new(Mutex::new(None)),
                    pool,
                    valid_bits: device.profiler_timestamp_valid_bits,
                    period_ns: device.profiler_timestamp_period_ns,
                    pending: false,
                    frame: None,
                    scopes: Vec::new(),
                }),
                Err(_) => {
                    crate::runtime::instrumentation::record_diagnostic(
                        "gpu.timestamp_pool.create_failed",
                        crate::runtime::instrumentation::DiagnosticSeverity::Warning,
                        1,
                    );
                    None
                }
            }
        } else {
            None
        };
        Ok(Self {
            device: device.raw.clone(),
            state: SlotState::Available,
            command_pool,
            command_buffer,
            fence,
            descriptor_pool,
            descriptor_sets,
            descriptor_bindings: DescriptorBindingState::default(),
            #[cfg(target_os = "linux")]
            composite_descriptor_pool,
            staging,
            #[cfg(feature = "instrumentation")]
            profiler_timestamps,
        })
    }
}

impl Drop for FrameSlot {
    fn drop(&mut self) {
        unsafe {
            #[cfg(feature = "instrumentation")]
            if let Some(timestamps) = self.profiler_timestamps.take() {
                self.device.destroy_query_pool(timestamps.pool, None);
            }
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.command_pool, None);
        }
    }
}

#[cfg(feature = "instrumentation")]
fn frame_stats_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("TELORGON_FRAME_STATS").as_deref() == Ok("1"))
}

#[cfg(feature = "instrumentation")]
fn log_gpu_stats(total_ns: u64, samples: Vec<(&'static str, u64)>) {
    if !frame_stats_enabled() {
        return;
    }
    struct Stats {
        start: std::time::Instant,
        frames: u64,
        total: u64,
        stages: std::collections::BTreeMap<&'static str, (u64, u64, u64)>,
    }
    static STATS: std::sync::OnceLock<std::sync::Mutex<Stats>> = std::sync::OnceLock::new();
    let Ok(mut stats) = STATS
        .get_or_init(|| {
            std::sync::Mutex::new(Stats {
                start: std::time::Instant::now(),
                frames: 0,
                total: 0,
                stages: std::collections::BTreeMap::new(),
            })
        })
        .lock()
    else {
        return;
    };
    stats.frames += 1;
    stats.total = stats.total.saturating_add(total_ns);
    for (label, elapsed) in samples {
        let stage = stats.stages.entry(label).or_default();
        stage.0 = stage.0.saturating_add(elapsed);
        stage.1 += 1;
        stage.2 = stage.2.max(elapsed);
    }
    if stats.start.elapsed().as_secs_f32() >= 2.0 {
        let frames = stats.frames as f64;
        eprintln!(
            "telorgon-gpu-stats: frames={} total_avg_ms={:.3}",
            stats.frames,
            stats.total as f64 / frames / 1e6
        );
        for (label, (elapsed, calls, max)) in &stats.stages {
            eprintln!(
                "telorgon-gpu-stats: stage={label} avg_per_frame_ms={:.3} calls_per_frame={:.2} max_call_ms={:.3}",
                *elapsed as f64 / frames / 1e6,
                *calls as f64 / frames,
                *max as f64 / 1e6
            );
        }
        stats.frames = 0;
        stats.total = 0;
        stats.stages.clear();
        stats.start = std::time::Instant::now();
    }
}

#[cfg(feature = "instrumentation")]
fn resolve_profiler_timestamps(slot: &mut FrameSlot) {
    let Some(timestamps) = slot.profiler_timestamps.as_mut() else {
        return;
    };
    if !timestamps.pending {
        return;
    }
    timestamps.pending = false;
    let frame = timestamps.frame.take();
    let mut values = [0_u64; 5];
    let result = unsafe {
        slot.device.get_query_pool_results(
            timestamps.pool,
            0,
            &mut values,
            vk::QueryResultFlags::TYPE_64,
        )
    };
    if let Err(result) = result {
        let label = if result == vk::Result::NOT_READY {
            "gpu.timestamps.not_ready"
        } else {
            "gpu.timestamps.read_failed"
        };
        crate::runtime::instrumentation::record_diagnostic(label, crate::runtime::instrumentation::DiagnosticSeverity::Warning, 1);
        return;
    }
    let relative = |query: usize| {
        ticks_to_ns(
            timestamp_delta(values[0], values[query], timestamps.valid_bits),
            timestamps.period_ns,
        )
    };
    let duration = |start: usize, end: usize| {
        ticks_to_ns(
            timestamp_delta(values[start], values[end], timestamps.valid_bits),
            timestamps.period_ns,
        )
    };
    let mut stage_samples = Vec::new();
    let mut stages_available = timestamps.scopes.is_empty();
    if !timestamps.scopes.is_empty() {
        let mut samples = vec![0_u64; timestamps.scopes.len() * 2];
        let result = unsafe {
            slot.device.get_query_pool_results(
                timestamps.pool,
                5,
                &mut samples,
                vk::QueryResultFlags::TYPE_64,
            )
        };
        if result.is_ok() {
            stages_available = true;
            for (label, pair) in timestamps.scopes.iter().zip(samples.chunks_exact(2)) {
                let start = ticks_to_ns(
                    timestamp_delta(values[0], pair[0], timestamps.valid_bits),
                    timestamps.period_ns,
                );
                let elapsed = ticks_to_ns(
                    timestamp_delta(pair[0], pair[1], timestamps.valid_bits),
                    timestamps.period_ns,
                );
                crate::runtime::instrumentation::record_gpu_span(label, frame, start, elapsed);
                stage_samples.push((*label, elapsed));
            }
        }
    }
    if let Ok(mut timing) = timestamps.timing.lock() {
        *timing = Some(super::GpuTiming {
            total_ns: duration(0, 4),
            upload_ns: duration(0, 1),
            stages: stage_samples.clone(),
            stages_available,
        });
    }
    log_gpu_stats(duration(0, 4), stage_samples);
    crate::runtime::instrumentation::record_gpu_span("gpu.total", frame, 0, duration(0, 4));
    crate::runtime::instrumentation::record_gpu_span("gpu.upload_copy", frame, 0, duration(0, 1));
    crate::runtime::instrumentation::record_gpu_span("gpu.render_pass", frame, relative(2), duration(2, 3));
}

#[cfg(feature = "instrumentation")]
fn timestamp_delta(start: u64, end: u64, valid_bits: u32) -> u64 {
    let difference = end.wrapping_sub(start);
    if valid_bits >= u64::BITS {
        difference
    } else if valid_bits == 0 {
        0
    } else {
        difference & ((1_u64 << valid_bits) - 1)
    }
}

#[cfg(feature = "instrumentation")]
fn ticks_to_ns(ticks: u64, period_ns: f32) -> u64 {
    let value = ticks as f64 * f64::from(period_ns);
    if !value.is_finite() || value <= 0.0 {
        0
    } else if value >= u64::MAX as f64 {
        u64::MAX
    } else {
        value.round() as u64
    }
}

pub(crate) struct FrameSlots {
    device: Arc<DeviceInner>,
    slots: Mutex<Vec<FrameSlot>>,
    retired: Mutex<Vec<RetiredSubmission>>,
}

impl FrameSlots {
    pub(crate) fn create(
        device: Arc<DeviceInner>,
        count: usize,
        staging_budget_bytes: u64,
    ) -> RenderResult<Arc<Self>> {
        let count = count.max(1);
        let staging_bytes = staging_budget_bytes / count as u64;
        if staging_bytes < size_of::<GpuView>() as u64 {
            return Err(RenderError::new(
                RenderErrorKind::OutOfMemory,
                "Vulkan staging budget is too small for the configured frame slots",
            ));
        }
        let mut slots = Vec::with_capacity(count);
        for _ in 0..count {
            slots.push(FrameSlot::create(&device, staging_bytes)?);
        }
        Ok(Arc::new(Self {
            device,
            slots: Mutex::new(slots),
            retired: Mutex::new(Vec::new()),
        }))
    }

    fn completed_value(&self) -> RenderResult<u64> {
        unsafe {
            self.device.raw.get_semaphore_counter_value(
                self.device
                    .completion_timeline
                    .expect("owned frame slots require a completion timeline"),
            )
        }
        .map_err(|result| vk_error("failed to query Vulkan completion timeline", result))
    }

    pub(crate) fn maintain(&self) -> RenderResult<()> {
        let completed = self.completed_value()?;
        {
            let mut slots = self
                .slots
                .lock()
                .map_err(|_| internal("Vulkan frame-slot lock poisoned"))?;
            for slot in slots.iter_mut() {
                if matches!(
                    slot.state,
                    SlotState::InFlight { completion, .. } if completion <= completed
                ) {
                    #[cfg(feature = "instrumentation")]
                    resolve_profiler_timestamps(slot);
                    slot.state = SlotState::Available;
                }
            }
        }
        let mut retired = self
            .retired
            .lock()
            .map_err(|_| internal("Vulkan retired-resource lock poisoned"))?;
        let mut index = 0;
        while index < retired.len() {
            if retired[index].completion_value <= completed {
                let completed = retired.swap_remove(index);
                for external in &completed.external_images {
                    external.complete_use(completed.frame_id);
                }
            } else {
                index += 1;
            }
        }
        Ok(())
    }

    fn try_begin(
        &self,
        device: &VulkanDevice,
        frame_id: u64,
    ) -> RenderResult<Option<(usize, FrameCore)>> {
        self.maintain()?;
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| internal("Vulkan frame-slot lock poisoned"))?;
        let Some((slot_index, slot)) = slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.state == SlotState::Available)
        else {
            return Ok(None);
        };
        unsafe {
            self.device
                .raw
                .reset_fences(&[slot.fence])
                .map_err(|result| vk_error("failed to reset Vulkan frame-slot fence", result))?;
            self.device
                .raw
                .reset_command_pool(slot.command_pool, vk::CommandPoolResetFlags::empty())
                .map_err(|result| {
                    vk_error("failed to reset Vulkan frame-slot command pool", result)
                })?;
            #[cfg(target_os = "linux")]
            slot.composite_descriptor_pool
                .lock()
                .map_err(|_| internal("composite descriptor arena poisoned"))?
                .reset()?;
            self.device
                .raw
                .begin_command_buffer(
                    slot.command_buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(|result| {
                    vk_error("failed to begin Vulkan frame-slot command buffer", result)
                })?;
        }
        #[cfg(feature = "instrumentation")]
        let profiler_query_pool = slot.profiler_timestamps.as_mut().map(|timestamps| {
            timestamps.pending = false;
            timestamps.frame = crate::runtime::instrumentation::current_frame_id();
            timestamps.timing = Arc::new(Mutex::new(None));
            unsafe {
                self.device.raw.cmd_reset_query_pool(
                    slot.command_buffer,
                    timestamps.pool,
                    0,
                    PROFILER_TIMESTAMP_QUERY_COUNT,
                );
                self.device.raw.cmd_write_timestamp2(
                    slot.command_buffer,
                    vk::PipelineStageFlags2::TOP_OF_PIPE,
                    timestamps.pool,
                    0,
                );
            }
            timestamps.pool
        });
        slot.state = SlotState::Recording { frame_id };
        Ok(Some((
            slot_index,
            FrameCore {
                device: device.clone(),
                frame_id,
                command_buffer: slot.command_buffer,
                descriptor_sets: slot.descriptor_sets,
                descriptor_bindings: slot.descriptor_bindings,
                #[cfg(target_os = "linux")]
                composite_descriptor_pool: Some(Arc::clone(&slot.composite_descriptor_pool)),
                staging: Arc::clone(&slot.staging),
                staging_bytes_used: 0,
                buffers: Vec::new(),
                images: Vec::new(),
                resources: Vec::new(),
                external_images: Vec::new(),
                rendered: false,
                #[cfg(feature = "instrumentation")]
                profiler_query_pool,
                #[cfg(feature = "instrumentation")]
                profiler_timestamp_mask: 0,
                #[cfg(feature = "instrumentation")]
                profiler_timestamps_complete: false,
                #[cfg(feature = "instrumentation")]
                gpu_scopes: Vec::new(),
            },
        )))
    }

    fn finish_recording(
        &self,
        slot_index: usize,
        frame_id: u64,
        descriptor_bindings: DescriptorBindingState,
        #[cfg(feature = "instrumentation")] profiler_timestamps_complete: bool,
        #[cfg(feature = "instrumentation")] gpu_scopes: Vec<&'static str>,
    ) -> RenderResult<()> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| internal("Vulkan frame-slot lock poisoned"))?;
        let slot = slots
            .get_mut(slot_index)
            .ok_or_else(|| internal("Vulkan frame-slot index is invalid"))?;
        if slot.state != (SlotState::Recording { frame_id }) {
            return Err(internal("Vulkan frame-slot recording state is invalid"));
        }
        slot.descriptor_bindings = descriptor_bindings;
        #[cfg(feature = "instrumentation")]
        if let Some(timestamps) = slot.profiler_timestamps.as_mut() {
            timestamps.pending = profiler_timestamps_complete;
            timestamps.scopes = gpu_scopes;
        }
        slot.state = SlotState::Recorded { frame_id };
        Ok(())
    }

    fn release_unsubmitted(
        &self,
        slot_index: usize,
        frame_id: u64,
        descriptor_bindings: Option<DescriptorBindingState>,
    ) {
        let Ok(mut slots) = self.slots.lock() else {
            return;
        };
        let Some(slot) = slots.get_mut(slot_index) else {
            return;
        };
        if matches!(
            slot.state,
            SlotState::Recording { frame_id: current }
                | SlotState::Recorded { frame_id: current }
                if current == frame_id
        ) {
            #[cfg(feature = "instrumentation")]
            if let Some(timestamps) = slot.profiler_timestamps.as_mut() {
                timestamps.pending = false;
                timestamps.frame = None;
            }
            if let Some(bindings) = descriptor_bindings {
                slot.descriptor_bindings = bindings;
            }
            slot.state = SlotState::Available;
        }
    }

    fn retire(
        &self,
        completion_value: u64,
        frame_id: u64,
        buffers: Vec<Arc<AllocatedBuffer>>,
        images: Vec<Arc<AllocatedImage>>,
        resources: Vec<Arc<dyn Send + Sync>>,
        external_images: Vec<Arc<ExternalImageInner>>,
    ) {
        if buffers.is_empty()
            && images.is_empty()
            && resources.is_empty()
            && external_images.is_empty()
        {
            return;
        }
        let mut retired = self
            .retired
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        retired.push(RetiredSubmission {
            completion_value,
            frame_id,
            _buffers: buffers,
            _images: images,
            _resources: resources,
            external_images,
        });
    }
}

impl Drop for FrameSlots {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.raw.device_wait_idle();
        }
        match self.retired.get_mut() {
            Ok(retired) => retired.clear(),
            Err(poisoned) => poisoned.into_inner().clear(),
        }
        match self.slots.get_mut() {
            Ok(slots) => slots.clear(),
            Err(poisoned) => poisoned.into_inner().clear(),
        }
    }
}

impl<'device> VulkanRecordingFrame<'device> {
    pub(crate) fn begin(device: &'device VulkanDevice, frame_id: u64) -> RenderResult<Self> {
        Self::try_begin(device, frame_id)?.ok_or_else(|| {
            RenderError::new(
                RenderErrorKind::HostContract,
                "all configured Vulkan frame slots are currently busy",
            )
        })
    }

    pub(crate) fn try_begin(
        device: &'device VulkanDevice,
        frame_id: u64,
    ) -> RenderResult<Option<Self>> {
        let frames = Arc::clone(
            device
                .frames
                .as_ref()
                .expect("owned recording requires owned frame slots"),
        );
        let Some((slot_index, core)) = frames.try_begin(device, frame_id)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            device,
            frames,
            frame_id,
            slot_index: Some(slot_index),
            core: Some(core),
            _thread_bound: PhantomData,
        }))
    }

    pub fn frame_id(&self) -> u64 {
        self.frame_id
    }

    pub fn device_id(&self) -> u64 {
        self.device.inner.id
    }

    pub fn context_mut(&mut self) -> VulkanFrameContext<'_> {
        VulkanFrameContext {
            core: self
                .core
                .as_mut()
                .expect("recording frame already finished"),
        }
    }

    pub fn finish(mut self) -> RenderResult<VulkanRecordedFrame> {
        #[allow(unused_mut)]
        let mut core = self
            .core
            .take()
            .ok_or_else(|| internal("Vulkan frame was already finished"))?;
        #[cfg(feature = "instrumentation")]
        core.write_profiler_timestamp(
            PROFILER_TIMESTAMP_TOTAL_END,
            vk::PipelineStageFlags2::ALL_COMMANDS,
        );
        if let Err(result) = unsafe {
            self.device
                .inner
                .raw
                .end_command_buffer(core.command_buffer)
        } {
            for external in &core.external_images {
                external.cancel_use(core.frame_id);
            }
            return Err(vk_error("failed to finish Vulkan command buffer", result));
        }
        let slot_index = self
            .slot_index
            .take()
            .ok_or_else(|| internal("Vulkan frame slot was already released"))?;
        if let Err(error) = self.frames.finish_recording(
            slot_index,
            self.frame_id,
            core.descriptor_bindings,
            #[cfg(feature = "instrumentation")]
            core.profiler_timestamps_complete,
            #[cfg(feature = "instrumentation")]
            std::mem::take(&mut core.gpu_scopes),
        ) {
            for external in &core.external_images {
                external.cancel_use(core.frame_id);
            }
            return Err(error);
        }
        Ok(VulkanRecordedFrame {
            device: self.device.inner.clone(),
            frames: Arc::clone(&self.frames),
            device_id: self.device.inner.id,
            frame_id: self.frame_id,
            command_buffer: core.command_buffer,
            slot_index: Some(slot_index),
            buffers: core.buffers,
            images: core.images,
            resources: core.resources,
            external_images: core.external_images,
        })
    }
}

impl Drop for VulkanRecordingFrame<'_> {
    fn drop(&mut self) {
        if let Some(core) = &self.core {
            for external in &core.external_images {
                external.cancel_use(core.frame_id);
            }
        }
        if let Some(slot_index) = self.slot_index.take() {
            self.frames.release_unsubmitted(
                slot_index,
                self.frame_id,
                self.core.as_ref().map(|core| core.descriptor_bindings),
            );
        }
    }
}

impl VulkanRecordedFrame {
    pub fn frame_id(&self) -> u64 {
        self.frame_id
    }

    pub fn device_id(&self) -> u64 {
        self.device_id
    }

    pub fn submit(self) -> RenderResult<SubmissionReceipt> {
        let (waits, signals) = owned_external_semaphores(&self.external_images);
        submit_recorded(self, &waits, &signals, None)
    }

    pub(crate) fn submit_with_binary_semaphores(
        self,
        wait: vk::Semaphore,
        signal: vk::Semaphore,
    ) -> RenderResult<SubmissionReceipt> {
        let (mut waits, mut signals) = owned_external_semaphores(&self.external_images);
        waits.push(
            vk::SemaphoreSubmitInfo::default()
                .semaphore(wait)
                .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .value(0),
        );
        signals.push(
            vk::SemaphoreSubmitInfo::default()
                .semaphore(signal)
                .stage_mask(vk::PipelineStageFlags2::ALL_GRAPHICS)
                .value(0),
        );
        submit_recorded(self, &waits, &signals, None)
    }

    pub(crate) fn submit_with_timeline_semaphore_and_keyed_mutex(
        self,
        semaphore: vk::Semaphore,
        wait_value: u64,
        signal_value: u64,
        memory: vk::DeviceMemory,
    ) -> RenderResult<SubmissionReceipt> {
        if wait_value >= signal_value {
            return Err(RenderError::new(
                RenderErrorKind::HostContract,
                "external timeline signal must be greater than its wait value",
            ));
        }
        let (mut waits, mut signals) = owned_external_semaphores(&self.external_images);
        waits.push(
            vk::SemaphoreSubmitInfo::default()
                .semaphore(semaphore)
                .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .value(wait_value),
        );
        signals.push(
            vk::SemaphoreSubmitInfo::default()
                .semaphore(semaphore)
                .stage_mask(vk::PipelineStageFlags2::ALL_GRAPHICS)
                .value(signal_value),
        );
        submit_recorded(self, &waits, &signals, Some(memory))
    }
}

impl Drop for VulkanRecordedFrame {
    fn drop(&mut self) {
        for external in &self.external_images {
            external.cancel_use(self.frame_id);
        }
        if let Some(slot_index) = self.slot_index.take() {
            self.frames
                .release_unsubmitted(slot_index, self.frame_id, None);
        }
    }
}

impl SubmissionReceipt {
    pub(crate) fn timing(&self) -> super::SubmissionTiming {
        super::SubmissionTiming {
            point: self.completion,
            submit_started: self.submit_started,
            submitted: self.submitted,
            gpu: self.timing.as_ref().and_then(|timing| timing.lock().ok()?.clone()),
        }
    }

    pub fn completion(&self) -> CompletionPoint {
        self.completion
    }

    pub fn wait(&mut self, timeout: Duration) -> RenderResult<()> {
        if self.completed {
            return Ok(());
        }
        let nanos = timeout.as_nanos().min(u64::MAX as u128) as u64;
        let semaphores = [self
            .device
            .completion_timeline
            .expect("owned receipts require a completion timeline")];
        let values = [self.completion.value];
        let wait_info = vk::SemaphoreWaitInfo::default()
            .semaphores(&semaphores)
            .values(&values);
        unsafe { self.device.raw.wait_semaphores(&wait_info, nanos) }.map_err(|result| {
            if result == vk::Result::TIMEOUT {
                RenderError::new(
                    RenderErrorKind::HostContract,
                    "Vulkan submission wait timed out",
                )
            } else {
                vk_error("failed to wait for Vulkan submission", result)
            }
        })?;
        self.completed = true;
        for external in &self.external_images {
            external.complete_use(self.completion.frame_id);
        }
        self.frames.maintain()?;
        Ok(())
    }

    pub fn poll(&mut self) -> RenderResult<bool> {
        if self.completed {
            return Ok(true);
        }
        let value = unsafe {
            self.device.raw.get_semaphore_counter_value(
                self.device
                    .completion_timeline
                    .expect("owned receipts require a completion timeline"),
            )
        }
        .map_err(|result| vk_error("failed to poll Vulkan completion timeline", result))?;
        let complete = value >= self.completion.value;
        self.completed = complete;
        if complete {
            for external in &self.external_images {
                external.complete_use(self.completion.frame_id);
            }
            self.frames.maintain()?;
        }
        Ok(complete)
    }

    /// Exports the release sync FDs for submitted Linux DMA-BUF reads.
    ///
    /// Submission has already established the binary semaphore signals. Each generation can be
    /// exported exactly once and the returned FDs transfer to the protocol owner.
    #[cfg(target_os = "linux")]
    pub fn export_dma_buf_release_sync_fds(
        &self,
    ) -> RenderResult<Vec<crate::graphics::renderers::vulkan::VulkanDmaBufReleaseSyncFd>> {
        let mut releases = Vec::new();
        for external in &self.external_images {
            if let Some(release) = unsafe { external.export_release_sync_fd() }? {
                releases.push(release);
            }
        }
        Ok(releases)
    }
}

impl Drop for SubmissionReceipt {
    fn drop(&mut self) {
        if self.completed {
            self.buffers.clear();
            self.images.clear();
            self.resources.clear();
            self.external_images.clear();
        } else {
            self.frames.retire(
                self.completion.value,
                self.completion.frame_id,
                std::mem::take(&mut self.buffers),
                std::mem::take(&mut self.images),
                std::mem::take(&mut self.resources),
                std::mem::take(&mut self.external_images),
            );
        }
    }
}

fn submit_recorded(
    mut frame: VulkanRecordedFrame,
    waits: &[vk::SemaphoreSubmitInfo<'_>],
    signals: &[vk::SemaphoreSubmitInfo<'_>],
    keyed_mutex_memory: Option<vk::DeviceMemory>,
) -> RenderResult<SubmissionReceipt> {
    let completion_value = frame
        .device
        .next_completion_value
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut all_signals = signals.to_vec();
    all_signals.push(
        vk::SemaphoreSubmitInfo::default()
            .semaphore(
                frame
                    .device
                    .completion_timeline
                    .expect("owned submission requires a completion timeline"),
            )
            .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .value(completion_value),
    );
    let command = [vk::CommandBufferSubmitInfo::default().command_buffer(frame.command_buffer)];
    let mut submit_info = vk::SubmitInfo2::default()
        .wait_semaphore_infos(waits)
        .command_buffer_infos(&command)
        .signal_semaphore_infos(&all_signals);
    let keyed_memory_storage = [keyed_mutex_memory.unwrap_or(vk::DeviceMemory::null())];
    let keyed_memories = &keyed_memory_storage[..usize::from(keyed_mutex_memory.is_some())];
    let acquire_keys = [0_u64];
    let acquire_timeouts = [100_u32];
    let release_keys = [1_u64];
    let mut keyed_mutex = vk::Win32KeyedMutexAcquireReleaseInfoKHR::default();
    if !keyed_memories.is_empty() {
        keyed_mutex = keyed_mutex
            .acquire_syncs(keyed_memories)
            .acquire_keys(&acquire_keys)
            .acquire_timeouts(&acquire_timeouts)
            .release_syncs(keyed_memories)
            .release_keys(&release_keys);
        submit_info = submit_info.push_next(&mut keyed_mutex);
    }
    let submit = [submit_info];
    let slot_index = frame
        .slot_index
        .ok_or_else(|| internal("Vulkan recorded frame slot is unavailable"))?;
    let timing;
    let submit_started;
    let submitted;
    {
        let mut slots = frame
            .frames
            .slots
            .lock()
            .map_err(|_| internal("Vulkan frame-slot lock poisoned"))?;
        let slot = slots
            .get_mut(slot_index)
            .ok_or_else(|| internal("Vulkan frame-slot index is invalid"))?;
        if slot.state
            != (SlotState::Recorded {
                frame_id: frame.frame_id,
            })
        {
            return Err(internal("Vulkan recorded frame-slot state is invalid"));
        }
        #[cfg(feature = "instrumentation")]
        {
            timing = slot.profiler_timestamps.as_ref().map(|timestamps| Arc::clone(&timestamps.timing));
        }
        #[cfg(not(feature = "instrumentation"))]
        {
            timing = None;
        }
        submit_started = std::time::Instant::now();
        let _queue = frame
            .device
            .queue_lock
            .lock()
            .map_err(|_| internal("Vulkan queue lock poisoned"))?;
        unsafe {
            frame
                .device
                .raw
                .queue_submit2(frame.device.queue, &submit, slot.fence)
        }
        .map_err(|result| vk_error("failed to submit Vulkan frame", result))?;
        submitted = std::time::Instant::now();
        slot.state = SlotState::InFlight {
            frame_id: frame.frame_id,
            completion: completion_value,
        };
    }
    frame.slot_index = None;
    Ok(SubmissionReceipt {
        timing,
        submit_started,
        submitted,
        device: frame.device.clone(),
        frames: Arc::clone(&frame.frames),
        completion: CompletionPoint {
            device_id: frame.device_id,
            frame_id: frame.frame_id,
            value: completion_value,
        },
        buffers: std::mem::take(&mut frame.buffers),
        images: std::mem::take(&mut frame.images),
        resources: std::mem::take(&mut frame.resources),
        external_images: std::mem::take(&mut frame.external_images),
        completed: false,
    })
}

fn owned_external_semaphores(
    images: &[Arc<ExternalImageInner>],
) -> (
    Vec<vk::SemaphoreSubmitInfo<'static>>,
    Vec<vk::SemaphoreSubmitInfo<'static>>,
) {
    use crate::graphics::renderers::vulkan::external_image::{VulkanExternalAcquire, VulkanExternalRelease};

    let waits = images
        .iter()
        .filter_map(|image| match image.acquire {
            VulkanExternalAcquire::BinarySemaphore(semaphore) => Some(
                vk::SemaphoreSubmitInfo::default()
                    .semaphore(semaphore)
                    .stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER),
            ),
            VulkanExternalAcquire::CommandStream => None,
        })
        .collect();
    let signals = images
        .iter()
        .filter_map(|image| match image.release {
            VulkanExternalRelease::BinarySemaphore(semaphore) => Some(
                vk::SemaphoreSubmitInfo::default()
                    .semaphore(semaphore)
                    .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS),
            ),
            VulkanExternalRelease::CommandStream => None,
        })
        .collect();
    (waits, signals)
}

#[cfg(all(test, feature = "instrumentation"))]
mod profiler_tests {
    use super::*;

    #[test]
    fn timestamp_delta_handles_reported_width_wrap() {
        assert_eq!(timestamp_delta(250, 4, 8), 10);
        assert_eq!(timestamp_delta(u64::MAX - 2, 3, 64), 6);
        assert_eq!(timestamp_delta(1, 2, 0), 0);
    }

    #[test]
    fn timestamp_period_conversion_is_saturating() {
        assert_eq!(ticks_to_ns(10, 2.5), 25);
        assert_eq!(ticks_to_ns(u64::MAX, f32::MAX), u64::MAX);
    }
}
