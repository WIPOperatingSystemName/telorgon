//! Nonblocking attributes/geometry/parent snapshots for unobserved X11 windows.
//! Feed lifecycle events through Windows before consuming subsequent replies.
//! Invalidation prevents mixed snapshots from overwriting newer event state.
use super::{
    Error, Result,
    requests::{Completion, Importance, ReplyKind, RequestId, Requests},
    transport::Transport,
    window::{Action, Geometry, Inspection, Windows},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};
use x11rb_protocol::{
    protocol::xproto,
    x11_utils::{Request, TryParse},
};
#[derive(Clone, Copy)]
enum Field {
    Attributes,
    Geometry,
    Parent,
}
struct Probe {
    token: Inspection,
    attributes: Option<xproto::GetWindowAttributesReply>,
    geometry: Option<xproto::GetGeometryReply>,
    parent: Option<u32>,
}
pub struct Inspector {
    enumeration: Option<(RequestId, u32, u32, usize)>,
    enumerated: bool,
    queued: VecDeque<u32>,
    probes: BTreeMap<u32, Probe>,
    pending: BTreeMap<u64, (Inspection, Field)>,
}
impl Default for Inspector {
    fn default() -> Self {
        Self::new()
    }
}
impl Inspector {
    pub fn new() -> Self {
        Self {
            enumeration: None,
            enumerated: false,
            queued: VecDeque::new(),
            probes: BTreeMap::new(),
            pending: BTreeMap::new(),
        }
    }
    pub(crate) fn enqueue(&mut self, xid: u32, windows: &Windows) -> Result<()> {
        if windows.get(xid).is_some() || self.queued.contains(&xid) {
            return Ok(());
        }
        if self.queued.len() >= windows.inspection_context().3 {
            return Err(Error("XWM queued inspection bound reached".into()));
        }
        self.queued.push_back(xid);
        Ok(())
    }
    /// Enumerate current root children once, after manager acquisition. Root
    /// enumeration is essential; disappearing individual children are optional.
    pub fn enumerate(
        &mut self,
        windows: &Windows,
        transport: &mut Transport,
        requests: &mut Requests,
        deadline: Instant,
    ) -> Result<()> {
        if self.enumeration.is_some() || self.enumerated {
            return Err(Error("XWM root enumeration already started".into()));
        }
        let (generation, root, manager, capacity) = windows.inspection_context();
        if requests.generation() != generation {
            return Err(Error("XWM enumeration generation mismatch".into()));
        }
        let (bytes, _) = Request::serialize(xproto::QueryTreeRequest { window: root }, 0);
        let id = requests.queue(
            transport,
            bytes,
            ReplyKind::Reply,
            Importance::Essential,
            deadline,
        )?;
        self.enumeration = Some((id, root, manager, capacity));
        Ok(())
    }
    /// Queue at most sixteen snapshots or one millisecond of work. Return true
    /// only when another immediate scheduling turn can make progress; a saturated
    /// request queue waits for replies rather than spinning the owner loop.
    pub fn schedule(
        &mut self,
        windows: &mut Windows,
        transport: &mut Transport,
        requests: &mut Requests,
        deadline: Instant,
    ) -> Result<bool> {
        self.schedule_until(
            windows,
            transport,
            requests,
            deadline,
            Instant::now() + Duration::from_millis(1),
        )
    }
    pub(crate) fn schedule_until(
        &mut self,
        windows: &mut Windows,
        transport: &mut Transport,
        requests: &mut Requests,
        deadline: Instant,
        work_deadline: Instant,
    ) -> Result<bool> {
        for _ in 0..16 {
            if self.pending.len() > 381
                || requests.available_slots() < 3
                || Instant::now() >= work_deadline
            {
                break;
            }
            let Some(xid) = self.queued.front().copied() else {
                break;
            };
            self.start(xid, windows, transport, requests, deadline)?;
            self.queued.pop_front();
        }
        Ok(!self.queued.is_empty() && self.pending.len() <= 381 && requests.available_slots() >= 3)
    }
    /// Component completion only; neither output readiness nor desktop policy.
    pub fn enumeration_complete(&self) -> bool {
        self.enumerated
            && self.enumeration.is_none()
            && self.queued.is_empty()
            && self.pending.is_empty()
            && self.probes.is_empty()
    }
    /// Queue three optional replies. A disappearing window is local cancellation,
    /// not failure of the XWM connection. Queue exhaustion remains explicit.
    pub fn start(
        &mut self,
        xid: u32,
        windows: &mut Windows,
        transport: &mut Transport,
        requests: &mut Requests,
        deadline: Instant,
    ) -> Result<()> {
        if self.pending.len() > 381 {
            return Err(Error("XWM inspection request bound reached".into()));
        }
        if windows.inspection_context().0 != requests.generation() {
            return Err(Error("XWM inspection generation mismatch".into()));
        }
        let Some(token) = windows.begin_inspection(xid)? else {
            return Ok(());
        };
        self.probes.insert(
            xid,
            Probe {
                token,
                attributes: None,
                geometry: None,
                parent: None,
            },
        );
        let result = (|| {
            self.send(
                xproto::GetWindowAttributesRequest { window: xid },
                Field::Attributes,
                token,
                transport,
                requests,
                deadline,
            )?;
            self.send(
                xproto::GetGeometryRequest { drawable: xid },
                Field::Geometry,
                token,
                transport,
                requests,
                deadline,
            )?;
            self.send(
                xproto::QueryTreeRequest { window: xid },
                Field::Parent,
                token,
                transport,
                requests,
                deadline,
            )
        })();
        if result.is_err() {
            self.probes.remove(&xid);
            windows.cancel_inspection(token);
        }
        result
    }
    fn send(
        &mut self,
        request: impl Request,
        field: Field,
        token: Inspection,
        transport: &mut Transport,
        requests: &mut Requests,
        deadline: Instant,
    ) -> Result<()> {
        let (bytes, fds) = Request::serialize(request, 0);
        if !fds.is_empty() {
            return Err(Error("unexpected inspection request descriptors".into()));
        }
        let id = requests.queue(
            transport,
            bytes,
            ReplyKind::Reply,
            Importance::Optional,
            deadline,
        )?;
        if id.generation != token.generation {
            return Err(Error("XWM inspection generation mismatch".into()));
        }
        self.pending.insert(id.sequence, (token, field));
        Ok(())
    }
    /// Returns false for completions owned by another adapter. Results contain
    /// newly observed windows only after all three replies and a valid lifetime.
    pub fn completion(
        &mut self,
        completion: &Completion,
        windows: &mut Windows,
        actions: &mut Vec<Action>,
    ) -> Result<bool> {
        let id = match completion {
            Completion::Reply(id, _) | Completion::Error(id, _) | Completion::TimedOut(id) => id,
            _ => return Ok(false),
        };
        if let Some((expected, root, manager, capacity)) = self.enumeration
            && *id == expected
        {
            let Completion::Reply(_, bytes) = completion else {
                return Err(Error("XWM root enumeration failed".into()));
            };
            let (tree, _) = xproto::QueryTreeReply::try_parse(bytes)
                .map_err(|_| Error("malformed XWM root tree reply".into()))?;
            if tree.root != root
                || tree.parent != 0
                || tree.children.len() > capacity.saturating_add(1)
            {
                return Err(Error("invalid or excessive XWM root tree".into()));
            }
            let mut unique = BTreeSet::new();
            for &child in &tree.children {
                if child == 0 || child == root || !unique.insert(child) {
                    return Err(Error("invalid duplicate XWM root child".into()));
                }
            }
            let children: VecDeque<_> = tree
                .children
                .into_iter()
                .filter(|xid| *xid != manager)
                .collect();
            if children.len() > capacity {
                return Err(Error("XWM root child bound reached".into()));
            }
            for child in children {
                self.enqueue(child, windows)?;
            }
            self.enumeration = None;
            self.enumerated = true;
            return Ok(true);
        }
        let Some(&(token, field)) = self.pending.get(&id.sequence) else {
            return Ok(false);
        };
        if id.generation != token.generation {
            return Ok(false);
        }
        self.pending.remove(&id.sequence);
        let Some(probe) = self.probes.get_mut(&token.xid).filter(|p| p.token == token) else {
            return Ok(true);
        };
        match completion {
            Completion::Reply(_, bytes) => {
                let malformed = |_| Error("malformed XWM window inspection reply".into());
                match field {
                    Field::Attributes => {
                        probe.attributes = Some(
                            xproto::GetWindowAttributesReply::try_parse(bytes)
                                .map_err(malformed)?
                                .0,
                        );
                    }
                    Field::Geometry => {
                        probe.geometry = Some(
                            xproto::GetGeometryReply::try_parse(bytes)
                                .map_err(malformed)?
                                .0,
                        );
                    }
                    Field::Parent => {
                        probe.parent = Some(
                            xproto::QueryTreeReply::try_parse(bytes)
                                .map_err(malformed)?
                                .0
                                .parent,
                        );
                    }
                }
            }
            Completion::Error(_, _) | Completion::TimedOut(_) => {
                self.probes.remove(&token.xid);
                if matches!(completion, Completion::Error(_, _)) {
                    windows.discard_inspection(token);
                } else {
                    windows.cancel_inspection(token);
                }
                return Ok(true);
            }
            _ => unreachable!(),
        }
        if let (Some(attributes), Some(geometry), Some(parent)) =
            (&probe.attributes, &probe.geometry, probe.parent)
        {
            let action = if attributes.class == xproto::WindowClass::INPUT_OUTPUT {
                windows.finish_inspection(
                    token,
                    parent,
                    Geometry {
                        x: geometry.x,
                        y: geometry.y,
                        width: geometry.width,
                        height: geometry.height,
                        border: geometry.border_width,
                    },
                    attributes.override_redirect,
                    attributes.map_state != xproto::MapState::UNMAPPED,
                )?
            } else {
                windows.discard_inspection(token);
                vec![]
            };
            self.probes.remove(&token.xid);
            actions.extend(action);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::super::discovery::tests::{flush_requests, ready, request};
    use super::*;
    use std::{io::Write, os::unix::net::UnixStream, time::Duration};
    use x11rb_protocol::x11_utils::Serialize;
    fn start() -> (Inspector, Windows, Transport, Requests, UnixStream, Instant) {
        let (mut transport, mut requests, _, mut peer, now) = ready();
        let mut windows = Windows::new(1, 16, 1, 2, 100).unwrap();
        let mut inspector = Inspector::new();
        inspector
            .start(
                10,
                &mut windows,
                &mut transport,
                &mut requests,
                now + Duration::from_secs(1),
            )
            .unwrap();
        flush_requests(&mut transport);
        for opcode in [3, 14, 15] {
            let bytes = request(&mut peer);
            assert_eq!(bytes[0], opcode);
            assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), 10);
        }
        (inspector, windows, transport, requests, peer, now)
    }
    fn reply(peer: &mut UnixStream, fields: impl AsRef<[u8]>) {
        let mut bytes = fields.as_ref().to_vec();
        bytes.resize(bytes.len().max(32), 0);
        let length = ((bytes.len() - 32) / 4) as u32;
        bytes[4..8].copy_from_slice(&length.to_ne_bytes());
        peer.write_all(&bytes).unwrap();
    }
    fn replies(peer: &mut UnixStream) {
        reply(
            peer,
            xproto::GetWindowAttributesReply {
                sequence: 42,
                class: xproto::WindowClass::INPUT_OUTPUT,
                map_state: xproto::MapState::VIEWABLE,
                override_redirect: true,
                ..Default::default()
            }
            .serialize(),
        );
        reply(
            peer,
            xproto::GetGeometryReply {
                sequence: 43,
                root: 1,
                width: 640,
                height: 480,
                x: -20,
                y: 30,
                ..Default::default()
            }
            .serialize(),
        );
        reply(
            peer,
            xproto::QueryTreeReply {
                sequence: 44,
                root: 1,
                parent: 1,
                children: vec![],
                ..Default::default()
            }
            .serialize(),
        );
    }
    fn pump(
        inspector: &mut Inspector,
        windows: &mut Windows,
        transport: &mut Transport,
        requests: &mut Requests,
    ) -> Vec<Action> {
        let mut actions = vec![];
        for _ in 0..10 {
            for packet in transport.dispatch().unwrap().packets {
                for completion in requests.ingest(packet).unwrap() {
                    assert!(
                        inspector
                            .completion(&completion, windows, &mut actions)
                            .unwrap()
                    );
                }
            }
        }
        actions
    }
    fn enumeration(
        capacity: usize,
    ) -> (Inspector, Windows, Transport, Requests, UnixStream, Instant) {
        let (mut transport, mut requests, _, mut peer, now) = ready();
        let windows = Windows::new(1, capacity, 1, 2, 100).unwrap();
        let mut inspector = Inspector::new();
        inspector
            .enumerate(
                &windows,
                &mut transport,
                &mut requests,
                now + Duration::from_secs(1),
            )
            .unwrap();
        flush_requests(&mut transport);
        let bytes = request(&mut peer);
        assert_eq!(bytes[0], 15);
        assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), 1);
        (inspector, windows, transport, requests, peer, now)
    }
    fn root_reply(peer: &mut UnixStream, children: Vec<u32>) {
        reply(
            peer,
            xproto::QueryTreeReply {
                sequence: 42,
                root: 1,
                parent: 0,
                children,
                ..Default::default()
            }
            .serialize(),
        );
    }
    #[test]
    fn root_enumeration_skips_manager_and_completes_after_child_snapshot() {
        let (mut i, mut w, mut t, mut r, mut peer, now) = enumeration(8);
        assert!(!i.enumeration_complete());
        root_reply(&mut peer, vec![2, 10]);
        assert!(pump(&mut i, &mut w, &mut t, &mut r).is_empty());
        assert!(!i.enumeration_complete());
        assert!(
            !i.schedule(&mut w, &mut t, &mut r, now + Duration::from_secs(1))
                .unwrap()
        );
        t.dispatch().unwrap();
        for opcode in [3, 14, 15] {
            let bytes = request(&mut peer);
            assert_eq!(bytes[0], opcode);
            assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), 10);
        }
        reply(
            &mut peer,
            xproto::GetWindowAttributesReply {
                sequence: 43,
                class: xproto::WindowClass::INPUT_OUTPUT,
                map_state: xproto::MapState::VIEWABLE,
                ..Default::default()
            }
            .serialize(),
        );
        reply(
            &mut peer,
            xproto::GetGeometryReply {
                sequence: 44,
                root: 1,
                width: 640,
                height: 480,
                ..Default::default()
            }
            .serialize(),
        );
        reply(
            &mut peer,
            xproto::QueryTreeReply {
                sequence: 45,
                root: 1,
                parent: 1,
                ..Default::default()
            }
            .serialize(),
        );
        assert_eq!(pump(&mut i, &mut w, &mut t, &mut r).len(), 1);
        assert!(i.enumeration_complete());
        assert!(w.get(2).is_none());
        assert!(w.get(10).unwrap().mapped);
    }
    #[test]
    fn enumeration_rejects_duplicate_and_excessive_children() {
        for children in [vec![10, 10], vec![10, 11, 12], vec![0], vec![1]] {
            let (mut i, mut w, mut t, mut r, mut peer, _) = enumeration(2);
            root_reply(&mut peer, children);
            let mut failed = false;
            for packet in t.dispatch().unwrap().packets {
                for completion in r.ingest(packet).unwrap() {
                    failed |= i.completion(&completion, &mut w, &mut vec![]).is_err();
                }
            }
            assert!(failed);
            assert!(!i.enumeration_complete());
            assert!(i.queued.is_empty());
        }
    }
    #[test]
    fn enumeration_schedules_bounded_batches_and_waits_at_request_limit() {
        let (mut i, mut w, mut t, mut r, mut peer, now) = enumeration(256);
        root_reply(&mut peer, (10..139).collect());
        pump(&mut i, &mut w, &mut t, &mut r);
        i.schedule(&mut w, &mut t, &mut r, now + Duration::from_secs(1))
            .unwrap();
        assert!(i.probes.len() <= 16);
        for _ in 0..100 {
            if !i
                .schedule(&mut w, &mut t, &mut r, now + Duration::from_secs(1))
                .unwrap()
            {
                break;
            }
        }
        assert_eq!(i.pending.len(), 384);
        assert_eq!(i.queued.len(), 1);
        assert!(
            !i.schedule(&mut w, &mut t, &mut r, now + Duration::from_secs(1))
                .unwrap()
        );
        assert!(!i.enumeration_complete());
        // Replies for one disappearing child free exactly three request slots.
        t.dispatch().unwrap();
        for sequence in [43u16, 44, 45] {
            let mut error = [0u8; 32];
            error[1] = 3;
            error[2..4].copy_from_slice(&sequence.to_ne_bytes());
            peer.write_all(&error).unwrap();
        }
        pump(&mut i, &mut w, &mut t, &mut r);
        assert_eq!(i.pending.len(), 381);
        i.schedule(&mut w, &mut t, &mut r, now + Duration::from_secs(1))
            .unwrap();
        assert!(i.queued.is_empty());
        assert_eq!(i.pending.len(), 384);
    }
    #[test]
    fn all_three_replies_are_required_to_create_a_snapshot() {
        let (mut i, mut w, mut t, mut r, mut peer, _) = start();
        reply(
            &mut peer,
            xproto::GetWindowAttributesReply {
                sequence: 42,
                class: xproto::WindowClass::INPUT_OUTPUT,
                map_state: xproto::MapState::VIEWABLE,
                override_redirect: true,
                ..Default::default()
            }
            .serialize(),
        );
        assert!(pump(&mut i, &mut w, &mut t, &mut r).is_empty());
        assert!(w.get(10).is_none());
        reply(
            &mut peer,
            xproto::GetGeometryReply {
                sequence: 43,
                root: 1,
                width: 640,
                height: 480,
                x: -20,
                y: 30,
                ..Default::default()
            }
            .serialize(),
        );
        assert!(pump(&mut i, &mut w, &mut t, &mut r).is_empty());
        reply(
            &mut peer,
            xproto::QueryTreeReply {
                sequence: 44,
                root: 1,
                parent: 1,
                ..Default::default()
            }
            .serialize(),
        );
        let actions = pump(&mut i, &mut w, &mut t, &mut r);
        let window = w.get(10).unwrap();
        assert_eq!(actions, vec![Action::Created(window.id)]);
        assert!(window.mapped && window.override_redirect);
        assert_eq!(window.geometry.x, -20);
        assert_eq!(window.geometry.width, 640);
        assert_eq!(w.presentable_surface(window.id), None);
        assert_eq!(r.outstanding(), 0);
    }
    #[test]
    fn destruction_and_xid_reuse_reject_old_snapshot() {
        let (mut i, mut w, mut t, mut r, mut peer, _) = start();
        let bytes: [u8; 32] = xproto::DestroyNotifyEvent {
            response_type: xproto::DESTROY_NOTIFY_EVENT,
            event: 1,
            window: 10,
            ..Default::default()
        }
        .into();
        w.event(&bytes).unwrap();
        let bytes: [u8; 32] = xproto::CreateNotifyEvent {
            response_type: xproto::CREATE_NOTIFY_EVENT,
            parent: 1,
            window: 10,
            width: 200,
            height: 100,
            ..Default::default()
        }
        .into();
        w.event(&bytes).unwrap();
        let id = w.get(10).unwrap().id;
        replies(&mut peer);
        assert!(pump(&mut i, &mut w, &mut t, &mut r).is_empty());
        assert_eq!(w.get(10).unwrap().id, id);
        assert_eq!(w.get(10).unwrap().geometry.width, 200);
        assert!(!w.get(10).unwrap().mapped);
    }
    #[test]
    fn timeouts_cancel_snapshot_and_late_replies_remain_accounted() {
        let (mut i, mut w, mut t, mut r, mut peer, now) = start();
        let mut actions = vec![];
        for completion in r.expire(now + Duration::from_secs(2)).unwrap() {
            assert!(i.completion(&completion, &mut w, &mut actions).unwrap());
        }
        assert!(actions.is_empty());
        assert!(w.get(10).is_none());
        replies(&mut peer);
        assert!(pump(&mut i, &mut w, &mut t, &mut r).is_empty());
        assert_eq!(r.outstanding(), 0);
        assert!(i.pending.is_empty());
        assert!(w.begin_inspection(10).unwrap().is_some());
    }
}
