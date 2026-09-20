//! Bounded, non-deleting property reads for selection transfers. The selection
//! owner dispatches request completions, enforces deadlines and cancels on ownership loss.
use super::{
    Error, Result,
    incr_wire::PropertyTarget,
    requests::{Completion, Importance, ReplyKind, RequestId, Requests},
    transport::Transport,
};
use std::time::Instant;
use x11rb_protocol::{
    protocol::xproto,
    x11_utils::{Request, TryParse},
};
const PART: usize = 16 * 1024;
const LIMIT: usize = 256 * 1024;

#[derive(Debug)]
pub enum ReadProgress {
    /// Completion belongs to another request, including an earlier generation.
    Unrelated,
    /// Queue the next bounded read when the owner's dispatch budget permits.
    More,
    Complete(xproto::GetPropertyReply),
}

pub struct PropertyRead {
    target: PropertyTarget,
    pending: Option<RequestId>,
    bytes: Vec<u8>,
    kind: Option<(u32, u8)>,
    remaining: Option<usize>,
    terminal: bool,
}
impl PropertyRead {
    pub fn new(target: PropertyTarget) -> Self {
        Self {
            target,
            pending: None,
            bytes: Vec::new(),
            kind: None,
            remaining: None,
            terminal: false,
        }
    }
    /// At most one 16-KiB reply is requested at a time, below transport packet limits.
    /// Queue rejection leaves this read retryable. No request deletes the property.
    pub fn queue(
        &mut self,
        transport: &mut Transport,
        requests: &mut Requests,
        deadline: Instant,
    ) -> Result<RequestId> {
        self.target.validate(requests)?;
        if self.terminal || self.pending.is_some() {
            return Err(Error("property read is not ready to queue".into()));
        }
        let (bytes, _) = Request::serialize(
            xproto::GetPropertyRequest {
                delete: false,
                window: self.target.window,
                property: self.target.property,
                type_: 0,
                long_offset: (self.bytes.len() / 4) as u32,
                long_length: (PART / 4) as u32,
            },
            0,
        );
        let id = requests.queue(
            transport,
            bytes,
            ReplyKind::Reply,
            Importance::Optional,
            deadline,
        )?;
        self.pending = Some(id);
        Ok(id)
    }
    pub fn cancel(&mut self) {
        self.terminal = true;
        self.pending = None;
        self.bytes = Vec::new();
    }
    fn fail(&mut self) -> Error {
        self.cancel();
        Error("invalid, changed or oversized selection property".into())
    }
    /// Consume only this read's pending completion. Matching errors/timeouts and
    /// malformed replies release buffering and terminate this read, never another
    /// transfer. The caller must handle Err locally instead of aborting desktop dispatch.
    pub fn completion(&mut self, completion: &Completion) -> Result<ReadProgress> {
        let id = match completion {
            Completion::Reply(id, _)
            | Completion::Checked(id)
            | Completion::Error(id, _)
            | Completion::TimedOut(id) => *id,
            Completion::Event(_) => return Ok(ReadProgress::Unrelated),
        };
        if self.pending != Some(id) || self.terminal {
            return Ok(ReadProgress::Unrelated);
        }
        let Completion::Reply(_, bytes) = completion else {
            self.cancel();
            return Err(Error(
                "selection property request failed or timed out".into(),
            ));
        };
        if bytes.len() < 32
            || bytes.len() > 32 + PART
            || bytes[0] != 1
            || u32::from_ne_bytes(bytes[4..8].try_into().unwrap()) as usize
                != (bytes.len() - 32) / 4
            || bytes.len() % 4 != 0
        {
            return Err(self.fail());
        }
        let (reply, _) = xproto::GetPropertyReply::try_parse(bytes).map_err(|_| self.fail())?;
        // Generated parsing extracts the declared items but permits trailing bytes.
        // Only the protocol's final zero-to-three padding bytes may remain.
        if bytes.len() != 32 + reply.value.len().div_ceil(4) * 4 {
            return Err(self.fail());
        }
        match self.accept(id, reply)? {
            Some(reply) => Ok(ReadProgress::Complete(reply)),
            None => Ok(ReadProgress::More),
        }
    }
    /// Return the assembled property only after the final part. Stale reply IDs
    /// leave the current read untouched. Same-size concurrent rewrites cannot be
    /// detected here; the owner must obey the selection property handshake.
    pub fn accept(
        &mut self,
        id: RequestId,
        reply: xproto::GetPropertyReply,
    ) -> Result<Option<xproto::GetPropertyReply>> {
        if self.pending != Some(id) || self.terminal {
            return Err(Error("stale selection property reply".into()));
        }
        self.pending = None;
        let unit = match reply.format {
            8 => 1,
            16 => 2,
            32 => 4,
            _ => 0,
        };
        let len = reply.value.len();
        let remaining = reply.bytes_after as usize;
        let total = len.checked_add(remaining);
        if reply.type_ == 0
            || unit == 0
            || len > PART
            || u64::from(reply.value_len) * unit != len as u64
            || total.is_none_or(|n| n > LIMIT - self.bytes.len())
            || self.remaining.is_some_and(|n| Some(n) != total)
            || self
                .kind
                .is_some_and(|kind| kind != (reply.type_, reply.format))
            || (remaining != 0 && (len == 0 || len % 4 != 0))
        {
            return Err(self.fail());
        }
        self.kind = Some((reply.type_, reply.format));
        self.bytes.extend_from_slice(&reply.value);
        self.remaining = Some(remaining);
        if remaining != 0 {
            return Ok(None);
        }
        self.terminal = true;
        let value = std::mem::take(&mut self.bytes);
        Ok(Some(xproto::GetPropertyReply {
            format: reply.format,
            sequence: reply.sequence,
            length: value.len().div_ceil(4) as u32,
            type_: reply.type_,
            bytes_after: 0,
            value_len: (value.len() / unit as usize) as u32,
            value,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb_protocol::x11_utils::Serialize;
    fn read() -> PropertyRead {
        PropertyRead::new(PropertyTarget {
            generation: 1,
            window: 10,
            property: 11,
        })
    }
    fn reply(bytes: &[u8], remaining: u32) -> xproto::GetPropertyReply {
        xproto::GetPropertyReply {
            type_: 12,
            format: 8,
            value_len: bytes.len() as u32,
            value: bytes.to_vec(),
            bytes_after: remaining,
            ..Default::default()
        }
    }
    fn deliver(
        read: &mut PropertyRead,
        reply: xproto::GetPropertyReply,
    ) -> Result<Option<xproto::GetPropertyReply>> {
        let id = RequestId {
            generation: 1,
            sequence: 1,
        };
        read.pending = Some(id);
        read.accept(id, reply)
    }
    #[test]
    fn maximum_property_moves_to_incr_and_deletion_waits_for_full_drain() {
        use super::super::incr::{ReceiveState, Receiver};
        let now = Instant::now();
        let mut reader = read();
        let header = xproto::GetPropertyReply {
            type_: 99,
            format: 32,
            value_len: 1,
            value: (LIMIT as u32).to_ne_bytes().to_vec(),
            ..Default::default()
        };
        let mut receiver = Receiver::new(&header, 99, LIMIT as u64, now).unwrap();
        receiver.deletion_queued(now).unwrap();
        for index in 0..LIMIT / PART {
            let bytes = vec![index as u8; PART];
            let remaining = LIMIT - (index + 1) * PART;
            let result = deliver(&mut reader, reply(&bytes, remaining as u32)).unwrap();
            if remaining != 0 {
                assert!(result.is_none());
                assert_eq!(reader.bytes.len(), (index + 1) * PART);
                assert_eq!(receiver.state(), ReceiveState::AwaitChunk);
            } else {
                let property = result.unwrap();
                let allocation = property.value.as_ptr();
                receiver.accept_owned_property(property, now).unwrap();
                assert!(reader.bytes.is_empty());
                assert_eq!(receiver.chunk().as_ptr(), allocation);
                assert_eq!(receiver.chunk().len(), LIMIT);
                for (index, chunk) in receiver.chunk().chunks(PART).enumerate() {
                    assert!(chunk.iter().all(|byte| *byte == index as u8));
                }
            }
        }
        receiver.consume(LIMIT - 1, now).unwrap();
        assert_eq!(receiver.state(), ReceiveState::DrainChunk);
        receiver.consume(1, now).unwrap();
        assert_eq!(receiver.state(), ReceiveState::DeleteProperty);
        receiver.deletion_queued(now).unwrap();
        receiver.accept_owned_property(reply(&[], 0), now).unwrap();
        receiver.deletion_queued(now).unwrap();
        assert_eq!(receiver.state(), ReceiveState::Complete);
    }
    #[test]
    fn cumulative_property_bound_is_checked_before_accumulation() {
        let mut reader = read();
        let first = vec![1; PART];
        assert!(
            deliver(&mut reader, reply(&first, (LIMIT - PART) as u32))
                .unwrap()
                .is_none()
        );
        assert_eq!(reader.bytes.len(), PART);
        // Each reply fits the per-read limit, but this would grow the property
        // beyond the allowed total after bytes already accumulated are included.
        assert!(deliver(&mut reader, reply(&first, (LIMIT - 2 * PART + 1) as u32)).is_err());
        assert!(reader.terminal);
        assert!(reader.bytes.is_empty());
    }
    #[test]
    fn transport_and_tracker_deliver_fragmented_replies_and_timeout() {
        use super::super::discovery::tests::{flush_requests, ready};
        use std::io::{Read, Write};
        use std::time::Duration;
        let (mut transport, mut requests, _, mut peer, now) = ready();
        let target = PropertyTarget {
            generation: requests.generation(),
            window: 10,
            property: 11,
        };
        let mut reader = PropertyRead::new(target);
        let id = reader
            .queue(&mut transport, &mut requests, now + Duration::from_secs(10))
            .unwrap();
        flush_requests(&mut transport);
        peer.read_exact(&mut [0; 24]).unwrap();
        let mut property = reply(b"abcd", 4);
        property.sequence = id.sequence as u16;
        property.length = 1;
        let wire = property.serialize();
        peer.write_all(&wire[..15]).unwrap();
        assert!(transport.dispatch().unwrap().packets.is_empty());
        peer.write_all(&wire[15..]).unwrap();
        let mut progressed = false;
        let deadline = Instant::now() + Duration::from_secs(2);
        while !progressed && Instant::now() < deadline {
            for packet in transport.dispatch().unwrap().packets {
                for completion in requests.ingest(packet).unwrap() {
                    if matches!(reader.completion(&completion).unwrap(), ReadProgress::More) {
                        progressed = true;
                    }
                }
            }
        }
        assert!(progressed);
        assert_eq!(reader.bytes, b"abcd");
        let next = reader
            .queue(&mut transport, &mut requests, now + Duration::from_secs(1))
            .unwrap();
        let mut timed_out = false;
        for completion in requests.expire(now + Duration::from_secs(1)).unwrap() {
            if matches!(completion, Completion::TimedOut(actual) if actual == next) {
                assert!(reader.completion(&completion).is_err());
                timed_out = true;
            }
        }
        assert!(timed_out);
        assert!(reader.bytes.is_empty());
        assert!(reader.terminal);
        // Expiry retains request accounting until server progress; another read
        // still queues without reviving the failed accumulator.
        let mut unrelated = PropertyRead::new(target);
        assert!(
            unrelated
                .queue(&mut transport, &mut requests, now + Duration::from_secs(10))
                .is_ok()
        );
    }
    #[test]
    fn completion_rejects_payload_beyond_declared_items() {
        let id = RequestId {
            generation: 1,
            sequence: 1,
        };
        let mut read = read();
        read.pending = Some(id);
        let mut property = reply(b"a", 0);
        property.length = 2; // Eight payload bytes, but only one declared item.
        let mut wire = property.serialize();
        wire.resize(40, 0);
        wire[4..8].copy_from_slice(&2u32.to_ne_bytes());
        assert!(read.completion(&Completion::Reply(id, wire)).is_err());
        assert!(read.terminal);
        assert!(read.bytes.is_empty());
    }
    #[test]
    fn completion_dispatch_contains_failures_and_ignores_other_requests() {
        let id = RequestId {
            generation: 1,
            sequence: 4,
        };
        for failure in [
            Completion::Error(id, 3),
            Completion::TimedOut(id),
            Completion::Checked(id),
            Completion::Reply(id, vec![0; 8]),
        ] {
            let mut read = read();
            read.pending = Some(id);
            read.bytes.extend_from_slice(b"held");
            assert!(matches!(
                read.completion(&Completion::TimedOut(RequestId {
                    generation: 2,
                    ..id
                }))
                .unwrap(),
                ReadProgress::Unrelated
            ));
            assert_eq!(read.bytes, b"held");
            assert!(read.completion(&failure).is_err());
            assert!(read.terminal);
            assert!(read.bytes.is_empty());
            assert!(matches!(
                read.completion(&failure).unwrap(),
                ReadProgress::Unrelated
            ));
        }
        let mut read = read();
        read.pending = Some(id);
        assert!(matches!(
            read.completion(&Completion::Event(vec![])).unwrap(),
            ReadProgress::Unrelated
        ));
        let mut first = reply(b"abcd", 3);
        first.length = 1;
        assert!(matches!(
            read.completion(&Completion::Reply(id, first.serialize()))
                .unwrap(),
            ReadProgress::More
        ));
        let next = RequestId { sequence: 5, ..id };
        read.pending = Some(next);
        let mut last = reply(b"efg", 0);
        last.length = 1;
        let mut last_wire = last.serialize();
        last_wire.resize(36, 0); // X11 pads replies to four-byte boundaries.
        let ReadProgress::Complete(result) = read
            .completion(&Completion::Reply(next, last_wire))
            .unwrap()
        else {
            panic!("expected completed property");
        };
        assert_eq!(result.value, b"abcdefg");
        assert!(read.terminal);
    }
    #[test]
    fn fragments_assemble_with_bounded_storage_and_stale_replies_do_not_consume_pending() {
        let mut read = read();
        let id = RequestId {
            generation: 1,
            sequence: 1,
        };
        read.pending = Some(id);
        assert!(
            read.accept(
                RequestId {
                    generation: 2,
                    ..id
                },
                reply(b"bad", 0)
            )
            .is_err()
        );
        assert_eq!(read.pending, Some(id));
        assert!(read.accept(id, reply(b"abcd", 3)).unwrap().is_none());
        let result = deliver(&mut read, reply(b"efg", 0)).unwrap().unwrap();
        assert_eq!(result.value, b"abcdefg");
        assert_eq!(result.value_len, 7);
        assert_eq!(result.length, 2);
        assert_eq!(result.bytes_after, 0);
        assert!(read.terminal);
        assert!(read.bytes.is_empty());
    }
    #[test]
    fn rejects_growth_type_changes_unaligned_progress_and_oversized_properties() {
        for bad in [
            reply(b"", 4),
            reply(b"a", 4),
            reply(b"abcd", LIMIT as u32),
            reply(&vec![0; PART + 1], 0),
        ] {
            let mut read = read();
            assert!(deliver(&mut read, bad).is_err());
            assert!(read.terminal);
            assert!(read.bytes.is_empty());
        }
        for changed in [
            reply(b"xyz", 0),
            xproto::GetPropertyReply {
                type_: 13,
                ..reply(b"xy", 0)
            },
        ] {
            let mut read = read();
            assert!(deliver(&mut read, reply(b"abcd", 2)).unwrap().is_none());
            assert!(deliver(&mut read, changed).is_err());
            assert!(read.bytes.is_empty());
        }
    }
    #[test]
    fn wire_reads_use_bounded_lengths_offsets_and_never_delete() {
        use super::super::discovery::tests::{flush_requests, ready};
        use std::io::Read;
        let (mut transport, mut requests, _, mut peer, now) = ready();
        let mut read = PropertyRead::new(PropertyTarget {
            generation: requests.generation(),
            window: 10,
            property: 11,
        });
        for offset in [0, 1] {
            let id = read.queue(&mut transport, &mut requests, now).unwrap();
            let count = requests.outstanding();
            assert!(read.queue(&mut transport, &mut requests, now).is_err());
            assert_eq!(count, requests.outstanding());
            flush_requests(&mut transport);
            let mut wire = [0; 24];
            peer.read_exact(&mut wire).unwrap();
            assert_eq!(wire[0], 20);
            assert_eq!(wire[1], 0); // delete=false
            let word = |at| u32::from_ne_bytes(wire[at..at + 4].try_into().unwrap());
            assert_eq!(word(4), 10);
            assert_eq!(word(8), 11);
            assert_eq!(word(12), 0); // AnyPropertyType
            assert_eq!(word(16), offset);
            assert_eq!(word(20), (PART / 4) as u32);
            let result = read
                .accept(id, reply(b"abcd", if offset == 0 { 4 } else { 0 }))
                .unwrap();
            assert_eq!(result.is_some(), offset == 1);
        }
        assert!(read.queue(&mut transport, &mut requests, now).is_err());
    }
}
