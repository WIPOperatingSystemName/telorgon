use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::foundation::SizeI;
use crate::graphics::presentation::wsi::{
    PresentCompletion, PresentDisposition, PresentOutcome, PresenterReconfigurePolicy,
    PresenterState, VulkanWinitSurface,
};
use crate::graphics::render::{RenderBackend, RenderRequest, RenderSceneDelta, TargetLoad, TargetStore};
use crate::graphics::renderers::vulkan::{VulkanDevice, VulkanInstance, VulkanScene};
use winit::event_loop::EventLoopProxy;
use winit::window::Window;

use super::HostEvent;
use super::resize::{ResizeInteractionPhase, ResizeUpdate, SurfaceResizeAction};
use super::vulkan_pipeline::{
    PipelineAcquireOutcome as AcquireOutcome, VulkanPresentationPipeline,
};
use crate::host::application::SceneDeltaQueue;

const DEFAULT_FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);
const WORKER_DELTA_CAPACITY: usize = 3;
const MAX_RETIRED_SWAPCHAINS: usize = 3;

pub(super) struct VulkanWorkerInit {
    pub(super) presenter: VulkanPresentationPipeline,
    pub(super) scene: VulkanScene,
    pub(super) device: VulkanDevice,
    pub(super) instance: VulkanInstance,
    pub(super) window: Arc<Window>,
}

pub(super) struct VulkanWork {
    pub(super) resize: Option<ResizeUpdate>,
    pub(super) deltas: Vec<RenderSceneDelta>,
    pub(super) snapshot: PresentationSnapshot,
    pub(super) force_present: bool,
    pub(super) frame_interval: Duration,
    #[cfg(feature = "profiler")]
    pub(super) profile_frame: Option<crate::runtime::instrumentation::ProfileFrameId>,
    #[cfg(feature = "profiler")]
    pub(super) profile_view: Option<crate::runtime::instrumentation::ProfileViewId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PresentationSnapshot {
    pub(super) metrics_revision: u64,
    pub(super) scene_epoch: u64,
}

pub(super) struct VulkanRenderWorker {
    shared: Arc<WorkerShared>,
    join: Option<JoinHandle<()>>,
}

impl VulkanRenderWorker {
    pub(super) fn spawn(
        init: VulkanWorkerInit,
        proxy: EventLoopProxy<HostEvent>,
    ) -> Result<Self, String> {
        let shared = Arc::new(WorkerShared::new(proxy));
        let worker_shared = Arc::clone(&shared);
        let join = thread::Builder::new()
            .name("telorgon-vulkan-present".to_owned())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| worker_main(init, &worker_shared)));
                if result.is_err() {
                    worker_shared.fail("Vulkan presentation worker panicked".to_owned());
                }
            })
            .map_err(|error| format!("failed to start Vulkan presentation worker: {error}"))?;
        Ok(Self {
            shared,
            join: Some(join),
        })
    }

    pub(super) fn submit(&self, work: VulkanWork) -> Result<(), String> {
        self.poll_error()?;
        #[cfg(feature = "profiler")]
        let profile_enqueued_at = Instant::now();
        let mut mailbox = self.shared.mailbox();
        if mailbox.shutdown {
            return Err("Vulkan presentation worker is shut down".to_owned());
        }
        if let Some(resize) = work.resize {
            mailbox.resize = Some(resize);
            if resize.surface != SurfaceResizeAction::Suspend {
                mailbox.suspend = false;
            }
        }
        for delta in work.deltas {
            mailbox.deltas.push(delta);
        }
        mailbox.snapshot = work.snapshot;
        mailbox.request_present |= work.force_present || !mailbox.deltas.is_empty();
        mailbox.frame_interval = work.frame_interval.max(Duration::from_millis(1));
        #[cfg(feature = "profiler")]
        if work.profile_frame.is_some() {
            mailbox.profile_frame = work.profile_frame;
            mailbox.profile_enqueued_at = Some(profile_enqueued_at);
        }
        #[cfg(feature = "profiler")]
        if work.profile_view.is_some() {
            mailbox.profile_view = work.profile_view;
        }
        #[cfg(feature = "profiler")]
        mailbox
            .profile_enqueued_at
            .get_or_insert(profile_enqueued_at);
        drop(mailbox);
        self.shared.wake.notify_one();
        Ok(())
    }

    pub(super) fn suspend(&self) -> Result<(), String> {
        self.poll_error()?;
        let mut mailbox = self.shared.mailbox();
        if mailbox.shutdown {
            return Ok(());
        }
        mailbox.suspend = true;
        mailbox.request_present = false;
        drop(mailbox);
        self.shared.wake.notify_one();
        Ok(())
    }

    pub(super) fn poll_error(&self) -> Result<(), String> {
        let error = self
            .shared
            .error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        error.map_or(Ok(()), Err)
    }

    /// Waits until the presentation engine has completed a frame at the requested metrics revision.
    ///
    /// This wait is used only by the bounded Windows `WM_SIZE` synchronization path. Rendering and
    /// Vulkan WSI remain on the worker thread, so the UI thread cannot accidentally perform GPU
    /// work while holding the native window callback.
    pub(super) fn wait_for_presented_resize(
        &self,
        metrics_revision: u64,
        timeout: Duration,
    ) -> Result<bool, String> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let mut presented = self
            .shared
            .presented_metrics_revision
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if *presented >= metrics_revision {
                return Ok(true);
            }
            drop(presented);
            self.poll_error()?;
            let now = Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            presented = self
                .shared
                .presented_metrics_revision
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *presented >= metrics_revision {
                return Ok(true);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let (next, result) = self
                .shared
                .presented_wake
                .wait_timeout(presented, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            presented = next;
            if result.timed_out() && *presented < metrics_revision {
                drop(presented);
                self.poll_error()?;
                return Ok(false);
            }
        }
    }

    pub(super) fn shutdown(&mut self) -> Result<(), String> {
        if self.join.is_none() {
            return self.poll_error();
        }
        {
            let mut mailbox = self.shared.mailbox();
            mailbox.shutdown = true;
        }
        self.shared.wake.notify_one();
        let join = self.join.take().expect("checked above");
        if join.join().is_err() {
            self.shared
                .fail("Vulkan presentation worker panicked during shutdown".to_owned());
        }
        self.poll_error()
    }
}

impl Drop for VulkanRenderWorker {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct WorkerShared {
    mailbox: Mutex<WorkerMailbox>,
    wake: Condvar,
    error: Mutex<Option<String>>,
    presented_metrics_revision: Mutex<u64>,
    presented_wake: Condvar,
    proxy: EventLoopProxy<HostEvent>,
}

impl WorkerShared {
    fn new(proxy: EventLoopProxy<HostEvent>) -> Self {
        Self {
            mailbox: Mutex::new(WorkerMailbox::new()),
            wake: Condvar::new(),
            error: Mutex::new(None),
            presented_metrics_revision: Mutex::new(0),
            presented_wake: Condvar::new(),
            proxy,
        }
    }

    fn mailbox(&self) -> MutexGuard<'_, WorkerMailbox> {
        self.mailbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn fail(&self, message: String) {
        let mut error = self
            .error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if error.is_none() {
            *error = Some(message);
            self.presented_wake.notify_all();
            let _ = self.proxy.send_event(HostEvent::PresentationWake);
        }
    }

    fn note_presented_resize(&self, metrics_revision: u64) {
        let mut presented = self
            .presented_metrics_revision
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if metrics_revision > *presented {
            *presented = metrics_revision;
            self.presented_wake.notify_all();
        }
    }
}

struct WorkerMailbox {
    resize: Option<ResizeUpdate>,
    deltas: SceneDeltaQueue,
    snapshot: PresentationSnapshot,
    request_present: bool,
    suspend: bool,
    shutdown: bool,
    frame_interval: Duration,
    #[cfg(feature = "profiler")]
    profile_frame: Option<crate::runtime::instrumentation::ProfileFrameId>,
    #[cfg(feature = "profiler")]
    profile_view: Option<crate::runtime::instrumentation::ProfileViewId>,
    #[cfg(feature = "profiler")]
    profile_enqueued_at: Option<Instant>,
}

impl WorkerMailbox {
    fn new() -> Self {
        Self {
            resize: None,
            deltas: SceneDeltaQueue::new(WORKER_DELTA_CAPACITY),
            snapshot: PresentationSnapshot::default(),
            request_present: false,
            suspend: false,
            shutdown: false,
            frame_interval: DEFAULT_FRAME_INTERVAL,
            #[cfg(feature = "profiler")]
            profile_frame: None,
            #[cfg(feature = "profiler")]
            profile_view: None,
            #[cfg(feature = "profiler")]
            profile_enqueued_at: None,
        }
    }

    fn has_work(&self) -> bool {
        self.resize.is_some() || !self.deltas.is_empty() || self.request_present || self.suspend
    }

    fn take(&mut self, retry_due: bool) -> WorkerBatch {
        let mut deltas = Vec::with_capacity(self.deltas.len());
        while let Some(delta) = self.deltas.pop() {
            deltas.push(delta);
        }
        WorkerBatch {
            resize: self.resize.take(),
            deltas,
            snapshot: self.snapshot,
            request_present: std::mem::take(&mut self.request_present) || retry_due,
            suspend: std::mem::take(&mut self.suspend),
            shutdown: self.shutdown,
            frame_interval: self.frame_interval,
            #[cfg(feature = "profiler")]
            profile_frame: self.profile_frame.take(),
            #[cfg(feature = "profiler")]
            profile_view: self.profile_view.take(),
            #[cfg(feature = "profiler")]
            profile_enqueued_at: self.profile_enqueued_at.take(),
        }
    }
}

struct WorkerBatch {
    resize: Option<ResizeUpdate>,
    deltas: Vec<RenderSceneDelta>,
    snapshot: PresentationSnapshot,
    request_present: bool,
    suspend: bool,
    shutdown: bool,
    frame_interval: Duration,
    #[cfg(feature = "profiler")]
    profile_frame: Option<crate::runtime::instrumentation::ProfileFrameId>,
    #[cfg(feature = "profiler")]
    profile_view: Option<crate::runtime::instrumentation::ProfileViewId>,
    #[cfg(feature = "profiler")]
    profile_enqueued_at: Option<Instant>,
}

fn wait_for_batch(shared: &WorkerShared, retry_at: Option<Instant>) -> WorkerBatch {
    let mut mailbox = shared.mailbox();
    loop {
        if mailbox.shutdown || mailbox.suspend {
            return mailbox.take(false);
        }
        if let Some(deadline) = retry_at {
            let now = Instant::now();
            if now >= deadline {
                return mailbox.take(true);
            }
            let timeout = deadline.saturating_duration_since(now);
            let (next, _) = shared
                .wake
                .wait_timeout(mailbox, timeout)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox = next;
            continue;
        }
        if mailbox.has_work() {
            return mailbox.take(false);
        }
        mailbox = shared
            .wake
            .wait(mailbox)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

fn worker_main(init: VulkanWorkerInit, shared: &WorkerShared) {
    let mut state = VulkanWorkerState {
        instance: init.instance,
        device: init.device,
        presenter: init.presenter,
        scene: init.scene,
        window: init.window,
        logical_extent: None,
        swapchain_extent: None,
        staged_surface_commit: None,
        metrics_revision: 0,
        scene_epoch: 0,
        acquire_guard: AcquireStallCircuitBreaker::default(),
        presented: false,
        presented_resize_revision: None,
        pending_resize_presentation: None,
        confirmed_resize_revision: 0,
        render_pending: true,
    };
    let initial_extent = VulkanWorkerState::window_extent(&state.window);
    state.logical_extent = Some(initial_extent);
    state.swapchain_extent = Some(initial_extent);
    let mut retry_at = None;
    loop {
        let batch = wait_for_batch(shared, retry_at);
        #[cfg(feature = "profiler")]
        crate::runtime::instrumentation::instant!("worker.wake");
        if batch.shutdown {
            break;
        }
        retry_at = None;
        let result = state.process(batch);
        if let Some(metrics_revision) = state.presented_resize_revision.take() {
            shared.note_presented_resize(metrics_revision);
        }
        match result {
            Ok(WorkerProgress::Idle) => {}
            Ok(WorkerProgress::RetryAfter(interval)) => {
                retry_at = Some(Instant::now() + interval);
            }
            Err(error) => {
                shared.fail(error);
                break;
            }
        }
    }
    if let Err(error) = state.shutdown() {
        shared.fail(error);
    }
}

struct VulkanWorkerState {
    presenter: VulkanPresentationPipeline,
    scene: VulkanScene,
    device: VulkanDevice,
    instance: VulkanInstance,
    window: Arc<Window>,
    logical_extent: Option<SizeI>,
    swapchain_extent: Option<SizeI>,
    staged_surface_commit: Option<PendingSurfaceCommit>,
    metrics_revision: u64,
    scene_epoch: u64,
    acquire_guard: AcquireStallCircuitBreaker,
    presented: bool,
    presented_resize_revision: Option<u64>,
    pending_resize_presentation: Option<PendingResizePresentation>,
    confirmed_resize_revision: u64,
    render_pending: bool,
}

#[derive(Clone, Copy, Debug)]
struct PendingResizePresentation {
    metrics_revision: u64,
    proof: ResizePresentationProof,
}

#[derive(Clone, Copy, Debug)]
enum ResizePresentationProof {
    PresentFence(PresentCompletion),
    StabilizeAfter(Instant),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingSurfaceCommit {
    update: ResizeUpdate,
    preview_presented: bool,
}

impl PendingSurfaceCommit {
    const fn new(update: ResizeUpdate) -> Self {
        Self {
            update,
            preview_presented: false,
        }
    }

    const fn is_ready(self) -> bool {
        self.preview_presented
    }

    fn mark_preview_presented(&mut self) {
        self.preview_presented = true;
    }
}

enum WorkerProgress {
    Idle,
    RetryAfter(Duration),
}

enum FrameAttempt {
    Presented {
        outcome: PresentOutcome,
        target_extent: SizeI,
        acquire_duration: Duration,
    },
    Suspended {
        acquire_duration: Duration,
    },
    NotReady {
        acquire_duration: Duration,
    },
    NeedsReconfigure {
        acquire_duration: Duration,
    },
}

impl FrameAttempt {
    const fn acquire_duration(&self) -> Duration {
        match self {
            Self::Presented {
                acquire_duration, ..
            }
            | Self::Suspended { acquire_duration }
            | Self::NotReady { acquire_duration }
            | Self::NeedsReconfigure { acquire_duration } => *acquire_duration,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AcquireStallCircuitBreaker {
    active_generation: Option<u64>,
    suppressed_generation: Option<u64>,
    breach_count: u64,
}

impl AcquireStallCircuitBreaker {
    fn observe_resize(&mut self, resize: ResizeUpdate) -> bool {
        match resize.phase {
            ResizeInteractionPhase::Started | ResizeInteractionPhase::Updating => {
                if resize.surface == SurfaceResizeAction::KeepCurrent {
                    self.active_generation = Some(resize.generation);
                    self.suppressed_generation == Some(resize.generation)
                } else {
                    // Responsive resize commits must keep attempting a current-extent frame. The
                    // breaker exists only to avoid repeatedly acquiring the old swapchain used by
                    // DeferredScaledPreview after that compatibility path has already stalled.
                    self.clear();
                    false
                }
            }
            ResizeInteractionPhase::Ended => {
                let was_suppressed = resize.surface == SurfaceResizeAction::CommitAfterPreview
                    && self.suppressed_generation == Some(resize.generation);
                self.active_generation = None;
                self.suppressed_generation = None;
                was_suppressed
            }
            ResizeInteractionPhase::Stable | ResizeInteractionPhase::Cancelled => {
                self.active_generation = None;
                self.suppressed_generation = None;
                false
            }
        }
    }

    fn observe_acquire(&mut self, duration: Duration, budget: Duration) -> bool {
        let Some(generation) = self.active_generation else {
            return false;
        };
        if duration <= budget {
            return false;
        }
        self.suppressed_generation = Some(generation);
        self.breach_count = self.breach_count.saturating_add(1);
        true
    }

    fn suppresses_preview(self) -> bool {
        self.active_generation.is_some() && self.active_generation == self.suppressed_generation
    }

    fn clear(&mut self) {
        self.active_generation = None;
        self.suppressed_generation = None;
    }
}

impl VulkanWorkerState {
    fn window_extent(window: &Window) -> SizeI {
        let size = window.inner_size();
        SizeI {
            width: size.width as i32,
            height: size.height as i32,
        }
    }

    fn process(&mut self, batch: WorkerBatch) -> Result<WorkerProgress, String> {
        #[cfg(feature = "profiler")]
        let _view_scope = crate::runtime::instrumentation::enter_view(batch.profile_view);
        #[cfg(feature = "profiler")]
        let _frame_scope = crate::runtime::instrumentation::enter_frame(batch.profile_frame);
        #[cfg(feature = "profiler")]
        if let Some(enqueued_at) = batch.profile_enqueued_at {
            crate::runtime::instrumentation::counter!(
                "presentation.worker.queue_age_ns",
                enqueued_at.elapsed().as_nanos()
            );
        }
        #[cfg(feature = "profiler")]
        let _process_span = crate::runtime::instrumentation::span!("vulkan.worker.process");
        let resize_suspends = batch
            .resize
            .is_some_and(|resize| resize.surface == SurfaceResizeAction::Suspend);
        if batch.suspend || resize_suspends {
            self.acquire_guard.clear();
            self.staged_surface_commit = None;
            self.pending_resize_presentation = None;
            if let Some(resize) = batch.resize {
                if resize.metrics_revision != batch.snapshot.metrics_revision {
                    return Err(format!(
                        "suspended resize metrics revision {} does not match presentation snapshot {}",
                        resize.metrics_revision, batch.snapshot.metrics_revision
                    ));
                }
                self.logical_extent = Some(resize.extent);
                self.metrics_revision = resize.metrics_revision;
            }
            for delta in batch.deltas {
                self.device
                    .apply_scene_delta(&mut self.scene, &delta)
                    .map_err(|error| format!("Vulkan scene update failed: {error}"))?;
            }
            self.scene_epoch = self.scene.epoch();
            if self.scene_epoch < batch.snapshot.scene_epoch {
                return Err(format!(
                    "suspended presentation snapshot requires scene epoch {}, worker has {}",
                    batch.snapshot.scene_epoch, self.scene_epoch
                ));
            }
            self.presenter
                .suspend()
                .map_err(|error| format!("failed to suspend Vulkan presenter: {error}"))?;
            self.swapchain_extent = None;
            self.presented = false;
            self.render_pending = false;
            return Ok(WorkerProgress::Idle);
        }
        if let Some(resize) = batch.resize {
            if resize.metrics_revision != batch.snapshot.metrics_revision {
                return Err(format!(
                    "resize metrics revision {} does not match presentation snapshot {}",
                    resize.metrics_revision, batch.snapshot.metrics_revision
                ));
            }
            self.logical_extent = Some(resize.extent);
            self.metrics_revision = resize.metrics_revision;
            if self
                .pending_resize_presentation
                .is_some_and(|pending| pending.metrics_revision < resize.metrics_revision)
            {
                self.pending_resize_presentation = None;
            }
            let suppress_final_preview = self.acquire_guard.observe_resize(resize);
            #[cfg(feature = "profiler")]
            {
                crate::runtime::instrumentation::counter!("presentation.resize.generation", resize.generation);
                crate::runtime::instrumentation::counter!(
                    "presentation.resize.metrics_revision",
                    resize.metrics_revision
                );
                crate::runtime::instrumentation::counter!(
                    "presentation.resize.phase",
                    resize_phase_code(resize.phase)
                );
            }
            match resize.surface {
                SurfaceResizeAction::KeepCurrent => {
                    // A newer interactive resize supersedes a staged commit that has not started
                    // recreating its swapchain yet.
                    self.staged_surface_commit = None;
                    self.presenter
                        .set_reconfigure_policy(PresenterReconfigurePolicy::DeferSuboptimal);
                    self.render_pending = true;
                }
                SurfaceResizeAction::Commit | SurfaceResizeAction::CommitAfterPreview => {
                    let surface_changed = self.swapchain_extent != Some(resize.extent);
                    if should_preview_before_surface_commit(
                        resize,
                        self.swapchain_extent,
                        self.presenter.recovery().state(),
                    ) && !suppress_final_preview
                    {
                        #[cfg(feature = "profiler")]
                        crate::runtime::instrumentation::instant!("presentation.resize.preview_queued");
                        // The old swapchain can still show a frame whose scene projection uses the
                        // final content extent. DWM scales that frame to the new client extent, so
                        // it has correct geometry while the new swapchain waits for its first image.
                        self.presenter
                            .set_reconfigure_policy(PresenterReconfigurePolicy::DeferSuboptimal);
                        self.staged_surface_commit = Some(PendingSurfaceCommit::new(resize));
                        self.render_pending = true;
                    } else {
                        self.staged_surface_commit = None;
                        self.presenter
                            .set_reconfigure_policy(PresenterReconfigurePolicy::Eager);
                        if self.presenter.resize(resize.extent) || surface_changed {
                            self.presented = false;
                            self.render_pending = true;
                        }
                    }
                }
                SurfaceResizeAction::Suspend => unreachable!("handled above"),
            }
        }
        for delta in batch.deltas {
            self.device
                .apply_scene_delta(&mut self.scene, &delta)
                .map_err(|error| format!("Vulkan scene update failed: {error}"))?;
            self.render_pending = true;
        }
        self.scene_epoch = self.scene.epoch();
        if self.scene_epoch < batch.snapshot.scene_epoch {
            return Err(format!(
                "presentation snapshot requires scene epoch {}, worker has {}",
                batch.snapshot.scene_epoch, self.scene_epoch
            ));
        }
        self.render_pending |= batch.request_present;
        if !self.render_pending {
            return Ok(WorkerProgress::Idle);
        }
        let drawable = self
            .logical_extent
            .is_some_and(|extent| extent.width > 0 && extent.height > 0);
        if !drawable {
            return Ok(WorkerProgress::Idle);
        }
        if self.acquire_guard.suppresses_preview() {
            #[cfg(feature = "profiler")]
            crate::runtime::instrumentation::instant!("presentation.resize.preview_suppressed");
            return Ok(WorkerProgress::Idle);
        }
        self.render_once(batch.frame_interval)
    }

    fn begin_ready_surface_commit(&mut self) {
        let ready = self
            .staged_surface_commit
            .is_some_and(PendingSurfaceCommit::is_ready);
        if !ready {
            return;
        }
        let commit = self
            .staged_surface_commit
            .take()
            .expect("surface commit readiness was checked above");
        #[cfg(feature = "profiler")]
        crate::runtime::instrumentation::instant!("presentation.resize.commit");
        self.presenter
            .set_reconfigure_policy(PresenterReconfigurePolicy::Eager);
        let surface_changed = self.swapchain_extent != Some(commit.update.extent);
        if self.presenter.resize(commit.update.extent) || surface_changed {
            self.presented = false;
            self.render_pending = true;
        }
    }

    fn force_staged_surface_commit(&mut self) {
        let Some(commit) = self.staged_surface_commit.take() else {
            return;
        };
        self.presenter
            .set_reconfigure_policy(PresenterReconfigurePolicy::Eager);
        self.presenter.resize(commit.update.extent);
        self.presented = false;
        self.render_pending = true;
    }

    fn poll_resize_presentation(&mut self) -> Result<bool, String> {
        let Some(pending) = self.pending_resize_presentation else {
            return Ok(true);
        };
        let complete = match pending.proof {
            ResizePresentationProof::PresentFence(completion) => self
                .presenter
                .poll_present_completion(completion)
                .map_err(|error| format!("failed to confirm resize presentation: {error}"))?,
            ResizePresentationProof::StabilizeAfter(ready_at) => Instant::now() >= ready_at,
        };
        if !complete {
            return Ok(false);
        }
        self.pending_resize_presentation = None;
        self.confirmed_resize_revision =
            self.confirmed_resize_revision.max(pending.metrics_revision);
        self.presented_resize_revision = Some(pending.metrics_revision);
        #[cfg(feature = "profiler")]
        crate::runtime::instrumentation::instant!("presentation.resize.present_complete");
        Ok(true)
    }

    fn render_once(&mut self, frame_interval: Duration) -> Result<WorkerProgress, String> {
        #[cfg(feature = "profiler")]
        let _presentation = crate::runtime::instrumentation::start_presentation("presentation.vulkan");
        if !self.poll_resize_presentation()? {
            return Ok(WorkerProgress::RetryAfter(Duration::from_millis(1)));
        }
        self.begin_ready_surface_commit();
        let recovery = self.presenter.recovery();
        if recovery.state() == PresenterState::NeedsReconfigure
            && let Some(extent) = self.logical_extent
            && recovery.requested_extent() != extent
        {
            self.presenter.resize(extent);
            self.presented = false;
        }
        self.presenter
            .enforce_retirement_limit(&self.device, MAX_RETIRED_SWAPCHAINS)
            .map_err(|error| format!("failed to bound retired Vulkan swapchains: {error}"))?;
        for _ in 0..2 {
            let Some(mut frame) = self
                .device
                .try_begin_owned_frame()
                .map_err(|error| format!("failed to begin Vulkan frame: {error}"))?
            else {
                #[cfg(feature = "profiler")]
                crate::runtime::instrumentation::instant!("presentation.retry.frame_slot");
                return Ok(WorkerProgress::RetryAfter(frame_interval));
            };
            let attempt = {
                let acquire_started_at = Instant::now();
                let acquire = {
                    #[cfg(feature = "profiler")]
                    let _span = crate::runtime::instrumentation::span!("swapchain.acquire");
                    self.presenter
                        .acquire(&self.device, &frame)
                        .map_err(|error| format!("failed to acquire Vulkan image: {error}"))?
                };
                let acquire_duration = acquire_started_at.elapsed();
                #[cfg(feature = "profiler")]
                crate::runtime::instrumentation::counter!(
                    "presentation.acquire.duration_ns",
                    acquire_duration.as_nanos()
                );
                match acquire {
                    AcquireOutcome::Ready(acquired) => {
                        let target = acquired.target();
                        let target_extent = target.info().extent;
                        let clear = self.scene.background();
                        let mut context = frame.context_mut();
                        let render_stats = {
                            #[cfg(feature = "profiler")]
                            let _span = crate::runtime::instrumentation::span!("command.record");
                            match self.device.render(
                                &mut self.scene,
                                &mut context,
                                &target,
                                &RenderRequest {
                                    force: !self.presented,
                                    load: TargetLoad::Clear(clear),
                                    store: TargetStore::Store,
                                    region: None,
                                },
                            ) {
                                Ok(stats) => stats,
                                Err(error) => {
                                    let _ = acquired.discard(&self.device);
                                    return Err(format!("Vulkan render failed: {error}"));
                                }
                            }
                        };
                        #[cfg(feature = "profiler")]
                        {
                            let memory = self.device.memory_metrics();
                            crate::runtime::instrumentation::counter!(
                                "gpu.memory.device_reserved_bytes",
                                memory.device_local_reserved_bytes
                            );
                            if let Some(budget) = memory.device_local_budget_bytes {
                                crate::runtime::instrumentation::counter!("gpu.memory.device_budget_bytes", budget);
                            }
                        }
                        let _ = render_stats;
                        let recorded = {
                            #[cfg(feature = "profiler")]
                            let _span = crate::runtime::instrumentation::span!("command.finish");
                            frame.finish().map_err(|error| {
                                format!("failed to finish Vulkan frame: {error}")
                            })?
                        };
                        let outcome = {
                            #[cfg(feature = "profiler")]
                            let _span = crate::runtime::instrumentation::span!("queue.submit_present");
                            acquired
                                .submit_and_present(&self.device, recorded)
                                .map_err(|error| {
                                    format!("failed to present Vulkan frame: {error}")
                                })?
                        };
                        FrameAttempt::Presented {
                            outcome,
                            target_extent,
                            acquire_duration,
                        }
                    }
                    AcquireOutcome::Suspended => {
                        drop(frame);
                        FrameAttempt::Suspended { acquire_duration }
                    }
                    AcquireOutcome::NotReady => {
                        drop(frame);
                        FrameAttempt::NotReady { acquire_duration }
                    }
                    AcquireOutcome::NeedsReconfigure => {
                        drop(frame);
                        FrameAttempt::NeedsReconfigure { acquire_duration }
                    }
                }
            };
            if self
                .acquire_guard
                .observe_acquire(attempt.acquire_duration(), frame_interval)
            {
                #[cfg(feature = "profiler")]
                {
                    crate::runtime::instrumentation::instant!("presentation.acquire.zero_timeout_breach");
                    crate::runtime::instrumentation::counter!(
                        "presentation.acquire.zero_timeout_breach_count",
                        self.acquire_guard.breach_count
                    );
                }
            }
            let needs_reconfigure = match attempt {
                FrameAttempt::Presented {
                    outcome,
                    target_extent,
                    ..
                } => {
                    #[cfg(feature = "profiler")]
                    match outcome.disposition {
                        PresentDisposition::Presented => {
                            crate::runtime::instrumentation::instant!("presentation.presented");
                        }
                        PresentDisposition::PresentedSuboptimal => {
                            crate::runtime::instrumentation::instant!("presentation.presented_suboptimal");
                        }
                        PresentDisposition::NeedsReconfigure => {
                            crate::runtime::instrumentation::instant!("presentation.needs_reconfigure");
                        }
                        PresentDisposition::SurfaceLost => {
                            crate::runtime::instrumentation::instant!("presentation.surface_lost");
                        }
                    }
                    let was_presented = matches!(
                        outcome.disposition,
                        PresentDisposition::Presented | PresentDisposition::PresentedSuboptimal
                    );
                    self.presented = was_presented;
                    if was_presented {
                        self.swapchain_extent = Some(target_extent);
                        if self.metrics_revision > self.confirmed_resize_revision
                            && let Some(metrics_revision) = completed_resize_revision(
                                self.logical_extent,
                                target_extent,
                                self.staged_surface_commit.is_some(),
                                self.metrics_revision,
                            )
                        {
                            let proof = outcome.presentation_completion.map_or_else(
                                || {
                                    ResizePresentationProof::StabilizeAfter(
                                        Instant::now() + frame_interval,
                                    )
                                },
                                ResizePresentationProof::PresentFence,
                            );
                            self.pending_resize_presentation = Some(PendingResizePresentation {
                                metrics_revision,
                                proof,
                            });
                        }
                        if let Some(commit) = self.staged_surface_commit.as_mut()
                            && !commit.is_ready()
                        {
                            commit.mark_preview_presented();
                            #[cfg(feature = "profiler")]
                            crate::runtime::instrumentation::instant!("presentation.resize.preview_presented");
                            self.render_pending = true;
                            return Ok(WorkerProgress::RetryAfter(frame_interval));
                        }
                    } else {
                        // OUT_OF_DATE/SURFACE_LOST means the old generation cannot carry the final
                        // preview. Promote the staged extent before normal presenter recovery.
                        self.force_staged_surface_commit();
                    }
                    self.render_pending = match outcome.disposition {
                        PresentDisposition::Presented => outcome.maintenance_pending,
                        PresentDisposition::PresentedSuboptimal => {
                            outcome.reconfigure_pending || outcome.maintenance_pending
                        }
                        PresentDisposition::NeedsReconfigure => true,
                        PresentDisposition::SurfaceLost => true,
                    };
                    if self.pending_resize_presentation.is_some() {
                        return Ok(WorkerProgress::RetryAfter(Duration::from_millis(1)));
                    }
                    // Acquire is nonblocking. A few frame-paced maintenance presents allow the
                    // successor generation to prove the retired swapchain is no longer in use,
                    // avoiding a later queue-idle stall when the retirement budget fills.
                    return if self.render_pending {
                        Ok(WorkerProgress::RetryAfter(frame_interval))
                    } else {
                        Ok(WorkerProgress::Idle)
                    };
                }
                FrameAttempt::Suspended { .. } => {
                    self.staged_surface_commit = None;
                    self.render_pending = false;
                    return Ok(WorkerProgress::Idle);
                }
                FrameAttempt::NotReady { .. } => {
                    #[cfg(feature = "profiler")]
                    crate::runtime::instrumentation::instant!("presentation.retry.acquire_not_ready");
                    return Ok(WorkerProgress::RetryAfter(frame_interval));
                }
                FrameAttempt::NeedsReconfigure { .. } => {
                    #[cfg(feature = "profiler")]
                    crate::runtime::instrumentation::instant!("presentation.retry.reconfigure");
                    self.force_staged_surface_commit();
                    true
                }
            };
            if needs_reconfigure {
                self.recover_presenter()?;
            }
        }
        #[cfg(feature = "profiler")]
        crate::runtime::instrumentation::instant!("presentation.retry.reconfigure");
        Ok(WorkerProgress::RetryAfter(frame_interval))
    }

    fn recover_presenter(&mut self) -> Result<(), String> {
        let state = self.presenter.recovery().state();
        let extent = self
            .logical_extent
            .unwrap_or_else(|| Self::window_extent(&self.window));
        if state == PresenterState::SurfaceLost {
            let surface = VulkanWinitSurface::create(&self.instance, &*self.window, &*self.window)
                .map_err(|error| format!("failed to recreate Vulkan surface: {error}"))?;
            self.presenter
                .replace_surface(&self.device, surface, extent)
                .map_err(|error| format!("failed to replace Vulkan surface: {error}"))?;
        } else {
            self.presenter
                .resume(&self.device, extent)
                .map_err(|error| format!("failed to reconfigure Vulkan presenter: {error}"))?;
        }
        self.swapchain_extent = Some(extent);
        self.presented = false;
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), String> {
        self.presenter
            .shutdown(&self.device)
            .map_err(|error| format!("failed to shut down Vulkan presenter: {error}"))
    }
}

fn should_preview_before_surface_commit(
    resize: ResizeUpdate,
    surface_extent: Option<SizeI>,
    presenter_state: PresenterState,
) -> bool {
    resize.surface == SurfaceResizeAction::CommitAfterPreview
        && resize.phase == ResizeInteractionPhase::Ended
        && surface_extent.is_some()
        && surface_extent != Some(resize.extent)
        && matches!(presenter_state, PresenterState::Ready)
}

fn completed_resize_revision(
    logical_extent: Option<SizeI>,
    target_extent: SizeI,
    staged_surface_commit: bool,
    metrics_revision: u64,
) -> Option<u64> {
    (logical_extent == Some(target_extent) && !staged_surface_commit).then_some(metrics_revision)
}

#[cfg(feature = "profiler")]
const fn resize_phase_code(phase: ResizeInteractionPhase) -> u8 {
    match phase {
        ResizeInteractionPhase::Stable => 0,
        ResizeInteractionPhase::Started => 1,
        ResizeInteractionPhase::Updating => 2,
        ResizeInteractionPhase::Ended => 3,
        ResizeInteractionPhase::Cancelled => 4,
    }
}

#[cfg(test)]
mod tests;
