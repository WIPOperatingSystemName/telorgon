//! ICCCM INCR send/receive flow control. The owner routes replies by generation,
//! requestor window and property, and queues DeleteProperty only when requested.
use super::{Error, Result};
use std::{
    borrow::Cow,
    time::{Duration, Instant},
};
use x11rb_protocol::protocol::xproto::GetPropertyReply;
const CHUNK_LIMIT: usize = 256 * 1024;
const IDLE: Duration = Duration::from_secs(10);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiveState {
    DeleteProperty,
    AwaitChunk,
    DrainChunk,
    Complete,
    Failed,
    Cancelled,
}
pub struct Receiver {
    state: ReceiveState,
    lower_bound: u64,
    limit: u64,
    total: u64,
    wire_type: Option<(u32, u8)>,
    bytes: Vec<u8>,
    consumed: usize,
    final_chunk: bool,
    progress: Instant,
}
impl Receiver {
    pub fn new(
        header: &GetPropertyReply,
        incr_atom: u32,
        limit: u64,
        now: Instant,
    ) -> Result<Self> {
        if incr_atom == 0
            || header.type_ != incr_atom
            || header.format != 32
            || header.value_len != 1
            || header.value.len() != 4
            || header.bytes_after != 0
            || limit == 0
        {
            return Err(Error("invalid INCR announcement".into()));
        }
        let lower_bound = u32::from_ne_bytes(header.value[..4].try_into().unwrap()) as u64;
        if lower_bound > limit {
            return Err(Error("INCR announcement exceeds transfer limit".into()));
        }
        Ok(Self {
            state: ReceiveState::DeleteProperty,
            lower_bound,
            limit,
            total: 0,
            wire_type: None,
            bytes: Vec::new(),
            consumed: 0,
            final_chunk: false,
            progress: now,
        })
    }
    pub fn state(&self) -> ReceiveState {
        self.state
    }
    pub fn deadline(&self) -> Option<Instant> {
        matches!(
            self.state,
            ReceiveState::DeleteProperty | ReceiveState::AwaitChunk | ReceiveState::DrainChunk
        )
        .then(|| self.progress + IDLE)
    }
    pub fn chunk(&self) -> &[u8] {
        &self.bytes[self.consumed..]
    }
    /// Validated property atom and format (bits per item), once data arrives.
    /// Chunk bytes remain in the connection's byte order; no conversion is done.
    pub fn property_type(&self) -> Option<(u32, u8)> {
        self.wire_type
    }
    fn fail(&mut self, message: &str) -> Error {
        if self.deadline().is_some() {
            self.state = ReceiveState::Failed;
        }
        self.bytes = Vec::new();
        self.consumed = 0;
        Error(message.into())
    }
    pub fn check_timeout(&mut self, now: Instant) -> Result<()> {
        if self.deadline().is_some_and(|deadline| now >= deadline) {
            return Err(self.fail("INCR inactivity timeout"));
        }
        Ok(())
    }
    /// Confirm that deletion was successfully queued, including final empty data.
    pub fn deletion_queued(&mut self, now: Instant) -> Result<()> {
        self.check_timeout(now)?;
        if self.state != ReceiveState::DeleteProperty {
            return Err(self.fail("unexpected INCR deletion"));
        }
        self.state = if self.final_chunk {
            ReceiveState::Complete
        } else {
            ReceiveState::AwaitChunk
        };
        self.progress = now;
        Ok(())
    }
    /// Accept one complete bounded property read. Partial reads must be assembled
    /// under the same bound before this call; never delete a partially read property.
    pub fn accept_property(&mut self, reply: &GetPropertyReply, now: Instant) -> Result<()> {
        self.accept(Cow::Borrowed(reply), now)
    }
    /// Take an assembled property without copying its payload. Prefer this when
    /// handing off PropertyRead completion so a maximum-size chunk is not duplicated.
    pub fn accept_owned_property(&mut self, reply: GetPropertyReply, now: Instant) -> Result<()> {
        self.accept(Cow::Owned(reply), now)
    }
    fn accept(&mut self, reply: Cow<'_, GetPropertyReply>, now: Instant) -> Result<()> {
        self.check_timeout(now)?;
        if self.state != ReceiveState::AwaitChunk {
            return Err(self.fail("INCR chunk arrived before acknowledgement"));
        }
        let unit = match reply.format {
            8 => 1,
            16 => 2,
            32 => 4,
            _ => 0,
        };
        if reply.type_ == 0
            || unit == 0
            || reply.bytes_after != 0
            || reply.value.len() > CHUNK_LIMIT
            || u64::from(reply.value_len) * unit != reply.value.len() as u64
            || self
                .wire_type
                .is_some_and(|kind| kind != (reply.type_, reply.format))
        {
            return Err(self.fail("invalid or oversized INCR chunk"));
        }
        self.wire_type = Some((reply.type_, reply.format));
        let Some(total) = self
            .total
            .checked_add(reply.value.len() as u64)
            .filter(|total| *total <= self.limit)
        else {
            return Err(self.fail("INCR transfer limit exceeded"));
        };
        self.total = total;
        self.progress = now;
        if reply.value.is_empty() {
            if total < self.lower_bound {
                return Err(self.fail("INCR ended below announced lower bound"));
            }
            self.final_chunk = true;
            self.state = ReceiveState::DeleteProperty;
        } else {
            self.bytes = match reply {
                Cow::Borrowed(reply) => reply.value.clone(),
                Cow::Owned(reply) => reply.value,
            };
            self.consumed = 0;
            self.state = ReceiveState::DrainChunk;
        }
        Ok(())
    }
    /// Acknowledge bytes accepted by the downstream endpoint, not merely offered.
    pub fn consume(&mut self, count: usize, now: Instant) -> Result<()> {
        self.check_timeout(now)?;
        if self.state != ReceiveState::DrainChunk || count > self.chunk().len() {
            return Err(self.fail("invalid INCR consumption"));
        }
        self.consumed += count;
        if count != 0 {
            self.progress = now;
        }
        if self.consumed == self.bytes.len() {
            self.bytes = Vec::new();
            self.consumed = 0;
            self.state = ReceiveState::DeleteProperty;
        }
        Ok(())
    }
    pub fn cancel(&mut self) {
        if self.deadline().is_some() {
            self.state = ReceiveState::Cancelled;
            self.bytes = Vec::new();
            self.consumed = 0;
        }
    }
}
/// Streaming send state. The owner queues the INCR announcement (lower bound zero)
/// and SelectionNotify, then feeds matching PropertyNotify(Deleted) events only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendState {
    Announce,
    AwaitDelete,
    NeedChunk,
    WriteChunk,
    Complete,
    Failed,
    Cancelled,
}
pub struct Sender {
    state: SendState,
    format: u8,
    chunk_limit: usize,
    limit: u64,
    total: u64,
    bytes: Vec<u8>,
    eof: bool,
    terminating: bool,
    progress: Instant,
}
impl Sender {
    /// chunk_limit must also fit the server's negotiated ChangeProperty request size.
    /// Creates a byte-format sender. Target conversions are separate.
    pub fn new(chunk_limit: usize, limit: u64, now: Instant) -> Result<Self> {
        Self::new_typed(8, chunk_limit, limit, now)
    }
    /// Creates a sender for an X property format of 8, 16 or 32 bits per item.
    /// Limits count bytes. Chunks must contain whole items in connection byte order.
    /// The owner chooses a fixed property type and uses this format for every write,
    /// including the empty terminator. This does not encode or convert target data.
    pub fn new_typed(format: u8, chunk_limit: usize, limit: u64, now: Instant) -> Result<Self> {
        if !matches!(format, 8 | 16 | 32)
            || chunk_limit < usize::from(format / 8)
            || chunk_limit > CHUNK_LIMIT
            || limit == 0
        {
            return Err(Error("invalid INCR sender limits".into()));
        }
        Ok(Self {
            state: SendState::Announce,
            format,
            chunk_limit,
            limit,
            total: 0,
            bytes: Vec::new(),
            eof: false,
            terminating: false,
            progress: now,
        })
    }
    pub fn state(&self) -> SendState {
        self.state
    }
    pub fn format(&self) -> u8 {
        self.format
    }
    pub fn deadline(&self) -> Option<Instant> {
        matches!(
            self.state,
            SendState::Announce
                | SendState::AwaitDelete
                | SendState::NeedChunk
                | SendState::WriteChunk
        )
        .then(|| self.progress + IDLE)
    }
    pub fn chunk(&self) -> Option<&[u8]> {
        (self.state == SendState::WriteChunk).then_some(self.bytes.as_slice())
    }
    fn fail(&mut self, message: &str) -> Error {
        if self.deadline().is_some() {
            self.state = SendState::Failed;
        }
        self.bytes = Vec::new();
        Error(message.into())
    }
    pub fn check_timeout(&mut self, now: Instant) -> Result<()> {
        if self.deadline().is_some_and(|deadline| now >= deadline) {
            return Err(self.fail("INCR sender inactivity timeout"));
        }
        Ok(())
    }
    pub fn announcement_queued(&mut self, now: Instant) -> Result<()> {
        self.check_timeout(now)?;
        if self.state != SendState::Announce {
            return Err(self.fail("unexpected INCR announcement"));
        }
        self.state = SendState::AwaitDelete;
        self.progress = now;
        Ok(())
    }
    pub fn property_deleted(&mut self, now: Instant) -> Result<()> {
        self.check_timeout(now)?;
        if self.state != SendState::AwaitDelete {
            return Err(self.fail("unexpected INCR property deletion"));
        }
        self.state = if self.terminating {
            SendState::Complete
        } else if self.eof {
            self.terminating = true;
            SendState::WriteChunk
        } else {
            SendState::NeedChunk
        };
        self.progress = now;
        Ok(())
    }
    /// Supply bytes only after deletion grants space. eof may accompany final data;
    /// an empty final marker is generated separately after that data is acknowledged.
    pub fn provide(&mut self, bytes: &[u8], eof: bool, now: Instant) -> Result<()> {
        self.check_timeout(now)?;
        if self.state != SendState::NeedChunk
            || bytes.len() > self.chunk_limit
            || bytes.len() % usize::from(self.format / 8) != 0
            || (bytes.is_empty() && !eof)
        {
            return Err(self.fail("invalid INCR sender chunk"));
        }
        let Some(total) = self
            .total
            .checked_add(bytes.len() as u64)
            .filter(|total| *total <= self.limit)
        else {
            return Err(self.fail("INCR sender transfer limit exceeded"));
        };
        self.total = total;
        self.bytes = bytes.to_vec();
        self.eof = eof;
        self.terminating = bytes.is_empty();
        self.state = SendState::WriteChunk;
        self.progress = now;
        Ok(())
    }
    /// Queue ChangeProperty with the exposed chunk before acknowledging this call.
    pub fn chunk_queued(&mut self, now: Instant) -> Result<()> {
        self.check_timeout(now)?;
        if self.state != SendState::WriteChunk {
            return Err(self.fail("unexpected INCR chunk acknowledgement"));
        }
        self.bytes = Vec::new();
        self.state = SendState::AwaitDelete;
        self.progress = now;
        Ok(())
    }
    pub fn cancel(&mut self) {
        if self.deadline().is_some() {
            self.state = SendState::Cancelled;
            self.bytes = Vec::new();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn property(kind: u32, format: u8, bytes: &[u8]) -> GetPropertyReply {
        GetPropertyReply {
            type_: kind,
            format,
            value_len: (bytes.len() / (format as usize / 8)) as u32,
            value: bytes.to_vec(),
            ..Default::default()
        }
    }
    fn receiver(now: Instant) -> Receiver {
        Receiver::new(&property(99, 32, &1u32.to_ne_bytes()), 99, 8, now).unwrap()
    }
    #[test]
    fn owned_property_handoff_reuses_payload_and_releases_it_on_cancel() {
        let now = Instant::now();
        let mut receiver = Receiver::new(
            &property(99, 32, &0u32.to_ne_bytes()),
            99,
            CHUNK_LIMIT as u64,
            now,
        )
        .unwrap();
        receiver.deletion_queued(now).unwrap();
        let data = vec![0xa5; CHUNK_LIMIT];
        let allocation = data.as_ptr();
        let reply = GetPropertyReply {
            type_: 10,
            format: 8,
            value_len: data.len() as u32,
            value: data,
            ..Default::default()
        };
        receiver.accept_owned_property(reply, now).unwrap();
        assert_eq!(receiver.chunk().as_ptr(), allocation);
        assert_eq!(receiver.chunk().len(), CHUNK_LIMIT);
        receiver.consume(17, now).unwrap();
        assert_eq!(receiver.chunk().len(), CHUNK_LIMIT - 17);
        receiver.cancel();
        assert!(receiver.chunk().is_empty());
        assert_eq!(receiver.bytes.capacity(), 0);
        assert_eq!(receiver.state(), ReceiveState::Cancelled);
    }
    #[test]
    fn typed_transfers_preserve_items_and_terminator_format() {
        let now = Instant::now();
        for format in [8, 16, 32] {
            let mut sender = Sender::new_typed(format, 9, 16, now).unwrap();
            let mut receiver = receiver(now);
            assert_eq!(receiver.property_type(), None);
            assert_eq!(sender.format(), format);
            sender.announcement_queued(now).unwrap();
            sender.property_deleted(now).unwrap();
            receiver.deletion_queued(now).unwrap();
            let bytes = [0xff, 0, 0x80, 1, 2, 3, 4, 5];
            sender.provide(&bytes, true, now).unwrap();
            receiver
                .accept_property(&property(10, sender.format(), sender.chunk().unwrap()), now)
                .unwrap();
            assert_eq!(receiver.property_type(), Some((10, format)));
            assert_eq!(receiver.chunk(), bytes);
            // Downstream byte writes may split items without changing wire framing.
            receiver.consume(1, now).unwrap();
            receiver.consume(7, now).unwrap();
            sender.chunk_queued(now).unwrap();
            receiver.deletion_queued(now).unwrap();
            sender.property_deleted(now).unwrap();
            receiver
                .accept_property(&property(10, sender.format(), sender.chunk().unwrap()), now)
                .unwrap();
            sender.chunk_queued(now).unwrap();
            receiver.deletion_queued(now).unwrap();
            sender.property_deleted(now).unwrap();
            assert_eq!(receiver.state(), ReceiveState::Complete);
            assert_eq!(sender.state(), SendState::Complete);
            assert_eq!(receiver.property_type(), Some((10, format)));
        }
    }
    #[test]
    fn typed_senders_reject_invalid_formats_and_split_items() {
        let now = Instant::now();
        for format in [0, 1, 7, 24, 64, 255] {
            assert!(Sender::new_typed(format, 8, 16, now).is_err());
        }
        for format in [16, 32] {
            let unit = usize::from(format / 8);
            assert!(Sender::new_typed(format, unit - 1, 16, now).is_err());
            let mut sender = Sender::new_typed(format, 8, 16, now).unwrap();
            sender.announcement_queued(now).unwrap();
            sender.property_deleted(now).unwrap();
            assert!(sender.provide(&[1], true, now).is_err());
            assert_eq!(sender.state(), SendState::Failed);
            assert_eq!(sender.chunk(), None);
        }
    }
    #[test]
    fn sender_receiver_roundtrip_waits_for_each_deletion_and_final_ack() {
        let now = Instant::now();
        let mut sender = Sender::new(4, 8, now).unwrap();
        let mut receiver =
            Receiver::new(&property(99, 32, &0u32.to_ne_bytes()), 99, 8, now).unwrap();
        sender.announcement_queued(now).unwrap();
        receiver.deletion_queued(now).unwrap();
        sender.property_deleted(now).unwrap();
        let mut delivered = Vec::new();
        for (bytes, eof) in [(b"ab".as_slice(), false), (b"cdef".as_slice(), true)] {
            sender.provide(bytes, eof, now).unwrap();
            receiver
                .accept_property(&property(10, 8, sender.chunk().unwrap()), now)
                .unwrap();
            sender.chunk_queued(now).unwrap();
            assert_eq!(sender.state(), SendState::AwaitDelete);
            delivered.extend_from_slice(receiver.chunk());
            receiver.consume(bytes.len(), now).unwrap();
            receiver.deletion_queued(now).unwrap();
            sender.property_deleted(now).unwrap();
        }
        assert_eq!(sender.chunk(), Some([].as_slice()));
        receiver
            .accept_property(&property(10, 8, sender.chunk().unwrap()), now)
            .unwrap();
        sender.chunk_queued(now).unwrap();
        assert_eq!(sender.state(), SendState::AwaitDelete);
        receiver.deletion_queued(now).unwrap();
        sender.property_deleted(now).unwrap();
        assert_eq!(sender.state(), SendState::Complete);
        assert_eq!(receiver.state(), ReceiveState::Complete);
        assert_eq!(delivered, b"abcdef");
    }
    #[test]
    fn cumulative_limits_cannot_be_evaded_with_small_chunks() {
        let now = Instant::now();
        let mut sender = Sender::new(4, 5, now).unwrap();
        let mut receiver =
            Receiver::new(&property(99, 32, &0u32.to_ne_bytes()), 99, 5, now).unwrap();
        sender.announcement_queued(now).unwrap();
        sender.property_deleted(now).unwrap();
        receiver.deletion_queued(now).unwrap();
        sender.provide(b"1234", false, now).unwrap();
        receiver
            .accept_property(&property(10, 8, b"1234"), now)
            .unwrap();
        sender.chunk_queued(now).unwrap();
        sender.property_deleted(now).unwrap();
        receiver.consume(4, now).unwrap();
        receiver.deletion_queued(now).unwrap();
        assert!(sender.provide(b"56", true, now).is_err());
        assert!(
            receiver
                .accept_property(&property(10, 8, b"56"), now)
                .is_err()
        );
        assert_eq!(sender.state(), SendState::Failed);
        assert_eq!(receiver.state(), ReceiveState::Failed);
        assert!(receiver.chunk().is_empty());
        assert!(sender.chunk().is_none());
    }
    #[test]
    fn only_positive_consumption_extends_a_blocked_receiver_deadline() {
        let now = Instant::now();
        let mut receiver = receiver(now);
        receiver.deletion_queued(now).unwrap();
        receiver
            .accept_property(&property(10, 8, b"abc"), now)
            .unwrap();
        receiver.consume(0, now + Duration::from_secs(9)).unwrap();
        assert_eq!(receiver.deadline(), Some(now + IDLE));
        receiver.consume(1, now + Duration::from_secs(9)).unwrap();
        assert_eq!(receiver.deadline(), Some(now + Duration::from_secs(19)));
        receiver
            .check_timeout(now + Duration::from_secs(10))
            .unwrap();
        assert!(
            receiver
                .check_timeout(now + Duration::from_secs(19))
                .is_err()
        );
        assert_eq!(receiver.state(), ReceiveState::Failed);
        assert!(receiver.chunk().is_empty());
    }

    #[test]
    fn late_callbacks_preserve_terminal_outcomes() {
        let now = Instant::now();
        for completed in [false, true] {
            let mut sender = Sender::new(4, 4, now).unwrap();
            let mut receiver =
                Receiver::new(&property(99, 32, &0u32.to_ne_bytes()), 99, 4, now).unwrap();
            if completed {
                sender.announcement_queued(now).unwrap();
                sender.property_deleted(now).unwrap();
                sender.provide(&[], true, now).unwrap();
                sender.chunk_queued(now).unwrap();
                sender.property_deleted(now).unwrap();
                receiver.deletion_queued(now).unwrap();
                receiver
                    .accept_property(&property(10, 8, &[]), now)
                    .unwrap();
                receiver.deletion_queued(now).unwrap();
            } else {
                sender.cancel();
                receiver.cancel();
            }
            let sent = sender.state();
            let received = receiver.state();
            assert!(sender.property_deleted(now).is_err());
            assert!(sender.provide(b"late", true, now).is_err());
            assert!(
                receiver
                    .accept_property(&property(10, 8, b"late"), now)
                    .is_err()
            );
            assert!(receiver.deletion_queued(now).is_err());
            sender.cancel();
            receiver.cancel();
            assert_eq!(sender.state(), sent);
            assert_eq!(receiver.state(), received);
            assert!(sender.deadline().is_none());
            assert!(receiver.deadline().is_none());
        }
    }

    #[test]
    fn sender_rejects_early_data_limits_and_stalls() {
        let now = Instant::now();
        let mut s = Sender::new(4, 4, now).unwrap();
        assert!(s.provide(b"x", false, now).is_err());
        let mut s = Sender::new(4, 4, now).unwrap();
        s.announcement_queued(now).unwrap();
        s.property_deleted(now).unwrap();
        assert!(s.provide(b"12345", false, now).is_err());
        let mut s = Sender::new(4, 4, now).unwrap();
        s.announcement_queued(now).unwrap();
        assert!(s.check_timeout(now + IDLE).is_err());
        assert!(s.chunk().is_none());
        let mut s = Sender::new(4, 4, now).unwrap();
        s.cancel();
        assert_eq!(s.state(), SendState::Cancelled);
    }

    #[test]
    fn deletion_waits_for_drain_and_final_empty_chunk() {
        let now = Instant::now();
        let mut receiver = receiver(now);
        receiver.deletion_queued(now).unwrap();
        receiver
            .accept_property(&property(10, 8, &[0, 255, 1]), now)
            .unwrap();
        receiver.consume(1, now).unwrap();
        assert_eq!(receiver.chunk(), &[255, 1]);
        assert_eq!(receiver.state(), ReceiveState::DrainChunk);
        receiver.consume(2, now).unwrap();
        assert_eq!(receiver.state(), ReceiveState::DeleteProperty);
        receiver.deletion_queued(now).unwrap();
        receiver
            .accept_property(&property(10, 8, &[]), now)
            .unwrap();
        assert_eq!(receiver.state(), ReceiveState::DeleteProperty);
        receiver.deletion_queued(now).unwrap();
        assert_eq!(receiver.state(), ReceiveState::Complete);
        assert!(receiver.deadline().is_none());
    }
    #[test]
    fn announcement_and_property_layout_are_checked_before_buffering() {
        let now = Instant::now();
        assert!(Receiver::new(&property(99, 32, &9u32.to_ne_bytes()), 99, 8, now).is_err());
        assert!(Receiver::new(&property(99, 8, &[1]), 99, 8, now).is_err());
        for malformed in [
            GetPropertyReply {
                bytes_after: 1,
                ..property(10, 8, b"x")
            },
            GetPropertyReply {
                value_len: 2,
                ..property(10, 8, b"x")
            },
            property(0, 8, b"x"),
        ] {
            let mut r = receiver(now);
            r.deletion_queued(now).unwrap();
            assert!(r.accept_property(&malformed, now).is_err());
            assert!(r.chunk().is_empty());
        }
        let mut r = receiver(now);
        r.deletion_queued(now).unwrap();
        assert!(r.accept_property(&property(10, 8, &[]), now).is_err());
        let mut r = Receiver::new(&property(99, 32, &0u32.to_ne_bytes()), 99, 8, now).unwrap();
        r.deletion_queued(now).unwrap();
        r.accept_property(&property(10, 8, &[]), now).unwrap();
        r.deletion_queued(now).unwrap();
        assert_eq!(r.state(), ReceiveState::Complete);
    }

    #[test]
    fn bounds_order_type_timeout_and_cancel_are_terminal() {
        let now = Instant::now();
        let mut r = receiver(now);
        assert!(r.accept_property(&property(10, 8, b"x"), now).is_err());
        let mut r = receiver(now);
        r.deletion_queued(now).unwrap();
        assert!(
            r.accept_property(&property(10, 8, b"oversized"), now)
                .is_err()
        );
        let mut r = receiver(now);
        r.deletion_queued(now).unwrap();
        r.accept_property(&property(10, 8, b"x"), now).unwrap();
        r.consume(1, now).unwrap();
        r.deletion_queued(now).unwrap();
        assert!(r.accept_property(&property(11, 8, b"y"), now).is_err());
        assert_eq!(r.state(), ReceiveState::Failed);
        assert!(r.chunk().is_empty());
        let mut r = receiver(now);
        assert!(r.check_timeout(now + IDLE).is_err());
        let mut r = receiver(now);
        r.cancel();
        assert_eq!(r.state(), ReceiveState::Cancelled);
    }
}
