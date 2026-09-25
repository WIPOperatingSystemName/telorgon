//! Owner-thread assembly between authorization, GPU capture and PipeWire delivery.

#[path = "capture_metadata.rs"]
mod metadata;

use super::capture::{CaptureSessions, RequesterId, SessionId, SessionState};
use super::renderer::{CaptureBuffer, CaptureJob, CaptureView, ShellRenderer};
use crate::host::application::{AppError, AppResult};
use crate::integrations::pipewire::{StreamStatus, VideoStream};
use crate::shell::capture::{CaptureLayout, CaptureOptions, CaptureSource, CaptureStopReason};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct ActiveStream {
    requester: RequesterId,
    period: Duration,
    video: VideoStream,
    buffer: Option<CaptureBuffer>,
    spares: Vec<crate::media::video::CapturePixels>,
    created: Instant,
    in_flight: bool,
    resizing: Option<u64>,
    node_id: Option<u32>,
    window: Option<super::capture_window::WindowCapture>,
    window_revision: u64,
    observed_revision: Option<(u64, u64, u64)>,
    revision: u64,
    last_damage_version: Option<u64>,
    pending_damage_version: Option<u64>,
    delivered_first: bool,
    reported_wait: bool,
    consumer_received: bool,
}

#[derive(Clone)]
struct VirtualCapture {
    label: String,
    frame_rate: std::num::NonZeroU32,
    revision: u64,
    scene: Option<super::capture_scene::CaptureScene>,
}

struct WindowSource {
    epoch: u64,
    label: String,
    // Retain only the negotiated extent across readiness gaps, never a stale capture scene.
    layout: Option<CaptureLayout>,
    capture: Option<super::capture_window::WindowCapture>,
}

#[derive(Default)]
pub(super) struct CaptureStreams {
    cursor: metadata::CursorState,
    streams: BTreeMap<SessionId, ActiveStream>,
    last_scheduled: Option<SessionId>,
    virtual_sources: BTreeMap<crate::shell::OutputId, VirtualCapture>,
    window_sources:
        BTreeMap<crate::shell::WindowId, WindowSource>,
}

impl CaptureStreams {
    /// Called only after the host chooser approves this exact requester/source.
    pub fn begin_approved(
        &mut self,
        sessions: &mut CaptureSessions,
        renderer: &ShellRenderer,
        requester: RequesterId,
        source: CaptureSource,
        options: CaptureOptions,
        layout: CaptureLayout,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> AppResult<SessionId> {
        let options = match source {
            CaptureSource::VirtualOutput(id) => {
                let output = self
                    .virtual_sources
                    .get(&id)
                    .filter(|output| {
                        output
                            .scene
                            .as_ref()
                            .is_some_and(|scene| scene.layout == layout)
                    })
                    .ok_or_else(|| {
                        AppError::new("virtual display capture source is no longer ready")
                    })?;
                CaptureOptions::new(
                    options.cursor(),
                    options.max_frame_rate().min(output.frame_rate),
                )
            }
            _ => options,
        };
        let window = match source {
            CaptureSource::Output(_) | CaptureSource::VirtualOutput(_) => None,
            CaptureSource::Window(id) => self.window_sources.get(&id)
                .filter(|window| sessions.source_epoch(source) == Some(window.epoch)
                    && window.layout == Some(layout))
                .ok_or_else(|| AppError::new("window capture source is no longer available"))?
                .capture.clone(),
        };
        let id = sessions
            .request(requester, source, options)
            .map_err(capture_error)?;
        let created = Instant::now();
        let prepared = (|| {
            sessions.authorize(id, true).map_err(capture_error)?;
            sessions
                .negotiate(id, requester, layout)
                .map_err(capture_error)?;
            let force_shm = std::env::var("TELORGON_CAPTURE_FORCE_SHM").as_deref() == Ok("1");
            let producer = if force_shm {
                eprintln!("telorgon-capture: stream={} forcing shared-memory transport (TELORGON_CAPTURE_FORCE_SHM=1)", id.get());
                None
            } else {
                renderer.screen_gpu_producer(layout, options.max_frame_rate().get())
            };
            let video = VideoStream::start_budgeted(
                id.get(),
                layout,
                options.max_frame_rate().get(),
                wake,
                producer,
                renderer.capture_memory_budget(),
            )
            .map_err(AppError::new)?;
            Ok(ActiveStream {
                requester,
                period: Duration::from_secs_f64(1.0 / options.max_frame_rate().get() as f64),
                video,
                buffer: None,
                spares: Vec::new(),
                created,
                in_flight: false,
                resizing: None,
                node_id: None,
                window,
                window_revision: 0,
                observed_revision: None,
                revision: 0,
                last_damage_version: None,
                pending_damage_version: None,
                delivered_first: false,
                reported_wait: false,
                consumer_received: false,
            })
        })();
        match prepared {
            Ok(stream) => {
                eprintln!("telorgon-capture: stream={} approved, producer created in {}ms", id.get(), created.elapsed().as_millis());
                self.streams.insert(id, stream);
                Ok(id)
            }
            Err(error) => {
                let _ = sessions.stop(id, requester, CaptureStopReason::StreamFailed);
                let _ = sessions.retired(id);
                Err(error)
            }
        }
    }

    pub fn refresh_virtual_outputs(&mut self, outputs: &super::virtual_outputs::VirtualOutputs) {
        self.virtual_sources = outputs
            .iter()
            .map(|(id, output)| {
                (
                    id,
                    VirtualCapture {
                        label: output.label.clone(),
                        frame_rate: output.frame_rate,
                        revision: output.revision,
                        scene: output.scene.clone(),
                    },
                )
            })
            .collect();
    }

    pub fn virtual_sources(&self) -> impl Iterator<Item = (crate::shell::OutputId, &str)> {
        self.virtual_sources
            .iter()
            .filter(|(_, output)| output.scene.is_some())
            .map(|(&id, output)| (id, output.label.as_str()))
    }

    pub fn virtual_layout(&self, id: crate::shell::OutputId) -> Option<CaptureLayout> {
        self.virtual_sources
            .get(&id)?
            .scene
            .as_ref()
            .map(|scene| scene.layout)
    }

    pub fn refresh_windows(
        &mut self,
        sessions: &CaptureSessions,
        windows: &BTreeMap<super::WaylandSurfaceId, super::ClientWindow>,
        layers: &[super::scene::ShellLayer],
        scale: crate::platform::contracts::ScaleFactor,
        discover: bool,
        native_label: impl Fn(super::WaylandSurfaceId) -> Option<String>,
    ) -> bool {
        let previous = std::mem::take(&mut self.window_sources);
        if discover {
            // Mapping lifetime owns discovery; an unsignaled GPU buffer only pauses frames.
            // Never remove/re-add a live source because its next image is not ready yet.
            for (&surface, window) in windows.iter().filter(|(_, w)|
                w.backend.is_some() && !w.minimized && w.presentation.revision != 0)
            {
                let Some(id) = window.desktop_id else { continue; };
                let Some(epoch) = sessions.source_epoch(CaptureSource::Window(id)) else { continue; };
                let title = window.frame_title.as_deref()
                    .filter(|s| !s.trim().is_empty()).unwrap_or(&window.application_identity);
                let label = crate::authoring::compose::portal::application_label(
                    native_label(surface).as_deref().filter(|s| !s.trim().is_empty()).unwrap_or(title),
                );
                let capture = super::capture_window::prepare(id, windows, layers, scale);
                let layout = capture.as_ref().map(|capture| capture.layout).or_else(||
                    previous.get(&id).filter(|old| old.epoch == epoch).and_then(|old| old.layout));
                self.window_sources.insert(id, WindowSource { epoch, label, layout, capture });
            }
        }

        for (&id, stream) in &mut self.streams {
            let next = sessions.get(id).and_then(|session| match session.source {
                CaptureSource::Window(window) => {
                    super::capture_window::prepare(window, windows, layers, scale)
                }
                CaptureSource::Output(_) | CaptureSource::VirtualOutput(_) => None,
            });
            if stream.window != next {
                stream.window_revision = stream.window_revision.wrapping_add(1).max(1);
                stream.window = next;
            }
        }
        !previous
            .iter()
            .map(|(id, window)| (id, window.epoch, &window.label))
            .eq(self
                .window_sources
                .iter()
                .map(|(id, window)| (id, window.epoch, &window.label)))
    }

    pub fn window_sources(&self) -> impl Iterator<Item = (crate::shell::WindowId, u64, &str)> {
        self.window_sources
            .iter()
            .map(|(&id, window)| (id, window.epoch, window.label.as_str()))
    }

    pub fn preview_scene(&self, source: CaptureSource, epoch: u64) -> Option<super::capture_scene::CaptureScene> {
        match source {
            CaptureSource::Window(id) => self.window_sources.get(&id)
                .filter(|window| window.epoch == epoch)
                .and_then(|window| window.capture.clone().map(|capture| capture.into_scene())),
            CaptureSource::VirtualOutput(id) => self.virtual_sources.get(&id)?.scene.clone(),
            CaptureSource::Output(_) => None,
        }
    }

    pub fn window_layout(&self, id: crate::shell::WindowId, epoch: u64) -> Option<CaptureLayout> {
        self.window_sources
            .get(&id)
            .filter(|window| window.epoch == epoch)
            .and_then(|window| window.layout)
    }

    pub fn node_id(&self, id: SessionId) -> Option<u32> {
        let stream = self.streams.get(&id)?;
        match stream.video.status() {
            StreamStatus::Ready { node_id } => Some(node_id),
            // Format negotiation needs a connected consumer. Preserve the published node while
            // resizing, including when the portal has not returned its initial Start reply yet.
            StreamStatus::Connecting => stream.node_id,
            _ => None,
        }
    }

    pub fn needs_embedded_cursor(&self, sessions: &CaptureSessions) -> bool {
        self.streams.keys().any(|id| {
            sessions.get(*id).is_some_and(|s| {
                s.state == SessionState::Streaming
                    && !matches!(s.source, CaptureSource::VirtualOutput(_))
                    && s.options.cursor() == crate::shell::capture::CaptureCursorMode::Embedded
            })
        })
    }

    pub fn fail_embedded_cursor(&self, sessions: &mut CaptureSessions) {
        for (&id, stream) in &self.streams {
            if sessions.get(id).is_some_and(|s| {
                !matches!(s.source, CaptureSource::VirtualOutput(_))
                    && s.options.cursor() == crate::shell::capture::CaptureCursorMode::Embedded
            }) {
                let _ = sessions.stop(id, stream.requester, CaptureStopReason::StreamFailed);
                stream.video.stop();
            }
        }
    }

    pub fn poll(
        &mut self,
        sessions: &mut CaptureSessions,
        renderer: &ShellRenderer,
        output_layout: Option<CaptureLayout>,
    ) {
        let mut retired = Vec::new();
        for (&id, stream) in &mut self.streams {
            let state = sessions.get(id).map(|s| s.state);
            if matches!(state, Some(SessionState::Stopped(_)) | None) {
                stream.video.stop();
                // In-flight jobs own their own buffer. Idle storage need not wait for
                // native teardown; submitted frame clones retain their own charges.
                stream.buffer = None;
                stream.spares.clear();
                stream.pending_damage_version = None;
                if !stream.in_flight && stream.video.retired() {
                    retired.push(id);
                }
                continue;
            }
            let status = stream.video.status();
            if !stream.consumer_received {
                let diagnostics = stream.video.delivery_diagnostics();
                if diagnostics.as_ref().is_some_and(|(_, stats)| stats.frames > 0) {
                    eprintln!("telorgon-capture: stream={} first PipeWire delivery after {}ms", id.get(), stream.created.elapsed().as_millis());
                    stream.consumer_received = true;
                } else if !stream.reported_wait && stream.created.elapsed() > Duration::from_secs(3) {
                    eprintln!("telorgon-capture: stream={} no PipeWire frames after 3s: {:?}, transport={:?}, negotiation={:?}, captured={} source_ready={} in_flight={}",
                        id.get(), status, diagnostics, stream.video.negotiation(), stream.delivered_first,
                        stream.window.is_some() || !matches!(sessions.get(id).unwrap().source, CaptureSource::Window(_)), stream.in_flight);
                    stream.reported_wait = true;
                }
            }
            if let StreamStatus::Ready { node_id } = &status {
                if stream
                    .node_id
                    .is_some_and(|published| published != *node_id)
                {
                    // A portal consumer was given the original node; never silently switch it.
                    let _ = sessions.stop(id, stream.requester, CaptureStopReason::StreamFailed);
                    stream.video.stop();
                    continue;
                }
                if stream.node_id.is_none() {
                    eprintln!("telorgon-capture: stream={} PipeWire node={} ready after {}ms", id.get(), node_id, stream.created.elapsed().as_millis());
                }
                stream.node_id = Some(*node_id);
            }
            match status {
                StreamStatus::Failed(_) | StreamStatus::Stopped => {
                    eprintln!("telorgon-capture: stream={} transport stopped: {:?}", id.get(), status);
                    let _ = sessions.stop(id, stream.requester, CaptureStopReason::StreamFailed);
                    continue;
                }
                StreamStatus::Connecting if stream.created.elapsed() > Duration::from_secs(10) => {
                    let _ = sessions.stop(id, stream.requester, CaptureStopReason::StreamFailed);
                    continue;
                }
                _ => {}
            }
            if matches!(status, StreamStatus::Ready { .. })
                && state == Some(SessionState::Negotiating)
            {
                if let Some(generation) = stream.resizing {
                    if stream.in_flight {
                        continue;
                    }
                    // Ready for a resized VideoStream means the new format was accepted and every
                    // old PipeWire buffer was removed. The completion path retired the old GPU job.
                    stream.buffer = None;
                    stream.spares.clear();
                    let prepared = (|| {
                        sessions
                            .generation_retired(id, stream.requester, generation)
                            .map_err(capture_error)?;
                        let layout = sessions
                            .get(id)
                            .and_then(|s| s.layout)
                            .ok_or_else(|| AppError::new("missing resize layout"))?;
                        stream.buffer = Some(renderer.allocate_capture(layout)?);
                        stream.spares = allocate_spares(renderer, layout)?;
                        sessions
                            .started(id, stream.requester)
                            .map_err(capture_error)
                    })();
                    stream.resizing = None;
                    if prepared.is_err() {
                        let _ =
                            sessions.stop(id, stream.requester, CaptureStopReason::StreamFailed);
                        stream.video.stop();
                        continue;
                    }
                } else {
                    let _ = sessions.started(id, stream.requester);
                }
            }
            let Some(session) = sessions.get(id) else {
                continue;
            };
            if session.state != SessionState::Streaming {
                continue;
            }
            let desired = match session.source {
                CaptureSource::Output(_) => output_layout,
                CaptureSource::VirtualOutput(output_id) => {
                    let Some(output) = self.virtual_sources.get(&output_id) else {
                        let _ = sessions.stop(
                            id,
                            stream.requester,
                            CaptureStopReason::SourceUnavailable,
                        );
                        stream.video.stop();
                        continue;
                    };
                    let Some(scene) = &output.scene else {
                        continue;
                    };
                    Some(scene.layout)
                }
                CaptureSource::Window(id) => {
                    // Content readiness can temporarily disappear during resize; the approved
                    // source remains the same and no stale snapshot is scheduled in that interval.
                    let Some(window) = stream.window.as_ref().filter(|w| w.window == id) else {
                        continue;
                    };
                    Some(window.layout)
                }
            };
            let Some(layout) = desired else {
                let _ = sessions.stop(id, stream.requester, CaptureStopReason::SourceUnavailable);
                stream.video.stop();
                continue;
            };
            if session.layout != Some(layout) {
                let fps = session.options.max_frame_rate().get();
                let resized = sessions
                    .renegotiate(id, stream.requester, layout)
                    .map_err(capture_error)
                    .and_then(|generation| {
                        stream.video.resize(layout, fps).map_err(AppError::new)?;
                        Ok(generation)
                    });
                match resized {
                    Ok(generation) => {
                        stream.resizing = Some(generation);
                        stream.last_damage_version = None;
                        stream.created = Instant::now();
                        stream.buffer = None;
                        stream.spares.clear();
                    }
                    Err(error) => {
                        eprintln!("telorgon-capture: stream={} allocation failed: {}", id.get(), error);
                        let _ =
                            sessions.stop(id, stream.requester, CaptureStopReason::StreamFailed);
                        stream.video.stop();
                    }
                }
            }
        }
        for id in retired {
            self.streams.remove(&id);
            let _ = sessions.retired(id);
        }
    }

    pub fn schedule(
        &mut self,
        sessions: &mut CaptureSessions,
        renderer: &mut ShellRenderer,
        now_ns: u64,
    ) -> AppResult<()> {
        // Keep a GPU frame slot available to the desktop; rotate eligible streams fairly.
        if sessions.has_pending_frame() {
            return Ok(());
        }
        let mut ids = self.streams.keys().copied().collect::<Vec<_>>();
        if let Some(last) = self.last_scheduled {
            let split = ids.partition_point(|id| *id <= last);
            ids.rotate_left(split);
        }
        for id in ids {
            let stream = self.streams.get_mut(&id).expect("retained stream");
            if stream.in_flight
                || sessions.get(id).map(|s| s.state) != Some(SessionState::Streaming)
            {
                continue;
            }
            // Publish the node before allocating/capturing. The consumer cannot negotiate
            // until the portal returns that node; capture only the negotiated transport.
            let negotiated = stream.video.negotiation();
            let Some(negotiated) = negotiated else { continue; };
            let gpu = matches!(negotiated.transport, crate::media::video::VideoTransport::DmaBuf(_));
            if stream.buffer.as_ref().is_none_or(|buffer|
                buffer.slot.is_gpu() != gpu || buffer.slot.video_format() != Some(negotiated.format))
            {
                let Some(layout) = sessions.get(id).and_then(|session| session.layout) else { continue; };
                eprintln!("telorgon-capture: stream={} negotiated {:?}", id.get(), negotiated);
                // No job is in flight here. Release old storage before replacement and
                // force a fresh approved capture even when the content revision is unchanged.
                stream.buffer = None;
                stream.spares.clear();
                let replacement = renderer.allocate_video_capture(
                    layout,
                    negotiated.format,
                    gpu,
                );
                match replacement {
                    Ok(buffer) => stream.buffer = Some(buffer),
                    Err(error) => {
                        eprintln!("telorgon-capture: stream={} negotiated allocation failed: {}", id.get(), error);
                        let _ = sessions.stop(id, stream.requester, CaptureStopReason::StreamFailed);
                        stream.video.stop();
                        continue;
                    }
                }
                if !gpu {
                    match allocate_spares(renderer, layout) {
                        Ok(spares) => stream.spares = spares,
                        Err(_) => {
                            let _ = sessions.stop(
                                id,
                                stream.requester,
                                CaptureStopReason::StreamFailed,
                            );
                            stream.video.stop();
                            continue;
                        }
                    }
                }
                stream.observed_revision = None;
                stream.last_damage_version = None;
            }
            let Some(buffer) = &mut stream.buffer else {
                continue;
            };
            if !buffer.slot.is_gpu() && buffer.pixels.is_empty() {
                let Some(pixels) = stream
                    .spares
                    .pop()
                    .or_else(|| stream.video.take_spare_owned())
                else {
                    continue;
                };
                buffer.pixels = pixels;
            }
            let session = sessions.get(id).expect("admitted session");
            let view = match session.source {
                CaptureSource::Output(_) => CaptureView::Output,
                CaptureSource::VirtualOutput(id) => {
                    let Some(scene) = self
                        .virtual_sources
                        .get(&id)
                        .and_then(|output| output.scene.as_ref())
                        .filter(|scene| scene.layout == buffer.layout)
                    else {
                        continue;
                    };
                    CaptureView::Scene(scene.clone())
                }
                CaptureSource::Window(id) => {
                    let Some(window) = stream
                        .window
                        .as_ref()
                        .filter(|w| w.window == id && w.layout == buffer.layout)
                    else {
                        continue;
                    };
                    CaptureView::Scene(window.clone().into_scene())
                }
            };
            let cursor = session.options.cursor();
            let cursor_revision = if cursor == crate::shell::capture::CaptureCursorMode::Metadata {
                self.cursor.revision()
            } else {
                0
            };
            let observed = match session.source {
                CaptureSource::VirtualOutput(id) => (self.virtual_sources[&id].revision, 0, 0),
                _ => (
                    stream.window_revision,
                    renderer.capture_revision(cursor),
                    cursor_revision,
                ),
            };
            if stream.observed_revision != Some(observed) {
                stream.revision = stream.revision.wrapping_add(1).max(1);
                stream.observed_revision = Some(observed);
            }
            let revision = stream.revision;
            let Ok(Some(ticket)) = sessions.begin_frame(id, now_ns, revision) else {
                continue;
            };
            let cursor_metadata = if cursor == crate::shell::capture::CaptureCursorMode::Metadata {
                let origin = match &view {
                    CaptureView::Output => Some(crate::foundation::PointI::default()),
                    CaptureView::Scene(scene) => scene.desktop_cursor_origin,
                };
                Some(origin.map_or_else(
                    || crate::media::video::VideoCursor {
                        id: 1,
                        x: 0,
                        y: 0,
                        hotspot_x: 0,
                        hotspot_y: 0,
                        bitmap: None,
                        visible: Some(false),
                    },
                    |origin| self.cursor.for_source(origin, buffer.layout),
                ))
            } else {
                None
            };
            let after = if stream.video.has_unsubmitted_frame() {
                None
            } else {
                stream.last_damage_version
            };
            let (damage_version, damage) = renderer.capture_damage(after, buffer.layout);
            stream.pending_damage_version = Some(damage_version);
            // Window content and embedded cursors have independent geometry histories.
            // Until those are available, full damage is the only conservative answer.
            let damage = if matches!(view, CaptureView::Output)
                && cursor != crate::shell::capture::CaptureCursorMode::Embedded
            {
                damage
            } else {
                Vec::new()
            };
            buffer
                .slot
                .set_metadata(crate::media::video::FrameMetadata {
                    damage,
                    cursor: cursor_metadata,
                    ..Default::default()
                });
            let job = CaptureJob {
                direct: None,
                view,
                ticket,
                cursor,
                buffer: stream.buffer.take().expect("ready buffer"),
            };
            match renderer.submit_capture_recoverable(job) {
                Ok(()) => {
                    stream.in_flight = true;
                    self.last_scheduled = Some(id);
                    break;
                }
                Err(failure) => {
                    let Some(job) = failure.rejected else {
                        return Err(failure.error);
                    };
                    drop(job);
                    stream.spares.clear();
                    stream.pending_damage_version = None;
                    let _ = sessions.finish_frame(ticket, false);
                    let _ = sessions.stop(id, stream.requester, CaptureStopReason::StreamFailed);
                    stream.video.stop();
                }
            }
        }
        Ok(())
    }

    pub fn complete(
        &mut self,
        sessions: &mut CaptureSessions,
        mut job: CaptureJob,
        success: bool,
    ) -> Vec<(u32, u64)> {
        if !success {
            let _ = sessions.finish_frame(job.ticket, false);
            if let Some(stream) = self.streams.get_mut(&job.ticket.session()) {
                let _ = sessions.stop(
                    job.ticket.session(),
                    stream.requester,
                    CaptureStopReason::StreamFailed,
                );
                stream.video.stop();
                stream.in_flight = false;
                // Pending receipts are retried by the completion worker. This is reached only
                // after completion/readback failure or terminal device loss; do not reuse the slot.
            }
            return Vec::new();
        }
        // A routing change can retire scene content while its GPU work is in flight.
        // Finish ownership accounting, but never publish the obsolete scene afterward.
        let current_scene = sessions.get(job.ticket.session()).is_some_and(|session| {
            match session.source {
                CaptureSource::VirtualOutput(id) => {
                    matches!(&job.view, CaptureView::Scene(scene)
                        if self.virtual_sources.get(&id).and_then(|output| output.scene.as_ref()) == Some(scene))
                }
                _ => true,
            }
        });
        let deliver = sessions
            .finish_frame(job.ticket, success && current_scene)
            .unwrap_or(false);
        let Some(stream) = self.streams.get_mut(&job.ticket.session()) else {
            return Vec::new();
        };
        stream.in_flight = false;
        let damage_version = stream.pending_damage_version.take();
        let mut sampled = Vec::new();
        if deliver {
            let delivered = if let Some(frame) = job.buffer.slot.take_gpu_frame() {
                stream.video.publish_gpu(frame).is_ok()
            } else {
                let pixels = std::mem::take(&mut job.buffer.pixels);
                let format = job.buffer.slot.video_format().unwrap_or_else(|| {
                    crate::media::video::VideoFormat::rgba(
                        job.buffer.layout.width(),
                        job.buffer.layout.height(),
                        sessions
                            .get(job.ticket.session())
                            .unwrap()
                            .options
                            .max_frame_rate()
                            .get(),
                    )
                });
                match stream
                    .video
                    .publish_owned(pixels, format, job.buffer.slot.metadata())
                {
                    Ok(reusable) => {
                        job.buffer.pixels =
                            reusable.or_else(|| stream.spares.pop()).unwrap_or_default();
                        true
                    }
                    Err(pixels) => {
                        job.buffer.pixels = pixels;
                        false
                    }
                }
            };
            if delivered {
                if !stream.delivered_first {
                    eprintln!("telorgon-capture: stream={} first frame submitted after {}ms", job.ticket.session().get(), stream.created.elapsed().as_millis());
                    stream.delivered_first = true;
                }
                stream.last_damage_version = damage_version;
                if let CaptureView::Scene(scene) = job.view {
                    sampled = scene.sampled;
                }
            } else {
                let _ = sessions.stop(
                    job.ticket.session(),
                    stream.requester,
                    CaptureStopReason::StreamFailed,
                );
            }
        } else {
            // Revoked/stale jobs may release finished GPU pixels, but never publish them.
            job.buffer.slot.take_gpu_frame();
        }
        if stream.resizing.is_none() {
            stream.buffer = Some(job.buffer);
        } // Otherwise the completed old-generation slot retires here, never entering the new pool.
        sampled
    }

    pub fn wait(&self, existing: Option<Duration>) -> Option<Duration> {
        let cadence = self
            .streams
            .values()
            .map(|stream| {
                if stream.video.is_stopping() {
                    // Native shutdown may await external GPU completion. Keep retirement
                    // observable without waking the compositor at the old capture FPS.
                    Duration::from_millis(100)
                } else {
                    stream.period
                }
            })
            .min();
        match (existing, cadence) {
            (Some(existing), Some(cadence)) => Some(existing.min(cadence)),
            (existing, None) => existing,
            (None, cadence) => cadence,
        }
    }
}

fn capture_error(error: super::capture::CaptureError) -> AppError {
    AppError::new(format!("capture admission: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::super::{
        client::maximize_preview_tests::test_window,
        scene::{ShellImageUpdate, ShellLayer, ShellLayerKey, ShellSceneKey},
    };
    use super::*;
    use crate::{
        core::{PointI, RectI, SizeI},
        render::{ImageAlphaMode, ImagePixelFormat},
    };
    #[test]
    fn discovery_binds_metadata_to_the_captured_mapping_epoch() {
        let id = crate::shell::WindowId::new(
            std::num::NonZeroU32::new(1).unwrap(),
            std::num::NonZeroU32::new(1).unwrap(),
        );
        let source = CaptureSource::Window(id);
        let mut sessions = CaptureSessions::new(Default::default());
        sessions.publish_source(source);
        let first = sessions.source_epoch(source).unwrap();
        let size = SizeI {
            width: 100,
            height: 80,
        };
        let mut root = test_window(size, PointI::default());
        root.desktop_id = Some(id);
        root.frame_title = Some("Selected window".into());
        let mut windows =
            BTreeMap::from([(super::super::WaylandSurfaceId::from_raw(1).unwrap(), root)]);
        let layers = vec![ShellLayer::image(
            ShellLayerKey::Surface(1),
            ShellSceneKey::Surface(1),
            1,
            ShellImageUpdate::Unchanged,
            size,
            RectI {
                x: 0,
                y: 0,
                width: 100,
                height: 80,
            },
            None,
            ImageAlphaMode::Opaque,
            ImagePixelFormat::Rgba8,
            true,
        )];
        let scale = crate::platform::contracts::ScaleFactor::new(1.0).unwrap();
        let mut streams = CaptureStreams::default();
        assert!(streams.refresh_windows(&sessions, &windows, &layers, scale, true, |_| None));
        assert_eq!(
            streams.window_sources().collect::<Vec<_>>(),
            vec![(id, first, "Selected window")]
        );
        assert!(streams.window_layout(id, first).is_some());
        assert!(!streams.refresh_windows(&sessions, &windows, &layers, scale, true, |_| None));
        let surface = super::super::WaylandSurfaceId::from_raw(1).unwrap();
        for _ in 0..12 {
            windows.get_mut(&surface).unwrap().presentation.content_ready = false;
            assert!(!streams.refresh_windows(&sessions, &windows, &[], scale, true, |_| None));
            assert_eq!(streams.window_sources().collect::<Vec<_>>(), vec![(id, first, "Selected window")]);
            assert!(streams.preview_scene(source, first).is_none());
            assert!(streams.window_layout(id, first).is_some());
            windows.get_mut(&surface).unwrap().presentation.content_ready = true;
            assert!(!streams.refresh_windows(&sessions, &windows, &layers, scale, true, |_| None));
            assert!(streams.preview_scene(source, first).is_some());
        }
        sessions.withdraw_source(source);
        sessions.publish_source(source);
        let remapped = sessions.source_epoch(source).unwrap();
        // The new registry epoch must not make an old snapshot appear current.
        assert!(streams.window_layout(id, remapped).is_none());
        assert!(streams.refresh_windows(&sessions, &windows, &layers, scale, true, |_| None));
        assert!(streams.window_layout(id, first).is_none());
        assert!(streams.window_layout(id, remapped).is_some());
        assert!(streams.refresh_windows(&sessions, &windows, &layers, scale, true,
            |_| Some("Firefox — Native Wayland title".into())));
        assert_eq!(streams.window_sources().next().unwrap().2, "Firefox — Native Wayland title");
        assert!(streams.refresh_windows(&sessions, &windows, &layers, scale, false, |_| None));
        assert_eq!(streams.window_sources().count(), 0);
    }
}

fn allocate_spares(
    renderer: &ShellRenderer,
    layout: crate::shell::capture::CaptureLayout,
) -> AppResult<Vec<crate::media::video::CapturePixels>> {
    let budget = renderer.capture_memory_budget();
    (0..2)
        .map(|_| {
            crate::media::video::CapturePixels::new(layout.byte_len(), budget.clone())
                .map_err(|error| AppError::new(error.to_string()))
        })
        .collect()
}
