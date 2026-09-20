use std::collections::{BTreeMap, VecDeque};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::compositor_render::{DmaBufImporter, dma_buf_image_id};
use crate::compositor_wayland::{
    BufferTransform, DmaBufFormat, DmaBufImage, ViewportSource, ViewportState, WaylandBufferId,
    WaylandSurfaceId,
};
use crate::core::{Affine2D, ColorRgba8, PointI, RectF, RectI, SizeF, SizeI};
use crate::presenter_vulkan_kms::GbmBuffer;
use crate::render::{
    BatchKey, BlendMode, ClipId, DrawItem, ImageAlphaMode, ImageId, ImageInstance,
    ImagePixelFormat, PipelineKind, PrimitiveKind, RenderBackend, RenderRequest, RenderScene,
    RenderSpatialNode, SpatialId, TargetLoad, TargetStore,
};
use crate::renderer_vulkan::{
    DeviceSelection, SubmissionReceipt, VulkanCompositePlacement, VulkanCompositeScene,
    VulkanConfig, VulkanDevice, VulkanDmaBufScanoutTarget, VulkanInstance,
    VulkanMaterializationTarget, VulkanScene,
};
use crate::scene::NodeId;

use super::super::geometry::{accumulated_damage, full_rect, intersect_rect};
mod glass;
mod motion;
mod motion_glass;
mod resources;
use super::super::scene::{ShellFrame, ShellSceneKey};
use super::capture::{CaptureBuffer, CaptureJob, CaptureView, CaptureSubmitFailure};
use crate::application_host::{AppError, AppResult};
use resources::{MaterializationResources, RetirementWorker, TargetAllocator, trim_spares};

pub(in crate::application_host::shell_wayland) struct VulkanCompletion {
    pub(in crate::application_host::shell_wayland) direct: Option<crate::compositor_wayland::DirectCaptureCompletion>,
    pub(in crate::application_host::shell_wayland) capture_pending: Option<SubmissionReceipt>,
    pub(in crate::application_host::shell_wayland) capture: Option<CaptureJob>,
    pub(in crate::application_host::shell_wayland) slot_index: usize,
    pub(in crate::application_host::shell_wayland) result: Result<(), String>,
    pub(in crate::application_host::shell_wayland) dma_bufs: Vec<DmaBufRetirement>,
}

struct VulkanCompletionRequest {
    capture: Option<CaptureJob>,
    slot_index: usize,
    receipt: SubmissionReceipt,
    dma_bufs: Vec<DmaBufRetirement>,
}

pub(in crate::application_host::shell_wayland) struct DmaBufPublication {
    pub surface: WaylandSurfaceId,
    pub revision: u64,
    pub buffer: WaylandBufferId,
    pub image: DmaBufImage,
    pub acquire: Option<OwnedFd>,
    pub buffer_scale: i32,
    pub buffer_transform: BufferTransform,
    pub viewport: Option<ViewportState>,
    pub output_scale: crate::platform::ScaleFactor,
    pub coordinate_density: i32,
    pub surface_damage: Vec<RectI>,
    pub buffer_damage: Vec<RectI>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::application_host::shell_wayland) struct DmaBufRetirement {
    pub surface: WaylandSurfaceId,
    pub revision: u64,
    pub buffer: WaylandBufferId,
}

pub(in crate::application_host::shell_wayland) struct DmaBufQueueResult {
    pub extent: SizeI,
    pub raster_extent: SizeI,
    pub pixel_format: ImagePixelFormat,
    pub alpha_mode: ImageAlphaMode,
    pub image: ImageId,
    pub damage: Option<RectI>,
    pub replaced: Option<DmaBufRetirement>,
}

pub(in crate::application_host::shell_wayland) struct DmaBufRelease {
    pub retirement: DmaBufRetirement,
    pub fence: OwnedFd,
}

pub(in crate::application_host::shell_wayland) struct VulkanRenderResult {
    pub releases: Vec<DmaBufRelease>,
    pub discarded: Vec<DmaBufRetirement>,
}

struct PendingDmaBufPublication {
    publication: DmaBufPublication,
    acquire_watch: super::super::dma_buf_readiness::AcquireWatch,
    content_version: u64,
    extent: SizeI,
    transform: Affine2D,
    alpha_mode: ImageAlphaMode,
    geometry: MaterializationGeometry,
    region: Option<RectI>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MaterializationGeometry {
    physical: SizeI,
    extent: SizeI,
    raster: SizeI,
    transform: Affine2D,
    scale: f32,
    alpha: ImageAlphaMode,
}

struct DmaBufMaterialization {
    source: VulkanScene,
    target: VulkanMaterializationTarget,
    retirement: DmaBufRetirement,
    content_version: u64,
    lease_generation: u64,
    geometry: MaterializationGeometry,
    region: Option<RectI>,
}

struct VulkanCompletionWorker {
    requests: Option<mpsc::Sender<VulkanCompletionRequest>>,
    completions: mpsc::Receiver<VulkanCompletion>,
    wake: OwnedFd,
    thread: Option<thread::JoinHandle<()>>,
}

impl VulkanCompletionWorker {
    fn new() -> AppResult<Self> {
        let raw = unsafe {
            crate::platform_linux::ffi::eventfd(
                0,
                crate::platform_linux::ffi::EFD_CLOEXEC | crate::platform_linux::ffi::EFD_NONBLOCK,
            )
        };
        if raw < 0 {
            return Err(AppError::new("failed to create Vulkan completion eventfd"));
        }
        let wake = unsafe { OwnedFd::from_raw_fd(raw) };
        let thread_wake = wake.try_clone().map_err(|error| {
            AppError::new(format!("failed to clone completion eventfd: {error}"))
        })?;
        let (request_tx, request_rx) = mpsc::channel::<VulkanCompletionRequest>();
        let (completion_tx, completion_rx) = mpsc::channel::<VulkanCompletion>();
        let thread = thread::Builder::new()
            .name("telorgon-vulkan-completion".to_owned())
            .spawn(move || {
                while let Ok(mut request) = request_rx.recv() {
                    #[cfg(feature = "profiler")]
                    let _wait = crate::profiler::span!("vulkan.scanout.completion_wait.worker");
                    let waited = request.receipt.wait(Duration::from_secs(2));
                    let pending = request.capture.is_some() && waited.as_ref().is_err_and(|error|
                        error.kind() != crate::render::RenderErrorKind::DeviceLost);
                    let mut result = waited.map_err(|error| error.to_string());
                    if result.is_ok() && let Some(capture) = &mut request.capture {
                        result = capture.buffer.slot.try_copy_into(&mut request.receipt, &mut capture.buffer.pixels)
                            .map_err(|error| error.to_string())
                            .and_then(|ready| if ready { Ok(()) } else { Err("capture did not complete with its submission".into()) });
                    }
                    // Keep the destination with the exact GPU job through wait timeouts.
                    // Delivery starts only after readback has completed or definitively failed.
                    let direct = if !pending {
                        request.capture.as_mut().and_then(|capture| {
                            capture.direct.take().map(|(job, timestamp_ns, transform)| {
                                if result.is_ok() {
                                    job.write(&capture.buffer.pixels, timestamp_ns, transform)
                                } else {
                                    job.fail()
                                }
                            })
                        })
                    } else { None };
                    let capture_pending = pending.then_some(request.receipt);
                    if completion_tx
                        .send(VulkanCompletion {
                            direct,
                            capture_pending,
                            capture: request.capture,
                            slot_index: request.slot_index,
                            result,
                            dma_bufs: request.dma_bufs,
                        })
                        .is_err()
                    {
                        break;
                    }
                    let value = 1_u64;
                    let _ = unsafe {
                        crate::platform_linux::ffi::write(
                            thread_wake.as_raw_fd(),
                            std::ptr::from_ref(&value).cast(),
                            std::mem::size_of::<u64>(),
                        )
                    };
                }
            })
            .map_err(|error| {
                AppError::new(format!("failed to start Vulkan completion worker: {error}"))
            })?;
        Ok(Self {
            requests: Some(request_tx),
            completions: completion_rx,
            wake,
            thread: Some(thread),
        })
    }

    fn event_fd(&self) -> i32 {
        self.wake.as_raw_fd()
    }

    fn submit(
        &self,
        slot_index: usize,
        receipt: SubmissionReceipt,
        dma_bufs: Vec<DmaBufRetirement>,
    ) -> AppResult<()> {
        self.requests
            .as_ref()
            .ok_or_else(|| AppError::new("Vulkan completion worker is stopped"))?
            .send(VulkanCompletionRequest {
                capture: None,
                slot_index,
                receipt,
                dma_bufs,
            })
            .map_err(|_| AppError::new("Vulkan completion worker stopped unexpectedly"))
    }

    fn submit_capture(&self, receipt: SubmissionReceipt, capture: CaptureJob) -> AppResult<()> {
        self.requests.as_ref().ok_or_else(|| AppError::new("Vulkan completion worker is stopped"))?
            .send(VulkanCompletionRequest { capture: Some(capture), slot_index: 0, receipt, dma_bufs: Vec::new() })
            .map_err(|_| AppError::new("Vulkan completion worker stopped unexpectedly"))
    }

    fn drain(&self) -> Vec<VulkanCompletion> {
        let mut value = 0_u64;
        loop {
            let read = unsafe {
                crate::platform_linux::ffi::read(
                    self.wake.as_raw_fd(),
                    std::ptr::from_mut(&mut value).cast(),
                    std::mem::size_of::<u64>(),
                )
            };
            if read != std::mem::size_of::<u64>() as isize {
                break;
            }
        }
        self.completions.try_iter().collect()
    }
}

impl Drop for VulkanCompletionWorker {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(in crate::application_host::shell_wayland) const VULKAN_STAGING_MIN_BYTES_PER_SLOT: u64 =
    16 * 1024 * 1024;
pub(in crate::application_host::shell_wayland) const VULKAN_STAGING_HEADROOM_BYTES_PER_SLOT: u64 =
    16 * 1024 * 1024;

pub(in crate::application_host::shell_wayland) fn vulkan_staging_budget_bytes(
    extent: SizeI,
    frame_slots: usize,
) -> AppResult<u64> {
    let frame_bytes = u64::try_from(extent.width)
        .ok()
        .and_then(|width| {
            u64::try_from(extent.height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| AppError::new("Vulkan scanout extent overflows its upload budget"))?;
    // A direct compositor may receive one full client image in addition to changed shell
    // resources. The budget is per reusable frame slot and performs no per-frame allocation.
    let bytes_per_slot = frame_bytes
        .checked_add(VULKAN_STAGING_HEADROOM_BYTES_PER_SLOT)
        .ok_or_else(|| AppError::new("Vulkan scanout staging headroom overflows its budget"))?
        .max(VULKAN_STAGING_MIN_BYTES_PER_SLOT);
    let frame_slots = u64::try_from(frame_slots.max(1))
        .map_err(|_| AppError::new("Vulkan frame-slot count overflows its staging budget"))?;
    bytes_per_slot
        .checked_mul(frame_slots)
        .ok_or_else(|| AppError::new("Vulkan frame slots overflow their staging budget"))
}

pub(in crate::application_host::shell_wayland) struct VulkanShellRenderer {
    // Drain submitted work before scene and target handles are destroyed.
    completion_worker: VulkanCompletionWorker,
    allocator: TargetAllocator,
    materialized: BTreeMap<ShellSceneKey, MaterializationResources>,
    spares: VecDeque<MaterializationResources>,
    device: VulkanDevice,
    scenes: BTreeMap<ShellSceneKey, VulkanScene>,
    capture_placements: Vec<super::super::scene::ShellPlacement>,
    capture_cursor_scene: Option<VulkanScene>,
    capture_cursor_placement: Option<super::super::scene::ShellPlacement>,
    capture_revisions: super::capture::CaptureRevisions,
    motion_snapshots: BTreeMap<u64, motion::MotionSnapshot>,
    motion_spares: Vec<VulkanMaterializationTarget>,
    motion_failed: bool,
    motion_glass: motion_glass::MotionGlass,
    glass_caches: BTreeMap<ShellSceneKey, glass::GlassCache>,
    targets: Vec<VulkanDmaBufScanoutTarget>,
    content_version: u64,
    target_versions: Vec<u64>,
    damage_history: VecDeque<(u64, Option<RectI>)>,
    dma_buf_importer: Option<DmaBufImporter>,
    pending_dma_bufs: BTreeMap<ShellSceneKey, PendingDmaBufPublication>,
    next_dma_buf_content_version: u64,
    // Last: all owner and GPU pins must disappear before retirement shutdown.
    retirement: RetirementWorker,
}

impl VulkanShellRenderer {
    pub(super) fn clear_capture_cursor(&mut self) {
        let had_scene = self.capture_cursor_scene.take().is_some();
        let had_placement = self.capture_cursor_placement.take().is_some();
        if had_scene || had_placement { self.capture_revisions.cursor_changed(); }
    }

    pub(super) fn capture_revision(&self, cursor: crate::shell::capture::CaptureCursorMode) -> u64 {
        self.capture_revisions.get(cursor)
    }

    pub(super) fn update_capture_cursor(&mut self, frame: ShellFrame) -> AppResult<()> {
        self.capture_revisions.cursor_changed();
        self.capture_cursor_placement = None;
        if !frame.live_scenes.contains(&ShellSceneKey::CursorImage) {
            // Reappearing sources start a fresh epoch in the CPU adapter too.
            self.capture_cursor_scene = None;
            return Ok(());
        }
        if self.capture_cursor_scene.is_none() {
            self.capture_cursor_scene = Some(self.device.create_scene().map_err(app_error)?);
        }
        let scene = self.capture_cursor_scene.as_mut().expect("cursor scene created");
        for update in &frame.updates {
            if update.key != ShellSceneKey::CursorImage {
                return Err(AppError::new("unexpected scene in capture cursor update"));
            }
            for delta in &update.deltas {
                if let Err(error) = self.device.apply_scene_delta(scene, delta) {
                    self.capture_cursor_scene = None;
                    return Err(app_error(error));
                }
            }
        }
        self.capture_cursor_placement = frame.placements.first().copied();
        Ok(())
    }

    pub(super) fn allocate_capture(&self, layout: crate::shell::capture::CaptureLayout) -> AppResult<CaptureBuffer> {
        if layout.stride() != layout.width().saturating_mul(4) || layout.width() > 8192 || layout.height() > 8192 {
            return Err(AppError::new("unsupported capture dimensions or stride"));
        }
        Ok(CaptureBuffer {
            layout,
            slot: crate::compositor_render::capture::VulkanCaptureSlot::new(&self.device, SizeI {
                width: layout.width() as i32, height: layout.height() as i32,
            }).map_err(app_error)?,
            pixels: vec![0; layout.byte_len()],
        })
    }

    pub(super) fn submit_capture_recoverable(&mut self, mut job: CaptureJob) -> Result<(), CaptureSubmitFailure> {
        let recorded: AppResult<crate::renderer_vulkan::VulkanRecordedFrame> = (|| {
        // Capture final desktop placements, then append the independently prepared cursor. This
        // works with either hardware or composited desktop cursors and never duplicates one.
        let indices = self.scenes.keys().copied().enumerate()
            .map(|(index, key)| (key, index)).collect::<BTreeMap<_, _>>();
        let (source, cursor_origin) = match &job.view {
            CaptureView::Output => (self.capture_placements.as_slice(), PointI::default()),
            #[cfg(feature = "shell-screencast-linux")]
            CaptureView::Window(window) => {
                if window.layout != job.buffer.layout {
                    return Err(AppError::new("window capture generation has a stale layout"));
                }
                (window.placements.as_slice(), window.origin)
            }
        };
        let mut placements = source.iter()
            .filter(|placement| placement.key != super::super::scene::ShellLayerKey::Cursor)
            .map(|placement| Ok(VulkanCompositePlacement {
                scene_index: *indices.get(&placement.scene).ok_or_else(|| AppError::new("capture scene was retired"))?,
                target: placement.target, clip: placement.clip, rounded_clips: placement.rounded_clips,
            })).collect::<AppResult<Vec<_>>>()?;
        let mut scenes = self.scenes.values_mut().map(|scene| VulkanCompositeScene { scene }).collect::<Vec<_>>();
        if job.cursor == crate::shell::capture::CaptureCursorMode::Embedded
            && let (Some(scene), Some(placement)) = (&mut self.capture_cursor_scene, self.capture_cursor_placement)
        {
            placements.push(VulkanCompositePlacement {
                scene_index: scenes.len(), target: RectI {
                    x: placement.target.x.checked_sub(cursor_origin.x).ok_or_else(|| AppError::new("capture cursor coordinate overflow"))?,
                    y: placement.target.y.checked_sub(cursor_origin.y).ok_or_else(|| AppError::new("capture cursor coordinate overflow"))?,
                    ..placement.target
                }, clip: Some(RectI { x: 0, y: 0, width: job.buffer.layout.width() as i32, height: job.buffer.layout.height() as i32 }),
                rounded_clips: placement.rounded_clips,
            });
            scenes.push(VulkanCompositeScene { scene });
        }
        let mut frame = self.device.begin_owned_frame().map_err(app_error)?;
        {
            let mut context = frame.context_mut();
            job.buffer.slot.record(&self.device, &mut scenes, &placements, &mut context).map_err(app_error)?;
        }
        frame.finish().map_err(app_error)
        })();
        let recorded = match recorded {
            Ok(recorded) => recorded,
            Err(error) => return Err(CaptureSubmitFailure { error, rejected: Some(Box::new(job)) }),
        };
        let receipt = recorded.submit().map_err(|error| CaptureSubmitFailure {
            error: app_error(error), rejected: None,
        })?;
        self.completion_worker.submit_capture(receipt, job).map_err(|error| CaptureSubmitFailure { error, rejected: None })
    }

    pub(super) fn retry_capture(&self, receipt: SubmissionReceipt, job: CaptureJob) -> AppResult<()> {
        self.completion_worker.submit_capture(receipt, job)
    }

    pub(super) fn motion_enabled(&self) -> bool {
        !self.motion_failed
    }
    /// Acknowledge a failed frame after the host has discarded its matching motion state.
    /// The next frame can seed fresh snapshots instead of referencing the discarded captures.
    pub(super) fn take_motion_failure(&mut self) -> bool {
        std::mem::take(&mut self.motion_failed)
    }
    fn from_targets(
        device: VulkanDevice,
        mut targets: Vec<VulkanDmaBufScanoutTarget>,
    ) -> AppResult<Self> {
        // Compile every desktop/offscreen variant before accepting interactive frames.
        let mut formats = vec![VulkanMaterializationTarget::FORMAT];
        for target in &mut targets {
            let format = target.target().format;
            if !formats.contains(&format) {
                formats.push(format);
            }
        }
        device
            .prewarm_compositor_pipelines(&formats)
            .map_err(app_error)?;
        let target_count = targets.len();
        let dma_buf_importer = DmaBufImporter::new(&device).ok();
        let completion_worker = VulkanCompletionWorker::new()?;
        let allocator = TargetAllocator::new(
            device.clone(),
            completion_worker.wake.try_clone().map_err(app_error)?,
        )?;
        Ok(Self {
            device,
            scenes: BTreeMap::new(),
            capture_placements: Vec::new(),
            capture_cursor_scene: None,
            capture_cursor_placement: None,
            capture_revisions: super::capture::CaptureRevisions::default(),
            motion_snapshots: BTreeMap::new(),
            motion_spares: Vec::new(),
            motion_failed: false,
            motion_glass: motion_glass::MotionGlass::default(),
            glass_caches: BTreeMap::new(),
            targets,
            content_version: 0,
            completion_worker,
            allocator,
            materialized: BTreeMap::new(),
            spares: VecDeque::new(),
            target_versions: vec![0; target_count],
            damage_history: VecDeque::new(),
            dma_buf_importer,
            pending_dma_bufs: BTreeMap::new(),
            next_dma_buf_content_version: 1,
            retirement: RetirementWorker::new()?,
        })
    }

    pub(super) fn dma_buf_formats(&self) -> Vec<DmaBufFormat> {
        self.dma_buf_importer
            .as_ref()
            .map_or_else(Vec::new, DmaBufImporter::advertised_formats)
    }

    pub(super) fn queue_dma_buf(
        &mut self,
        publication: DmaBufPublication,
        display: &crate::wayland_server::Display,
    ) -> AppResult<DmaBufQueueResult> {
        let importer = self
            .dma_buf_importer
            .as_ref()
            .ok_or_else(|| AppError::new("Vulkan DMA-BUF import is unavailable"))?;
        let (_, alpha_mode) = importer
            .image_metadata(&publication.image)
            .map_err(app_error)?;
        let (extent, transform) = dma_buf_surface_mapping(
            publication.image.descriptor.size,
            publication.buffer_scale,
            publication.buffer_transform,
            publication.viewport,
            publication.image.descriptor.flags.y_invert,
        )?;
        let raster_scale = super::super::geometry::surface_raster_scale(
            publication.output_scale,
            publication.coordinate_density,
        )
        .get();
        let raster_extent = materialization_extent(extent, raster_scale);
        let content_version = self.next_dma_buf_content_version;
        self.next_dma_buf_content_version = self
            .next_dma_buf_content_version
            .checked_add(1)
            .ok_or_else(|| AppError::new("DMA-BUF content version exhausted"))?;
        let scene = ShellSceneKey::Surface(publication.surface.get());
        let geometry = MaterializationGeometry {
            physical: publication.image.descriptor.size,
            extent,
            raster: raster_extent,
            transform,
            scale: raster_scale,
            alpha: alpha_mode,
        };
        let mut region = materialization_damage(
            &publication.surface_damage,
            &publication.buffer_damage,
            geometry,
        );
        let previous_revision = self
            .pending_dma_bufs
            .get(&scene)
            .map(|previous| previous.publication.revision)
            .or_else(|| {
                self.materialized
                    .get(&scene)
                    .and_then(|resource| resource.revision)
            });
        let previous_geometry = self
            .pending_dma_bufs
            .get(&scene)
            .map(|p| p.geometry)
            .or_else(|| self.materialized.get(&scene).and_then(|r| r.geometry));
        // Output damage must use the same compatibility decision as the owned-image copy.
        region = compatible_materialization_damage(
            previous_geometry,
            previous_revision,
            geometry,
            publication.revision,
            region,
        );
        if let Some(previous) = self.pending_dma_bufs.get(&scene) {
            region = if previous.geometry == geometry {
                merge_damage(previous.region, region)
            } else {
                None
            };
        }
        let acquire_watch = super::super::dma_buf_readiness::AcquireWatch::new(
            display,
            publication.acquire.as_ref(),
        )?;
        let replaced = self
            .pending_dma_bufs
            .insert(
                scene,
                PendingDmaBufPublication {
                    publication,
                    acquire_watch,
                    content_version,
                    extent,
                    transform,
                    alpha_mode,
                    geometry,
                    region,
                },
            )
            .map(|pending| DmaBufRetirement {
                surface: pending.publication.surface,
                revision: pending.publication.revision,
                buffer: pending.publication.buffer,
            });
        Ok(DmaBufQueueResult {
            extent,
            raster_extent,
            pixel_format: ImagePixelFormat::Rgba8,
            alpha_mode,
            image: dma_buf_image_id(),
            damage: region,
            replaced,
        })
    }

    pub(super) fn cancel_dma_buf_surface(
        &mut self,
        surface: WaylandSurfaceId,
    ) -> Option<DmaBufRetirement> {
        if let Some(old) = self
            .materialized
            .remove(&ShellSceneKey::Surface(surface.get()))
        {
            self.spares.push_back(old);
            trim_spares(&mut self.spares, &mut self.retirement);
        }
        self.pending_dma_bufs
            .remove(&ShellSceneKey::Surface(surface.get()))
            .map(|pending| DmaBufRetirement {
                surface,
                revision: pending.publication.revision,
                buffer: pending.publication.buffer,
            })
    }

    pub(super) fn take_acquire_wakeup(&mut self) -> bool {
        self.pending_dma_bufs
            .values_mut()
            .fold(false, |wake, pending| {
                pending.acquire_watch.take_wakeup() || wake
            })
    }

    pub(super) fn poll_allocations(
        &mut self,
        trace: &mut super::super::latency_trace::LatencyTrace,
    ) -> AppResult<bool> {
        let Some(target) = self.allocator.take(trace)? else {
            return Ok(false);
        };
        self.spares.push_back(MaterializationResources {
            target,
            source: self.device.create_scene().map_err(app_error)?,
            geometry: None,
            revision: None,
        });
        Ok(true)
    }

    /// Prepare independent eligible surfaces without holding up desktop placement changes.
    pub(super) fn prepare_render(
        &mut self,
        eligible: &std::collections::BTreeSet<WaylandSurfaceId>,
        trace: &mut super::super::latency_trace::LatencyTrace,
    ) -> AppResult<std::collections::BTreeSet<WaylandSurfaceId>> {
        self.poll_allocations(trace)?;
        let mut ready = eligible.clone();
        for (key, pending) in &mut self.pending_dma_bufs {
            let surface = pending.publication.surface;
            if !eligible.contains(&surface) {
                continue;
            }
            if !pending.acquire_watch.ready()? {
                ready.remove(&surface);
                continue;
            }
            if self
                .materialized
                .get(key)
                .is_some_and(|r| r.target.extent() == pending.geometry.raster)
            {
                ready.insert(surface);
                continue;
            }
            if let Some(index) = self.spares.iter().position(|r| {
                r.target.extent() == pending.geometry.raster && r.target.can_recycle()
            }) {
                let mut resource = self.spares.remove(index).expect("spare exists");
                resource.geometry = None;
                resource.revision = None;
                if let Some(old) = self.materialized.insert(*key, resource) {
                    self.spares.push_back(old);
                }
                ready.insert(surface);
                trace.event("dmabuf_target_reused", [1, 0, 0, 0]);
            } else {
                ready.remove(&surface);
                self.allocator
                    .request(pending.geometry.raster, trace.enabled())?;
                trace.event("dmabuf_allocation_pending", [surface.get() as u64, 0, 0, 0]);
            }
        }
        trim_spares(&mut self.spares, &mut self.retirement);
        Ok(ready)
    }

    pub(super) fn render(
        &mut self,
        target_index: usize,
        frame: ShellFrame,
        trace: &mut super::super::latency_trace::LatencyTrace,
    ) -> AppResult<VulkanRenderResult> {
        self.content_version = self.content_version.wrapping_add(1).max(1);
        let damage = frame
            .damage
            .and_then(|rect| intersect_rect(rect, full_rect(frame.extent)));
        self.damage_history
            .push_back((self.content_version, damage));
        while self.damage_history.len() > 64 {
            self.damage_history.pop_front();
        }
        trace.phase("vulkan_dmabuf_prepare");
        let (mut materializations, discarded) =
            self.prepare_dma_bufs(&frame.surface_revisions, trace)?;
        trace.phase("vulkan_scene_updates");
        let materialized_scenes = materializations
            .iter()
            .map(|materialization| ShellSceneKey::Surface(materialization.retirement.surface.get()))
            .collect::<std::collections::BTreeSet<_>>();
        for update in &frame.updates {
            let scene = match self.scenes.entry(update.key) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(self.device.create_scene().map_err(app_error)?)
                }
            };
            if matches!(update.key, ShellSceneKey::Surface(_))
                && !materialized_scenes.contains(&update.key)
            {
                scene.remove_materialized_image(dma_buf_image_id());
            }
            for delta in &update.deltas {
                self.device
                    .apply_scene_delta(scene, delta)
                    .map_err(|error| {
                        AppError::new(format!(
                            "Vulkan scene update {:?} epoch {} (retained epoch {}): {error}",
                            update.key,
                            delta.epoch,
                            scene.epoch()
                        ))
                    })?;
            }
        }
        self.scenes.retain(|key, _| frame.live_scenes.contains(key)
            || matches!(key, ShellSceneKey::TileGlass(id) if frame.glass.contains_key(&ShellSceneKey::TilePreview(*id)))
            || matches!(key, ShellSceneKey::ResizeGlass(id) if frame.glass.contains_key(&ShellSceneKey::ResizeVeil(*id))));
        let stale = self
            .materialized
            .keys()
            .filter(|key| !frame.live_scenes.contains(key))
            .copied()
            .collect::<Vec<_>>();
        for key in stale {
            self.spares
                .push_back(self.materialized.remove(&key).expect("stale resource"));
        }
        trim_spares(&mut self.spares, &mut self.retirement);
        let previous_target_version = *self
            .target_versions
            .get(target_index)
            .ok_or_else(|| AppError::new("Vulkan scanout target index is invalid"))?;
        let mut render_damage = accumulated_damage(
            previous_target_version,
            self.content_version,
            &self.damage_history,
            frame.extent,
        );

        trace.phase("vulkan_record");
        let receipt = {
            let target = self
                .targets
                .get_mut(target_index)
                .ok_or_else(|| AppError::new("Vulkan scanout target index is invalid"))?
                .target();
            let mut recording = self.device.begin_owned_frame().map_err(app_error)?;
            {
                let mut context = recording.context_mut();
                for materialization in &mut materializations {
                    // Pin render destinations even if the final scene is fully clipped this frame.
                    context.core.images.push(materialization.target.image());
                    let target = materialization.target.target();
                    let placement = [VulkanCompositePlacement {
                        scene_index: 0,
                        target: full_rect(materialization.target.extent()),
                        clip: None,
                        rounded_clips: [None; 2],
                    }];
                    let mut source = [VulkanCompositeScene {
                        scene: &mut materialization.source,
                    }];
                    self.device
                        .render_composite(
                            &mut source,
                            &placement,
                            &mut context,
                            &target,
                            &RenderRequest {
                                force: true,
                                load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 0)),
                                store: TargetStore::Store,
                                // Clear only the affected render area, including for transparent
                                // clients; blending over stale pixels would accumulate alpha.
                                region: materialization.region,
                            },
                        )
                        .map_err(app_error)?;
                }
                let base_placements = if frame.motion.fallback.is_empty() {
                    &frame.placements
                } else {
                    &frame.motion.fallback
                };
                let capture_scope = context.core.begin_gpu_scope("gpu.motion.snapshots");
                let mut motion_ok = !self.motion_failed
                    && motion::record_motion(
                        &self.device,
                        &mut self.scenes,
                        &mut self.motion_snapshots,
                        &mut self.motion_spares,
                        &frame.motion,
                        &frame.glass,
                        &frame.preview_borders,
                        &mut self.motion_glass,
                        &mut context,
                    )?;
                context.core.end_gpu_scope(capture_scope);
                let effect_scope = context
                    .core
                    .begin_gpu_scope(if frame.motion.divider_dragging {
                        "gpu.tile_divider.resolve"
                    } else {
                        "gpu.motion.resolve"
                    });
                let live_output = if motion_ok {
                    motion_glass::record_output(
                        &self.device,
                        &mut self.scenes,
                        &self.motion_snapshots,
                        &mut self.motion_spares,
                        &mut self.motion_glass,
                        &mut self.glass_caches,
                        &frame,
                        &mut context,
                    )?
                } else {
                    None
                };
                context.core.end_gpu_scope(effect_scope);
                motion_ok &= live_output.is_some();
                if !motion_ok && !self.motion_failed {
                    eprintln!(
                        "Telorgon: window motion fell back after snapshot/effect resource exhaustion; discarding this transition and retrying with fresh snapshots"
                    );
                    self.motion_failed = true;
                    // Immediate geometry can uncover pixels outside the animated damage.
                    // Record full damage for this slot and the other scanout slots too.
                    render_damage = None;
                    if let Some((_, damage)) = self.damage_history.back_mut() {
                        *damage = None;
                    }
                    self.motion_snapshots.clear();
                    self.motion_spares.clear();
                    self.motion_glass = motion_glass::MotionGlass::default();
                }
                let output_placements = if let Some(output) = live_output {
                    output
                } else {
                    self.glass_caches
                        .retain(|key, _| frame.glass.contains_key(key));
                    glass::record_glass(
                        &self.device,
                        &mut self.scenes,
                        &mut self.glass_caches,
                        &frame,
                        base_placements,
                        &mut context,
                    )?
                };
                let scene_indices = self
                    .scenes
                    .keys()
                    .copied()
                    .enumerate()
                    .map(|(index, key)| (key, index))
                    .collect::<BTreeMap<_, _>>();
                self.capture_placements.clear();
                self.capture_placements.extend_from_slice(&output_placements);
                self.capture_revisions.output_changed();
                let placements = output_placements
                    .iter()
                    .map(|placement| {
                        Ok(VulkanCompositePlacement {
                            scene_index: *scene_indices.get(&placement.scene).ok_or_else(|| {
                                AppError::new(format!(
                                    "Vulkan desktop scene {:?} has no retained content",
                                    placement.scene
                                ))
                            })?,
                            target: placement.target,
                            clip: placement.clip,
                            rounded_clips: placement.rounded_clips,
                        })
                    })
                    .collect::<AppResult<Vec<_>>>()?;
                let mut scenes = self
                    .scenes
                    .values_mut()
                    .map(|scene| VulkanCompositeScene { scene })
                    .collect::<Vec<_>>();

                let composite_scope = context.core.begin_gpu_scope("gpu.desktop.compose");
                self.device
                    .render_composite(
                        &mut scenes,
                        &placements,
                        &mut context,
                        &target,
                        &RenderRequest {
                            force: true,
                            load: if render_damage.is_some() {
                                TargetLoad::Preserve
                            } else {
                                TargetLoad::Clear(ColorRgba8 {
                                    r: 0,
                                    g: 0,
                                    b: 0,
                                    a: 255,
                                })
                            },
                            store: TargetStore::Store,
                            region: render_damage,
                        },
                    )
                    .map_err(app_error)?;
                context.core.end_gpu_scope(composite_scope);
            }
            trace.phase("vulkan_submit");
            recording
                .finish()
                .and_then(|frame| frame.submit())
                .map_err(app_error)?
        };
        trace.phase("vulkan_release_export_enqueue");
        self.targets[target_index].mark_initialized();
        for (surface, revision) in &frame.surface_revisions {
            if let Some(scene) = self.scenes.get(&ShellSceneKey::Surface(*surface)) {
                for image in &scene.images {
                    super::super::resize_trace::event(
                        *surface,
                        "gpu-submitted",
                        format_args!(
                            "slot={} render={} revision={} image={:?} content_version={} binding={:?} rect={:?} damage={:?}",
                            target_index,
                            self.content_version,
                            revision,
                            image.image,
                            image.content_version,
                            scene.retained_image_trace_metadata(image.image),
                            image.rect,
                            render_damage
                        ),
                    );
                }
            }
        }
        self.target_versions[target_index] = self.content_version;
        let exported = receipt
            .export_dma_buf_release_sync_fds()
            .map_err(app_error)?;
        if exported.len() != materializations.len() {
            return Err(AppError::new(
                "Vulkan did not export one release fence per DMA-BUF materialization",
            ));
        }
        let mut releases = Vec::with_capacity(exported.len());
        for release in exported {
            let materialization = materializations
                .iter()
                .find(|materialization| {
                    materialization.content_version == release.content_version
                        && materialization.lease_generation == release.lease_generation
                })
                .ok_or_else(|| {
                    AppError::new("Vulkan returned an unknown DMA-BUF release generation")
                })?;
            releases.push(DmaBufRelease {
                retirement: materialization.retirement,
                fence: release.sync_fd,
            });
        }
        let mut dma_bufs = Vec::with_capacity(materializations.len());
        for mut materialization in materializations {
            materialization.target.mark_initialized();
            materialization
                .source
                .remove_external_image(dma_buf_image_id());
            dma_bufs.push(materialization.retirement);
            self.materialized.insert(
                ShellSceneKey::Surface(materialization.retirement.surface.get()),
                MaterializationResources {
                    target: materialization.target,
                    source: materialization.source,
                    geometry: Some(materialization.geometry),
                    revision: Some(materialization.retirement.revision),
                },
            );
        }
        self.completion_worker
            .submit(target_index, receipt, dma_bufs)?;
        Ok(VulkanRenderResult {
            releases,
            discarded,
        })
    }

    fn prepare_dma_bufs(
        &mut self,
        surface_revisions: &[(u32, u64)],
        trace: &mut super::super::latency_trace::LatencyTrace,
    ) -> AppResult<(Vec<DmaBufMaterialization>, Vec<DmaBufRetirement>)> {
        let pending = std::mem::take(&mut self.pending_dma_bufs);
        let mut materializations = Vec::new();
        let discarded = Vec::new();
        for (scene_key, pending) in pending {
            let retirement = DmaBufRetirement {
                surface: pending.publication.surface,
                revision: pending.publication.revision,
                buffer: pending.publication.buffer,
            };
            if !surface_revisions.contains(&(retirement.surface.get(), retirement.revision)) {
                self.pending_dma_bufs.insert(scene_key, pending);
                continue;
            }
            trace.phase("vulkan_dmabuf_reuse");
            let resources = self.materialized.remove(&scene_key).ok_or_else(|| {
                AppError::new("DMA-BUF target must be prepared before consuming the desktop frame")
            })?;
            let region =
                if resources.geometry == Some(pending.geometry) && resources.target.initialized() {
                    pending.region
                } else {
                    None
                };
            let target = resources.target;
            let mut source = resources.source;
            source.remove_external_image(dma_buf_image_id());
            let full_pixels =
                u64::from(target.extent().width as u32) * u64::from(target.extent().height as u32);
            let pixels = region.map_or(full_pixels, |r| {
                u64::from(r.width as u32) * u64::from(r.height as u32)
            });
            trace.event(
                "dmabuf_materialized_pixels",
                [pixels, full_pixels, u64::from(region.is_some()), 0],
            );
            let physical_extent = pending.publication.image.descriptor.size;
            trace.phase("vulkan_dmabuf_import");
            let lease_generation = self
                .dma_buf_importer
                .as_mut()
                .ok_or_else(|| AppError::new("Vulkan DMA-BUF import is unavailable"))?
                .import_and_bind(
                    &self.device,
                    &mut source,
                    pending.publication.buffer,
                    pending.content_version,
                    pending.publication.image,
                    pending.publication.acquire,
                    vec![full_rect(physical_extent)],
                )
                .map_err(app_error)?;
            if let Some(importer) = &mut self.dma_buf_importer {
                let [hits, misses] = importer.take_cache_counts();
                trace.event("dmabuf_import_cache", [hits, misses, 0, 0]);
            }
            trace.phase("vulkan_dmabuf_source_delta");
            self.device
                .apply_scene_delta(
                    &mut source,
                    &dma_buf_source_delta(
                        physical_extent,
                        pending.extent,
                        pending.transform,
                        pending.content_version,
                        pending.alpha_mode,
                    ),
                )
                .map_err(app_error)?;
            trace.phase("vulkan_dmabuf_bind");
            let scene = match self.scenes.entry(scene_key) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(self.device.create_scene().map_err(app_error)?)
                }
            };
            scene
                .bind_materialized_image(dma_buf_image_id(), &target, pending.alpha_mode)
                .map_err(app_error)?;
            super::super::resize_trace::event(
                retirement.surface.get(),
                "materialized-binding",
                format_args!(
                    "revision={} buffer={:?} image={:?} content_version={} lease={} source_extent={:?} binding={:?}",
                    retirement.revision,
                    retirement.buffer,
                    dma_buf_image_id(),
                    pending.content_version,
                    lease_generation,
                    physical_extent,
                    scene.retained_image_trace_metadata(dma_buf_image_id())
                ),
            );
            materializations.push(DmaBufMaterialization {
                source,
                target,
                retirement,
                content_version: pending.content_version,
                lease_generation,
                geometry: pending.geometry,
                region,
            });
        }
        Ok((materializations, discarded))
    }

    pub(super) fn completion_event_fd(&self) -> i32 {
        self.completion_worker.event_fd()
    }

    pub(super) fn drain_completions(&self) -> Vec<VulkanCompletion> {
        self.completion_worker.drain()
    }
}

fn materialization_extent(extent: SizeI, scale: f32) -> SizeI {
    SizeI {
        width: (extent.width as f32 * scale).round().max(1.0) as i32,
        height: (extent.height as f32 * scale).round().max(1.0) as i32,
    }
}

// None means full redraw, including unreported damage. Never turn missing history into no work.
fn damage_history_contiguous(previous: Option<u64>, revision: u64) -> bool {
    previous
        .is_some_and(|previous| previous == revision || previous.checked_add(1) == Some(revision))
}

fn merge_damage(a: Option<RectI>, b: Option<RectI>) -> Option<RectI> {
    let (a, b) = (a?, b?);
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Some(RectI {
        x,
        y,
        width: a.right().max(b.right()) - x,
        height: a.bottom().max(b.bottom()) - y,
    })
}

fn compatible_materialization_damage(
    previous: Option<MaterializationGeometry>,
    previous_revision: Option<u64>,
    geometry: MaterializationGeometry,
    revision: u64,
    damage: Option<RectI>,
) -> Option<RectI> {
    if previous == Some(geometry) && damage_history_contiguous(previous_revision, revision) {
        damage
    } else {
        None
    }
}

fn materialization_damage(
    surface: &[RectI],
    buffer: &[RectI],
    geometry: MaterializationGeometry,
) -> Option<RectI> {
    let mut damage = None;
    // Expand outward by a source pixel before mapping to cover linear filtering's footprint.
    for (rect, transform) in surface
        .iter()
        .map(|r| (r, Affine2D::IDENTITY))
        .chain(buffer.iter().map(|r| (r, geometry.transform)))
    {
        if rect.width <= 0 || rect.height <= 0 {
            continue;
        }
        let logical = transform.transform_rect(RectF {
            x: rect.x as f32 - 1.0,
            y: rect.y as f32 - 1.0,
            width: rect.width as f32 + 2.0,
            height: rect.height as f32 + 2.0,
        });
        let left = (logical.x * geometry.scale)
            .floor()
            .max(0.0)
            .min(geometry.raster.width as f32) as i32;
        let top = (logical.y * geometry.scale)
            .floor()
            .max(0.0)
            .min(geometry.raster.height as f32) as i32;
        let right = (logical.right() * geometry.scale)
            .ceil()
            .max(0.0)
            .min(geometry.raster.width as f32) as i32;
        let bottom = (logical.bottom() * geometry.scale)
            .ceil()
            .max(0.0)
            .min(geometry.raster.height as f32) as i32;
        if right > left && bottom > top {
            let rect = RectI {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            };
            damage = Some(damage.map_or(rect, |old| merge_damage(Some(old), Some(rect)).unwrap()));
        }
    }
    damage.filter(|r| *r != full_rect(geometry.raster))
}

fn dma_buf_surface_mapping(
    physical: SizeI,
    buffer_scale: i32,
    transform: BufferTransform,
    viewport: Option<ViewportState>,
    y_invert: bool,
) -> AppResult<(SizeI, Affine2D)> {
    if physical.width <= 0 || physical.height <= 0 || buffer_scale <= 0 {
        return Err(AppError::new("DMA-BUF surface geometry is invalid"));
    }
    let swap_axes = matches!(
        transform,
        BufferTransform::Rotate90
            | BufferTransform::Rotate270
            | BufferTransform::Flipped90
            | BufferTransform::Flipped270
    );
    let transformed = if swap_axes {
        SizeI {
            width: physical.height,
            height: physical.width,
        }
    } else {
        physical
    };
    if transformed.width % buffer_scale != 0 || transformed.height % buffer_scale != 0 {
        return Err(AppError::new(
            "DMA-BUF transformed extent is not divisible by its buffer scale",
        ));
    }
    let logical = SizeI {
        width: transformed.width / buffer_scale,
        height: transformed.height / buffer_scale,
    };
    let viewport = viewport.unwrap_or_default();
    let source = viewport.source.unwrap_or(ViewportSource {
        x: 0.0,
        y: 0.0,
        width: f64::from(logical.width),
        height: f64::from(logical.height),
    });
    if !source.x.is_finite()
        || !source.y.is_finite()
        || !source.width.is_finite()
        || !source.height.is_finite()
        || source.x < 0.0
        || source.y < 0.0
        || source.width <= 0.0
        || source.height <= 0.0
        || source.x + source.width > f64::from(logical.width)
        || source.y + source.height > f64::from(logical.height)
    {
        return Err(AppError::new(
            "DMA-BUF viewport source lies outside the logical surface",
        ));
    }
    let extent = viewport.destination.unwrap_or(SizeI {
        width: source.width as i32,
        height: source.height as i32,
    });
    if extent.width <= 0 || extent.height <= 0 {
        return Err(AppError::new("DMA-BUF viewport destination is invalid"));
    }

    let width = physical.width as f32;
    let height = physical.height as f32;
    let transformed = match transform {
        BufferTransform::Normal => Affine2D::IDENTITY,
        BufferTransform::Rotate90 => Affine2D {
            m11: 0.0,
            m12: 1.0,
            m21: -1.0,
            m22: 0.0,
            tx: height,
            ty: 0.0,
        },
        BufferTransform::Rotate180 => Affine2D {
            m11: -1.0,
            m12: 0.0,
            m21: 0.0,
            m22: -1.0,
            tx: width,
            ty: height,
        },
        BufferTransform::Rotate270 => Affine2D {
            m11: 0.0,
            m12: -1.0,
            m21: 1.0,
            m22: 0.0,
            tx: 0.0,
            ty: width,
        },
        BufferTransform::Flipped => Affine2D {
            m11: -1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            tx: width,
            ty: 0.0,
        },
        BufferTransform::Flipped90 => Affine2D {
            m11: 0.0,
            m12: -1.0,
            m21: -1.0,
            m22: 0.0,
            tx: height,
            ty: width,
        },
        BufferTransform::Flipped180 => Affine2D {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: -1.0,
            tx: 0.0,
            ty: height,
        },
        BufferTransform::Flipped270 => Affine2D {
            m11: 0.0,
            m12: 1.0,
            m21: 1.0,
            m22: 0.0,
            tx: 0.0,
            ty: 0.0,
        },
    };
    let origin = if y_invert {
        Affine2D {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: -1.0,
            tx: 0.0,
            ty: height,
        }
    } else {
        Affine2D::IDENTITY
    };
    let transformed = transformed.then(origin);
    let scale = 1.0 / buffer_scale as f32;
    let logical_scale = Affine2D {
        m11: scale,
        m12: 0.0,
        m21: 0.0,
        m22: scale,
        tx: 0.0,
        ty: 0.0,
    };
    let viewport_scale_x = extent.width as f32 / source.width as f32;
    let viewport_scale_y = extent.height as f32 / source.height as f32;
    let viewport_transform = Affine2D {
        m11: viewport_scale_x,
        m12: 0.0,
        m21: 0.0,
        m22: viewport_scale_y,
        tx: -(source.x as f32) * viewport_scale_x,
        ty: -(source.y as f32) * viewport_scale_y,
    };
    Ok((
        extent,
        viewport_transform.then(logical_scale.then(transformed)),
    ))
}

fn dma_buf_source_delta(
    physical: SizeI,
    extent: SizeI,
    transform: Affine2D,
    content_version: u64,
    alpha_mode: ImageAlphaMode,
) -> crate::render::RenderSceneDelta {
    let mut source = RenderScene::default();
    source.background = ColorRgba8::rgba(0, 0, 0, 0);
    source.extent = SizeF {
        width: extent.width as f32,
        height: extent.height as f32,
    };
    source.damage.full = true;
    source.spatial_nodes.upsert(
        NodeId::new(2, 1),
        RenderSpatialNode {
            id: SpatialId(1),
            transform,
        },
    );
    source.images.upsert(
        NodeId::new(1, 1),
        ImageInstance {
            node: NodeId::new(1, 1),
            image: dma_buf_image_id(),
            tint: None,
            rect: RectF {
                x: 0.0,
                y: 0.0,
                width: physical.width as f32,
                height: physical.height as f32,
            },
            view_bounds: RectF {
                x: 0.0,
                y: 0.0,
                width: extent.width as f32,
                height: extent.height as f32,
            },
            content_version,
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(1),
        },
    );
    source.set_draw_order(vec![DrawItem {
        kind: PrimitiveKind::Image,
        index: 0,
        batch: BatchKey {
            pipeline: PipelineKind::Image,
            resource: dma_buf_image_id().0,
            clip: ClipId(0),
            blend: if alpha_mode == ImageAlphaMode::Opaque {
                BlendMode::Opaque
            } else {
                BlendMode::Alpha
            },
            target: 0,
        },
    }]);
    let mut delta = source
        .take_delta()
        .expect("new DMA-BUF source scene always produces a delta");
    // This temporary CPU scene starts at epoch 1, but its Vulkan destination survives updates
    // and pool reassignment. Use the host's globally increasing publication version so later
    // image/geometry updates pass the retained backend's stale-delta gate.
    delta.epoch = content_version;
    delta
}

fn app_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_source_scene_accepts_each_publication_and_preserves_stale_delta_guard() {
        use crate::render::{ImageColorEncoding, ImageResourceDelta, ImageResourceUpdate};
        let mut retained = VulkanScene::default();
        // CPU resource metadata stands in for the external binding; no Vulkan device or FD is
        // needed to exercise the real backend epoch gate, validation and retained scene updates.
        let physical = SizeI {
            width: 8,
            height: 8,
        };
        let mut last = None;
        for version in [1, 2, 7, 24] {
            let extent = SizeI {
                width: 8,
                height: if version == 1 { 8 } else { 4 },
            };
            let mut delta = dma_buf_source_delta(
                physical,
                extent,
                Affine2D::IDENTITY,
                version,
                ImageAlphaMode::Opaque,
            );
            delta
                .image_resources
                .push(ImageResourceDelta::Write(ImageResourceUpdate {
                    image: dma_buf_image_id(),
                    content_version: version,
                    extent: physical,
                    rect: full_rect(physical),
                    row_bytes: 8 * 4,
                    color_encoding: ImageColorEncoding::Srgb,
                    alpha_mode: ImageAlphaMode::Opaque,
                    pixel_format: ImagePixelFormat::Rgba8,
                    pixels: vec![255; 8 * 8 * 4].into(),
                }));
            let stats = retained.apply_delta_checked(&delta).unwrap();
            assert_eq!(
                stats.epoch, version,
                "publication {version} must not be skipped"
            );
            assert_eq!(retained.images[0].content_version, version);
            assert_eq!(retained.images[0].view_bounds.height, extent.height as f32);
            last = Some(delta);
        }
        let mut stale = last.unwrap();
        stale.epoch = 2;
        stale.image_len = 0;
        stale.images.clear();
        let stats = retained.apply_delta_checked(&stale).unwrap();
        assert_eq!(stats.epoch, 24);
        assert_eq!(retained.images[0].content_version, 24);
    }

    #[test]
    fn geometry_changes_invalidate_output_damage_even_at_the_same_extent() {
        let geometry = damage_geometry(1.0);
        let damage = Some(RectI {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        });
        assert_eq!(
            compatible_materialization_damage(Some(geometry), Some(1), geometry, 2, damage),
            damage
        );
        let mut transformed = geometry;
        transformed.transform = Affine2D::translation(1.0, 0.0);
        assert_eq!(
            compatible_materialization_damage(Some(geometry), Some(1), transformed, 2, damage),
            None
        );
        assert_eq!(
            compatible_materialization_damage(Some(geometry), Some(1), geometry, 3, damage),
            None
        );
        assert_eq!(
            compatible_materialization_damage(None, Some(1), geometry, 2, damage),
            None
        );
    }

    fn damage_geometry(scale: f32) -> MaterializationGeometry {
        MaterializationGeometry {
            physical: SizeI {
                width: 100,
                height: 100,
            },
            extent: SizeI {
                width: 100,
                height: 100,
            },
            raster: SizeI {
                width: (100.0 * scale) as i32,
                height: (100.0 * scale) as i32,
            },
            transform: Affine2D::IDENTITY,
            scale,
            alpha: ImageAlphaMode::Premultiplied,
        }
    }

    #[test]
    fn materialization_damage_keeps_spaces_and_filter_coverage() {
        let mut geometry = damage_geometry(3.0);
        geometry.transform = Affine2D {
            m11: 0.5,
            m22: 0.5,
            ..Affine2D::IDENTITY
        };
        let rect = RectI {
            x: 20,
            y: 10,
            width: 10,
            height: 10,
        };
        assert_eq!(
            materialization_damage(&[rect], &[], geometry),
            Some(RectI {
                x: 57,
                y: 27,
                width: 36,
                height: 36
            })
        );
        assert_eq!(
            materialization_damage(&[], &[rect], geometry),
            Some(RectI {
                x: 28,
                y: 13,
                width: 19,
                height: 19
            })
        );
        let union = materialization_damage(&[rect], &[rect], geometry).unwrap();
        assert_eq!(
            union,
            RectI {
                x: 28,
                y: 13,
                width: 65,
                height: 50
            }
        );
    }

    #[test]
    fn damage_rotation_clips_and_unknown_history_forces_full() {
        let mut geometry = damage_geometry(1.0);
        geometry.transform = Affine2D {
            m11: 0.0,
            m12: 1.0,
            m21: -1.0,
            m22: 0.0,
            tx: 100.0,
            ty: 0.0,
        };
        let damage = materialization_damage(
            &[],
            &[RectI {
                x: 0,
                y: 0,
                width: 10,
                height: 20,
            }],
            geometry,
        );
        assert_eq!(
            damage,
            Some(RectI {
                x: 79,
                y: 0,
                width: 21,
                height: 11
            })
        );
        assert_eq!(materialization_damage(&[], &[], geometry), None);
        assert_eq!(
            materialization_damage(&[full_rect(geometry.raster)], &[], geometry),
            None
        );
        assert_eq!(merge_damage(damage, None), None);
        assert_eq!(merge_damage(None, damage), None);
    }

    #[test]
    fn missing_snapshot_revisions_invalidate_partial_damage() {
        assert!(!damage_history_contiguous(None, 10));
        assert!(damage_history_contiguous(Some(10), 11));
        assert!(damage_history_contiguous(Some(11), 11));
        assert!(!damage_history_contiguous(Some(10), 12));
        assert!(!damage_history_contiguous(Some(12), 10));
        assert!(!damage_history_contiguous(Some(u64::MAX), 0));
    }

    #[test]
    fn skipped_publications_accumulate_damage_until_materialized() {
        let a = RectI {
            x: 5,
            y: 5,
            width: 10,
            height: 10,
        };
        let b = RectI {
            x: 40,
            y: 30,
            width: 5,
            height: 5,
        };
        let c = RectI {
            x: 10,
            y: 20,
            width: 2,
            height: 2,
        };
        let merged = merge_damage(merge_damage(Some(a), Some(b)), Some(c));
        assert_eq!(
            merged,
            Some(RectI {
                x: 5,
                y: 5,
                width: 40,
                height: 30
            })
        );
        let mut changed = damage_geometry(1.0);
        changed.alpha = ImageAlphaMode::Opaque;
        assert_ne!(
            Some(changed),
            Some(damage_geometry(1.0)),
            "alpha changes invalidate retained content history"
        );
    }

    fn assert_rect_close(actual: RectF, expected: RectF) {
        for (actual, expected) in [
            (actual.x, expected.x),
            (actual.y, expected.y),
            (actual.width, expected.width),
            (actual.height, expected.height),
        ] {
            assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
        }
    }

    #[test]
    fn every_wayland_buffer_transform_maps_into_its_logical_extent() {
        let physical = SizeI {
            width: 120,
            height: 80,
        };
        for transform in [
            BufferTransform::Normal,
            BufferTransform::Rotate90,
            BufferTransform::Rotate180,
            BufferTransform::Rotate270,
            BufferTransform::Flipped,
            BufferTransform::Flipped90,
            BufferTransform::Flipped180,
            BufferTransform::Flipped270,
        ] {
            let (extent, mapping) =
                dma_buf_surface_mapping(physical, 2, transform, None, false).unwrap();
            let expected = if matches!(
                transform,
                BufferTransform::Rotate90
                    | BufferTransform::Rotate270
                    | BufferTransform::Flipped90
                    | BufferTransform::Flipped270
            ) {
                SizeI {
                    width: 40,
                    height: 60,
                }
            } else {
                SizeI {
                    width: 60,
                    height: 40,
                }
            };
            assert_eq!(extent, expected);
            assert_rect_close(
                mapping.transform_rect(RectF {
                    x: 0.0,
                    y: 0.0,
                    width: physical.width as f32,
                    height: physical.height as f32,
                }),
                RectF {
                    x: 0.0,
                    y: 0.0,
                    width: expected.width as f32,
                    height: expected.height as f32,
                },
            );
        }
    }

    #[test]
    fn dma_buf_viewport_crop_maps_exactly_to_its_destination() {
        let (extent, mapping) = dma_buf_surface_mapping(
            SizeI {
                width: 200,
                height: 100,
            },
            1,
            BufferTransform::Normal,
            Some(ViewportState {
                source: Some(ViewportSource {
                    x: 50.0,
                    y: 20.0,
                    width: 100.0,
                    height: 50.0,
                }),
                destination: Some(SizeI {
                    width: 400,
                    height: 200,
                }),
            }),
            false,
        )
        .unwrap();

        assert_eq!(
            extent,
            SizeI {
                width: 400,
                height: 200
            }
        );
        assert_rect_close(
            mapping.transform_rect(RectF {
                x: 50.0,
                y: 20.0,
                width: 100.0,
                height: 50.0,
            }),
            RectF {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
            },
        );
    }

    #[test]
    fn dma_buf_y_invert_is_normalized_before_surface_transform() {
        let physical = SizeI {
            width: 120,
            height: 80,
        };
        let (_, normal) =
            dma_buf_surface_mapping(physical, 2, BufferTransform::Normal, None, false).unwrap();
        let (_, inverted) =
            dma_buf_surface_mapping(physical, 2, BufferTransform::Normal, None, true).unwrap();
        let point = crate::core::PointF { x: 10.0, y: 6.0 };

        assert_eq!(
            normal.transform_point(point),
            crate::core::PointF { x: 5.0, y: 3.0 }
        );
        assert_eq!(
            inverted.transform_point(point),
            crate::core::PointF { x: 5.0, y: 37.0 }
        );
    }
}

/// One device per startup attempt, reused while evaluating scanout candidates.
pub(super) struct PreparedVulkan {
    device: VulkanDevice,
}
impl PreparedVulkan {
    pub(super) fn new(
        fd: &OwnedFd,
        extent: SizeI,
        slots: usize,
    ) -> crate::render::RenderResult<Self> {
        let config = VulkanConfig {
            enable_validation: false,
            frames_in_flight: slots,
            staging_budget_bytes: vulkan_staging_budget_bytes(extent, slots).map_err(|e| {
                crate::render::RenderError::new(
                    crate::render::RenderErrorKind::InvalidTarget,
                    e.to_string(),
                )
            })?,
            ..VulkanConfig::default()
        };
        let instance = VulkanInstance::load(&config, &[])?;
        let adapter_index = instance.drm_adapter(fd)?;
        let device = VulkanDevice::create_owned(
            instance,
            &config,
            &DeviceSelection { adapter_index },
            None,
        )?;
        eprintln!(
            "telorgon-kms: matched Vulkan adapter {}",
            device.capabilities().adapter_name
        );
        Ok(Self { device })
    }
    pub(super) fn modifiers(
        &self,
        fourcc: u32,
        extent: SizeI,
    ) -> crate::render::RenderResult<Vec<u64>> {
        VulkanDmaBufScanoutTarget::supported_modifiers(&self.device, fourcc, extent)
    }
    pub(super) fn import(
        &self,
        buffer: &GbmBuffer<'_, '_>,
    ) -> crate::render::RenderResult<VulkanDmaBufScanoutTarget> {
        let format = buffer.format();
        let mut planes = buffer.export_planes().map_err(|e| {
            crate::render::RenderError::new(crate::render::RenderErrorKind::Internal, e.to_string())
        })?;
        if planes.len() != 1 {
            return Err(crate::render::RenderError::new(
                crate::render::RenderErrorKind::Unsupported,
                "Vulkan scanout requires one memory plane",
            ));
        }
        let plane = planes.pop().unwrap();
        unsafe {
            VulkanDmaBufScanoutTarget::import(
                &self.device,
                plane.fd,
                format.fourcc,
                format.modifier,
                buffer.size(),
                u64::from(plane.offset),
                plane.stride,
            )
        }
    }
    pub(super) fn finish(
        &self,
        targets: Vec<VulkanDmaBufScanoutTarget>,
    ) -> AppResult<VulkanShellRenderer> {
        VulkanShellRenderer::from_targets(self.device.clone(), targets)
    }
}

#[cfg(test)]
mod density_regressions {
    use super::*;
    #[test]
    fn x11_coordinate_density_is_removed_before_raster_scaling() {
        for scale in [1.0_f32, 1.5, 2.0, 3.0] {
            let density = scale.ceil();
            let extent = SizeI {
                width: (300.0 * density) as i32,
                height: (200.0 * density) as i32,
            };
            assert_eq!(
                materialization_extent(
                    extent,
                    super::super::super::geometry::surface_raster_scale(
                        crate::platform::ScaleFactor::new(scale).unwrap(),
                        density as i32
                    )
                    .get()
                ),
                SizeI {
                    width: (300.0 * scale) as i32,
                    height: (200.0 * scale) as i32,
                }
            );
        }
        assert_eq!(
            materialization_extent(
                SizeI {
                    width: 901,
                    height: 601
                },
                1.0
            ),
            SizeI {
                width: 901,
                height: 601
            }
        );
    }
}
