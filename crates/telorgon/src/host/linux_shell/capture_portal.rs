//! Owner-thread portal requests, consent decisions and stream readiness.

use super::capture::{CaptureSessions, RequesterId, SessionId, SessionState};
use super::capture_streams::CaptureStreams;
use super::renderer::ShellRenderer;
use crate::host::application::{AppError, AppResult, declaration::RegisteredShellWidget};
use crate::authoring::compose::{Signal, SignalWriter};
use crate::foundation::SizeI;
use crate::integrations::portals::{Lease, Portal, StartRequest, StreamInfo};
use crate::shell::{
    OutputId,
    capture::{CaptureLayout, CaptureSource, CaptureStopReason},
};
use crate::components::shell::capture::{
    CapturePicker, CaptureDecision, CaptureIndicator, CaptureUi, CaptureUiSnapshot,
    application_label,
};
use std::collections::{BTreeMap, VecDeque};
use std::num::NonZeroU32;
use std::sync::{
    Arc,
    mpsc::{Receiver, sync_channel},
};

struct Sharing {
    id: SessionId,
    requester: RequesterId,
    lease: Arc<Lease>,
    label: String,
    request: Option<StartRequest>,
}

pub(super) struct CapturePortal {
    portal: Portal,
    pending: VecDeque<StartRequest>,
    sharing: BTreeMap<u64, Sharing>,
    decisions: Receiver<CaptureDecision>,
    snapshot: SignalWriter<CaptureUiSnapshot>,
    wake: Arc<dyn Fn() + Send + Sync>,
    failure: Option<String>,
    output_label: String,
}
impl CapturePortal {
    pub fn new(
        wake: Arc<dyn Fn() + Send + Sync>,
        chooser: Option<crate::host::application::declaration::CaptureChooserFactory>,
        source_types: u32,
        activate_frontend: bool,
        output_label: String,
    ) -> AppResult<(Self, [RegisteredShellWidget; 2])> {
        let portal = Portal::start(wake.clone(), source_types, activate_frontend).map_err(AppError::new)?;
        let (signal, snapshot) = Signal::new(CaptureUiSnapshot::default());
        let (send, decisions) = sync_channel(16);
        let ui = CaptureUi {
            snapshot: signal,
            decisions: send,
            wake: wake.clone(),
        };
        let widgets = [
            match chooser {
                Some(factory) => factory(ui.clone()),
                None => RegisteredShellWidget::new(CapturePicker::new(ui.clone())),
            },
            RegisteredShellWidget::new(CaptureIndicator::new(ui)),
        ];
        Ok((
            Self {
                portal,
                pending: VecDeque::new(),
                sharing: BTreeMap::new(),
                decisions,
                snapshot,
                wake,
                failure: None,
                output_label,
            },
            widgets,
        ))
    }

    pub fn wants_window_sources(&self) -> bool {
        self.pending
            .iter()
            .any(|request| request.source_types & 2 != 0 && !request.lease.closed())
    }

    pub fn revoke_cancelled(&self, sessions: &mut CaptureSessions) {
        for sharing in self.sharing.values() {
            if sharing.lease.closed() {
                let _ = sessions.stop(sharing.id, sharing.requester, CaptureStopReason::Requested);
            }
        }
    }

    pub fn poll(
        &mut self,
        sessions: &mut CaptureSessions,
        streams: &mut CaptureStreams,
        renderer: &ShellRenderer,
        physical_size: SizeI,
        logical_size: SizeI,
        scale: crate::platform::contracts::ScaleFactor,
        locked: bool,
    ) {
        while let Some(request) = self.portal.try_request() {
            if locked || request.lease.closed() || self.pending.len() + self.sharing.len() >= 8 {
                request.lease.close();
                request.complete(Err(1));
            } else {
                self.pending.push_back(request);
            }
        }
        if locked {
            for request in &self.pending {
                request.lease.close();
            }
            for sharing in self.sharing.values() {
                sharing.lease.close();
            }
        }
        while let Ok(decision) = self.decisions.try_recv() {
            match decision {
                CaptureDecision::Approve(id, _, _) | CaptureDecision::Deny(id)
                    if self
                        .pending
                        .front()
                        .is_some_and(|request| request.lease.id == id) =>
                {
                    let request = self.pending.pop_front().unwrap();
                    let (source, epoch) = match decision {
                        CaptureDecision::Approve(_, source, epoch) => (source, epoch),
                        _ => (CaptureSource::Output(OutputId::MIN), 0),
                    };
                    if locked
                        || request.lease.closed()
                        || matches!(decision, CaptureDecision::Deny(_))
                        || !request.allows_source(source)
                        || sessions.source_epoch(source) != Some(epoch)
                    {
                        if !locked && !request.lease.closed() && matches!(decision, CaptureDecision::Approve(..)) {
                            self.failure = Some("That source is no longer available. Please share again.".into());
                        }
                        request.lease.close();
                        request.complete(Err(1));
                        continue;
                    }
                    let selected_layout = match source {
                        CaptureSource::Output(id) if id == OutputId::MIN => {
                            capture_layout(physical_size)
                        }
                        CaptureSource::Window(id) => streams.window_layout(id, epoch),
                        _ => None,
                    };
                    let started = selected_layout
                        .ok_or_else(|| AppError::new("invalid capture extent"))
                        .and_then(|layout| {
                            streams.begin_approved(
                                sessions,
                                renderer,
                                RequesterId::Portal(request.requester),
                                source,
                                request.options,
                                layout,
                                self.wake.clone(),
                            )
                        });
                    match started {
                        Ok(id) => {
                            self.failure = None;
                            self.sharing.insert(
                                request.lease.id,
                                Sharing {
                                    id,
                                    requester: RequesterId::Portal(request.requester),
                                    lease: request.lease.clone(),
                                    label: application_label(&request.app_id),
                                    request: Some(request),
                                },
                            );
                        }
                        Err(error) => {
                            eprintln!("telorgon-capture: {error}");
                            self.failure = Some("Could not start screen sharing. Try sharing again.".into());
                            request.lease.close();
                            request.complete(Err(2));
                        }
                    }
                }
                CaptureDecision::DismissFailure => self.failure = None,
                CaptureDecision::Stop(id) => {
                    if let Some(sharing) = self.sharing.get(&id) {
                        sharing.lease.close();
                    }
                }
                _ => {} // A decision for a dismissed/replaced chooser cannot approve another one.
            }
        }
        self.pending.retain(|request| !request.lease.closed());
        self.revoke_cancelled(sessions);
        self.sharing.retain(|_, sharing| {
            let active = sessions
                .get(sharing.id)
                .is_some_and(|s| !matches!(s.state, SessionState::Stopped(_)));
            if !active || sharing.lease.closed() {
                if !sharing.lease.closed() {
                    self.failure = Some("Screen sharing stopped because the source or stream became unavailable.".into());
                }
                sharing.lease.close();
                if let Some(request) = sharing.request.take() {
                    request.complete(Err(2));
                }
                return false;
            }
            if let Some(node_id) = streams.node_id(sharing.id) {
                let session = sessions.get(sharing.id).expect("active sharing session");
                if let Some(info) =
                    stream_info(node_id, session.source, session.layout, logical_size, scale)
                    && let Some(request) = sharing.request.take()
                {
                    request.complete(Ok(info));
                }
            }
            true
        });
        let monitor = CaptureSource::Output(OutputId::MIN);
        let mut sources: Vec<(CaptureSource, u64, String)> = self
            .pending
            .front()
            .filter(|request| request.allows_source(monitor))
            .and_then(|_| {
                sessions
                    .source_epoch(monitor)
                    .map(|epoch| vec![(monitor, epoch, self.output_label.clone())])
            })
            .unwrap_or_default();
        if let Some(request) = self.pending.front() {
            sources.extend(streams.window_sources().filter_map(|(id, epoch, label)| {
                let source = CaptureSource::Window(id);
                if sessions.source_epoch(source) != Some(epoch) {
                    return None;
                }
                request
                    .allows_source(source)
                    .then(|| (source, epoch, label.to_owned()))
            }));
        }
        self.snapshot.publish_if_changed(CaptureUiSnapshot {
            sources,
            failure: self.failure.clone(),
            starting: self.sharing.iter().filter_map(|(&id, sharing)| sharing.request.is_some().then_some(id)).collect(),
            pending: self
                .pending
                .front()
                .map(|r| (r.lease.id, application_label(&r.app_id))),
            sharing: self
                .sharing
                .iter()
                .map(|(&id, s)| (id, s.label.clone()))
                .collect(),
        });
    }
}
impl Drop for CapturePortal {
    fn drop(&mut self) {
        for request in &self.pending {
            request.lease.close();
        }
        for sharing in self.sharing.values() {
            sharing.lease.close();
        }
    }
}

pub(super) fn capture_layout(size: SizeI) -> Option<CaptureLayout> {
    let width = NonZeroU32::new(u32::try_from(size.width).ok()?)?;
    let height = NonZeroU32::new(u32::try_from(size.height).ok()?)?;
    CaptureLayout::rgba8(width, height, width.get().checked_mul(4)?)
}

// Resolve metadata at reply time: window layout and output scale may change after consent.
fn stream_info(
    node_id: u32,
    source: CaptureSource,
    layout: Option<CaptureLayout>,
    output_logical_size: SizeI,
    scale: crate::platform::contracts::ScaleFactor,
) -> Option<StreamInfo> {
    let size = match source {
        CaptureSource::Output(_) => output_logical_size,
        CaptureSource::Window(_) => {
            let layout = layout?;
            scale.logical_size(SizeI {
                width: i32::try_from(layout.width()).ok()?,
                height: i32::try_from(layout.height()).ok()?,
            })
        }
    };
    Some(StreamInfo {
        node_id,
        width: size.width,
        height: size.height,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_metadata_uses_current_window_layout_and_scale() {
        let source = CaptureSource::Window(crate::shell::WindowId::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));
        let scale = crate::platform::contracts::ScaleFactor::new(1.25).unwrap();
        let output = SizeI {
            width: 1920,
            height: 1080,
        };
        let initial = capture_layout(SizeI {
            width: 500,
            height: 250,
        });
        let resized = capture_layout(SizeI {
            width: 1000,
            height: 750,
        });
        let first = stream_info(42, source, initial, output, scale).unwrap();
        let current = stream_info(42, source, resized, output, scale).unwrap();
        assert_eq!((first.width, first.height), (400, 200));
        assert_eq!((current.width, current.height), (800, 600));
        assert_eq!(current.source, source);
        assert_eq!(current.node_id, 42);
        assert!(stream_info(42, source, None, output, scale).is_none());
        let monitor = stream_info(
            42,
            CaptureSource::Output(OutputId::MIN),
            resized,
            output,
            scale,
        )
        .unwrap();
        assert_eq!((monitor.width, monitor.height), (1920, 1080));
    }
}
