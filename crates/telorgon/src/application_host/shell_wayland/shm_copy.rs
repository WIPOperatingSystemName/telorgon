use super::*;
use std::sync::mpsc;
use std::thread;

const SHM_COPY_QUEUE_CAPACITY: usize = 4;

pub(super) struct ShmCopyRequest {
    pub(super) snapshot: crate::compositor_wayland::SurfaceStateSnapshot,
    viewport: Option<crate::compositor_wayland::ViewportState>,
    reader: crate::compositor_wayland::ShmBufferReader,
    output_scale: crate::platform::ScaleFactor,
}

impl ShmCopyRequest {
    pub(super) fn new(
        snapshot: crate::compositor_wayland::SurfaceStateSnapshot,
        viewport: Option<crate::compositor_wayland::ViewportState>,
        reader: crate::compositor_wayland::ShmBufferReader,
        output_scale: crate::platform::ScaleFactor,
        coordinate_density: i32,
    ) -> Self {
        Self {
            snapshot,
            viewport,
            reader,
            output_scale: super::geometry::surface_raster_scale(output_scale, coordinate_density),
        }
    }

    pub(super) fn buffer(&self) -> crate::compositor_wayland::WaylandBufferId {
        self.snapshot
            .attachment
            .expect("SHM copy requests originate from attached surfaces")
            .buffer
    }

    fn execute(self) -> ShmCopyCompletion {
        #[cfg(feature = "profiler")]
        let _span = crate::profiler::span!("compositor.shm.copy.full.worker");
        let buffer = self.buffer();
        let revision = self.snapshot.revision;
        let result = self
            .reader
            .read_full()
            .map_err(|error| error.to_string())
            .and_then(|shm| {
                #[cfg(feature = "profiler")]
                crate::profiler::record_instant_value(
                    "compositor.shm.copy_bytes",
                    shm.pixels.len() as u64,
                );
                shm_image_resource(buffer, revision, shm).map_err(|error| error.to_string())
            })
            .and_then(|image| {
                crate::compositor_render::transform_surface_image_at_scale(
                    image,
                    self.snapshot.buffer_scale,
                    self.snapshot.buffer_transform,
                    self.viewport,
                    self.output_scale,
                )
                .map_err(|error| error.to_string())
            })
            // Preparing both owners here keeps all whole-buffer copying off the compositor/input
            // owner. Later commits patch the retained client copy and scene snapshot independently.
            .map(|(image, logical)| PreparedClientImage::full_scaled(image, logical));
        ShmCopyCompletion {
            snapshot: self.snapshot,
            buffer,
            result,
        }
    }
}

pub(super) struct ShmCopyCompletion {
    pub(super) snapshot: crate::compositor_wayland::SurfaceStateSnapshot,
    pub(super) buffer: crate::compositor_wayland::WaylandBufferId,
    pub(super) result: Result<PreparedClientImage, String>,
}

pub(super) struct ShmCopyWorker {
    requests: Option<mpsc::SyncSender<ShmCopyRequest>>,
    completions: mpsc::Receiver<ShmCopyCompletion>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ShmCopyWorker {
    pub(super) fn new(wake: EventNotifier) -> AppResult<Self> {
        let (request_tx, request_rx) =
            mpsc::sync_channel::<ShmCopyRequest>(SHM_COPY_QUEUE_CAPACITY);
        let (completion_tx, completion_rx) = mpsc::channel::<ShmCopyCompletion>();
        let thread = thread::Builder::new()
            .name("telorgon-shm-copy".to_owned())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    if completion_tx.send(request.execute()).is_err() {
                        break;
                    }
                    wake.notify();
                }
            })
            .map_err(|error| AppError::new(format!("failed to start SHM copy worker: {error}")))?;
        Ok(Self {
            requests: Some(request_tx),
            completions: completion_rx,
            thread: Some(thread),
        })
    }

    /// Queues ordinary work without blocking the compositor owner. A full mailbox returns the
    /// request so the owner can preserve FIFO ordering in its bounded deferred queue.
    pub(super) fn try_submit(&self, request: ShmCopyRequest) -> AppResult<Option<ShmCopyRequest>> {
        match self
            .requests
            .as_ref()
            .ok_or_else(|| AppError::new("SHM copy worker is stopped"))?
            .try_send(request)
        {
            Ok(()) => Ok(None),
            Err(mpsc::TrySendError::Full(request)) => Ok(Some(request)),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                Err(AppError::new("SHM copy worker stopped unexpectedly"))
            }
        }
    }

    pub(super) fn drain(&self) -> impl Iterator<Item = ShmCopyCompletion> + '_ {
        self.completions.try_iter()
    }
}

impl Drop for ShmCopyWorker {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
