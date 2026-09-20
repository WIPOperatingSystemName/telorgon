//! Reusable direct-protocol capture storage under the shared host session budget.
use super::{
    capture::{CaptureSessions, RequesterId, SessionId},
    renderer::{CaptureBuffer, CaptureJob, CaptureView, ShellRenderer},
};
use crate::{
    application_host::{AppError, AppResult},
    compositor_wayland::{DirectCaptureJob, NativeCompositor, ProtocolObjectId},
    shell::{
        OutputId,
        capture::{
            CaptureCursorMode, CaptureLayout, CaptureOptions, CaptureSource, CaptureStopReason,
        },
    },
};
use std::{collections::BTreeMap, num::NonZeroU32};

struct Stream {
    id: SessionId,
    requester: RequesterId,
    buffer: Option<CaptureBuffer>,
}

#[derive(Default)]
pub(super) struct DirectCaptures {
    streams: BTreeMap<ProtocolObjectId, Stream>,
    pending: Option<DirectCaptureJob>,
}

impl DirectCaptures {
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn schedule(
        &mut self,
        native: &mut NativeCompositor<'_>,
        sessions: &mut CaptureSessions,
        renderer: &mut ShellRenderer,
        now_ns: u64,
    ) -> AppResult<()> {
        if let Some(request) = &self.pending
            && !native
                .direct_capture_job_live(request)
                .map_err(|e| AppError::new(e.to_string()))?
        {
            // Return the handed-off destination before native terminal delivery;
            // source refresh may be waiting for this ownership acknowledgement.
            if let Some(request) = self.pending.take() {
                native
                    .complete_direct_capture(request.fail())
                    .map_err(|e| AppError::new(e.to_string()))?;
            }
        }
        if sessions.has_pending_frame() {
            return Ok(());
        }
        if self.pending.is_none() {
            self.pending = native
                .take_direct_capture_job()
                .map_err(|e| AppError::new(e.to_string()))?;
        }
        let Some(request) = self.pending.take() else {
            return Ok(());
        };
        let admitted = self.admit(&request, sessions, renderer);
        match admitted {
            Ok(false) => {
                self.pending = Some(request);
                return Ok(());
            }
            Err(error) => {
                eprintln!("telorgon-capture: {error}");
                native
                    .complete_direct_capture(request.fail())
                    .map_err(|e| AppError::new(e.to_string()))?;
                return Ok(());
            }
            Ok(true) => {}
        }
        let fallback = request.failure_completion();
        self.pending = Some(request);
        let Some(timestamp_ns) = crate::platform::linux::monotonic_time_microseconds()
            .and_then(|us| us.checked_mul(1000))
        else {
            self.pending = None;
            native
                .complete_direct_capture(fallback)
                .map_err(|e| AppError::new(e.to_string()))?;
            return Ok(());
        };
        let mut pending = self.pending.take();
        let job = self.prepare(&mut pending, sessions, now_ns, timestamp_ns);
        self.pending = pending;
        match job {
            Ok(Some(job)) => {
                if let Err(failure) = renderer.submit_capture_recoverable(job) {
                    native
                        .complete_direct_capture(fallback)
                        .map_err(|e| AppError::new(e.to_string()))?;
                    if let Some(job) = failure.rejected {
                        self.reject_recording(sessions, *job);
                        eprintln!("telorgon-capture: {}", failure.error);
                    } else {
                        return Err(failure.error);
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.pending = None;
                native
                    .complete_direct_capture(fallback)
                    .map_err(|e| AppError::new(e.to_string()))?;
                eprintln!("telorgon-capture: {error}");
            }
        }
        Ok(())
    }

    /// Admit a protocol session once. Existing sessions retain their reusable GPU
    /// storage and accounting between individual client frame requests. False defers
    /// admission until the previous GPU buffer has returned.
    pub fn admit(
        &mut self,
        request: &DirectCaptureJob,
        sessions: &mut CaptureSessions,
        renderer: &ShellRenderer,
    ) -> AppResult<bool> {
        let requester = RequesterId::Direct(request.requester);
        let source = CaptureSource::Output(OutputId::MIN);
        // The managed host currently has one output, registered as native output 1.
        if request.output != 1 {
            return Err(AppError::new("unsupported direct capture output"));
        }
        let layout = layout(request)?;
        if let Some(stream) = self.streams.get_mut(&request.session) {
            if stream.requester != requester {
                return Err(AppError::new("capture session owner changed"));
            }
            let Some(buffer) = stream.buffer.as_ref() else {
                return Ok(false);
            };
            if buffer.layout == layout {
                return Ok(true);
            }
            let id = stream.id;
            // Reserve both generations before allocating the replacement. No old
            // GPU use remains: buffer ownership has returned to this stream.
            let generation = sessions
                .renegotiate(id, requester, layout)
                .map_err(|e| AppError::new(format!("direct capture resize budget: {e:?}")))?;
            let resized = (|| {
                let replacement = renderer.allocate_capture(layout)?;
                drop(stream.buffer.take());
                sessions
                    .generation_retired(id, requester, generation)
                    .map_err(|e| {
                        AppError::new(format!("direct capture generation retirement: {e:?}"))
                    })?;
                stream.buffer = Some(replacement);
                sessions
                    .started(id, requester)
                    .map_err(|e| AppError::new(format!("direct capture resize startup: {e:?}")))?;
                Ok(true)
            })();
            if resized.is_err() {
                // No submitted job owns either generation on this path.
                drop(self.streams.remove(&request.session));
                let _ = sessions.stop(id, requester, CaptureStopReason::StreamFailed);
                let _ = sessions.retired(id);
            }
            return resized;
        }
        let cursor = if request.paint_cursor {
            CaptureCursorMode::Embedded
        } else {
            CaptureCursorMode::Hidden
        };
        let id = sessions
            .request(
                requester,
                source,
                CaptureOptions::new(cursor, NonZeroU32::new(60).unwrap()),
            )
            .map_err(|e| AppError::new(format!("direct capture admission: {e:?}")))?;
        let allocated = (|| {
            sessions
                .authorize(id, true)
                .map_err(|e| AppError::new(format!("direct capture authorization: {e:?}")))?;
            sessions
                .negotiate(id, requester, layout)
                .map_err(|e| AppError::new(format!("direct capture budget: {e:?}")))?;
            let buffer = renderer.allocate_capture(layout)?;
            sessions
                .started(id, requester)
                .map_err(|e| AppError::new(format!("direct capture startup: {e:?}")))?;
            Ok(buffer)
        })();
        match allocated {
            Ok(buffer) => {
                self.streams.insert(
                    request.session,
                    Stream {
                        id,
                        requester,
                        buffer: Some(buffer),
                    },
                );
                Ok(true)
            }
            Err(error) => {
                let _ = sessions.stop(id, requester, CaptureStopReason::StreamFailed);
                let _ = sessions.retired(id);
                Err(error)
            }
        }
    }

    /// Preserve request ownership when pacing/global admission asks the host to retry.
    pub fn prepare(
        &mut self,
        request: &mut Option<DirectCaptureJob>,
        sessions: &mut CaptureSessions,
        now_ns: u64,
        timestamp_ns: u64,
    ) -> AppResult<Option<CaptureJob>> {
        let Some(direct) = request.as_ref() else {
            return Ok(None);
        };
        let stream = self
            .streams
            .get_mut(&direct.session)
            .ok_or_else(|| AppError::new("direct session not admitted"))?;
        let Some(buffer) = stream.buffer.as_ref() else {
            return Ok(None);
        };
        if buffer.layout != layout(direct)? {
            return Err(AppError::new(
                "direct capture resize requires renegotiation",
            ));
        }
        // Every explicit protocol frame is eligible even when desktop damage is unchanged.
        let revision = now_ns;
        let Some(ticket) = sessions
            .begin_frame(stream.id, now_ns, revision)
            .map_err(|e| AppError::new(format!("direct capture scheduling: {e:?}")))?
        else {
            return Ok(None);
        };
        let direct = request.take().expect("checked direct request");
        let cursor = if direct.paint_cursor {
            CaptureCursorMode::Embedded
        } else {
            CaptureCursorMode::Hidden
        };
        Ok(Some(CaptureJob {
            view: CaptureView::Output,
            ticket,
            cursor,
            direct: Some((
                direct,
                timestamp_ns,
                crate::integrations::wayland::compositor::OutputTransform::Normal,
            )),
            buffer: stream.buffer.take().expect("checked capture buffer"),
        }))
    }

    pub fn owns(&self, id: SessionId) -> bool {
        self.streams.values().any(|stream| stream.id == id)
    }

    /// Only for jobs returned before queue submission. Recording may already
    /// have marked staging pending, so do not reuse the rejected GPU slot.
    fn reject_recording(&mut self, sessions: &mut CaptureSessions, job: CaptureJob) {
        let id = job.ticket.session();
        let _ = sessions.finish_frame(job.ticket, false);
        let protocol = self
            .streams
            .iter()
            .find_map(|(&protocol, stream)| (stream.id == id).then_some(protocol));
        drop(job);
        if let Some(protocol) = protocol {
            let stream = self
                .streams
                .remove(&protocol)
                .expect("rejected direct stream");
            let requester = stream.requester;
            drop(stream);
            let _ = sessions.stop(id, requester, CaptureStopReason::StreamFailed);
            let _ = sessions.retired(id);
        }
    }

    pub fn complete(&mut self, sessions: &mut CaptureSessions, job: CaptureJob, success: bool) {
        let _ = sessions.finish_frame(job.ticket, success);
        if let Some(stream) = self
            .streams
            .values_mut()
            .find(|stream| stream.id == job.ticket.session())
        {
            stream.buffer = Some(job.buffer);
        }
    }

    pub fn retire(
        &mut self,
        sessions: &mut CaptureSessions,
        native: &mut NativeCompositor<'_>,
    ) -> AppResult<()> {
        let mut retired = Vec::new();
        for (&protocol, stream) in &self.streams {
            if !native
                .direct_capture_session_live(protocol)
                .map_err(|e| AppError::new(e.to_string()))?
            {
                let _ = sessions.stop(
                    stream.id,
                    stream.requester,
                    CaptureStopReason::SourceUnavailable,
                );
                if stream.buffer.is_some() {
                    retired.push(protocol);
                }
            }
        }
        for protocol in retired {
            let stream = self
                .streams
                .remove(&protocol)
                .expect("retired direct stream");
            let id = stream.id;
            drop(stream);
            let _ = sessions.retired(id);
        }
        Ok(())
    }
}

fn layout(request: &DirectCaptureJob) -> AppResult<CaptureLayout> {
    let width = u32::try_from(request.size.width)
        .ok()
        .and_then(NonZeroU32::new);
    let height = u32::try_from(request.size.height)
        .ok()
        .and_then(NonZeroU32::new);
    width
        .zip(height)
        .and_then(|(w, h)| {
            w.get()
                .checked_mul(4)
                .and_then(|stride| CaptureLayout::rgba8(w, h, stride))
        })
        .ok_or_else(|| AppError::new("invalid direct capture size"))
}
