//! Bounded asynchronous XWM request accounting over x11rb's sans-I/O connection.
//! Timed-out optional requests retain their sequence slots until server progress
//! resolves them. Timeouts therefore cannot turn a stalled peer into an unbounded
//! queue or permit a delayed reply to attach to a reused request identity.
use super::{Error, Result, transport::Transport};
use std::{collections::BTreeMap, time::Instant};
use x11rb_protocol::connection::{Connection, PollReply, ReplyFdKind};

const LIMIT: usize = 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestId {
    pub generation: u64,
    pub sequence: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyKind {
    Reply,
    Void,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Importance {
    Essential,
    Optional,
}
#[derive(Debug)]
pub enum Completion {
    Reply(RequestId, Vec<u8>),
    Checked(RequestId),
    Error(RequestId, u8),
    TimedOut(RequestId),
    Event(Vec<u8>),
}
struct Pending {
    kind: ReplyKind,
    importance: Importance,
    deadline: Instant,
    expired: bool,
}
pub struct Requests {
    generation: u64,
    connection: Connection,
    pending: BTreeMap<u64, Pending>,
    last_sent: u64,
    last_read: u64,
    failed: bool,
}
impl Requests {
    pub fn new(generation: u64) -> Result<Self> {
        if generation == 0 {
            return Err(Error("invalid XWM request generation".into()));
        }
        Ok(Self {
            generation,
            connection: Connection::new(),
            pending: BTreeMap::new(),
            last_sent: 0,
            last_read: 0,
            failed: false,
        })
    }
    pub(crate) fn last_sequence(&self) -> u64 {
        self.last_sent
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn deadline(&self) -> Option<Instant> {
        self.pending
            .values()
            .filter(|p| !p.expired)
            .map(|p| p.deadline)
            .min()
    }
    pub fn available_slots(&self) -> usize {
        LIMIT - self.pending.len()
    }
    pub fn outstanding(&self) -> usize {
        self.pending.len()
    }
    fn register(
        &mut self,
        kind: ReplyKind,
        importance: Importance,
        deadline: Instant,
    ) -> Result<RequestId> {
        if self.failed {
            return Err(Error("XWM request tracker has failed".into()));
        }
        if self.pending.len() >= LIMIT
            || self.last_sent == u64::MAX
            || self
                .pending
                .first_key_value()
                .is_some_and(|(first, _)| self.last_sent - first >= 65534)
        {
            return Err(Error("XWM outstanding request bound reached".into()));
        }
        let sequence = self
            .connection
            .send_request(match kind {
                ReplyKind::Reply => ReplyFdKind::ReplyWithoutFDs,
                ReplyKind::Void => ReplyFdKind::NoReply,
            })
            .ok_or_else(|| Error("XWM requires an asynchronous synchronization request".into()))?;
        self.last_sent = sequence;
        self.pending.insert(
            sequence,
            Pending {
                kind,
                importance,
                deadline,
                expired: false,
            },
        );
        Ok(RequestId {
            generation: self.generation,
            sequence,
        })
    }
    /// Use this for every post-setup XWM request so sequence numbers match bytes
    /// on the wire. A rejected queue attempt consumes no request identity.
    pub fn queue(
        &mut self,
        transport: &mut Transport,
        bytes: Vec<u8>,
        kind: ReplyKind,
        importance: Importance,
        deadline: Instant,
    ) -> Result<RequestId> {
        transport.check_queue(&bytes)?;
        let id = self.register(kind, importance, deadline)?;
        transport.queue(bytes)?;
        Ok(id)
    }
    pub fn expire(&mut self, now: Instant) -> Result<Vec<Completion>> {
        if self.failed {
            return Err(Error("XWM request tracker has failed".into()));
        }
        let mut results = vec![];
        for (&sequence, pending) in &mut self.pending {
            if pending.expired || now < pending.deadline {
                continue;
            }
            if pending.importance == Importance::Essential {
                self.failed = true;
                return Err(Error("essential XWM request timed out".into()));
            }
            pending.expired = true;
            results.push(Completion::TimedOut(RequestId {
                generation: self.generation,
                sequence,
            }));
        }
        Ok(results)
    }
    /// Consume one bounded, transport-framed packet. No FD-bearing requests are
    /// admitted. The host must include this work in its per-turn XWM budget.
    pub fn ingest(&mut self, packet: Vec<u8>) -> Result<Vec<Completion>> {
        if self.failed {
            return Err(Error("XWM request tracker has failed".into()));
        }
        let result = self.ingest_inner(packet);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn ingest_inner(&mut self, mut packet: Vec<u8>) -> Result<Vec<Completion>> {
        if packet.len() < 32 || packet.len() > 256 * 1024 {
            return Err(Error("invalid XWM packet size".into()));
        }
        let kind = packet[0];
        let extended = kind == 1 || kind & 0x7f == 35;
        let expected = if extended {
            32u64 + 4 * u64::from(u32::from_ne_bytes(packet[4..8].try_into().unwrap()))
        } else {
            32
        };
        if packet.len() as u64 != expected {
            return Err(Error("invalid XWM packet framing".into()));
        }
        if kind & 0x7f != 11 {
            let wire = u16::from_ne_bytes([packet[2], packet[3]]);
            let mut sequence = (self.last_read & !65535) | u64::from(wire);
            if sequence < self.last_read {
                sequence = sequence
                    .checked_add(65536)
                    .ok_or_else(|| Error("XWM sequence exhausted".into()))?;
            }
            if sequence > self.last_sent {
                return Err(Error("XWM packet refers to an unsent request".into()));
            }
            if kind <= 1 {
                let pending = self
                    .pending
                    .get(&sequence)
                    .ok_or_else(|| Error("unexpected XWM response sequence".into()))?;
                if kind == 1 && pending.kind != ReplyKind::Reply {
                    return Err(Error("reply to a void XWM request".into()));
                }
            }
            self.last_read = sequence;
        } else {
            // KeymapNotify has no sequence, including its SendEvent variant.
            // x11rb 0.13.2 recognizes only the unflagged variant internally.
            packet[0] = 11;
        }
        self.connection.enqueue_packet(packet);
        let mut results = vec![];
        let mut remove = vec![];
        for (&sequence, pending) in &self.pending {
            let id = RequestId {
                generation: self.generation,
                sequence,
            };
            let result = match pending.kind {
                ReplyKind::Reply => self
                    .connection
                    .poll_for_reply_or_error(sequence)
                    .map(|(bytes, _)| PollReply::Reply(bytes))
                    .unwrap_or(PollReply::TryAgain),
                ReplyKind::Void => self.connection.poll_check_for_reply_or_error(sequence),
            };
            match result {
                PollReply::TryAgain => {}
                PollReply::NoReply => {
                    remove.push(sequence);
                    if !pending.expired {
                        results.push(Completion::Checked(id));
                    }
                }
                PollReply::Reply(bytes) => {
                    remove.push(sequence);
                    if pending.expired {
                        continue;
                    }
                    if bytes[0] == 0 {
                        if pending.importance == Importance::Essential {
                            return Err(Error("essential XWM request failed".into()));
                        }
                        results.push(Completion::Error(id, bytes[1]));
                    } else {
                        results.push(Completion::Reply(id, bytes));
                    }
                }
            }
        }
        for sequence in remove {
            self.pending.remove(&sequence);
        }
        while let Some((mut event, _)) = self.connection.poll_for_event_with_sequence() {
            if kind == 139 && event[0] == 11 {
                event[0] = 139;
            }
            if event[0] == 0 {
                return Err(Error("untracked XWM protocol error".into()));
            }
            results.push(Completion::Event(event));
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn reply(sequence: u64, error: bool) -> Vec<u8> {
        let mut packet = vec![0; 32];
        packet[0] = if error { 0 } else { 1 };
        packet[1] = if error { 3 } else { 0 };
        packet[2..4].copy_from_slice(&(sequence as u16).to_ne_bytes());
        packet
    }
    #[test]
    fn barrier_checks_void_requests_and_errors_keep_their_request_identity() {
        let mut tracker = Requests::new(4).unwrap();
        let now = Instant::now();
        let first = tracker
            .register(ReplyKind::Void, Importance::Optional, now)
            .unwrap();
        let second = tracker
            .register(ReplyKind::Reply, Importance::Essential, now)
            .unwrap();
        let results = tracker.ingest(reply(first.sequence, true)).unwrap();
        assert!(matches!(results.as_slice(), [Completion::Error(id,3)] if *id == first));
        assert!(
            matches!(tracker.ingest(reply(second.sequence,false)).unwrap().as_slice(), [Completion::Reply(id,_)] if *id == second)
        );
        let third = tracker
            .register(ReplyKind::Void, Importance::Essential, now)
            .unwrap();
        let barrier = tracker
            .register(ReplyKind::Reply, Importance::Essential, now)
            .unwrap();
        let results = tracker.ingest(reply(barrier.sequence, false)).unwrap();
        assert!(
            matches!(results.as_slice(), [Completion::Checked(id), Completion::Reply(_, _)] if *id == third)
        );
        assert_eq!(tracker.outstanding(), 0);
    }
    #[test]
    fn optional_timeout_retains_bounded_slots_and_discards_late_data() {
        let mut tracker = Requests::new(1).unwrap();
        let now = Instant::now();
        for _ in 0..LIMIT {
            tracker
                .register(ReplyKind::Reply, Importance::Optional, now)
                .unwrap();
        }
        assert_eq!(tracker.expire(now).unwrap().len(), LIMIT);
        assert!(tracker.expire(now).unwrap().is_empty());
        assert!(
            tracker
                .register(ReplyKind::Reply, Importance::Optional, now)
                .is_err()
        );
        assert!(tracker.ingest(reply(1, false)).unwrap().is_empty());
        assert_eq!(
            tracker
                .register(ReplyKind::Reply, Importance::Optional, now)
                .unwrap()
                .sequence,
            LIMIT as u64 + 1
        );
    }
    #[test]
    fn essential_timeout_and_malformed_response_fail_only_this_tracker() {
        let now = Instant::now();
        let mut tracker = Requests::new(1).unwrap();
        tracker
            .register(ReplyKind::Reply, Importance::Essential, now)
            .unwrap();
        assert!(
            tracker
                .expire(now - Duration::from_millis(1))
                .unwrap()
                .is_empty()
        );
        assert!(tracker.expire(now).is_err());
        assert!(tracker.ingest(reply(1, false)).is_err());
        let mut other = Requests::new(2).unwrap();
        assert!(other.ingest(reply(99, false)).is_err());
    }
    #[test]
    fn sequence_wrap_does_not_reuse_request_identity() {
        let mut tracker = Requests::new(9).unwrap();
        let now = Instant::now();
        for seq in 1..=65537 {
            let id = tracker
                .register(ReplyKind::Reply, Importance::Optional, now)
                .unwrap();
            assert_eq!(id.sequence, seq);
            assert!(
                matches!(tracker.ingest(reply(seq,false)).unwrap().as_slice(), [Completion::Reply(actual,_)] if *actual == id)
            );
        }
    }

    #[test]
    fn malformed_order_and_unsent_sequences_are_terminal_before_helper_enqueue() {
        let now = Instant::now();
        let mut tracker = Requests::new(1).unwrap();
        tracker
            .register(ReplyKind::Reply, Importance::Optional, now)
            .unwrap();
        tracker
            .register(ReplyKind::Reply, Importance::Optional, now)
            .unwrap();
        tracker.ingest(reply(2, false)).unwrap();
        assert!(tracker.ingest(reply(1, false)).is_err());
        assert!(tracker.ingest(reply(2, false)).is_err());
        let mut tracker = Requests::new(2).unwrap();
        let mut event = vec![0; 32];
        event[0] = 12;
        event[2..4].copy_from_slice(&1u16.to_ne_bytes());
        assert!(tracker.ingest(event).is_err());
    }

    #[test]
    fn flagged_keymap_event_has_no_sequence_and_failed_queue_consumes_none() {
        let now = Instant::now();
        let mut tracker = Requests::new(1).unwrap();
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut transport = Transport::new(socket).unwrap();
        assert!(
            tracker
                .queue(
                    &mut transport,
                    vec![43, 0, 1, 0],
                    ReplyKind::Reply,
                    Importance::Optional,
                    now
                )
                .is_err()
        );
        let id = tracker
            .register(ReplyKind::Reply, Importance::Optional, now)
            .unwrap();
        assert_eq!(id.sequence, 1);
        let mut keymap = vec![255; 32];
        keymap[0] = 139;
        assert!(
            matches!(tracker.ingest(keymap.clone()).unwrap().as_slice(), [Completion::Event(bytes)] if *bytes == keymap)
        );
        assert!(
            matches!(tracker.ingest(reply(1,false)).unwrap().as_slice(), [Completion::Reply(actual,_)] if *actual == id)
        );
    }
}
