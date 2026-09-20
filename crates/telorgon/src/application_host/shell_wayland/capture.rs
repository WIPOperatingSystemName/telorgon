//! Owner-thread capture admission. Transport adapters never own authorization state.

use crate::shell::capture::{CaptureLayout, CaptureOptions, CaptureSource, CaptureStopReason};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RequesterId {
    Portal(u64),
    Direct(crate::compositor_wayland::ClientId),
}

/// Monotonic and never recycled, including after removal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SessionId(u64);
impl SessionId {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FrameTicket {
    session: SessionId,
    sequence: u64,
    revision: u64,
    generation: u64,
}
impl FrameTicket {
    pub fn session(self) -> SessionId {
        self.session
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionState {
    Requested,
    Authorized,
    Negotiating,
    Streaming,
    Stopped(CaptureStopReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CaptureError {
    Locked,
    Unavailable,
    Limit,
    UnknownSession,
    WrongRequester,
    InvalidState,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CaptureLimits {
    pub sessions: usize,
    pub sessions_per_requester: usize,
    pub max_dimension: u32,
    pub max_frame_rate: u32,
    pub buffers_per_session: usize,
    pub total_bytes: usize,
}

impl Default for CaptureLimits {
    fn default() -> Self {
        Self {
            sessions: 8,
            sessions_per_requester: 2,
            max_dimension: 8192,
            max_frame_rate: 60,
            buffers_per_session: 3,
            total_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub(super) struct CaptureSession {
    pub requester: RequesterId,
    pub source: CaptureSource,
    pub options: CaptureOptions,
    pub state: SessionState,
    pub layout: Option<CaptureLayout>,
    reserved_bytes: usize,
    generation: u64,
    retiring_generation: Option<(u64, usize)>,
    pending_frame: Option<FrameTicket>,
    next_sequence: u64,
    next_frame_ns: u64,
    last_revision: Option<u64>,
}

pub(super) struct CaptureSessions {
    limits: CaptureLimits,
    next_id: u64,
    locked: bool,
    sources: BTreeSet<CaptureSource>,
    source_epochs: BTreeMap<CaptureSource, u64>,
    next_source_epoch: u64,
    sessions: BTreeMap<SessionId, CaptureSession>,
    reserved_bytes: usize,
}

impl CaptureSessions {
    pub fn new(limits: CaptureLimits) -> Self {
        Self {
            limits,
            next_id: 1,
            locked: false,
            sources: BTreeSet::new(),
            source_epochs: BTreeMap::new(),
            next_source_epoch: 1,
            sessions: BTreeMap::new(),
            reserved_bytes: 0,
        }
    }

    pub fn publish_source(&mut self, source: CaptureSource) {
        if self.sources.contains(&source) {
            return;
        }
        let Some(next) = self.next_source_epoch.checked_add(1) else {
            return;
        };
        self.source_epochs.insert(source, self.next_source_epoch);
        self.next_source_epoch = next;
        self.sources.insert(source);
    }

    pub fn source_epoch(&self, source: CaptureSource) -> Option<u64> {
        self.source_epochs.get(&source).copied()
    }

    /// Reconcile against host truth, never a transport's cached window list.
    pub fn sync_sources(&mut self, sources: impl IntoIterator<Item = CaptureSource>) {
        let next: BTreeSet<_> = sources.into_iter().collect();
        for session in self.sessions.values_mut() {
            if !next.contains(&session.source) {
                Self::stop_session(session, CaptureStopReason::SourceUnavailable);
            }
        }
        self.sources.retain(|source| next.contains(source));
        self.source_epochs.retain(|source, _| next.contains(source));
        for source in next {
            self.publish_source(source);
        }
    }

    pub fn sources(&self) -> impl Iterator<Item = CaptureSource> + '_ {
        self.sources.iter().copied()
    }

    pub fn withdraw_source(&mut self, source: CaptureSource) {
        self.sources.remove(&source);
        self.source_epochs.remove(&source);
        for session in self.sessions.values_mut().filter(|s| s.source == source) {
            Self::stop_session(session, CaptureStopReason::SourceUnavailable);
        }
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
        if locked {
            for session in self.sessions.values_mut() {
                Self::stop_session(session, CaptureStopReason::SessionLocked);
            }
        }
    }

    pub fn request(
        &mut self,
        requester: RequesterId,
        source: CaptureSource,
        options: CaptureOptions,
    ) -> Result<SessionId, CaptureError> {
        if self.locked {
            return Err(CaptureError::Locked);
        }
        if !self.sources.contains(&source) {
            return Err(CaptureError::Unavailable);
        }
        if self.sessions.len() >= self.limits.sessions
            || self
                .sessions
                .values()
                .filter(|s| s.requester == requester)
                .count()
                >= self.limits.sessions_per_requester
            || options.max_frame_rate().get() > self.limits.max_frame_rate
        {
            return Err(CaptureError::Limit);
        }
        let next = self.next_id.checked_add(1).ok_or(CaptureError::Limit)?;
        let id = SessionId(self.next_id);
        self.next_id = next;
        self.sessions.insert(
            id,
            CaptureSession {
                requester,
                source,
                options,
                state: SessionState::Requested,
                layout: None,
                reserved_bytes: 0,
                generation: 0,
                retiring_generation: None,
                pending_frame: None,
                next_sequence: 1,
                next_frame_ns: 0,
                last_revision: None,
            },
        );
        Ok(id)
    }

    /// Called by the host chooser authority, not by a requesting application.
    pub fn authorize(&mut self, id: SessionId, allowed: bool) -> Result<(), CaptureError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(CaptureError::UnknownSession)?;
        if session.state != SessionState::Requested {
            return Err(CaptureError::InvalidState);
        }
        session.state = if allowed {
            SessionState::Authorized
        } else {
            SessionState::Stopped(CaptureStopReason::Denied)
        };
        Ok(())
    }

    pub fn negotiate(
        &mut self,
        id: SessionId,
        requester: RequesterId,
        layout: CaptureLayout,
    ) -> Result<(), CaptureError> {
        let bytes = self.layout_bytes(layout)?;
        let total = self
            .reserved_bytes
            .checked_add(bytes)
            .ok_or(CaptureError::Limit)?;
        if total > self.limits.total_bytes {
            return Err(CaptureError::Limit);
        }
        let session = self.owned(id, requester)?;
        if session.state != SessionState::Authorized {
            return Err(CaptureError::InvalidState);
        }
        session.layout = Some(layout);
        session.generation = 1;
        session.reserved_bytes = bytes;
        session.state = SessionState::Negotiating;
        self.reserved_bytes = total;
        Ok(())
    }

    /// Reserve a replacement generation without releasing storage still held by the GPU or
    /// consumer. The adapter must stop publishing the old generation before calling this, then
    /// retire its resources and renegotiate the transport before calling `started` again.
    /// Only one retiring generation is admitted, bounding repeated resize requests.
    pub fn renegotiate(
        &mut self,
        id: SessionId,
        requester: RequesterId,
        layout: CaptureLayout,
    ) -> Result<u64, CaptureError> {
        let bytes = self.layout_bytes(layout)?;
        let total = self
            .reserved_bytes
            .checked_add(bytes)
            .ok_or(CaptureError::Limit)?;
        if total > self.limits.total_bytes {
            return Err(CaptureError::Limit);
        }
        let session = self.owned(id, requester)?;
        if session.state != SessionState::Streaming || session.retiring_generation.is_some() {
            return Err(CaptureError::InvalidState);
        }
        let next = session
            .generation
            .checked_add(1)
            .ok_or(CaptureError::Limit)?;
        let retained = session
            .reserved_bytes
            .checked_add(bytes)
            .ok_or(CaptureError::Limit)?;
        let previous = session.generation;
        session.retiring_generation = Some((previous, session.reserved_bytes));
        session.generation = next;
        session.reserved_bytes = retained;
        session.layout = Some(layout);
        session.state = SessionState::Negotiating;
        session.last_revision = None;
        session.next_frame_ns = 0;
        self.reserved_bytes = total;
        Ok(previous)
    }

    /// The resource owner confirms that both GPU and transport uses of the old generation
    /// have ended. Frame completion alone is deliberately insufficient to release its budget.
    pub fn generation_retired(
        &mut self,
        id: SessionId,
        requester: RequesterId,
        generation: u64,
    ) -> Result<(), CaptureError> {
        let session = self.owned(id, requester)?;
        let Some((retiring, bytes)) = session.retiring_generation else {
            return Err(CaptureError::InvalidState);
        };
        if retiring != generation
            || session
                .pending_frame
                .is_some_and(|f| f.generation == generation)
        {
            return Err(CaptureError::InvalidState);
        }
        session.retiring_generation = None;
        session.reserved_bytes -= bytes;
        self.reserved_bytes -= bytes;
        Ok(())
    }

    fn layout_bytes(&self, layout: CaptureLayout) -> Result<usize, CaptureError> {
        // Nominal producer, transport, staging and target storage. Exact native alignment and
        // metadata accounting remains a separate release gate.
        if self.limits.buffers_per_session == 0
            || layout.width() > self.limits.max_dimension
            || layout.height() > self.limits.max_dimension
        {
            return Err(CaptureError::Limit);
        }
        self.limits
            .buffers_per_session
            .checked_mul(2)
            .and_then(|slots| slots.checked_add(2))
            .and_then(|slots| layout.byte_len().checked_mul(slots))
            .ok_or(CaptureError::Limit)
    }

    pub fn started(&mut self, id: SessionId, requester: RequesterId) -> Result<(), CaptureError> {
        let session = self.owned(id, requester)?;
        if session.state != SessionState::Negotiating || session.retiring_generation.is_some() {
            return Err(CaptureError::InvalidState);
        }
        session.state = SessionState::Streaming;
        Ok(())
    }

    pub fn has_pending_frame(&self) -> bool {
        self.sessions
            .values()
            .any(|session| session.pending_frame.is_some())
    }

    /// Admit at most one GPU capture across all transports, coalescing skipped source revisions.
    /// The host's revision must include cursor changes when the stream embeds its cursor.
    pub fn begin_frame(
        &mut self,
        id: SessionId,
        now_ns: u64,
        revision: u64,
    ) -> Result<Option<FrameTicket>, CaptureError> {
        let capture_busy = self.has_pending_frame();
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(CaptureError::UnknownSession)?;
        if session.state != SessionState::Streaming {
            return Err(CaptureError::InvalidState);
        }
        if capture_busy || now_ns < session.next_frame_ns || session.last_revision == Some(revision)
        {
            return Ok(None);
        }
        let next = session
            .next_sequence
            .checked_add(1)
            .ok_or(CaptureError::Limit)?;
        let ticket = FrameTicket {
            session: id,
            sequence: session.next_sequence,
            revision,
            generation: session.generation,
        };
        session.next_sequence = next;
        session.pending_frame = Some(ticket);
        let interval =
            1_000_000_000_u64.div_ceil(u64::from(session.options.max_frame_rate().get()));
        session.next_frame_ns = now_ns.saturating_add(interval);
        Ok(Some(ticket))
    }

    /// Complete once. False means the frame must not be delivered (failure or revoked session).
    /// GPU retirement is still required before calling this for cancelled work.
    pub fn finish_frame(
        &mut self,
        ticket: FrameTicket,
        success: bool,
    ) -> Result<bool, CaptureError> {
        let session = self
            .sessions
            .get_mut(&ticket.session)
            .ok_or(CaptureError::UnknownSession)?;
        if session.pending_frame != Some(ticket) {
            return Err(CaptureError::InvalidState);
        }
        session.pending_frame = None;
        let current_generation = ticket.generation == session.generation;
        if success && current_generation {
            session.last_revision = Some(ticket.revision);
        }
        Ok(success && current_generation && session.state == SessionState::Streaming)
    }

    pub fn stop(
        &mut self,
        id: SessionId,
        requester: RequesterId,
        reason: CaptureStopReason,
    ) -> Result<(), CaptureError> {
        Self::stop_session(self.owned(id, requester)?, reason);
        Ok(())
    }

    pub fn disconnect(&mut self, requester: RequesterId) {
        for session in self
            .sessions
            .values_mut()
            .filter(|s| s.requester == requester)
        {
            Self::stop_session(session, CaptureStopReason::RequesterDisconnected);
        }
    }

    /// Only the resource owner calls this after all GPU/consumer uses have retired.
    /// Stopping alone never releases memory accounting for still-live buffers.
    pub fn retired(&mut self, id: SessionId) -> Result<(), CaptureError> {
        let session = self.sessions.get(&id).ok_or(CaptureError::UnknownSession)?;
        if !matches!(session.state, SessionState::Stopped(_)) || session.pending_frame.is_some() {
            return Err(CaptureError::InvalidState);
        }
        self.reserved_bytes -= session.reserved_bytes;
        self.sessions.remove(&id);
        Ok(())
    }

    pub fn get(&self, id: SessionId) -> Option<&CaptureSession> {
        self.sessions.get(&id)
    }

    fn owned(
        &mut self,
        id: SessionId,
        requester: RequesterId,
    ) -> Result<&mut CaptureSession, CaptureError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(CaptureError::UnknownSession)?;
        if session.requester != requester {
            return Err(CaptureError::WrongRequester);
        }
        Ok(session)
    }

    fn stop_session(session: &mut CaptureSession, reason: CaptureStopReason) {
        if !matches!(session.state, SessionState::Stopped(_)) {
            session.state = SessionState::Stopped(reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{OutputId, WindowId};
    use std::num::NonZeroU32;

    fn setup() -> (CaptureSessions, CaptureSource, RequesterId) {
        let mut sessions = CaptureSessions::new(CaptureLimits::default());
        let source = CaptureSource::Output(OutputId::MIN);
        sessions.publish_source(source);
        (sessions, source, RequesterId::Portal(1))
    }

    fn layout() -> CaptureLayout {
        CaptureLayout::rgba8(
            NonZeroU32::new(1920).unwrap(),
            NonZeroU32::new(1080).unwrap(),
            7680,
        )
        .unwrap()
    }

    #[test]
    fn shared_gpu_admission_waits_for_cancelled_other_transport_to_retire() {
        let (mut engine, source, portal) = setup();
        let direct = RequesterId::Direct(crate::compositor_wayland::ClientId::from_raw(1).unwrap());
        let p = engine
            .request(portal, source, CaptureOptions::default())
            .unwrap();
        let d = engine
            .request(direct, source, CaptureOptions::default())
            .unwrap();
        for (id, requester) in [(p, portal), (d, direct)] {
            engine.authorize(id, true).unwrap();
            engine.negotiate(id, requester, layout()).unwrap();
            engine.started(id, requester).unwrap();
        }
        let ticket = engine.begin_frame(p, 0, 1).unwrap().unwrap();
        assert!(engine.has_pending_frame());
        assert_eq!(engine.begin_frame(d, 0, 1).unwrap(), None);
        engine
            .stop(p, portal, CaptureStopReason::Requested)
            .unwrap();
        assert_eq!(engine.begin_frame(d, 0, 1).unwrap(), None);
        assert_eq!(engine.retired(p), Err(CaptureError::InvalidState));
        assert!(!engine.finish_frame(ticket, true).unwrap());
        assert!(!engine.has_pending_frame());
        engine.retired(p).unwrap();
        // Failed admission did not consume the direct source revision or its pacing slot.
        let direct_ticket = engine.begin_frame(d, 0, 1).unwrap().unwrap();
        assert!(engine.finish_frame(direct_ticket, true).unwrap());
    }

    #[test]
    fn portal_and_direct_requester_numbers_do_not_share_authority_or_quotas() {
        let (mut engine, source, portal) = setup();
        let direct = RequesterId::Direct(crate::compositor_wayland::ClientId::from_raw(1).unwrap());
        let p = engine
            .request(portal, source, CaptureOptions::default())
            .unwrap();
        engine
            .request(portal, source, CaptureOptions::default())
            .unwrap();
        assert_eq!(
            engine.request(portal, source, CaptureOptions::default()),
            Err(CaptureError::Limit)
        );
        let d = engine
            .request(direct, source, CaptureOptions::default())
            .unwrap();
        assert_eq!(
            engine.stop(p, direct, CaptureStopReason::Requested),
            Err(CaptureError::WrongRequester)
        );
        assert_eq!(
            engine.stop(d, portal, CaptureStopReason::Requested),
            Err(CaptureError::WrongRequester)
        );
        engine.disconnect(portal);
        assert!(matches!(
            engine.get(p).unwrap().state,
            SessionState::Stopped(_)
        ));
        assert_eq!(engine.get(d).unwrap().state, SessionState::Requested);
    }

    #[test]
    fn resize_retires_old_gpu_and_consumer_storage_before_resuming_same_grant() {
        let (mut engine, source, owner) = setup();
        let id = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        engine.authorize(id, true).unwrap();
        engine.negotiate(id, owner, layout()).unwrap();
        engine.started(id, owner).unwrap();
        let old = engine.begin_frame(id, 0, 7).unwrap().unwrap();
        let replacement = CaptureLayout::rgba8(
            NonZeroU32::new(1280).unwrap(),
            NonZeroU32::new(720).unwrap(),
            5120,
        )
        .unwrap();
        let generation = engine.renegotiate(id, owner, replacement).unwrap();
        assert_eq!(
            engine.reserved_bytes,
            (layout().byte_len() + replacement.byte_len()) * 8
        );
        assert_eq!(engine.started(id, owner), Err(CaptureError::InvalidState));
        assert_eq!(
            engine.generation_retired(id, owner, generation),
            Err(CaptureError::InvalidState)
        );
        assert!(!engine.finish_frame(old, true).unwrap());
        // GPU completion must not release memory still retained by a transport consumer.
        assert_eq!(
            engine.reserved_bytes,
            (layout().byte_len() + replacement.byte_len()) * 8
        );
        assert_eq!(
            engine.generation_retired(id, owner, generation + 1),
            Err(CaptureError::InvalidState)
        );
        engine.generation_retired(id, owner, generation).unwrap();
        assert_eq!(
            engine.generation_retired(id, owner, generation),
            Err(CaptureError::InvalidState)
        );
        assert_eq!(engine.reserved_bytes, replacement.byte_len() * 8);
        engine.started(id, owner).unwrap();
        let next = engine.begin_frame(id, 0, 7).unwrap().unwrap();
        assert_ne!(old.generation, next.generation);
        assert!(engine.finish_frame(next, true).unwrap());
        assert_eq!(engine.get(id).unwrap().source, source);
        assert_eq!(engine.get(id).unwrap().requester, owner);
        assert_eq!(engine.get(id).unwrap().layout, Some(replacement));
    }

    #[test]
    fn resize_admission_is_atomic_and_revocation_cannot_restart_a_generation() {
        let (mut engine, source, owner) = setup();
        let id = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        engine.authorize(id, true).unwrap();
        engine.negotiate(id, owner, layout()).unwrap();
        engine.started(id, owner).unwrap();
        let bytes = engine.reserved_bytes;
        engine.limits.total_bytes = bytes;
        assert_eq!(
            engine.renegotiate(id, owner, layout()),
            Err(CaptureError::Limit)
        );
        assert_eq!(engine.get(id).unwrap().state, SessionState::Streaming);
        assert_eq!(engine.get(id).unwrap().generation, 1);
        assert_eq!(engine.reserved_bytes, bytes);
        engine.limits.total_bytes = usize::MAX;
        assert_eq!(
            engine.renegotiate(id, RequesterId::Portal(2), layout()),
            Err(CaptureError::WrongRequester)
        );
        let generation = engine.renegotiate(id, owner, layout()).unwrap();
        assert_eq!(
            engine.renegotiate(id, owner, layout()),
            Err(CaptureError::InvalidState)
        );
        engine.set_locked(true);
        engine.generation_retired(id, owner, generation).unwrap();
        assert_eq!(engine.started(id, owner), Err(CaptureError::InvalidState));
        engine.retired(id).unwrap();
        assert_eq!(engine.reserved_bytes, 0);
    }

    #[test]
    fn approval_cannot_be_bypassed_or_used_by_another_requester() {
        let (mut engine, source, owner) = setup();
        let id = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        assert_eq!(
            engine.negotiate(id, owner, layout()),
            Err(CaptureError::InvalidState)
        );
        engine.authorize(id, true).unwrap();
        assert_eq!(
            engine.negotiate(id, RequesterId::Portal(2), layout()),
            Err(CaptureError::WrongRequester)
        );
        engine.negotiate(id, owner, layout()).unwrap();
        engine.started(id, owner).unwrap();
        assert_eq!(engine.get(id).unwrap().state, SessionState::Streaming);
        assert_eq!(engine.get(id).unwrap().options, CaptureOptions::default());
    }

    #[test]
    fn source_remap_changes_capture_epoch_without_restarting_old_sessions() {
        let (mut engine, source, owner) = setup();
        let first = engine.source_epoch(source).unwrap();
        engine.sync_sources([source]);
        assert_eq!(engine.source_epoch(source), Some(first));
        let id = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        engine.withdraw_source(source);
        assert_eq!(engine.source_epoch(source), None);
        engine.publish_source(source);
        let remapped = engine.source_epoch(source).unwrap();
        assert_ne!(first, remapped);
        assert_eq!(engine.authorize(id, true), Err(CaptureError::InvalidState));
        engine.sync_sources([]);
        engine.sync_sources([source]);
        assert_ne!(engine.source_epoch(source), Some(remapped));
        engine.withdraw_source(source);
        engine.next_source_epoch = u64::MAX;
        engine.publish_source(source);
        assert_eq!(engine.source_epoch(source), None);
        assert_eq!(
            engine.request(owner, source, CaptureOptions::default()),
            Err(CaptureError::Unavailable)
        );
    }

    #[test]
    fn stopped_buffers_remain_charged_until_owner_retires_them() {
        let (mut engine, source, owner) = setup();
        engine.limits.total_bytes = layout().byte_len() * 8;
        let id = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        engine.authorize(id, true).unwrap();
        engine.negotiate(id, owner, layout()).unwrap();
        engine
            .stop(id, owner, CaptureStopReason::Requested)
            .unwrap();
        let next = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        engine.authorize(next, true).unwrap();
        assert_eq!(
            engine.negotiate(next, owner, layout()),
            Err(CaptureError::Limit)
        );
        assert_eq!(engine.started(id, owner), Err(CaptureError::InvalidState));
        engine.retired(id).unwrap();
        engine.negotiate(next, owner, layout()).unwrap();
        assert_ne!(id, next);
    }

    #[test]
    fn lock_revokes_all_states_and_unlock_does_not_resume_them() {
        let (mut engine, source, owner) = setup();
        let id = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        engine.set_locked(true);
        assert_eq!(
            engine.request(owner, source, CaptureOptions::default()),
            Err(CaptureError::Locked)
        );
        engine.set_locked(false);
        assert_eq!(engine.authorize(id, true), Err(CaptureError::InvalidState));
        engine
            .stop(id, owner, CaptureStopReason::Requested)
            .unwrap();
        assert_eq!(
            engine.get(id).unwrap().state,
            SessionState::Stopped(CaptureStopReason::SessionLocked)
        );
    }

    #[test]
    fn source_incarnations_do_not_inherit_authority() {
        let (mut engine, _, owner) = setup();
        let nz = |n| NonZeroU32::new(n).unwrap();
        let first = CaptureSource::Window(WindowId::new(nz(1), nz(1)));
        let replacement = CaptureSource::Window(WindowId::new(nz(1), nz(2)));
        engine.publish_source(first);
        let id = engine
            .request(owner, first, CaptureOptions::default())
            .unwrap();
        engine.authorize(id, true).unwrap();
        engine.withdraw_source(first);
        engine.publish_source(replacement);
        assert_eq!(
            engine.get(id).unwrap().state,
            SessionState::Stopped(CaptureStopReason::SourceUnavailable)
        );
        assert_eq!(
            engine.request(owner, first, CaptureOptions::default()),
            Err(CaptureError::Unavailable)
        );
    }

    #[test]
    fn denial_disconnect_and_quotas_do_not_leak_sessions() {
        let (mut engine, source, owner) = setup();
        engine.limits.sessions_per_requester = 1;
        let id = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        assert_eq!(
            engine.request(owner, source, CaptureOptions::default()),
            Err(CaptureError::Limit)
        );
        engine.authorize(id, false).unwrap();
        assert_eq!(engine.started(id, owner), Err(CaptureError::InvalidState));
        engine.retired(id).unwrap();
        let next = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        engine.disconnect(owner);
        assert_eq!(
            engine.get(next).unwrap().state,
            SessionState::Stopped(CaptureStopReason::RequesterDisconnected)
        );
        engine.retired(next).unwrap();
        assert!(engine.sessions.is_empty());
    }

    #[test]
    fn failed_admission_is_atomic_and_live_sessions_cannot_retire() {
        let (mut engine, source, owner) = setup();
        let id = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        engine.authorize(id, true).unwrap();
        engine.limits.total_bytes = 1;
        assert_eq!(
            engine.negotiate(id, owner, layout()),
            Err(CaptureError::Limit)
        );
        assert_eq!(engine.reserved_bytes, 0);
        assert!(engine.get(id).unwrap().layout.is_none());
        assert_eq!(engine.get(id).unwrap().state, SessionState::Authorized);
        assert_eq!(engine.retired(id), Err(CaptureError::InvalidState));
        engine.limits.total_bytes = usize::MAX;
        engine.limits.buffers_per_session = usize::MAX;
        assert_eq!(
            engine.negotiate(id, owner, layout()),
            Err(CaptureError::Limit)
        );
        assert_eq!(engine.reserved_bytes, 0);
    }

    #[test]
    fn exhausted_identity_space_never_recycles_a_session() {
        let (mut engine, source, owner) = setup();
        engine.next_id = u64::MAX;
        assert_eq!(
            engine.request(owner, source, CaptureOptions::default()),
            Err(CaptureError::Limit)
        );
        assert!(engine.sessions.is_empty());
    }

    #[test]
    fn host_reconciliation_removes_missing_sources_without_retargeting_sessions() {
        let (mut engine, source, owner) = setup();
        let id = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        engine.authorize(id, true).unwrap();
        let replacement = CaptureSource::Output(OutputId::from_raw(2).unwrap());
        engine.sync_sources([replacement]);
        assert_eq!(engine.sources().collect::<Vec<_>>(), vec![replacement]);
        assert_eq!(engine.get(id).unwrap().source, source);
        assert_eq!(
            engine.get(id).unwrap().state,
            SessionState::Stopped(CaptureStopReason::SourceUnavailable)
        );
        engine.sync_sources([source, replacement]);
        assert_eq!(
            engine.negotiate(id, owner, layout()),
            Err(CaptureError::InvalidState)
        );
    }

    #[test]
    fn scheduling_captures_initial_frame_coalesces_damage_and_retries_failure() {
        let (mut engine, source, owner) = setup();
        let id = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        engine.authorize(id, true).unwrap();
        engine.negotiate(id, owner, layout()).unwrap();
        engine.started(id, owner).unwrap();
        let first = engine.begin_frame(id, 0, 0).unwrap().unwrap();
        assert!(engine.begin_frame(id, 40_000_000, 2).unwrap().is_none());
        assert!(engine.finish_frame(first, true).unwrap());
        assert!(engine.begin_frame(id, 1, 2).unwrap().is_none());
        assert!(engine.begin_frame(id, 40_000_000, 0).unwrap().is_none());
        let newest = engine.begin_frame(id, 40_000_000, 3).unwrap().unwrap();
        assert!(!engine.finish_frame(newest, false).unwrap());
        assert!(engine.begin_frame(id, 80_000_000, 3).unwrap().is_some());
        assert_eq!(
            engine.finish_frame(first, true),
            Err(CaptureError::InvalidState)
        );
    }

    #[test]
    fn cancelled_gpu_work_cannot_deliver_or_release_budget_before_completion() {
        let (mut engine, source, owner) = setup();
        let id = engine
            .request(owner, source, CaptureOptions::default())
            .unwrap();
        engine.authorize(id, true).unwrap();
        engine.negotiate(id, owner, layout()).unwrap();
        engine.started(id, owner).unwrap();
        let frame = engine.begin_frame(id, 0, 1).unwrap().unwrap();
        engine.set_locked(true);
        assert_eq!(engine.retired(id), Err(CaptureError::InvalidState));
        assert!(!engine.finish_frame(frame, true).unwrap());
        assert_eq!(
            engine.finish_frame(frame, true),
            Err(CaptureError::InvalidState)
        );
        engine.retired(id).unwrap();
        assert_eq!(engine.reserved_bytes, 0);
    }
}
