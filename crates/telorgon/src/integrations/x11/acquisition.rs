//! Asynchronous selection acquisition. A queued SetSelectionOwner is not evidence
//! of ownership; both its checked completion and matching owner query are required.
use super::selection::{Change, Owner, Ownership, Ownerships, Selection, Snapshot};
use super::{
    Error, Result,
    requests::{Completion, Importance, ReplyKind, RequestId, Requests},
    transport::Transport,
};
use std::time::Instant;
use x11rb_protocol::{
    protocol::xproto,
    x11_utils::{Request, TryParse},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Pending,
    Acquired,
    Rejected,
    Failed,
    Cancelled,
}
/// Snapshot confirmed by the server query, not a lease against later ownership loss.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Confirmed {
    pub generation: u64,
    pub selection: u32,
    pub owner: u32,
    pub timestamp: u32,
}
pub struct Acquisition {
    set: RequestId,
    query: Option<RequestId>,
    selection: u32,
    owner: u32,
    timestamp: u32,
    deadline: Instant,
    checked: bool,
    verified: bool,
    state: State,
}

/// Couples one native source's proxy acquisition to its ledger publication.
/// The bridge maps the selection atom to Selection when constructing this object.
pub struct Publication {
    acquisition: Acquisition,
    selection: Selection,
    source: u64,
    published: Option<Ownership>,
    baseline: (u64, Option<Snapshot>),
}
pub enum PublicationUpdate {
    Unchanged,
    Published(Change),
    Revoked(Snapshot),
}
pub struct PublicationDispatch {
    pub handled: bool,
    pub update: PublicationUpdate,
    /// Report locally after processing update; cleanup must not be skipped on error.
    pub error: Option<Error>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyEvent {
    Unrelated,
    AwaitConfirmation,
    Mirrored,
    Stale,
}
impl Publication {
    /// Decode a notification on the host's subscription window before classifying
    /// it. Events from another generation never enter this publication's policy.
    pub fn proxy_packet(
        &self,
        generation: u64,
        first_event: u8,
        watch_window: u32,
        bytes: &[u8],
        owners: &Ownerships,
    ) -> Result<ProxyEvent> {
        if generation != self.acquisition.set.generation || owners.generation() != generation {
            return Ok(ProxyEvent::Unrelated);
        }
        let Some(event) = super::selection::owner_event(
            bytes,
            first_event,
            watch_window,
            self.acquisition.selection,
        )?
        else {
            return Ok(ProxyEvent::Unrelated);
        };
        Ok(self.proxy_event(
            generation,
            event.selection,
            event.owner,
            event.selection_timestamp,
            owners,
        ))
    }
    /// Classify an authenticated XFixes owner notification routed by generation.
    /// `selection_timestamp` is the selection acquisition timestamp, not the
    /// notification's general event time. Local echoes never become X11 sources.
    pub fn proxy_event(
        &self,
        generation: u64,
        selection: u32,
        owner: u32,
        selection_timestamp: u32,
        owners: &Ownerships,
    ) -> ProxyEvent {
        if generation != self.acquisition.set.generation
            || owners.generation() != generation
            || selection != self.acquisition.selection
            || owner != self.acquisition.owner
        {
            return ProxyEvent::Unrelated;
        }
        if selection_timestamp != self.acquisition.timestamp {
            return ProxyEvent::Stale;
        }
        if let Some(token) = self.published {
            return if self.acquisition.state == State::Acquired && owners.is_current(token) {
                ProxyEvent::Mirrored
            } else {
                ProxyEvent::Stale
            };
        }
        if matches!(self.acquisition.state, State::Pending | State::Acquired)
            && self.baseline == (owners.revision(self.selection), owners.get(self.selection))
        {
            ProxyEvent::AwaitConfirmation
        } else {
            ProxyEvent::Stale
        }
    }
    /// Cancel and revoke together for lock/teardown; process the returned snapshot
    /// before discarding this publication or closing its readiness sources.
    pub fn cancel(&mut self, owners: &mut Ownerships) -> Result<PublicationUpdate> {
        if owners.generation() != self.acquisition.set.generation {
            return Err(Error(
                "selection publication server generation mismatch".into(),
            ));
        }
        self.acquisition.cancel();
        self.synchronize(owners)
    }
    /// Dispatch and synchronize even on acquisition errors, returning any revoked
    /// snapshot alongside the error so the host can close its offers/transfers.
    pub fn completion(
        &mut self,
        completion: &Completion,
        owners: &mut Ownerships,
    ) -> Result<PublicationDispatch> {
        if owners.generation() != self.acquisition.set.generation {
            return Err(Error(
                "selection publication server generation mismatch".into(),
            ));
        }
        let result = self.acquisition.completion(completion);
        let update = self.synchronize(owners)?;
        let (handled, error) = match result {
            Ok(handled) => (handled, None),
            Err(error) => (true, Some(error)),
        };
        Ok(PublicationDispatch {
            handled,
            update,
            error,
        })
    }
    pub fn new(
        acquisition: Acquisition,
        selection: Selection,
        source: u64,
        owners: &Ownerships,
    ) -> Result<Self> {
        if source == 0 || owners.generation() != acquisition.set.generation {
            return Err(Error("invalid native selection source".into()));
        }
        Ok(Self {
            acquisition,
            selection,
            source,
            published: None,
            baseline: (owners.revision(selection), owners.get(selection)),
        })
    }
    pub fn acquisition_mut(&mut self) -> &mut Acquisition {
        &mut self.acquisition
    }
    /// Call after every acquisition completion, including errors, and cancellation.
    /// Returned revoked/replaced snapshots identify offers/transfers to tear down
    /// before exposing the new selection to clients. Repeated calls are idempotent.
    pub fn synchronize(&mut self, owners: &mut Ownerships) -> Result<PublicationUpdate> {
        if owners.generation() != self.acquisition.set.generation {
            return Err(Error(
                "selection publication server generation mismatch".into(),
            ));
        }
        if self.published.is_none()
            && self.baseline != (owners.revision(self.selection), owners.get(self.selection))
        {
            self.acquisition.cancel();
            return Ok(PublicationUpdate::Unchanged);
        }
        if self.acquisition.state == State::Acquired {
            if self.published.is_some() {
                return Ok(PublicationUpdate::Unchanged);
            }
            let change = owners.replace(self.selection, Owner::Native(self.source))?;
            self.published = Some(change.current.ownership);
            return Ok(PublicationUpdate::Published(change));
        }
        if self.acquisition.state != State::Pending {
            if let Some(token) = self.published.take() {
                if let Some(old) = owners.revoke(token) {
                    return Ok(PublicationUpdate::Revoked(old));
                }
            }
        }
        Ok(PublicationUpdate::Unchanged)
    }
}
impl Acquisition {
    /// The caller supplies a compositor-owned X window and an event-derived
    /// nonzero server timestamp. No ownership ledger is published by this method.
    pub fn begin(
        selection: u32,
        owner: u32,
        timestamp: u32,
        transport: &mut Transport,
        requests: &mut Requests,
        deadline: Instant,
    ) -> Result<Self> {
        if selection == 0 || owner == 0 || timestamp == 0 {
            return Err(Error("invalid selection acquisition".into()));
        }
        let (bytes, _) = Request::serialize(
            xproto::SetSelectionOwnerRequest {
                owner,
                selection,
                time: timestamp,
            },
            0,
        );
        let set = requests.queue(
            transport,
            bytes,
            ReplyKind::Void,
            Importance::Optional,
            deadline,
        )?;
        Ok(Self {
            set,
            query: None,
            selection,
            owner,
            timestamp,
            deadline,
            checked: false,
            verified: false,
            state: State::Pending,
        })
    }
    pub fn state(&self) -> State {
        self.state
    }
    /// Invalidate local publication authority after lock, teardown or ownership
    /// loss. This does not send SetSelectionOwner(None), which could race a new
    /// owner; the bridge handles wire cleanup and revokes any published token.
    pub fn cancel(&mut self) {
        if matches!(self.state, State::Pending | State::Acquired) {
            self.state = State::Cancelled;
        }
    }
    /// Publish only after the owner has processed earlier ownership-loss events.
    pub fn confirmed(&self) -> Option<Confirmed> {
        (self.state == State::Acquired).then_some(Confirmed {
            generation: self.set.generation,
            selection: self.selection,
            owner: self.owner,
            timestamp: self.timestamp,
        })
    }
    /// Retry on queue backpressure before the deadline; never repeat SetSelectionOwner.
    /// This reply also provides the ordering barrier for the earlier void request.
    pub fn queue_check(
        &mut self,
        transport: &mut Transport,
        requests: &mut Requests,
    ) -> Result<RequestId> {
        if self.state != State::Pending
            || self.query.is_some()
            || requests.generation() != self.set.generation
        {
            return Err(Error("selection acquisition check is not ready".into()));
        }
        let (bytes, _) = Request::serialize(
            xproto::GetSelectionOwnerRequest {
                selection: self.selection,
            },
            0,
        );
        let id = requests.queue(
            transport,
            bytes,
            ReplyKind::Reply,
            Importance::Optional,
            self.deadline,
        )?;
        self.query = Some(id);
        Ok(id)
    }
    /// Returns false for unrelated or late completions. Failed/rejected attempts
    /// must not publish selection offers or update the ownership ledger.
    /// A later SelectionClear still revokes a successfully acquired ownership.
    /// Route events only from this server generation. Modular event ordering
    /// requires acquisition age below half the X timestamp range; the host must
    /// use extended clock tracking or refresh ownership before that becomes ambiguous.
    pub fn completion(&mut self, completion: &Completion) -> Result<bool> {
        if let Completion::Event(bytes) = completion {
            // Only server-generated ownership events. Synthetic client messages
            // cannot revoke the compositor's publication state.
            if bytes.first() != Some(&xproto::SELECTION_CLEAR_EVENT) {
                return Ok(false);
            }
            if bytes.len() != 32 {
                self.cancel();
                return Err(Error("malformed selection clear event".into()));
            }
            let (event, _) = xproto::SelectionClearEvent::try_parse(bytes).map_err(|_| {
                self.cancel();
                Error("malformed selection clear event".into())
            })?;
            if !matches!(self.state, State::Pending | State::Acquired)
                || event.owner != self.owner
                || event.selection != self.selection
                || event.time == 0
                || event.time.wrapping_sub(self.timestamp) >= 1 << 31
            {
                return Ok(false);
            }
            self.cancel();
            return Ok(true);
        }
        let id = match completion {
            Completion::Reply(id, _)
            | Completion::Checked(id)
            | Completion::Error(id, _)
            | Completion::TimedOut(id) => *id,
            Completion::Event(_) => return Ok(false),
        };
        if self.state != State::Pending || (id != self.set && self.query != Some(id)) {
            return Ok(false);
        }
        match completion {
            Completion::Checked(_) if id == self.set => self.checked = true,
            Completion::Reply(_, bytes) if self.query == Some(id) => {
                if bytes.len() != 32 || bytes[0] != 1 || bytes[4..8] != [0; 4] {
                    self.state = State::Failed;
                    return Err(Error("malformed selection owner reply".into()));
                }
                let (reply, _) =
                    xproto::GetSelectionOwnerReply::try_parse(bytes).map_err(|_| {
                        self.state = State::Failed;
                        Error("malformed selection owner reply".into())
                    })?;
                if reply.owner != self.owner {
                    self.state = State::Rejected;
                } else {
                    self.verified = true;
                }
            }
            _ => self.state = State::Failed,
        }
        if self.state == State::Pending && self.checked && self.verified {
            self.state = State::Acquired;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::super::discovery::tests::{flush_requests, ready};
    use super::*;
    use std::io::Read;
    use x11rb_protocol::x11_utils::Serialize;
    #[test]
    fn proxy_packets_use_selection_timestamp_and_authenticated_endpoint() {
        use x11rb_protocol::protocol::xfixes;
        let (mut transport, mut requests, _, _peer, now) = ready();
        let generation = requests.generation();
        let owners = Ownerships::new(generation).unwrap();
        let claim = Acquisition::begin(11, 10, 12, &mut transport, &mut requests, now).unwrap();
        let publication = Publication::new(claim, Selection::Clipboard, 7, &owners).unwrap();
        let event = xfixes::SelectionNotifyEvent {
            response_type: 87,
            window: 20,
            owner: 10,
            selection: 11,
            timestamp: 99,
            selection_timestamp: 12,
            ..Default::default()
        };
        let mut wire: [u8; 32] = event.into();
        assert_eq!(
            publication
                .proxy_packet(generation, 87, 20, &wire, &owners)
                .unwrap(),
            ProxyEvent::AwaitConfirmation
        );
        assert_eq!(
            publication
                .proxy_packet(generation + 1, 87, 20, &wire, &owners)
                .unwrap(),
            ProxyEvent::Unrelated
        );
        assert_eq!(
            publication
                .proxy_packet(generation, 87, 21, &wire, &owners)
                .unwrap(),
            ProxyEvent::Unrelated
        );
        wire[0] |= 0x80;
        assert_eq!(
            publication
                .proxy_packet(generation, 87, 20, &wire, &owners)
                .unwrap(),
            ProxyEvent::Unrelated
        );
    }
    #[test]
    fn proxy_echoes_do_not_become_external_selection_sources() {
        let (mut transport, mut requests, _, _peer, now) = ready();
        let generation = requests.generation();
        let mut owners = Ownerships::new(generation).unwrap();
        let claim = Acquisition::begin(11, 10, 12, &mut transport, &mut requests, now).unwrap();
        let mut publication = Publication::new(claim, Selection::Clipboard, 7, &owners).unwrap();
        assert_eq!(
            publication.proxy_event(generation, 11, 10, 12, &owners),
            ProxyEvent::AwaitConfirmation
        );
        assert_eq!(
            publication.proxy_event(generation, 11, 20, 12, &owners),
            ProxyEvent::Unrelated
        );
        assert_eq!(
            publication.proxy_event(generation + 1, 11, 10, 12, &owners),
            ProxyEvent::Unrelated
        );
        assert_eq!(
            publication.proxy_event(generation, 11, 10, 9, &owners),
            ProxyEvent::Stale
        );
        publication.acquisition.state = State::Acquired;
        publication.synchronize(&mut owners).unwrap();
        assert_eq!(
            publication.proxy_event(generation, 11, 10, 12, &owners),
            ProxyEvent::Mirrored
        );
        owners
            .replace(Selection::Clipboard, Owner::Native(8))
            .unwrap();
        assert_eq!(
            publication.proxy_event(generation, 11, 10, 12, &owners),
            ProxyEvent::Stale
        );
        publication.cancel(&mut owners).unwrap();
        assert_eq!(
            publication.proxy_event(generation, 11, 10, 12, &owners),
            ProxyEvent::Stale
        );
    }
    #[test]
    fn cancelled_old_publication_preserves_new_owner_and_primary() {
        let (mut transport, mut requests, _, _peer, now) = ready();
        let mut owners = Ownerships::new(requests.generation()).unwrap();
        let claim = Acquisition::begin(11, 10, 12, &mut transport, &mut requests, now).unwrap();
        let mut publication = Publication::new(claim, Selection::Clipboard, 7, &owners).unwrap();
        publication.acquisition.state = State::Acquired;
        assert!(matches!(
            publication.synchronize(&mut owners).unwrap(),
            PublicationUpdate::Published(_)
        ));
        let new = owners
            .replace(Selection::Clipboard, Owner::Native(8))
            .unwrap()
            .current;
        let primary = owners
            .replace(Selection::Primary, Owner::Native(9))
            .unwrap()
            .current;
        assert!(matches!(
            publication.cancel(&mut owners).unwrap(),
            PublicationUpdate::Unchanged
        ));
        assert_eq!(owners.get(Selection::Clipboard), Some(new));
        assert_eq!(owners.get(Selection::Primary), Some(primary));
        assert_eq!(publication.acquisition.confirmed(), None);
        assert!(matches!(
            publication.synchronize(&mut owners).unwrap(),
            PublicationUpdate::Unchanged
        ));
    }
    #[test]
    fn delayed_publication_cannot_overwrite_replaced_or_cleared_source() {
        for cleared in [false, true] {
            let (mut transport, mut requests, _, _peer, now) = ready();
            let mut owners = Ownerships::new(requests.generation()).unwrap();
            let claim = Acquisition::begin(11, 10, 12, &mut transport, &mut requests, now).unwrap();
            let mut publication =
                Publication::new(claim, Selection::Clipboard, 7, &owners).unwrap();
            owners
                .replace(Selection::Clipboard, Owner::Native(8))
                .unwrap();
            if cleared {
                owners.clear(Selection::Clipboard);
            }
            let expected = owners.get(Selection::Clipboard);
            publication.acquisition.state = State::Acquired;
            assert!(matches!(
                publication.synchronize(&mut owners).unwrap(),
                PublicationUpdate::Unchanged
            ));
            assert_eq!(owners.get(Selection::Clipboard), expected);
            assert_eq!(publication.acquisition.state(), State::Cancelled);
        }
    }
    #[test]
    fn publication_waits_for_confirmation_and_revokes_only_its_token() {
        for malformed in [false, true] {
            let (mut transport, mut requests, _, _peer, now) = ready();
            let mut owners = Ownerships::new(requests.generation()).unwrap();
            let claim = Acquisition::begin(11, 10, 12, &mut transport, &mut requests, now).unwrap();
            let mut publication =
                Publication::new(claim, Selection::Clipboard, 7, &owners).unwrap();
            assert!(matches!(
                publication.synchronize(&mut owners).unwrap(),
                PublicationUpdate::Unchanged
            ));
            assert!(owners.get(Selection::Clipboard).is_none());
            let claim = publication.acquisition_mut();
            let query = claim.queue_check(&mut transport, &mut requests).unwrap();
            let mut reply = xproto::GetSelectionOwnerReply {
                owner: 10,
                ..Default::default()
            }
            .serialize()
            .to_vec();
            reply.resize(32, 0);
            claim.completion(&Completion::Checked(claim.set)).unwrap();
            claim.completion(&Completion::Reply(query, reply)).unwrap();
            let PublicationUpdate::Published(change) =
                publication.synchronize(&mut owners).unwrap()
            else {
                panic!("not published");
            };
            assert_eq!(change.current.owner, Owner::Native(7));
            assert!(matches!(
                publication.synchronize(&mut owners).unwrap(),
                PublicationUpdate::Unchanged
            ));
            let update = if malformed {
                let dispatch = publication
                    .completion(
                        &Completion::Event(vec![xproto::SELECTION_CLEAR_EVENT]),
                        &mut owners,
                    )
                    .unwrap();
                assert!(dispatch.handled);
                assert!(dispatch.error.is_some());
                dispatch.update
            } else {
                publication.cancel(&mut owners).unwrap()
            };
            let PublicationUpdate::Revoked(old) = update else {
                panic!("not revoked");
            };
            assert_eq!(old, change.current);
            assert!(owners.get(Selection::Clipboard).is_none());
            assert!(matches!(
                publication.synchronize(&mut owners).unwrap(),
                PublicationUpdate::Unchanged
            ));
        }
    }
    #[test]
    fn malformed_clear_invalidates_confirmed_metadata() {
        for size in [1, 31, 33] {
            let (mut transport, mut requests, _, _peer, now) = ready();
            let mut claim =
                Acquisition::begin(11, 10, 12, &mut transport, &mut requests, now).unwrap();
            claim.state = State::Acquired;
            let mut wire = vec![0; size];
            wire[0] = xproto::SELECTION_CLEAR_EVENT;
            assert!(claim.completion(&Completion::Event(wire)).is_err());
            assert_eq!(claim.confirmed(), None);
            assert_eq!(claim.state(), State::Cancelled);
        }
    }
    #[test]
    fn server_clear_revokes_only_matching_nonstale_acquisition() {
        for (timestamp, acquired) in [
            (12, false),
            (12, true),
            (u32::MAX - 2, false),
            (u32::MAX - 2, true),
        ] {
            let (mut transport, mut requests, _, _peer, now) = ready();
            let mut claim =
                Acquisition::begin(11, 10, timestamp, &mut transport, &mut requests, now).unwrap();
            let query = claim.queue_check(&mut transport, &mut requests).unwrap();
            let mut reply = xproto::GetSelectionOwnerReply {
                owner: 10,
                ..Default::default()
            }
            .serialize()
            .to_vec();
            reply.resize(32, 0);
            claim.completion(&Completion::Checked(claim.set)).unwrap();
            if acquired {
                claim
                    .completion(&Completion::Reply(query, reply.clone()))
                    .unwrap();
                assert!(claim.confirmed().is_some());
            }
            let initial = claim.state();
            let clear = xproto::SelectionClearEvent {
                response_type: xproto::SELECTION_CLEAR_EVENT,
                owner: 10,
                selection: 11,
                time: timestamp.wrapping_add(4),
                ..Default::default()
            };
            for unrelated in [
                xproto::SelectionClearEvent { owner: 20, ..clear },
                xproto::SelectionClearEvent {
                    selection: 20,
                    ..clear
                },
                xproto::SelectionClearEvent {
                    time: timestamp.wrapping_sub(1),
                    ..clear
                },
                xproto::SelectionClearEvent {
                    response_type: xproto::SELECTION_CLEAR_EVENT | 0x80,
                    ..clear
                },
            ] {
                let wire: [u8; 32] = unrelated.into();
                assert!(!claim.completion(&Completion::Event(wire.to_vec())).unwrap());
                assert_eq!(claim.state(), initial);
            }
            let wire: [u8; 32] = clear.into();
            assert!(claim.completion(&Completion::Event(wire.to_vec())).unwrap());
            assert_eq!(claim.state(), State::Cancelled);
            assert_eq!(claim.confirmed(), None);
            assert!(!claim.completion(&Completion::Reply(query, reply)).unwrap());
            assert_eq!(claim.state(), State::Cancelled);
            assert!(!claim.completion(&Completion::Event(wire.to_vec())).unwrap());
        }
    }
    #[test]
    fn cancellation_invalidates_pending_and_confirmed_acquisition() {
        for acquired in [false, true] {
            let (mut transport, mut requests, _, _peer, now) = ready();
            let mut claim =
                Acquisition::begin(11, 10, 12, &mut transport, &mut requests, now).unwrap();
            let query = claim.queue_check(&mut transport, &mut requests).unwrap();
            let mut reply = xproto::GetSelectionOwnerReply {
                owner: 10,
                ..Default::default()
            }
            .serialize()
            .to_vec();
            reply.resize(32, 0);
            if acquired {
                claim.completion(&Completion::Checked(claim.set)).unwrap();
                claim
                    .completion(&Completion::Reply(query, reply.clone()))
                    .unwrap();
                assert!(claim.confirmed().is_some());
            }
            claim.cancel();
            claim.cancel();
            assert_eq!(claim.state(), State::Cancelled);
            assert_eq!(claim.confirmed(), None);
            assert!(!claim.completion(&Completion::Reply(query, reply)).unwrap());
            assert!(claim.queue_check(&mut transport, &mut requests).is_err());
        }
    }
    #[test]
    fn query_backpressure_preserves_single_set_and_timeout_finishes_attempt() {
        let (mut transport, mut requests, _, _peer, now) = ready();
        while requests.available_slots() > 1 {
            let (bytes, _) = Request::serialize(xproto::GetInputFocusRequest, 0);
            requests
                .queue(
                    &mut transport,
                    bytes,
                    ReplyKind::Reply,
                    Importance::Optional,
                    now + std::time::Duration::from_secs(10),
                )
                .unwrap();
        }
        let mut claim = Acquisition::begin(11, 10, 12, &mut transport, &mut requests, now).unwrap();
        let count = requests.outstanding();
        for _ in 0..3 {
            assert!(claim.queue_check(&mut transport, &mut requests).is_err());
            assert_eq!(claim.state(), State::Pending);
            assert_eq!(claim.confirmed(), None);
            assert_eq!(requests.outstanding(), count);
            assert!(claim.query.is_none());
        }
        for completion in requests.expire(now).unwrap() {
            claim.completion(&completion).unwrap();
        }
        assert_eq!(claim.state(), State::Failed);
        assert!(claim.queue_check(&mut transport, &mut requests).is_err());
        assert_eq!(requests.outstanding(), count);
    }
    #[test]
    fn owner_query_and_checked_set_are_both_required() {
        for actual in [10, 20, 0] {
            let (mut transport, mut requests, _, mut peer, now) = ready();
            let mut claim =
                Acquisition::begin(11, 10, 12, &mut transport, &mut requests, now).unwrap();
            assert_eq!(claim.state(), State::Pending);
            assert_eq!(claim.confirmed(), None);
            let query = claim.queue_check(&mut transport, &mut requests).unwrap();
            assert!(claim.queue_check(&mut transport, &mut requests).is_err());
            flush_requests(&mut transport);
            let mut wire = [0; 24];
            peer.read_exact(&mut wire).unwrap();
            assert_eq!(wire[0], 22); // SetSelectionOwner
            assert_eq!(wire[16], 23); // GetSelectionOwner
            assert_eq!(u32::from_ne_bytes(wire[4..8].try_into().unwrap()), 10);
            assert_eq!(u32::from_ne_bytes(wire[8..12].try_into().unwrap()), 11);
            assert_eq!(u32::from_ne_bytes(wire[12..16].try_into().unwrap()), 12);
            let mut reply = xproto::GetSelectionOwnerReply {
                sequence: query.sequence as u16,
                owner: actual,
                ..Default::default()
            }
            .serialize()
            .to_vec();
            reply.resize(32, 0); // Reply serialization omits trailing protocol padding.
            let completions = requests.ingest(reply).unwrap();
            assert!(
                completions
                    .iter()
                    .any(|c| matches!(c, Completion::Checked(id) if *id == claim.set))
            );
            for c in completions {
                claim.completion(&c).unwrap();
            }
            assert_eq!(
                claim.confirmed(),
                (actual == 10).then_some(Confirmed {
                    generation: requests.generation(),
                    selection: 11,
                    owner: 10,
                    timestamp: 12,
                })
            );
            assert_eq!(
                claim.state(),
                if actual == 10 {
                    State::Acquired
                } else {
                    State::Rejected
                }
            );
        }
    }
    #[test]
    fn failed_or_expired_attempt_cannot_be_revived_by_a_late_owner_reply() {
        for timeout in [false, true] {
            let (mut transport, mut requests, _, _peer, now) = ready();
            let mut claim =
                Acquisition::begin(11, 10, 12, &mut transport, &mut requests, now).unwrap();
            let query = claim.queue_check(&mut transport, &mut requests).unwrap();
            let failure = if timeout {
                Completion::TimedOut(claim.set)
            } else {
                Completion::Error(claim.set, 3)
            };
            claim.completion(&failure).unwrap();
            assert_eq!(claim.state(), State::Failed);
            let mut reply = xproto::GetSelectionOwnerReply {
                sequence: query.sequence as u16,
                owner: 10,
                ..Default::default()
            }
            .serialize()
            .to_vec();
            reply.resize(32, 0);
            assert!(
                !claim
                    .completion(&Completion::Reply(query, reply.to_vec()))
                    .unwrap()
            );
            assert_eq!(claim.state(), State::Failed);
        }
    }
}
