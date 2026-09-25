//! Request-scoped, bounded thumbnail capture for the trusted picker, never the requesting app.
use super::{
    capture::{CaptureSessions, RequesterId, SessionId},
    capture_scene::CaptureScene,
    capture_streams::CaptureStreams,
    renderer::{CaptureBuffer, CaptureJob, CaptureView, ShellRenderer},
    scene::ShellLayerKey,
};
use crate::{
    PortalSourcePreview, RectI,
    shell::capture::{CaptureLayout, CaptureOptions, CaptureSource, CaptureStopReason},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
    sync::Arc,
};

const WIDTH: u32 = 384;
const HEIGHT: u32 = 216;
// Target 15 updates per source per second, with a global 120-capture/s budget.
// Keep one job in flight and yield to application captures before admitting previews.
const SOURCE_INTERVAL: u64 = 66_666_667;
const MIN_INTERVAL: u64 = 8_333_334;
fn capture_interval(sources: usize) -> u64 {
    (SOURCE_INTERVAL / sources.max(1) as u64).max(MIN_INTERVAL)
}
const MAX_SOURCES: usize = 1024;

struct Pending {
    session: SessionId,
    request: u64,
    source: CaptureSource,
    epoch: u64,
}
#[derive(Default)]
pub(super) struct PortalPreviews {
    request: Option<u64>,
    sources: Vec<(CaptureSource, u64)>,
    frames: BTreeMap<CaptureSource, PortalSourcePreview>,
    excluded: BTreeSet<u32>,
    pending: Option<Pending>,
    buffer: Option<CaptureBuffer>,
    next: usize,
    next_ns: u64,
    revision: u64,
}
impl PortalPreviews {
    pub fn sync(
        &mut self,
        request: Option<u64>,
        sources: &[(CaptureSource, u64, String)],
        excluded: BTreeSet<u32>,
        sessions: &mut CaptureSessions,
    ) {
        if self.request != request {
            self.frames.clear();
            self.buffer = None;
            self.next = 0;
            self.next_ns = 0;
        }
        self.request = request;
        self.sources = if request.is_some() {
            sources
                .iter()
                .take(MAX_SOURCES)
                .map(|(s, e, _)| (*s, *e))
                .collect()
        } else {
            Vec::new()
        };
        self.excluded = excluded;
        self.frames
            .retain(|source, frame| self.sources.contains(&(*source, frame.epoch)));
        if let Some(pending) = &self.pending {
            if request != Some(pending.request)
                || !self.sources.contains(&(pending.source, pending.epoch))
            {
                let _ = sessions.stop(
                    pending.session,
                    RequesterId::PickerPreview(pending.request),
                    CaptureStopReason::SourceUnavailable,
                );
            }
        }
    }
    pub fn frames(&self) -> Vec<PortalSourcePreview> {
        self.frames.values().cloned().collect()
    }
    pub fn active(&self) -> bool {
        self.request.is_some() && !self.sources.is_empty()
    }
    pub fn wait(
        &self,
        existing: Option<std::time::Duration>,
        now: u64,
    ) -> Option<std::time::Duration> {
        if !self.active() || self.pending.is_some() {
            return existing;
        }
        let delay =
            std::time::Duration::from_nanos(self.next_ns.saturating_sub(now).max(1_000_000));
        Some(existing.map_or(delay, |wait| wait.min(delay)))
    }
    pub fn owns(&self, id: SessionId) -> bool {
        self.pending.as_ref().is_some_and(|p| p.session == id)
    }

    pub fn schedule(
        &mut self,
        sessions: &mut CaptureSessions,
        streams: &CaptureStreams,
        renderer: &mut ShellRenderer,
        output: CaptureLayout,
        now: u64,
    ) -> crate::host::application::AppResult<()> {
        if !self.active()
            || self.pending.is_some()
            || sessions.has_pending_frame()
            || now < self.next_ns
        {
            return Ok(());
        }
        self.next_ns = now.saturating_add(capture_interval(self.sources.len()));
        let request = self.request.unwrap();
        let (source, epoch) = self.sources[self.next % self.sources.len()];
        self.next = (self.next + 1) % self.sources.len();
        if sessions.source_epoch(source) != Some(epoch) {
            return Ok(());
        }
        let scene = match source {
            CaptureSource::Output(id) if id == crate::shell::OutputId::MIN => {
                renderer.preview_output_scene(output)
            }
            _ => streams.preview_scene(source, epoch),
        };
        let Some(scene) = scene.filter(|s| !s.placements.is_empty()) else {
            return Ok(());
        };
        let scene = thumbnail(scene, &self.excluded);
        let owner = RequesterId::PickerPreview(request);
        let Ok(id) = sessions.request(owner, source, CaptureOptions::default()) else {
            return Ok(());
        };
        let admitted = sessions
            .authorize(id, true)
            .and_then(|_| sessions.negotiate(id, owner, scene.layout))
            .and_then(|_| sessions.started(id, owner));
        if admitted.is_err() {
            retire(sessions, id, owner);
            return Ok(());
        }
        // One bounded stream of GPU work reuses its target/readback allocation after
        // completion. Every render clears the target, including when switching sources.
        let buffer = match self.buffer.take() {
            Some(buffer) => buffer,
            None => match renderer.allocate_capture(scene.layout) {
                Ok(buffer) => buffer,
                Err(_) => { retire(sessions, id, owner); return Ok(()); }
            },
        };
        let Ok(Some(ticket)) = sessions.begin_frame(id, now, now.max(1)) else {
            retire(sessions, id, owner);
            return Ok(());
        };
        self.pending = Some(Pending {
            session: id,
            request,
            source,
            epoch,
        });
        let job = CaptureJob {
            view: CaptureView::Scene(scene),
            direct: None,
            ticket,
            cursor: crate::shell::capture::CaptureCursorMode::Hidden,
            buffer,
        };
        if let Err(failure) = renderer.submit_capture_recoverable(job) {
            if let Some(job) = failure.rejected {
                self.complete(sessions, *job, false);
            } else {
                // Submission/worker loss must reach the host's renderer failure path; the
                // outstanding frame cannot be silently reused or left blocking other captures.
                return Err(failure.error);
            }
        }
        Ok(())
    }
    pub fn complete(&mut self, sessions: &mut CaptureSessions, job: CaptureJob, success: bool) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let deliver = sessions.finish_frame(job.ticket, success).unwrap_or(false)
            && self.request == Some(pending.request)
            && self.sources.contains(&(pending.source, pending.epoch))
            && sessions.source_epoch(pending.source) == Some(pending.epoch);
        if deliver
            && job.buffer.pixels.len() == (WIDTH * HEIGHT * 4) as usize
            && !self.frames.get(&pending.source).is_some_and(|old| {
                old.epoch == pending.epoch && old.pixels.as_ref() == &job.buffer.pixels[..]
            })
        {
            self.revision = self.revision.wrapping_add(1).max(1);
            self.frames.insert(
                pending.source,
                PortalSourcePreview {
                    source: pending.source,
                    epoch: pending.epoch,
                    revision: self.revision,
                    width: WIDTH,
                    height: HEIGHT,
                    pixels: Arc::from(job.buffer.pixels.as_ref()),
                },
            );
        }
        let id = pending.session;
        if success && self.request == Some(pending.request) {
            self.buffer = Some(job.buffer);
        }
        retire(sessions, id, RequesterId::PickerPreview(pending.request));
    }
}
fn retire(sessions: &mut CaptureSessions, id: SessionId, owner: RequesterId) {
    let _ = sessions.stop(id, owner, CaptureStopReason::RequesterDisconnected);
    let _ = sessions.retired(id);
}
fn thumbnail(mut scene: CaptureScene, excluded: &BTreeSet<u32>) -> CaptureScene {
    let scale = (WIDTH as f32 / scene.layout.width() as f32)
        .min(HEIGHT as f32 / scene.layout.height() as f32);
    let x = (WIDTH as f32 - scene.layout.width() as f32 * scale) * 0.5;
    let y = (HEIGHT as f32 - scene.layout.height() as f32 * scale) * 0.5;
    let map = |r: RectI| RectI {
        x: (x + r.x as f32 * scale).round() as i32,
        y: (y + r.y as f32 * scale).round() as i32,
        width: (r.width as f32 * scale).round().max(1.0) as i32,
        height: (r.height as f32 * scale).round().max(1.0) as i32,
    };
    scene.placements.retain(|p| match p.key {
        ShellLayerKey::Surface(id)
        | ShellLayerKey::Frame(id, _)
        | ShellLayerKey::FrameShadow(id)
        | ShellLayerKey::ContentBackground(id)
        | ShellLayerKey::ContentBorder(id)
        | ShellLayerKey::ContentCorners(id)
        | ShellLayerKey::Motion(id)
        | ShellLayerKey::MotionShadow(id)
        | ShellLayerKey::LegacyControl(id, _) => !excluded.contains(&id),
        ShellLayerKey::WindowPreview(..)
        | ShellLayerKey::OutputPreview(..)
        | ShellLayerKey::Cursor
        | ShellLayerKey::DragIcon(_)
        | ShellLayerKey::TilePreview(_)
        | ShellLayerKey::ResizeVeil(_)
        | ShellLayerKey::ResizePreviewBorder(_) => false,
        _ => true,
    });
    for placement in &mut scene.placements {
        placement.target = map(placement.target);
        placement.clip = placement.clip.map(map);
        for clip in placement.rounded_clips.iter_mut().flatten() {
            clip.rect.x = x + clip.rect.x * scale;
            clip.rect.y = y + clip.rect.y * scale;
            clip.rect.width *= scale;
            clip.rect.height *= scale;
            clip.radii.top_left *= scale;
            clip.radii.top_right *= scale;
            clip.radii.bottom_left *= scale;
            clip.radii.bottom_right *= scale;
        }
    }
    scene.layout = CaptureLayout::rgba8(
        NonZeroU32::new(WIDTH).unwrap(),
        NonZeroU32::new(HEIGHT).unwrap(),
        WIDTH * 4,
    )
    .unwrap();
    scene.desktop_cursor_origin = None;
    scene.sampled.clear();
    scene
}

#[cfg(test)]
mod tests {
    use super::super::{
        capture::CaptureLimits,
        scene::{ShellPlacement, ShellSceneKey},
    };
    use super::*;
    #[test]
    fn preview_cadence_is_per_source_and_globally_bounded() {
        for count in 1..=8 {
            assert!(capture_interval(count) * count as u64 <= SOURCE_INTERVAL + 8);
        }
        assert_eq!(capture_interval(1024), MIN_INTERVAL);
        let source = CaptureSource::Output(crate::shell::OutputId::MIN);
        let previews = PortalPreviews {
            request: Some(1),
            sources: vec![(source, 1)],
            next_ns: 40_000_000,
            ..Default::default()
        };
        assert_eq!(
            previews.wait(None, 30_000_000),
            Some(std::time::Duration::from_millis(10))
        );
        assert_eq!(
            previews.wait(Some(std::time::Duration::from_millis(2)), 30_000_000),
            Some(std::time::Duration::from_millis(2))
        );
    }
    fn layout(width: u32, height: u32) -> CaptureLayout {
        CaptureLayout::rgba8(
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap(),
            width * 4,
        )
        .unwrap()
    }
    #[test]
    fn thumbnails_preserve_aspect_and_exclude_picker_and_recursive_previews() {
        let placement = |key| ShellPlacement {
            key,
            scene: ShellSceneKey::Surface(2),
            target: RectI {
                x: 0,
                y: 0,
                width: 400,
                height: 400,
            },
            clip: None,
            rounded_clips: [None, None],
        };
        let scene = CaptureScene {
            layout: layout(400, 400),
            desktop_cursor_origin: None,
            sampled: vec![(2, 3)],
            placements: vec![
                placement(ShellLayerKey::Surface(2)),
                placement(ShellLayerKey::Surface(9)),
                placement(ShellLayerKey::Frame(9, 0)),
                placement(ShellLayerKey::OutputPreview(1, 0, 0)),
            ],
        };
        let result = thumbnail(scene, &BTreeSet::from([9]));
        assert_eq!(result.layout, layout(384, 216));
        assert_eq!(result.placements.len(), 1);
        assert_eq!(
            result.placements[0].target,
            RectI {
                x: 84,
                y: 0,
                width: 216,
                height: 216
            }
        );
        assert!(result.sampled.is_empty());
    }
    #[test]
    fn ending_request_or_replacing_source_clears_thumbnails_and_revokes_inflight_work() {
        let source = CaptureSource::Output(crate::shell::OutputId::MIN);
        let mut sessions = CaptureSessions::new(CaptureLimits::default());
        sessions.publish_source(source);
        let epoch = sessions.source_epoch(source).unwrap();
        let mut previews = PortalPreviews::default();
        previews.sync(
            Some(3),
            &[(source, epoch, "Monitor".into())],
            BTreeSet::new(),
            &mut sessions,
        );
        let owner = RequesterId::PickerPreview(3);
        let id = sessions
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        sessions.authorize(id, true).unwrap();
        previews.pending = Some(Pending {
            session: id,
            request: 3,
            source,
            epoch,
        });
        previews.frames.insert(
            source,
            PortalSourcePreview {
                source,
                epoch,
                revision: 1,
                width: 1,
                height: 1,
                pixels: vec![0; 4].into(),
            },
        );
        previews.sync(
            Some(3),
            &[(source, epoch + 1, "Monitor".into())],
            BTreeSet::new(),
            &mut sessions,
        );
        assert!(previews.frames().is_empty());
        assert!(matches!(
            sessions.get(id).unwrap().state,
            super::super::capture::SessionState::Stopped(_)
        ));
        previews.sync(None, &[], BTreeSet::new(), &mut sessions);
        assert!(!previews.active());
    }
    #[test]
    fn trusted_thumbnail_does_not_consume_application_stream_quota() {
        let mut sessions = CaptureSessions::new(CaptureLimits {
            sessions: 1,
            ..Default::default()
        });
        let source = CaptureSource::Output(crate::shell::OutputId::MIN);
        sessions.publish_source(source);
        assert!(
            sessions
                .request(
                    RequesterId::PickerPreview(1),
                    source,
                    CaptureOptions::default()
                )
                .is_ok()
        );
        assert!(
            sessions
                .request(RequesterId::Portal(1), source, CaptureOptions::default())
                .is_ok()
        );
        assert!(
            sessions
                .request(
                    RequesterId::PickerPreview(2),
                    source,
                    CaptureOptions::default()
                )
                .is_err()
        );
        assert!(
            sessions
                .request(RequesterId::Portal(2), source, CaptureOptions::default())
                .is_err()
        );
    }
}
