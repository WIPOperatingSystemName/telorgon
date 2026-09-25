//! Owner-thread portal requests, consent decisions and stream readiness.

use super::capture::{CaptureSessions, RequesterId, SessionId, SessionState};
use super::capture_streams::CaptureStreams;
use super::renderer::ShellRenderer;
use crate::authoring::compose::{Signal, SignalWriter};
use crate::authoring::compose::portal::{
    CaptureDecision, ScreenCastPortalContext, ScreenCastPortalSnapshot,
    application_label,
};
use crate::foundation::SizeI;
use crate::host::application::{AppError, AppResult, declaration::RegisteredShellWidget};
use crate::integrations::portals::{Lease, Portal, StartRequest, StreamInfo};
use crate::shell::{
    OutputId,
    capture::{CaptureLayout, CaptureSource, CaptureStopReason},
};
use std::collections::{BTreeMap, VecDeque};
use std::num::NonZeroU32;
use std::sync::{
    Arc,
    mpsc::{Receiver, sync_channel},
};

struct Sharing {
    audio: Option<crate::host::application::share_audio::PortalAudioLease>,
    ids: Vec<SessionId>,
    requester: RequesterId,
    lease: Arc<Lease>,
    label: String,
    request: Option<StartRequest>,
    restore_keys: Vec<crate::integrations::portals::RestoreSource>,
    restored: bool,
}

pub(super) struct CapturePortal {
    audio: Option<crate::ShareAudio>,
    picker_process: Option<super::portal_picker_window::PickerProcess>,
    pub(super) previews: super::portal_previews::PortalPreviews,
    virtual_outputs: super::virtual_outputs::VirtualOutputs,
    portal: Portal,
    pending: VecDeque<StartRequest>,
    sharing: BTreeMap<u64, Sharing>,
    decisions: Receiver<CaptureDecision>,
    snapshot: SignalWriter<ScreenCastPortalSnapshot>,
    wake: Arc<dyn Fn() + Send + Sync>,
    failure: Option<String>,
    output_label: String,
    identities: super::capture_restore::Identities,
    permission_reply: Option<crate::integrations::portals::PermissionReply>,
    permissions_loaded: bool,
    permission_error: Option<String>,
    saved_permissions: Vec<crate::authoring::compose::portal::SavedCapturePermission>,
}
impl CapturePortal {
    pub fn new(
        wake: Arc<dyn Fn() + Send + Sync>,
        design: Option<crate::ScreenCastPortal>,
        socket: &std::path::Path,
        source_types: u32,
        activate_frontend: bool,
        output_label: String,
        output_restore_key: Option<String>,
    ) -> AppResult<(Self, Vec<RegisteredShellWidget>)> {
        let design = design.ok_or_else(|| {
            AppError::new("portal capture requires Compositor::screen_cast_portal")
        })?;
        design.validate()?;
        let identities =
            super::capture_restore::Identities::new(output_restore_key).map_err(AppError::new)?;
        let portal =
            Portal::start(wake.clone(), source_types, activate_frontend).map_err(AppError::new)?;
        let (signal, snapshot) = Signal::new(ScreenCastPortalSnapshot::default());
        let (send, decisions) = sync_channel(16);
        let ui = ScreenCastPortalContext {
            snapshot: signal,
            decisions: send,
            wake: wake.clone(),
        };
        let surfaces = design.compose(ui.clone())?;
        let picker_process = surfaces.window.map(|config| {
            super::portal_picker_window::PickerProcess::start(config, ui, socket)
                .map_err(|error| AppError::new(error.to_string()))
        }).transpose()?;
        let widgets = surfaces.widgets;
        Ok((
            Self {
                audio: surfaces.audio,
                picker_process: picker_process,
                previews: Default::default(),
                virtual_outputs: Default::default(),
                portal,
                pending: VecDeque::new(),
                sharing: BTreeMap::new(),
                decisions,
                snapshot,
                wake,
                failure: None,
                output_label,
                identities,
                permission_reply: None,
                permissions_loaded: false,
                permission_error: None,
                saved_permissions: Vec::new(),
            },
            widgets,
        ))
    }

    pub fn virtual_sources(&self) -> impl Iterator<Item = CaptureSource> + '_ {
        self.virtual_outputs
            .iter()
            .map(|(id, _)| CaptureSource::VirtualOutput(id))
    }

    pub fn sync_virtual_capture(&self, streams: &mut CaptureStreams) {
        streams.refresh_virtual_outputs(&self.virtual_outputs);
    }

    pub fn sync_virtual_membership(
        &mut self,
        compositor: &mut crate::integrations::wayland::compositor::NativeCompositor<'_>,
        windows: &mut BTreeMap<super::WaylandSurfaceId, super::ClientWindow>,
    ) -> AppResult<bool> {
        self.virtual_outputs.sync_membership(compositor, windows)
    }

    pub fn append_virtual_previews(
        &self,
        layers: &mut Vec<super::scene::ShellLayer>,
        widgets: &[super::WidgetLayer],
        locked: bool,
    ) {
        for widget in widgets {
            let Some(index) = layers
                .iter()
                .position(|layer| layer.key == super::scene::ShellLayerKey::Widget(widget.id))
            else {
                continue;
            };
            let previews = widget.virtual_preview_layers(layers, &self.virtual_outputs, locked);
            // Insert directly above its widget, preserving later overlays and the cursor.
            layers.splice(index + 1..index + 1, previews);
        }
    }

    pub fn virtual_fullscreen(
        &mut self,
        window: &super::ClientWindow,
        fullscreen: bool,
        output: Option<u32>,
    ) -> AppResult<Option<SizeI>> {
        let Some(id) = window.desktop_id else {
            return Ok(None);
        };
        let result = self.virtual_outputs.fullscreen(
            id,
            fullscreen,
            output,
            (window.position, window.requested_size),
        );
        if let Err(error) = &result {
            self.failure = Some(error.to_string());
        }
        (self.wake)();
        result
    }

    pub fn sync_virtual_geometry(
        &mut self,
        windows: &mut BTreeMap<super::WaylandSurfaceId, super::ClientWindow>,
        scheduler: &mut super::ConfigureScheduler,
    ) -> bool {
        self.virtual_outputs.sync_geometry(windows, scheduler)
    }

    pub fn refresh_virtual_outputs(
        &mut self,
        windows: &BTreeMap<super::WaylandSurfaceId, super::ClientWindow>,
        layers: &[super::scene::ShellLayer],
        scale: crate::platform::contracts::ScaleFactor,
    ) -> bool {
        match self
            .virtual_outputs
            .refresh_from_host(windows, layers, scale)
        {
            Ok(changed) => changed,
            Err(error) => {
                let message = format!("Virtual display unavailable: {error}");
                if self.failure.as_ref() != Some(&message) {
                    self.failure = Some(message);
                    (self.wake)();
                }
                false
            }
        }
    }

    pub fn wants_window_sources(&self) -> bool {
        self.pending
            .iter()
            .any(|request| request.source_types & 2 != 0 && !request.lease.closed())
    }

    pub fn revoke_cancelled(&self, sessions: &mut CaptureSessions) {
        for sharing in self.sharing.values() {
            if sharing.lease.closed() {
                stop_group(
                    sessions,
                    &sharing.ids,
                    sharing.requester,
                    CaptureStopReason::Requested,
                );
            }
        }
    }

    pub fn poll<'display>(
        &mut self,
        sessions: &mut CaptureSessions,
        streams: &mut CaptureStreams,
        renderer: &ShellRenderer,
        physical_size: SizeI,
        logical_size: SizeI,
        scale: crate::platform::contracts::ScaleFactor,
        locked: bool,
        compositor: &mut crate::integrations::wayland::compositor::NativeCompositor<'display>,
        display: &'display crate::integrations::wayland::server::Display,
        windows: &BTreeMap<super::WaylandSurfaceId, super::ClientWindow>,
    ) {
        self.sync_virtual_capture(streams);
        self.identities
            .observe_output(sessions.source_epoch(CaptureSource::Output(OutputId::MIN)));
        if let Some(reply) = &self.permission_reply {
            let result = match reply.try_recv() {
                Ok(result) => Some(result),
                Err(async_channel::TryRecvError::Closed) => {
                    Some(Err("permission worker stopped".into()))
                }
                Err(async_channel::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.permission_reply = None;
                match result {
                    Ok(entries) => {
                        self.permissions_loaded = true;
                        self.permission_error = None;
                        self.saved_permissions = entries
                            .into_iter()
                            .map(|(app_id, grants)| {
                                crate::authoring::compose::portal::SavedCapturePermission {
                                    app_id,
                                    grants,
                                }
                            })
                            .collect()
                    }
                    Err(error) => {
                        self.permission_error =
                            Some(format!("Saved permissions could not be updated: {error}"))
                    }
                }
            }
        }
        while let Some(error) = self.portal.try_failure() {
            self.failure = Some(error);
        }
        let previous_front = self.pending.front().map(|r| r.lease.id);
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
        let mut automatic = None;
        if let Some(request) = self.pending.front_mut() {
            if previous_front == Some(request.lease.id) && !locked && !request.lease.closed() {
                if let Some(grant) = &request.restore {
                    let monitor = CaptureSource::Output(OutputId::MIN);
                    let candidates = sessions
                        .source_epoch(monitor)
                        .map(|epoch| (monitor, epoch))
                        .into_iter()
                        .chain(streams.virtual_sources().filter_map(|(id, _)| {
                            let source = CaptureSource::VirtualOutput(id);
                            sessions.source_epoch(source).map(|epoch| (source, epoch))
                        }))
                        .chain(streams.window_sources().filter_map(|(id, epoch, _)| {
                            let source = CaptureSource::Window(id);
                            (sessions.source_epoch(source) == Some(epoch))
                                .then_some((source, epoch))
                        }));
                    match self.identities.resolve(&grant.sources, candidates) {
                        Some(selected) => {
                            automatic =
                                Some(CaptureDecision::ApproveRestored(request.lease.id, selected))
                        }
                        None => request.discard_restore(), // Missing/ambiguous source returns to ordinary consent.
                    }
                }
            }
        }
        while let Some(decision) = automatic.take().or_else(|| self.decisions.try_recv().ok()) {
            let decision = match decision {
                CaptureDecision::Approve(id, source, epoch) => {
                    CaptureDecision::ApproveMany(id, vec![(source, epoch)])
                }
                other => other,
            };
            match decision {
                CaptureDecision::ApproveWithAudio(id, _)
                | CaptureDecision::ApproveMany(id, _)
                | CaptureDecision::ApproveRemembered(id, _)
                | CaptureDecision::ApproveRestored(id, _)
                | CaptureDecision::Deny(id)
                    if self
                        .pending
                        .front()
                        .is_some_and(|request| request.lease.id == id) =>
                {
                    let request = self.pending.pop_front().unwrap();
                    let selected = match &decision {
                        CaptureDecision::ApproveWithAudio(_, selected)
                        | CaptureDecision::ApproveMany(_, selected)
                        | CaptureDecision::ApproveRemembered(_, selected)
                        | CaptureDecision::ApproveRestored(_, selected) => selected.as_slice(),
                        _ => &[],
                    };
                    let remember = matches!(decision, CaptureDecision::ApproveRemembered(..));
                    let restored = matches!(decision, CaptureDecision::ApproveRestored(..));
                    if locked
                        || (remember && !matches!(request.persistence, 1 | 2))
                        || request.lease.closed()
                        || !valid_selection(selected, request.multiple, |source, epoch| {
                            request.allows_source(source)
                                && sessions.source_epoch(source) == Some(epoch)
                        })
                    {
                        if !locked && !request.lease.closed() && !selected.is_empty() {
                            self.failure = Some(
                                "That selection is no longer available. Please share again.".into(),
                            );
                        }
                        request.lease.close();
                        request.complete(Err(1));
                        continue;
                    }
                    let restore_keys = if remember || restored {
                        let keys: Option<Vec<_>> = selected
                            .iter()
                            .map(|&(source, epoch)| self.identities.key(source, epoch))
                            .collect();
                        let Some(keys) = keys else {
                            self.failure = Some(
                                "That source cannot be remembered. Please share again.".into(),
                            );
                            request.lease.close();
                            request.complete(Err(2));
                            continue;
                        };
                        keys
                    } else {
                        Vec::new()
                    };
                    if restored
                        && !request
                            .restore
                            .as_ref()
                            .is_some_and(|g| g.sources == restore_keys)
                    {
                        request.lease.close();
                        request.complete(Err(2));
                        continue;
                    }
                    let audio = if matches!(decision, CaptureDecision::ApproveWithAudio(..)) {
                        let started = self.audio.as_ref().ok_or_else(|| "Screen-share audio is not configured".to_owned())
                            .and_then(|policy| crate::host::application::share_audio::PortalAudioLease::start(policy,
                                crate::PortalAudioRequest { request: id, application_id: request.app_id.clone(), sources: selected.to_vec() }));
                        match started {
                            Ok(audio) => Some(audio),
                            Err(error) => { self.failure = Some(error); request.lease.close(); request.complete(Err(2)); continue; }
                        }
                    } else { None };
                    // Validate every extent before creating any streams. Existing capture admission
                    // accounts all members globally, including stopped buffers awaiting retirement.
                    let layouts: Option<Vec<_>> = selected
                        .iter()
                        .map(|&(source, epoch)| {
                            let layout = match source {
                                CaptureSource::Output(id) if id == OutputId::MIN => {
                                    capture_layout(physical_size)
                                }
                                CaptureSource::Window(id) => streams.window_layout(id, epoch),
                                CaptureSource::VirtualOutput(id) => streams.virtual_layout(id),
                                _ => None,
                            }?;
                            Some((source, layout))
                        })
                        .collect();
                    let mut ids = Vec::with_capacity(selected.len());
                    let started = layouts
                        .ok_or_else(|| AppError::new("invalid capture extent"))
                        .and_then(|layouts| {
                            for (source, layout) in layouts {
                                match streams.begin_approved(
                                    sessions,
                                    renderer,
                                    RequesterId::Portal(request.requester),
                                    source,
                                    request.options,
                                    layout,
                                    self.wake.clone(),
                                ) {
                                    Ok(id) => ids.push(id),
                                    Err(error) => {
                                        stop_group(
                                            sessions,
                                            &ids,
                                            RequesterId::Portal(request.requester),
                                            CaptureStopReason::StreamFailed,
                                        );
                                        return Err(error);
                                    }
                                }
                            }
                            Ok(ids)
                        });
                    match started {
                        Ok(ids) => {
                            self.failure = None;
                            self.sharing.insert(
                                request.lease.id,
                                Sharing {
                                    audio,
                                    ids,
                                    requester: RequesterId::Portal(request.requester),
                                    lease: request.lease.clone(),
                                    label: application_label(&request.app_id),
                                    request: Some(request),
                                    restore_keys,
                                    restored,
                                },
                            );
                        }
                        Err(error) => {
                            eprintln!("telorgon-capture: {error}");
                            self.failure =
                                Some("Could not start screen sharing. Try sharing again.".into());
                            request.lease.close();
                            request.complete(Err(2));
                        }
                    }
                }
                CaptureDecision::RefreshPermissions | CaptureDecision::ForgetApplication(_) => {
                    if self.permission_reply.is_some() {
                        self.permission_error = Some("A saved-permission operation is still pending. Try again when it finishes.".into());
                    } else {
                        let app = match decision {
                            CaptureDecision::ForgetApplication(app) => Some(app),
                            _ => None,
                        };
                        match self.portal.manage_permissions(app, self.wake.clone()) {
                            Ok(reply) => {
                                self.permissions_loaded = false;
                                self.permission_error = None;
                                self.permission_reply = Some(reply);
                            }
                            Err(error) => {
                                self.permissions_loaded = false;
                                self.permission_error =
                                    Some(format!("Saved permissions could not be updated: {error}"))
                            }
                        }
                    }
                }
                CaptureDecision::RouteVirtualDisplay(id, routes) => {
                    if locked
                        || routes.iter().any(|(id, _)| {
                            !windows.values().any(|window| {
                                window.desktop_id == Some(*id)
                                    && window.backend.is_some()
                                    && !window.minimized
                                    && window.presentation.revision != 0
                            })
                        })
                    {
                        self.failure = Some("Virtual display routing requires an unlocked session and mapped windows.".into());
                    } else if let Err(error) = self.virtual_outputs.route(id, routes) {
                        self.failure = Some(error.to_string());
                    }
                    self.sync_virtual_capture(streams);
                    (self.wake)();
                }
                CaptureDecision::CreateVirtualDisplay(config) => {
                    if locked {
                        self.failure =
                            Some("Unlock the session before creating a virtual display.".into());
                    } else if let Err(error) = self.virtual_outputs.create(
                        compositor,
                        display,
                        config.label,
                        SizeI {
                            width: config.width as i32,
                            height: config.height as i32,
                        },
                        NonZeroU32::new(config.frame_rate).unwrap(),
                        crate::foundation::PointI {
                            x: self
                                .virtual_outputs
                                .iter()
                                .map(|(_, output)| {
                                    output
                                        .position
                                        .x
                                        .saturating_add(output.layout.width() as i32)
                                })
                                .max()
                                .unwrap_or(logical_size.width)
                                .max(logical_size.width),
                            y: 0,
                        },
                    ) {
                        self.failure = Some(error.to_string());
                    }
                    self.sync_virtual_capture(streams);
                    (self.wake)();
                }
                CaptureDecision::RemoveVirtualDisplay(id) => {
                    sessions.withdraw_source(CaptureSource::VirtualOutput(id));
                    if let Err(error) = self.virtual_outputs.remove(compositor, id) {
                        self.failure = Some(error.to_string());
                    }
                    self.sync_virtual_capture(streams);
                    (self.wake)();
                }
                CaptureDecision::DismissFailure => self.failure = None,
                CaptureDecision::StopAndForget(id) => {
                    if let Some(sharing) = self.sharing.get(&id) {
                        self.portal.forget(&sharing.lease);
                    }
                }
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
            let audio_ready = sharing.audio.as_mut().map_or(Ok(true), |audio| audio.poll());
            if let Err(error) = &audio_ready { self.failure = Some(error.clone()); }
            let active = audio_ready.is_ok() && sharing.ids.iter().all(|&id| {
                sessions
                    .get(id)
                    .is_some_and(|s| !matches!(s.state, SessionState::Stopped(_)))
            });
            if !active || sharing.lease.closed() {
                if !sharing.lease.closed() && audio_ready.is_ok() {
                    self.failure = Some(
                        "Screen sharing stopped because a source or stream became unavailable."
                            .into(),
                    );
                }
                sharing.lease.close();
                stop_group(
                    sessions,
                    &sharing.ids,
                    sharing.requester,
                    CaptureStopReason::StreamFailed,
                );
                if let Some(request) = sharing.request.take() {
                    request.complete(Err(2));
                }
                return false;
            }
            if sharing.request.is_some() && audio_ready == Ok(true) {
                let ready: Option<Vec<_>> = sharing
                    .ids
                    .iter()
                    .map(|&id| {
                        let node_id = streams.node_id(id)?;
                        let session = sessions.get(id)?;
                        stream_info(node_id, session.source, session.layout, logical_size, scale)
                    })
                    .collect();
                if let Some(infos) = ready {
                    eprintln!("telorgon-capture: portal Start response ready for {} stream(s)", infos.len());
                    let request = sharing.request.take().unwrap();
                    if sharing.restored {
                        request.complete_restored(infos, std::mem::take(&mut sharing.restore_keys));
                    } else if sharing.restore_keys.is_empty() {
                        request.complete(Ok(infos));
                    } else {
                        request
                            .complete_persistent(infos, std::mem::take(&mut sharing.restore_keys));
                    }
                }
            }
            true
        });
        let picker_pid = self.picker_process.as_ref().and_then(|picker| picker.process_id());
        let picker_windows: std::collections::BTreeSet<_> = windows
            .iter()
            .filter_map(|(surface, window)| {
                if picker_pid.is_some() && compositor.surface_process_id(*surface) == picker_pid {
                    window.desktop_id
                } else {
                    None
                }
            })
            .collect();
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
            sources.extend(streams.virtual_sources().filter_map(|(id, label)| {
                let source = CaptureSource::VirtualOutput(id);
                let epoch = sessions.source_epoch(source)?;
                request
                    .allows_source(source)
                    .then(|| (source, epoch, label.to_owned()))
            }));
            sources.extend(streams.window_sources().filter_map(|(id, epoch, label)| {
                let source = CaptureSource::Window(id);
                if picker_windows.contains(&id) || sessions.source_epoch(source) != Some(epoch) {
                    return None;
                }
                request
                    .allows_source(source)
                    .then(|| (source, epoch, label.to_owned()))
            }));
        }
        if self
            .pending
            .front()
            .is_some_and(|r| r.restore.is_some() && previous_front != Some(r.lease.id))
        {
            // Discovery uses the pending queue before this poll. Wake when a restored request
            // becomes front so its full source set is available even on an otherwise idle host.
            (self.wake)();
        }
        let preview_request = self.pending.front()
            .filter(|r| r.restore.is_none() && !r.lease.closed() && !locked)
            .map(|r|r.lease.id);
        let excluded = windows.keys().filter(|surface| picker_pid.is_some() && compositor.surface_process_id(**surface) == picker_pid)
            .map(|surface|surface.get()).collect();
        self.previews.sync(preview_request, &sources, excluded, sessions);
        self.snapshot.publish_if_changed(ScreenCastPortalSnapshot {
            multiple: self.pending.front().is_some_and(|r| r.multiple),
            audio_available: !locked && self.pending.front().is_some_and(|request|
                request.restore.is_none() && self.audio.as_ref().is_some_and(|policy| policy.available(request.lease.id, &request.app_id))),
            source_types: self.pending.front().map_or(0, |r| r.source_types),
            virtual_controls: self
                .pending
                .front()
                .is_some_and(|r| r.source_types & 4 != 0),
            virtual_windows: windows
                .values()
                .filter(|window| {
                    window.backend.is_some()
                        && !window.minimized
                        && window.presentation.revision != 0
                })
                .filter_map(|window| {
                    window.desktop_id.map(|id| {
                        (
                            id,
                            application_label(
                                window
                                    .frame_title
                                    .as_deref()
                                    .unwrap_or(&window.application_identity),
                            ),
                        )
                    })
                })
                .take(256)
                .collect(),
            virtual_displays: self
                .virtual_outputs
                .iter()
                .map(
                    |(id, output)| crate::authoring::compose::portal::VirtualDisplaySnapshot {
                        id,
                        label: output.label.clone(),
                        width: output.layout.width(),
                        height: output.layout.height(),
                        frame_rate: output.frame_rate.get(),
                        windows: output.routes().to_vec(),
                    },
                )
                .collect(),
            saved_permissions: self.saved_permissions.clone(),
            permissions_pending: self.permission_reply.is_some(),
            permissions_loaded: self.permissions_loaded,
            permission_error: self.permission_error.clone(),
            can_remember: self
                .pending
                .front()
                .is_some_and(|r| matches!(r.persistence, 1 | 2)),
            remember_for_application: self.pending.front().is_some_and(|r| r.persistence == 1),
            remembered: self
                .sharing
                .iter()
                .filter_map(|(&id, s)| s.lease.has_saved_grant().then_some(id))
                .collect(),
            sources,
            previews: self.previews.frames(),
            failure: self.failure.clone(),
            starting: self
                .sharing
                .iter()
                .filter_map(|(&id, sharing)| sharing.request.is_some().then_some(id))
                .collect(),
            pending: self
                .pending
                .front()
                .filter(|r| r.restore.is_none())
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

fn stop_group(
    sessions: &mut CaptureSessions,
    ids: &[SessionId],
    requester: RequesterId,
    reason: CaptureStopReason,
) {
    for &id in ids {
        let _ = sessions.stop(id, requester, reason);
    }
}

fn valid_selection(
    selected: &[(CaptureSource, u64)],
    multiple: bool,
    mut available: impl FnMut(CaptureSource, u64) -> bool,
) -> bool {
    !selected.is_empty()
        && selected.len()
            <= if multiple {
                crate::integrations::portals::MAX_STREAMS
            } else {
                1
            }
        && selected
            .iter()
            .enumerate()
            .all(|(index, &(source, epoch))| {
                !selected[..index].iter().any(|&(other, _)| other == source)
                    && available(source, epoch)
            })
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
        CaptureSource::VirtualOutput(_) => {
            let layout = layout?;
            SizeI {
                width: i32::try_from(layout.width()).ok()?,
                height: i32::try_from(layout.height()).ok()?,
            }
        }
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
    fn group_stop_revokes_pending_frames_and_preserves_other_requesters() {
        use super::super::capture::CaptureLimits;
        use crate::shell::capture::CaptureOptions;
        let mut sessions = CaptureSessions::new(CaptureLimits::default());
        let source = CaptureSource::Output(OutputId::MIN);
        sessions.publish_source(source);
        let requester = RequesterId::Portal(1);
        let layout = capture_layout(SizeI {
            width: 8,
            height: 8,
        })
        .unwrap();
        let mut ids = Vec::new();
        for _ in 0..3 {
            let id = sessions
                .request(requester, source, CaptureOptions::default())
                .unwrap();
            sessions.authorize(id, true).unwrap();
            sessions.negotiate(id, requester, layout).unwrap();
            sessions.started(id, requester).unwrap();
            ids.push(id);
        }
        let other = sessions
            .request(RequesterId::Portal(2), source, CaptureOptions::default())
            .unwrap();
        let pending = sessions.begin_frame(ids[0], 0, 1).unwrap().unwrap();
        // A member may already have failed before group cleanup runs.
        sessions
            .stop(ids[1], requester, CaptureStopReason::SourceUnavailable)
            .unwrap();
        stop_group(
            &mut sessions,
            &ids,
            requester,
            CaptureStopReason::StreamFailed,
        );
        assert!(!sessions.finish_frame(pending, true).unwrap());
        for id in ids {
            assert!(matches!(
                sessions.get(id).unwrap().state,
                SessionState::Stopped(_)
            ));
        }
        assert_eq!(sessions.get(other).unwrap().state, SessionState::Requested);
    }

    #[test]
    fn group_consent_rejects_duplicates_stale_epochs_and_single_source_escalation() {
        let a = CaptureSource::Output(OutputId::MIN);
        let b = CaptureSource::Output(OutputId::from_raw(2).unwrap());
        let available = |source, epoch| (source == a && epoch == 1) || (source == b && epoch == 2);
        assert!(valid_selection(&[(a, 1), (b, 2)], true, available));
        assert!(!valid_selection(&[(a, 1), (b, 2)], false, available));
        assert!(!valid_selection(&[(a, 1), (a, 1)], true, available));
        assert!(!valid_selection(&[(a, 1), (b, 3)], true, available));
        assert!(!valid_selection(&[], true, available));
        assert!(!valid_selection(&vec![(a, 1); 9], true, |_, _| true));
    }

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
