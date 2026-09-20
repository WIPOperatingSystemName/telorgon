//! Owner-thread assembly between authorization, GPU capture and PipeWire delivery.

use super::capture::{CaptureSessions, RequesterId, SessionId, SessionState};
use super::renderer::{CaptureBuffer, CaptureJob, CaptureView, ShellRenderer};
use crate::application_host::{AppError, AppResult};
use crate::screencast_linux::{StreamStatus, VideoStream};
use crate::shell::capture::{CaptureLayout, CaptureOptions, CaptureSource, CaptureStopReason};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct ActiveStream {
    requester: RequesterId,
    video: VideoStream,
    buffer: Option<CaptureBuffer>,
    spares: Vec<Vec<u8>>,
    created: Instant,
    in_flight: bool,
    resizing: Option<u64>,
    node_id: Option<u32>,
    window: Option<super::capture_window::WindowCapture>,
    window_revision: u64,
    observed_revision: Option<(u64, u64)>,
    revision: u64,
}

#[derive(Default)]
pub(super) struct CaptureStreams {
    streams: BTreeMap<SessionId, ActiveStream>,
    last_scheduled: Option<SessionId>,
    window_sources:
        BTreeMap<crate::shell::WindowId, (u64, String, super::capture_window::WindowCapture)>,
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
        let window = match source {
            CaptureSource::Output(_) => None,
            CaptureSource::Window(id) => Some(
                self.window_sources
                    .get(&id)
                    .filter(|(epoch, _, _)| sessions.source_epoch(source) == Some(*epoch))
                    .map(|(_, _, snapshot)| snapshot)
                    .filter(|snapshot| snapshot.layout == layout)
                    .ok_or_else(|| AppError::new("window capture source is no longer ready"))?
                    .clone(),
            ),
        };
        let id = sessions
            .request(requester, source, options)
            .map_err(capture_error)?;
        let prepared = (|| {
            sessions.authorize(id, true).map_err(capture_error)?;
            sessions
                .negotiate(id, requester, layout)
                .map_err(capture_error)?;
            let buffer = renderer.allocate_capture(layout)?;
            let video = VideoStream::start(id.get(), layout, options.max_frame_rate().get(), wake)
                .map_err(AppError::new)?;
            Ok(ActiveStream {
                requester,
                video,
                buffer: Some(buffer),
                spares: vec![vec![0; layout.byte_len()], vec![0; layout.byte_len()]],
                created: Instant::now(),
                in_flight: false,
                resizing: None,
                node_id: None,
                window,
                window_revision: 0,
                observed_revision: None,
                revision: 0,
            })
        })();
        match prepared {
            Ok(stream) => {
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

    pub fn refresh_windows(
        &mut self,
        sessions: &CaptureSessions,
        windows: &BTreeMap<super::WaylandSurfaceId, super::ClientWindow>,
        layers: &[super::scene::ShellLayer],
        scale: crate::platform::ScaleFactor,
        discover: bool,
    ) -> bool {
        let previous = std::mem::take(&mut self.window_sources);
        if discover {
            for window in windows.values().filter(|w| w.backend.is_some()) {
                let Some(id) = window.desktop_id else {
                    continue;
                };
                if let Some(snapshot) = super::capture_window::prepare(id, windows, layers, scale) {
                    let title = window
                        .frame_title
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or(&window.application_identity);
                    let label = crate::shell_components::capture::application_label(title);
                    if let Some(epoch) = sessions.source_epoch(CaptureSource::Window(id)) {
                        self.window_sources.insert(id, (epoch, label, snapshot));
                    }
                }
            }
        }
        for (&id, stream) in &mut self.streams {
            let next = sessions.get(id).and_then(|session| match session.source {
                CaptureSource::Window(window) => {
                    super::capture_window::prepare(window, windows, layers, scale)
                }
                CaptureSource::Output(_) => None,
            });
            if stream.window != next {
                stream.window_revision = stream.window_revision.wrapping_add(1).max(1);
                stream.window = next;
            }
        }
        !previous
            .iter()
            .map(|(id, (epoch, label, _))| (id, epoch, label))
            .eq(self
                .window_sources
                .iter()
                .map(|(id, (epoch, label, _))| (id, epoch, label)))
    }

    pub fn window_sources(&self) -> impl Iterator<Item = (crate::shell::WindowId, u64, &str)> {
        self.window_sources
            .iter()
            .map(|(&id, (epoch, label, _))| (id, *epoch, label.as_str()))
    }

    pub fn window_layout(&self, id: crate::shell::WindowId, epoch: u64) -> Option<CaptureLayout> {
        self.window_sources
            .get(&id)
            .filter(|(current, _, _)| *current == epoch)
            .map(|(_, _, snapshot)| snapshot.layout)
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
                    && s.options.cursor() == crate::shell::capture::CaptureCursorMode::Embedded
            })
        })
    }

    pub fn fail_embedded_cursor(&self, sessions: &mut CaptureSessions) {
        for (&id, stream) in &self.streams {
            if sessions.get(id).is_some_and(|s| {
                s.options.cursor() == crate::shell::capture::CaptureCursorMode::Embedded
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
                if !stream.in_flight && stream.video.retired() {
                    retired.push(id);
                }
                continue;
            }
            let status = stream.video.status();
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
                stream.node_id = Some(*node_id);
            }
            match status {
                StreamStatus::Failed(_) | StreamStatus::Stopped => {
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
                        stream.spares =
                            vec![vec![0; layout.byte_len()], vec![0; layout.byte_len()]];
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
                        stream.created = Instant::now();
                        stream.buffer = None;
                        stream.spares.clear();
                    }
                    Err(_) => {
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
            let Some(buffer) = &mut stream.buffer else {
                continue;
            };
            if buffer.pixels.is_empty() {
                let Some(pixels) = stream.spares.pop().or_else(|| stream.video.take_spare()) else {
                    continue;
                };
                buffer.pixels = pixels;
            }
            let session = sessions.get(id).expect("admitted session");
            let view = match session.source {
                CaptureSource::Output(_) => CaptureView::Output,
                CaptureSource::Window(id) => {
                    let Some(window) = stream
                        .window
                        .as_ref()
                        .filter(|w| w.window == id && w.layout == buffer.layout)
                    else {
                        continue;
                    };
                    CaptureView::Window(window.clone())
                }
            };
            let cursor = session.options.cursor();
            let observed = (stream.window_revision, renderer.capture_revision(cursor));
            if stream.observed_revision != Some(observed) {
                stream.revision = stream.revision.wrapping_add(1).max(1);
                stream.observed_revision = Some(observed);
            }
            let revision = stream.revision;
            let Ok(Some(ticket)) = sessions.begin_frame(id, now_ns, revision) else {
                continue;
            };
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
                    let Some(job) = failure.rejected else { return Err(failure.error); };
                    stream.buffer = Some(job.buffer);
                    let _ = sessions.finish_frame(ticket, false);
                    let _ = sessions.stop(id, stream.requester, CaptureStopReason::StreamFailed);
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
        let deliver = sessions.finish_frame(job.ticket, success).unwrap_or(false);
        let Some(stream) = self.streams.get_mut(&job.ticket.session()) else {
            return Vec::new();
        };
        stream.in_flight = false;
        let mut sampled = Vec::new();
        if deliver {
            let pixels = std::mem::take(&mut job.buffer.pixels);
            match stream.video.publish(pixels) {
                Ok(reusable) => {
                    if let CaptureView::Window(window) = job.view {
                        sampled = window.into_surface_revisions();
                    }
                    job.buffer.pixels =
                        reusable.or_else(|| stream.spares.pop()).unwrap_or_default();
                }
                Err(pixels) => {
                    job.buffer.pixels = pixels;
                    let _ = sessions.stop(
                        job.ticket.session(),
                        stream.requester,
                        CaptureStopReason::StreamFailed,
                    );
                }
            }
        }
        if stream.resizing.is_none() {
            stream.buffer = Some(job.buffer);
        } // Otherwise the completed old-generation slot retires here, never entering the new pool.
        sampled
    }

    pub fn wait(&self, existing: Option<Duration>) -> Option<Duration> {
        if self.streams.is_empty() {
            existing
        } else {
            Some(
                existing
                    .unwrap_or(Duration::from_millis(16))
                    .min(Duration::from_millis(16)),
            )
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
        let windows =
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
        let scale = crate::platform::ScaleFactor::new(1.0).unwrap();
        let mut streams = CaptureStreams::default();
        assert!(streams.refresh_windows(&sessions, &windows, &layers, scale, true));
        assert_eq!(
            streams.window_sources().collect::<Vec<_>>(),
            vec![(id, first, "Selected window")]
        );
        assert!(streams.window_layout(id, first).is_some());
        assert!(!streams.refresh_windows(&sessions, &windows, &layers, scale, true));
        sessions.withdraw_source(source);
        sessions.publish_source(source);
        let remapped = sessions.source_epoch(source).unwrap();
        // The new registry epoch must not make an old snapshot appear current.
        assert!(streams.window_layout(id, remapped).is_none());
        assert!(streams.refresh_windows(&sessions, &windows, &layers, scale, true));
        assert!(streams.window_layout(id, first).is_none());
        assert!(streams.window_layout(id, remapped).is_some());
        assert!(streams.refresh_windows(&sessions, &windows, &layers, scale, false));
        assert_eq!(streams.window_sources().count(), 0);
    }
}
