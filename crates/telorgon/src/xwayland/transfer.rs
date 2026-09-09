//! Bounded byte forwarding for compositor-created selection socket endpoints.
//! Socket FDs can be passed as Wayland transfer FDs. X11 INCR/property handshakes
//! and selection ownership remain the caller's responsibility.
use super::{
    Error, Result,
    selection::{Ownership, Ownerships},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, RawFd},
        unix::net::UnixStream,
    },
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Generation(u64),
    Selection(Ownership),
}
const BUFFER_LIMIT: usize = 256 * 1024;
const TURN_BYTES: usize = 64 * 1024;
const IDLE: Duration = Duration::from_secs(10);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Complete,
    Cancelled,
    Failed,
}
#[derive(Clone, Copy, Debug)]
pub struct Turn {
    pub status: Status,
    pub readable_interest: bool,
    pub writable_interest: bool,
    pub reschedule: bool,
}
pub struct Transfer {
    endpoints: Option<(UnixStream, UnixStream)>,
    buffer: VecDeque<u8>,
    received: u64,
    limit: u64,
    eof: bool,
    progress: Instant,
    status: Status,
}
impl Transfer {
    pub const DEFAULT_LIMIT: u64 = 64 * 1024 * 1024;
    /// Takes exclusive ownership; these endpoints must not have retained clones.
    pub fn new(input: UnixStream, output: UnixStream, limit: u64, now: Instant) -> Result<Self> {
        if limit == 0 {
            return Err(Error("selection transfer limit must be positive".into()));
        }
        input.set_nonblocking(true)?;
        output.set_nonblocking(true)?;
        Ok(Self {
            endpoints: Some((input, output)),
            buffer: VecDeque::new(),
            received: 0,
            limit,
            eof: false,
            progress: now,
            status: Status::Running,
        })
    }
    pub fn fds(&self) -> Option<(RawFd, RawFd)> {
        self.endpoints
            .as_ref()
            .map(|(input, output)| (input.as_raw_fd(), output.as_raw_fd()))
    }
    pub fn deadline(&self) -> Option<Instant> {
        (self.status == Status::Running).then(|| self.progress + IDLE)
    }
    pub fn status(&self) -> Status {
        self.status
    }
    pub fn buffered_bytes(&self) -> usize {
        self.buffer.len()
    }
    pub fn cancel(&mut self) {
        if self.status == Status::Running {
            self.finish(Status::Cancelled);
        }
    }
    fn finish(&mut self, status: Status) {
        self.status = status;
        self.endpoints = None;
        self.buffer = VecDeque::new();
    }
    pub fn dispatch(&mut self, now: Instant) -> Result<Turn> {
        self.dispatch_until(now, Instant::now() + Duration::from_millis(1))
    }
    fn dispatch_until(&mut self, now: Instant, work_deadline: Instant) -> Result<Turn> {
        let result = self.pump(now, work_deadline);
        if result.is_err() {
            self.finish(Status::Failed);
        }
        result
    }
    fn pump(&mut self, now: Instant, work_deadline: Instant) -> Result<Turn> {
        if self.status != Status::Running {
            return Ok(Turn {
                status: self.status,
                readable_interest: false,
                writable_interest: false,
                reschedule: false,
            });
        }
        if now >= self.progress + IDLE {
            return Err(Error("selection transfer inactivity timeout".into()));
        }
        let mut work = 0;
        let (input, output) = self.endpoints.as_mut().unwrap();
        while work < TURN_BYTES && Instant::now() < work_deadline {
            let mut progressed = false;
            if !self.buffer.is_empty() {
                let bytes = self.buffer.as_slices().0;
                match output.write(&bytes[..bytes.len().min(TURN_BYTES - work)]) {
                    Ok(0) => return Err(Error("selection transfer output closed".into())),
                    Ok(count) => {
                        self.buffer.drain(..count);
                        work += count;
                        progressed = true;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            if !self.eof && self.buffer.len() < BUFFER_LIMIT && work < TURN_BYTES {
                let mut bytes = [0; 8192];
                let count = bytes
                    .len()
                    .min(BUFFER_LIMIT - self.buffer.len())
                    .min(TURN_BYTES - work);
                match input.read(&mut bytes[..count]) {
                    Ok(0) => {
                        self.eof = true;
                        progressed = true;
                    }
                    Ok(count) => {
                        self.received = self
                            .received
                            .checked_add(count as u64)
                            .ok_or_else(|| Error("selection transfer size overflow".into()))?;
                        if self.received > self.limit {
                            return Err(Error("selection transfer size limit exceeded".into()));
                        }
                        self.buffer.extend(&bytes[..count]);
                        work += count;
                        progressed = true;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            if progressed {
                self.progress = now;
            } else {
                break;
            }
            if self.eof && self.buffer.is_empty() {
                break;
            }
        }
        if self.eof && self.buffer.is_empty() {
            self.finish(Status::Complete);
        }
        Ok(Turn {
            status: self.status,
            readable_interest: self.status == Status::Running
                && !self.eof
                && self.buffer.len() < BUFFER_LIMIT,
            writable_interest: self.status == Status::Running && !self.buffer.is_empty(),
            reschedule: self.status == Status::Running
                && (work >= TURN_BYTES || Instant::now() >= work_deadline),
        })
    }
}
/// Stable only within its originating registry. Removed identities are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TransferId(u64);

/// Admission and teardown for selection transfers. Prefer typed ownership tokens
/// for selections; opaque generation scopes remain available for other callers.
#[derive(Default)]
pub struct Transfers {
    next: u64,
    last_dispatched: Option<TransferId>,
    entries: BTreeMap<TransferId, (Scope, Transfer)>,
}
pub struct Batch {
    pub updates: Vec<(TransferId, Result<Turn>)>,
    /// Retain these IDs for another turn even if no new readiness edge arrives.
    pub reschedule: BTreeSet<TransferId>,
}
impl Transfers {
    pub const LIMIT: usize = 16;
    /// Round-robin service of ready or expired transfers under one shared 1 ms budget.
    /// Failures remain per-transfer; callers must observe and remove terminal entries.
    pub fn dispatch_ready(&mut self, ready: &BTreeSet<TransferId>, now: Instant) -> Batch {
        self.dispatch_ready_until(ready, now, Instant::now() + Duration::from_millis(1))
    }
    fn dispatch_ready_until(
        &mut self,
        ready: &BTreeSet<TransferId>,
        now: Instant,
        work_deadline: Instant,
    ) -> Batch {
        let mut ids: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(id, (_, transfer))| {
                (transfer.status() == Status::Running
                    && (ready.contains(id)
                        || transfer.deadline().is_some_and(|deadline| deadline <= now)))
                .then_some(*id)
            })
            .collect();
        if let Some(last) = self.last_dispatched {
            let pivot = ids.partition_point(|id| *id <= last);
            ids.rotate_left(pivot);
        }
        let mut batch = Batch {
            updates: Vec::new(),
            reschedule: BTreeSet::new(),
        };
        for id in ids {
            if Instant::now() >= work_deadline {
                batch.reschedule.insert(id);
                continue;
            }
            let result = self
                .entries
                .get_mut(&id)
                .unwrap()
                .1
                .dispatch_until(now, work_deadline);
            if result.as_ref().is_ok_and(|turn| turn.reschedule) {
                batch.reschedule.insert(id);
            }
            self.last_dispatched = Some(id);
            batch.updates.push((id, result));
        }
        batch
    }
    /// Register only against a currently live selection ownership. Full tokens
    /// keep server generations and clipboard/PRIMARY revisions distinct.
    pub fn insert_owned(
        &mut self,
        owners: &Ownerships,
        ownership: Ownership,
        transfer: Transfer,
    ) -> Result<TransferId> {
        if !owners.is_current(ownership) {
            return Err(Error("selection transfer ownership is stale".into()));
        }
        self.insert_scoped(Scope::Selection(ownership), transfer)
    }
    pub fn ownership_ids(&self, ownership: Ownership) -> Vec<TransferId> {
        self.scope_ids(Scope::Selection(ownership))
    }
    /// Unregister readiness sources for ownership_ids first, then close endpoints.
    pub fn cancel_ownership(&mut self, ownership: Ownership) -> Vec<TransferId> {
        self.cancel_scope(Scope::Selection(ownership))
    }
    /// Includes superseded ownerships still awaiting teardown, not just live ledger entries.
    pub fn selection_server_ids(&self, generation: u64) -> Vec<TransferId> {
        self.entries
            .iter()
            .filter_map(|(id, (scope, _))| {
                matches!(scope, Scope::Selection(owner) if owner.generation() == generation)
                    .then_some(*id)
            })
            .collect()
    }
    /// Unregister selection_server_ids before closing endpoints. Legacy opaque
    /// generation scopes and transfers from other servers are unaffected.
    pub fn cancel_selection_server(&mut self, generation: u64) -> Vec<TransferId> {
        let ids = self.selection_server_ids(generation);
        for id in &ids {
            self.entries.remove(id);
        }
        ids
    }
    /// On rejection the supplied transfer is dropped, closing its owned endpoints.
    pub fn insert(&mut self, generation: u64, transfer: Transfer) -> Result<TransferId> {
        self.insert_scoped(Scope::Generation(generation), transfer)
    }
    fn insert_scoped(&mut self, scope: Scope, transfer: Transfer) -> Result<TransferId> {
        if self.entries.len() >= Self::LIMIT {
            return Err(Error("selection transfer concurrency limit reached".into()));
        }
        if transfer.status() != Status::Running {
            return Err(Error(
                "cannot register a terminal selection transfer".into(),
            ));
        }
        let next = self
            .next
            .checked_add(1)
            .ok_or_else(|| Error("selection transfer identities exhausted".into()))?;
        let id = TransferId(next);
        self.entries.insert(id, (scope, transfer));
        self.next = next;
        Ok(id)
    }
    pub fn get(&self, id: TransferId) -> Option<&Transfer> {
        self.entries.get(&id).map(|(_, transfer)| transfer)
    }
    pub fn get_mut(&mut self, id: TransferId) -> Option<&mut Transfer> {
        self.entries.get_mut(&id).map(|(_, transfer)| transfer)
    }
    pub fn remove(&mut self, id: TransferId) -> Option<Transfer> {
        self.entries.remove(&id).map(|(_, transfer)| transfer)
    }
    /// Call after unregistering readiness sources for these IDs. Returned identities
    /// can be used to discard queued callbacks; lookup will no longer resolve them.
    pub fn cancel_generation(&mut self, generation: u64) -> Vec<TransferId> {
        self.cancel_scope(Scope::Generation(generation))
    }
    fn cancel_scope(&mut self, scope: Scope) -> Vec<TransferId> {
        let ids = self.scope_ids(scope);
        for id in &ids {
            self.entries.remove(id);
        }
        ids
    }
    pub fn generation_ids(&self, generation: u64) -> Vec<TransferId> {
        self.scope_ids(Scope::Generation(generation))
    }
    fn scope_ids(&self, scope: Scope) -> Vec<TransferId> {
        self.entries
            .iter()
            .filter_map(|(id, (owner, _))| (*owner == scope).then_some(*id))
            .collect()
    }
    pub fn deadline(&self) -> Option<Instant> {
        self.entries
            .values()
            .filter_map(|(_, transfer)| transfer.deadline())
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn transfer(limit: u64) -> (Transfer, UnixStream, UnixStream, Instant) {
        let (sender, input) = UnixStream::pair().unwrap();
        let (output, receiver) = UnixStream::pair().unwrap();
        let now = Instant::now();
        (
            Transfer::new(input, output, limit, now).unwrap(),
            sender,
            receiver,
            now,
        )
    }
    #[test]
    fn server_teardown_includes_superseded_ownerships_and_preserves_other_servers() {
        use super::super::selection::{Owner, Selection};
        let mut owners = Ownerships::new(1).unwrap();
        let first = owners
            .replace(Selection::Clipboard, Owner::X11(10))
            .unwrap()
            .current
            .ownership;
        let mut registry = Transfers::default();
        let (old, _s1, mut r1, _) = transfer(8);
        let old = registry.insert_owned(&owners, first, old).unwrap();
        let replacement = owners
            .replace(Selection::Clipboard, Owner::X11(11))
            .unwrap()
            .current
            .ownership;
        let (fresh, _s2, mut r2, _) = transfer(8);
        let fresh = registry.insert_owned(&owners, replacement, fresh).unwrap();
        let mut next_server = Ownerships::new(2).unwrap();
        let next_owner = next_server
            .replace(Selection::Clipboard, Owner::X11(10))
            .unwrap()
            .current
            .ownership;
        let (other, _s3, _r3, _) = transfer(8);
        let other = registry
            .insert_owned(&next_server, next_owner, other)
            .unwrap();
        owners.clear_all();
        assert_eq!(registry.selection_server_ids(1), vec![old, fresh]);
        assert_eq!(registry.cancel_selection_server(1), vec![old, fresh]);
        assert_eq!(r1.read(&mut [0; 1]).unwrap(), 0);
        assert_eq!(r2.read(&mut [0; 1]).unwrap(), 0);
        assert!(registry.get(other).is_some());
        assert!(registry.cancel_selection_server(1).is_empty());
        assert_eq!(registry.selection_server_ids(2), vec![other]);
    }
    #[test]
    fn ownership_replacement_cancels_only_matching_transfers() {
        use super::super::selection::{Owner, Selection};
        let mut owners = Ownerships::new(1).unwrap();
        let clipboard = owners
            .replace(Selection::Clipboard, Owner::X11(10))
            .unwrap()
            .current
            .ownership;
        let primary = owners
            .replace(Selection::Primary, Owner::Native(8))
            .unwrap()
            .current
            .ownership;
        let mut registry = Transfers::default();
        let (old, _sender, mut receiver, _) = transfer(8);
        let old_id = registry.insert_owned(&owners, clipboard, old).unwrap();
        let (other, _sender2, _receiver2, _) = transfer(8);
        let primary_id = registry.insert_owned(&owners, primary, other).unwrap();
        let (legacy, _sender3, _receiver3, _) = transfer(8);
        let legacy_id = registry.insert(clipboard.revision(), legacy).unwrap();
        assert_eq!(registry.ownership_ids(clipboard), vec![old_id]);
        let change = owners
            .replace(Selection::Clipboard, Owner::X11(10))
            .unwrap();
        let (stale, _sender4, mut rejected_receiver, _) = transfer(8);
        assert!(registry.insert_owned(&owners, clipboard, stale).is_err());
        assert_eq!(rejected_receiver.read(&mut [0; 1]).unwrap(), 0);
        assert_eq!(
            registry.cancel_ownership(change.previous.unwrap().ownership),
            vec![old_id]
        );
        assert_eq!(receiver.read(&mut [0; 1]).unwrap(), 0);
        assert!(registry.get(old_id).is_none());
        assert!(registry.get(primary_id).is_some());
        assert!(registry.get(legacy_id).is_some());
        let (fresh, _sender5, _receiver5, _) = transfer(8);
        let fresh_id = registry
            .insert_owned(&owners, change.current.ownership, fresh)
            .unwrap();
        assert!(registry.cancel_ownership(clipboard).is_empty());
        assert!(registry.get(fresh_id).is_some());
        assert_eq!(
            registry.cancel_generation(clipboard.revision()),
            vec![legacy_id]
        );
        assert!(registry.get(primary_id).is_some());
        assert!(registry.get(fresh_id).is_some());
    }
    #[test]
    fn registry_dispatch_retains_deferred_work_and_contains_expiry() {
        let mut registry = Transfers::default();
        let (old, _sender, _receiver, now) = transfer(3);
        let old = registry.insert(1, old).unwrap();
        let (mut fresh, mut sender, _receiver2, _) = transfer(3);
        fresh.progress = now + IDLE;
        sender.write_all(b"ok").unwrap();
        let fresh = registry.insert(2, fresh).unwrap();
        let ready = [fresh].into();
        let batch = registry.dispatch_ready_until(&ready, now + IDLE, Instant::now());
        assert!(batch.updates.is_empty());
        assert_eq!(batch.reschedule, [old, fresh].into());
        registry.last_dispatched = Some(old);
        let batch = registry.dispatch_ready_until(
            &ready,
            now + IDLE,
            Instant::now() + Duration::from_secs(1),
        );
        assert_eq!(batch.updates.len(), 2);
        assert_eq!(batch.updates[0].0, fresh);
        assert!(
            batch
                .updates
                .iter()
                .any(|(id, result)| *id == old && result.is_err())
        );
        assert!(
            batch
                .updates
                .iter()
                .any(|(id, result)| *id == fresh && result.is_ok())
        );
        assert_eq!(registry.get(old).unwrap().status(), Status::Failed);
        assert_eq!(registry.get(fresh).unwrap().status(), Status::Running);
        assert_eq!(registry.get(fresh).unwrap().received, 2);
    }

    #[test]
    fn registry_bounds_admission_and_revokes_only_the_selected_generation() {
        let mut registry = Transfers::default();
        let mut peers = Vec::new();
        let mut ids = Vec::new();
        for index in 0..Transfers::LIMIT {
            let (transfer, sender, receiver, _) = transfer(3);
            peers.push((sender, receiver));
            ids.push(registry.insert((index % 2) as u64, transfer).unwrap());
        }
        let (extra, _, _, _) = transfer(3);
        assert!(registry.insert(0, extra).is_err());
        assert!(registry.deadline().is_some());
        let expected = registry.generation_ids(0);
        assert_eq!(expected.len(), 8);
        assert_eq!(registry.cancel_generation(0), expected);
        for index in (0..Transfers::LIMIT).step_by(2) {
            peers[index]
                .1
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            assert_eq!(peers[index].1.read(&mut [0; 1]).unwrap(), 0);
        }
        for id in expected {
            assert!(registry.get_mut(id).is_none());
        }
        assert!(registry.get(ids[1]).is_some());
        assert!(registry.cancel_generation(0).is_empty());
        let (fresh, sender, receiver, _) = transfer(3);
        peers.push((sender, receiver));
        let fresh = registry.insert(0, fresh).unwrap();
        assert!(!ids.contains(&fresh));
        assert!(registry.remove(fresh).is_some());
        assert!(registry.remove(fresh).is_none());
        registry.next = u64::MAX;
        let (extra, _, _, _) = transfer(3);
        assert!(registry.insert(2, extra).is_err());
        assert_eq!(registry.generation_ids(1).len(), 8);
    }

    #[test]
    fn forwards_binary_and_completes_only_after_eof_and_drain() {
        let (mut transfer, mut sender, mut receiver, now) = transfer(4);
        sender.write_all(&[0, 255, 1, 0]).unwrap();
        for _ in 0..10 {
            transfer.dispatch(now).unwrap();
        }
        assert_eq!(transfer.status(), Status::Running);
        sender.shutdown(std::net::Shutdown::Write).unwrap();
        for _ in 0..10 {
            transfer.dispatch(now).unwrap();
        }
        assert_eq!(transfer.status(), Status::Complete);
        let mut result = Vec::new();
        receiver.read_to_end(&mut result).unwrap();
        assert_eq!(result, [0, 255, 1, 0]);
        assert!(transfer.fds().is_none());
    }
    #[test]
    fn backpressure_recovery_preserves_order_across_many_turns() {
        let data: Vec<u8> = (0..1024 * 1024).map(|index| (index % 251) as u8).collect();
        let (mut transfer, mut sender, mut receiver, now) = transfer(data.len() as u64);
        sender.set_nonblocking(true).unwrap();
        receiver.set_nonblocking(true).unwrap();
        let mut sent = 0;
        let mut received = Vec::new();
        let mut blocked = false;
        let mut closed = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while transfer.status() == Status::Running {
            assert!(
                Instant::now() < deadline,
                "transfer failed to make progress"
            );
            if sent < data.len() {
                match sender.write(&data[sent..data.len().min(sent + 8192)]) {
                    Ok(count) => sent += count,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => panic!("{error}"),
                }
            } else if !closed {
                sender.shutdown(std::net::Shutdown::Write).unwrap();
                closed = true;
            }
            let turn = transfer.dispatch(now).unwrap();
            assert!(transfer.buffered_bytes() <= BUFFER_LIMIT);
            if transfer.buffered_bytes() == BUFFER_LIMIT {
                blocked = true;
                assert!(!turn.readable_interest);
            }
            if blocked {
                let mut buffer = [0; 4093];
                match receiver.read(&mut buffer) {
                    Ok(count) => received.extend_from_slice(&buffer[..count]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => panic!("{error}"),
                }
            }
        }
        assert!(blocked);
        assert_eq!(transfer.status(), Status::Complete);
        receiver.set_nonblocking(false).unwrap();
        receiver.read_to_end(&mut received).unwrap();
        assert_eq!(received, data);
    }

    #[test]
    fn stalled_receiver_bounds_buffer_and_times_out_without_reporting_completion() {
        let (mut transfer, mut sender, _receiver, now) = transfer(4 * 1024 * 1024);
        sender.set_nonblocking(true).unwrap();
        let bytes = [7; 8192];
        let mut bounded = false;
        for _ in 0..2048 {
            match sender.write(&bytes) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("{error}"),
            }
            let turn = transfer.dispatch(now).unwrap();
            assert!(transfer.buffered_bytes() <= BUFFER_LIMIT);
            if transfer.buffered_bytes() == BUFFER_LIMIT {
                assert!(!turn.readable_interest);
                assert!(turn.writable_interest);
                bounded = true;
                break;
            }
        }
        assert!(bounded);
        assert!(transfer.dispatch(now + IDLE).is_err());
        assert_eq!(transfer.status(), Status::Failed);
        assert_eq!(transfer.buffered_bytes(), 0);
        assert!(transfer.fds().is_none());
    }

    #[test]
    fn size_timeout_cancellation_and_disconnect_close_owned_endpoints() {
        let (mut transfer, mut sender, _receiver, now) = transfer(3);
        sender.write_all(b"four").unwrap();
        assert!(transfer.dispatch(now).is_err());
        assert_eq!(transfer.status(), Status::Failed);
        assert!(transfer.fds().is_none());
        let (mut transfer, _sender, _receiver, now) = self::transfer(3);
        assert!(transfer.dispatch(now + IDLE).is_err());
        let (mut transfer, _sender, _receiver, _) = self::transfer(3);
        transfer.cancel();
        transfer.cancel();
        assert_eq!(transfer.status(), Status::Cancelled);
        assert!(transfer.deadline().is_none());
        let (mut transfer, mut sender, receiver, now) = self::transfer(3);
        drop(receiver);
        sender.write_all(b"x").unwrap();
        let mut failed = false;
        for _ in 0..10 {
            if transfer.dispatch(now).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
    }
}
