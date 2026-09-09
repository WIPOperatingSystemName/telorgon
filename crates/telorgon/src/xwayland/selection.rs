//! Selection ownership revisions, independent of X window/surface lifetimes.
//! Wire timestamp validation and proxy-owner echo routing belong to the bridge.
use super::{Error, Result};

/// Atom identifiers belonging to one private XWM connection generation.
/// Recognition is not an advertisement that a conversion is implemented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Atoms {
    pub generation: u64,
    pub clipboard: u32,
    pub targets: u32,
    pub timestamp: u32,
    pub incr: u32,
    pub utf8_string: u32,
    pub text: u32,
    pub compound_text: u32,
}
impl Atoms {
    pub(crate) fn discovered(
        generation: u64,
        atoms: &std::collections::BTreeMap<&str, u32>,
    ) -> Result<Self> {
        let get = |name| {
            atoms
                .get(name)
                .copied()
                .filter(|id| *id != 0)
                .ok_or_else(|| Error("missing selection atom".into()))
        };
        if generation == 0 {
            return Err(Error("invalid selection atom generation".into()));
        }
        Ok(Self {
            generation,
            clipboard: get("CLIPBOARD")?,
            targets: get("TARGETS")?,
            timestamp: get("TIMESTAMP")?,
            incr: get("INCR")?,
            utf8_string: get("UTF8_STRING")?,
            text: get("TEXT")?,
            compound_text: get("COMPOUND_TEXT")?,
        })
    }
    pub fn selection(self, selection: Selection) -> u32 {
        match selection {
            Selection::Primary => u32::from(x11rb_protocol::protocol::xproto::AtomEnum::PRIMARY),
            Selection::Clipboard => self.clipboard,
        }
    }
    /// Decode selection routing only for the connection that supplied these IDs.
    pub fn identify(self, generation: u64, atom: u32) -> Result<Option<Selection>> {
        if generation != self.generation {
            return Err(Error("stale selection atom generation".into()));
        }
        Ok(if atom == self.selection(Selection::Primary) {
            Some(Selection::Primary)
        } else if atom == self.clipboard {
            Some(Selection::Clipboard)
        } else {
            None
        })
    }
}

/// Host-owned mapping from one XFixes subscription to a desktop selection.
pub struct Subscription {
    pub generation: u64,
    pub selection: Selection,
    pub atom: u32,
    pub window: u32,
    pub proxy_window: u32,
    pub first_event: u8,
}
impl Subscription {
    /// Queue this exact subscription context after extension discovery. Retain
    /// the returned ID for checked completion before relying on notifications.
    pub fn queue(
        &self,
        opcode: u8,
        enabled: bool,
        transport: &mut super::transport::Transport,
        requests: &mut super::requests::Requests,
        deadline: std::time::Instant,
    ) -> Result<super::requests::RequestId> {
        if self.generation != requests.generation()
            || self.proxy_window == 0
            || !(64..128).contains(&self.first_event)
        {
            return Err(Error("invalid selection subscription context".into()));
        }
        watch(
            opcode,
            self.window,
            self.atom,
            enabled,
            transport,
            requests,
            deadline,
        )
    }
    /// Process packets from this subscription's connection in server order.
    /// The old snapshot in the result must be used for transfer/offer cleanup.
    pub fn event(&self, bytes: &[u8], owners: &mut Ownerships) -> Result<OwnershipUpdate> {
        if self.generation != owners.generation() || self.proxy_window == 0 {
            return Err(Error(
                "invalid selection subscription generation or proxy".into(),
            ));
        }
        let Some(event) = owner_event(bytes, self.first_event, self.window, self.atom)? else {
            return Ok(OwnershipUpdate::Unchanged);
        };
        owners.external_owner(
            self.generation,
            self.selection,
            event.owner,
            self.proxy_window,
        )
    }
}

/// Decode only server-generated XFixes selection events for this subscription.
/// The caller supplies the negotiated extension event base and routes packets
/// from the correct Xwayland generation. This does not mutate ownership policy.
pub fn owner_event(
    bytes: &[u8],
    first_event: u8,
    window: u32,
    selection: u32,
) -> Result<Option<x11rb_protocol::protocol::xfixes::SelectionNotifyEvent>> {
    use x11rb_protocol::{protocol::xfixes, x11_utils::TryParse};
    let code = first_event
        .checked_add(xfixes::SELECTION_NOTIFY_EVENT)
        .filter(|code| *code >= 64 && *code < 128)
        .ok_or_else(|| Error("invalid XFixes event base".into()))?;
    if window == 0 || selection == 0 {
        return Err(Error("invalid selection subscription endpoint".into()));
    }
    if bytes.first() != Some(&code) {
        return Ok(None);
    }
    if bytes.len() != 32 {
        return Err(Error("malformed XFixes selection event".into()));
    }
    let (event, _) = xfixes::SelectionNotifyEvent::try_parse(bytes)
        .map_err(|_| Error("malformed XFixes selection event".into()))?;
    if event.window != window || event.selection != selection {
        return Ok(None);
    }
    if !matches!(
        event.subtype,
        xfixes::SelectionEvent::SET_SELECTION_OWNER
            | xfixes::SelectionEvent::SELECTION_WINDOW_DESTROY
            | xfixes::SelectionEvent::SELECTION_CLIENT_CLOSE
    ) {
        return Err(Error("unknown XFixes selection event subtype".into()));
    }
    Ok(Some(event))
}

/// Select all XFixes ownership-loss/change notifications on a compositor-owned
/// window. The caller must supply the discovered XFixes opcode after negotiating
/// its supported version, retain the request ID and check it with a reply barrier.
pub fn watch(
    opcode: u8,
    window: u32,
    selection: u32,
    enabled: bool,
    transport: &mut super::transport::Transport,
    requests: &mut super::requests::Requests,
    deadline: std::time::Instant,
) -> Result<super::requests::RequestId> {
    use super::requests::{Importance, ReplyKind};
    use x11rb_protocol::{protocol::xfixes, x11_utils::Request};
    if opcode < 128 || window == 0 || selection == 0 {
        return Err(Error("invalid XFixes selection subscription".into()));
    }
    let mask = if enabled {
        xfixes::SelectionEventMask::SET_SELECTION_OWNER
            | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
            | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE
    } else {
        0u32.into()
    };
    let (bytes, _) = Request::serialize(
        xfixes::SelectSelectionInputRequest {
            window,
            selection,
            event_mask: mask,
        },
        opcode,
    );
    requests.queue(
        transport,
        bytes,
        ReplyKind::Void,
        Importance::Optional,
        deadline,
    )
}

/// Check a request against a confirmed ownership interval in the X server's
/// millisecond clock. Bounds must be explicit nonzero server timestamps, never
/// wall time or an application's claimed clock. The caller must establish that
/// the interval is shorter than half the clock range; longer history needs an
/// extended clock or refreshed ownership, not this modular comparison.
/// CurrentTime requests refer to the interval's current end. This does not prove
/// ownership: callers must separately check the live ownership token.
pub fn request_in_interval(acquired: u32, current: u32, requested: u32) -> bool {
    if acquired == 0 || current == 0 {
        return false;
    }
    let span = current.wrapping_sub(acquired);
    if span >= 1 << 31 {
        return false;
    }
    requested == 0 || requested.wrapping_sub(acquired) <= span
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Clipboard,
    Primary,
}
impl Selection {
    fn index(self) -> usize {
        match self {
            Self::Clipboard => 0,
            Self::Primary => 1,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Compositor-assigned source identity, never a client's reusable wire ID.
    Native(u64),
    /// Selection owner XID; need not be a managed or associated desktop window.
    X11(u32),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ownership {
    generation: u64,
    revision: u64,
    selection: Selection,
}
impl Ownership {
    pub fn generation(self) -> u64 {
        self.generation
    }
    pub fn revision(self) -> u64 {
        self.revision
    }
    pub fn selection(self) -> Selection {
        self.selection
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub ownership: Ownership,
    pub owner: Owner,
}
pub struct Change {
    pub previous: Option<Snapshot>,
    pub current: Snapshot,
}
pub enum OwnershipUpdate {
    Unchanged,
    Replaced(Change),
    Cleared(Snapshot),
}
/// One ledger per Xwayland generation. Generations must never be reused by the
/// host. Callers cancel transfers/offers bearing a returned previous ownership
/// before publishing its replacement. Only two live records are retained.
pub struct Ownerships {
    generation: u64,
    next: u64,
    current: [Option<Snapshot>; 2],
    revisions: [u64; 2],
}
impl Ownerships {
    /// Apply an authenticated, subscription-filtered notification in server order.
    /// Local proxy notifications are handled by Publication and never become X11
    /// sources here. The host maps the wire selection atom to Selection first.
    /// Process returned old snapshots for offer/transfer cancellation.
    pub fn external_owner(
        &mut self,
        generation: u64,
        selection: Selection,
        owner: u32,
        proxy_window: u32,
    ) -> Result<OwnershipUpdate> {
        if generation != self.generation || proxy_window == 0 {
            return Err(Error("invalid external selection ownership context".into()));
        }
        if owner == proxy_window {
            return Ok(OwnershipUpdate::Unchanged);
        }
        if owner == 0 {
            return Ok(match self.clear(selection) {
                Some(old) => OwnershipUpdate::Cleared(old),
                None => OwnershipUpdate::Unchanged,
            });
        }
        self.replace(selection, Owner::X11(owner))
            .map(OwnershipUpdate::Replaced)
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn new(generation: u64) -> Result<Self> {
        if generation == 0 {
            return Err(Error("invalid selection server generation".into()));
        }
        Ok(Self {
            generation,
            next: 0,
            current: [None; 2],
            revisions: [0; 2],
        })
    }
    pub fn get(&self, selection: Selection) -> Option<Snapshot> {
        self.current[selection.index()]
    }
    /// Last replacement revision, retained after clear to detect replace/clear races.
    pub fn revision(&self, selection: Selection) -> u64 {
        self.revisions[selection.index()]
    }
    pub fn is_current(&self, ownership: Ownership) -> bool {
        self.get(ownership.selection)
            .is_some_and(|s| s.ownership == ownership)
    }
    /// Every accepted source/content change gets a new revision, including a
    /// repeated owner. Do not call for a proxy echo of the current publication.
    /// Rejection leaves the existing owner intact; exhaustion never reuses IDs.
    pub fn replace(&mut self, selection: Selection, owner: Owner) -> Result<Change> {
        if matches!(owner, Owner::Native(0) | Owner::X11(0)) {
            return Err(Error("invalid selection owner".into()));
        }
        let revision = self
            .next
            .checked_add(1)
            .ok_or_else(|| Error("selection ownership revisions exhausted".into()))?;
        let current = Snapshot {
            ownership: Ownership {
                generation: self.generation,
                revision,
                selection,
            },
            owner,
        };
        let previous = self.current[selection.index()].replace(current);
        self.next = revision;
        self.revisions[selection.index()] = revision;
        Ok(Change { previous, current })
    }
    /// Revocation does not allocate a revision, so it works even after exhaustion.
    pub fn clear(&mut self, selection: Selection) -> Option<Snapshot> {
        self.current[selection.index()].take()
    }
    /// Revoke only the ownership associated with a routed event or asynchronous
    /// failure. A delayed notification must not clear a replacement source.
    pub fn revoke(&mut self, ownership: Ownership) -> Option<Snapshot> {
        if self.is_current(ownership) {
            self.clear(ownership.selection)
        } else {
            None
        }
    }
    pub fn clear_all(&mut self) -> [Option<Snapshot>; 2] {
        std::mem::replace(&mut self.current, [None; 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subscription_routes_packets_to_only_its_ledger_selection() {
        use x11rb_protocol::protocol::xfixes;
        let mut owners = Ownerships::new(1).unwrap();
        let subscription = Subscription {
            generation: 1,
            selection: Selection::Clipboard,
            atom: 11,
            window: 10,
            proxy_window: 12,
            first_event: 87,
        };
        let event = xfixes::SelectionNotifyEvent {
            response_type: 87,
            window: 10,
            owner: 20,
            selection: 11,
            ..Default::default()
        };
        let wire: [u8; 32] = event.into();
        assert!(matches!(
            subscription.event(&wire, &mut owners).unwrap(),
            OwnershipUpdate::Replaced(_)
        ));
        assert_eq!(
            owners.get(Selection::Clipboard).unwrap().owner,
            Owner::X11(20)
        );
        assert!(owners.get(Selection::Primary).is_none());
        let unrelated: [u8; 32] = xfixes::SelectionNotifyEvent {
            selection: 1,
            ..event
        }
        .into();
        assert!(matches!(
            subscription.event(&unrelated, &mut owners).unwrap(),
            OwnershipUpdate::Unchanged
        ));
        let local: [u8; 32] = xfixes::SelectionNotifyEvent { owner: 12, ..event }.into();
        assert!(matches!(
            subscription.event(&local, &mut owners).unwrap(),
            OwnershipUpdate::Unchanged
        ));
        let cleared: [u8; 32] = xfixes::SelectionNotifyEvent { owner: 0, ..event }.into();
        assert!(matches!(
            subscription.event(&cleared, &mut owners).unwrap(),
            OwnershipUpdate::Cleared(_)
        ));
        assert!(owners.get(Selection::Clipboard).is_none());
        assert!(
            subscription
                .event(&wire, &mut Ownerships::new(2).unwrap())
                .is_err()
        );
    }
    #[test]
    fn external_owner_updates_exclude_proxy_and_preserve_other_selection() {
        let mut owners = Ownerships::new(1).unwrap();
        let primary = owners
            .replace(Selection::Primary, Owner::Native(7))
            .unwrap()
            .current;
        assert!(matches!(
            owners
                .external_owner(1, Selection::Clipboard, 10, 10)
                .unwrap(),
            OwnershipUpdate::Unchanged
        ));
        let OwnershipUpdate::Replaced(first) = owners
            .external_owner(1, Selection::Clipboard, 20, 10)
            .unwrap()
        else {
            panic!("missing owner");
        };
        let OwnershipUpdate::Replaced(second) = owners
            .external_owner(1, Selection::Clipboard, 20, 10)
            .unwrap()
        else {
            panic!("missing replacement");
        };
        assert_eq!(second.previous, Some(first.current));
        assert!(!owners.is_current(first.current.ownership));
        assert!(
            owners
                .external_owner(2, Selection::Clipboard, 0, 10)
                .is_err()
        );
        assert_eq!(owners.get(Selection::Clipboard), Some(second.current));
        let OwnershipUpdate::Cleared(old) = owners
            .external_owner(1, Selection::Clipboard, 0, 10)
            .unwrap()
        else {
            panic!("missing clear");
        };
        assert_eq!(old, second.current);
        assert!(matches!(
            owners
                .external_owner(1, Selection::Clipboard, 0, 10)
                .unwrap(),
            OwnershipUpdate::Unchanged
        ));
        assert_eq!(owners.get(Selection::Primary), Some(primary));
    }
    #[test]
    fn ownership_events_reject_synthetic_malformed_and_unknown_packets() {
        use x11rb_protocol::protocol::xfixes;
        for subtype in [0u8, 1, 2] {
            let event = xfixes::SelectionNotifyEvent {
                response_type: 87,
                subtype: subtype.into(),
                window: 10,
                selection: 11,
                owner: 12,
                timestamp: 14,
                selection_timestamp: 13,
                ..Default::default()
            };
            let mut wire: [u8; 32] = event.into();
            let decoded = owner_event(&wire, 87, 10, 11).unwrap().unwrap();
            assert_eq!(decoded.owner, 12);
            assert_eq!(decoded.selection_timestamp, 13);
            assert!(owner_event(&wire, 87, 20, 11).unwrap().is_none());
            assert!(owner_event(&wire, 87, 10, 20).unwrap().is_none());
            assert!(owner_event(&wire[..31], 87, 10, 11).is_err());
            wire[0] |= 0x80;
            assert!(owner_event(&wire, 87, 10, 11).unwrap().is_none());
            wire[0] = 87;
            wire[1] = 255;
            assert!(owner_event(&wire, 87, 10, 11).is_err());
        }
        assert!(owner_event(&[], 0, 10, 11).is_err());
    }
    #[test]
    fn watch_serializes_all_loss_reasons_and_can_unsubscribe() {
        use super::super::discovery::tests::{flush_requests, ready};
        use std::io::Read;
        let (mut transport, mut requests, _, mut peer, now) = ready();
        let count = requests.outstanding();
        assert!(watch(127, 10, 11, true, &mut transport, &mut requests, now).is_err());
        assert!(watch(138, 0, 11, true, &mut transport, &mut requests, now).is_err());
        assert_eq!(requests.outstanding(), count);
        let mut subscription = Subscription {
            generation: requests.generation() + 1,
            selection: Selection::Clipboard,
            atom: 11,
            window: 10,
            proxy_window: 12,
            first_event: 87,
        };
        assert!(
            subscription
                .queue(138, true, &mut transport, &mut requests, now)
                .is_err()
        );
        assert_eq!(requests.outstanding(), count);
        subscription.generation = requests.generation();
        for enabled in [true, false] {
            subscription
                .queue(138, enabled, &mut transport, &mut requests, now)
                .unwrap();
            flush_requests(&mut transport);
            let mut wire = [0; 16];
            peer.read_exact(&mut wire).unwrap();
            assert_eq!(wire[0], 138);
            assert_eq!(wire[1], 2);
            assert_eq!(u32::from_ne_bytes(wire[4..8].try_into().unwrap()), 10);
            assert_eq!(u32::from_ne_bytes(wire[8..12].try_into().unwrap()), 11);
            assert_eq!(
                u32::from_ne_bytes(wire[12..16].try_into().unwrap()),
                if enabled { 7 } else { 0 }
            );
        }
    }
    #[test]
    fn request_intervals_handle_wrap_boundaries_and_current_time() {
        for (acquired, current) in [(10, 20), (u32::MAX - 5, 4)] {
            assert!(request_in_interval(acquired, current, acquired));
            assert!(request_in_interval(acquired, current, current));
            assert!(request_in_interval(acquired, current, 0));
            assert!(!request_in_interval(
                acquired,
                current,
                acquired.wrapping_sub(1)
            ));
            assert!(!request_in_interval(
                acquired,
                current,
                current.wrapping_add(1)
            ));
        }
        assert!(request_in_interval(u32::MAX - 5, 4, 1));
        assert!(request_in_interval(10, 10, 10));
        assert!(!request_in_interval(20, 10, 15));
        assert!(!request_in_interval(1, 0x8000_0001, 1));
        assert!(!request_in_interval(0, 10, 0));
        assert!(!request_in_interval(1, 0, 0));
    }
    #[test]
    fn delayed_revocation_cannot_clear_replacement_or_other_selection() {
        let mut owners = Ownerships::new(1).unwrap();
        let old = owners
            .replace(Selection::Clipboard, Owner::X11(10))
            .unwrap()
            .current;
        let replacement = owners
            .replace(Selection::Clipboard, Owner::X11(10))
            .unwrap()
            .current;
        let primary = owners
            .replace(Selection::Primary, Owner::Native(1))
            .unwrap()
            .current;
        assert_eq!(owners.revoke(old.ownership), None);
        assert_eq!(owners.get(Selection::Clipboard), Some(replacement));
        assert_eq!(owners.get(Selection::Primary), Some(primary));
        let mut restarted = Ownerships::new(2).unwrap();
        let current = restarted
            .replace(Selection::Clipboard, Owner::X11(10))
            .unwrap()
            .current;
        assert_eq!(restarted.revoke(old.ownership), None);
        assert_eq!(restarted.get(Selection::Clipboard), Some(current));
        assert_eq!(owners.revoke(replacement.ownership), Some(replacement));
        assert_eq!(owners.revoke(replacement.ownership), None);
        assert_eq!(owners.get(Selection::Clipboard), None);
        assert_eq!(owners.get(Selection::Primary), Some(primary));
    }
    #[test]
    fn same_owner_replacement_invalidates_old_work_without_touching_primary() {
        let mut owners = Ownerships::new(1).unwrap();
        let first = owners
            .replace(Selection::Clipboard, Owner::X11(10))
            .unwrap()
            .current;
        let primary = owners
            .replace(Selection::Primary, Owner::Native(7))
            .unwrap()
            .current;
        let change = owners
            .replace(Selection::Clipboard, Owner::X11(10))
            .unwrap();
        assert_eq!(change.previous, Some(first));
        assert!(!owners.is_current(first.ownership));
        assert!(owners.is_current(change.current.ownership));
        assert!(owners.is_current(primary.ownership));
        assert_ne!(
            first.ownership.revision(),
            change.current.ownership.revision()
        );
        assert_eq!(owners.clear(Selection::Clipboard), Some(change.current));
        assert!(!owners.is_current(change.current.ownership));
        assert!(owners.is_current(primary.ownership));
        assert_eq!(owners.clear_all(), [None, Some(primary)]);
        assert!(!owners.is_current(primary.ownership));
    }
    #[test]
    fn restart_invalidates_tokens_and_exhaustion_preserves_revocation() {
        assert!(Ownerships::new(0).is_err());
        let mut old = Ownerships::new(1).unwrap();
        let token = old
            .replace(Selection::Clipboard, Owner::Native(1))
            .unwrap()
            .current
            .ownership;
        let mut owners = Ownerships::new(2).unwrap();
        let current = owners
            .replace(Selection::Clipboard, Owner::Native(1))
            .unwrap()
            .current;
        assert!(!owners.is_current(token));
        for owner in [Owner::Native(0), Owner::X11(0)] {
            assert!(owners.replace(Selection::Clipboard, owner).is_err());
            assert_eq!(owners.get(Selection::Clipboard), Some(current));
        }
        owners.next = u64::MAX;
        assert!(
            owners
                .replace(Selection::Clipboard, Owner::Native(2))
                .is_err()
        );
        assert_eq!(owners.get(Selection::Clipboard), Some(current));
        owners.clear_all();
        assert!(!owners.is_current(current.ownership));
    }
}
