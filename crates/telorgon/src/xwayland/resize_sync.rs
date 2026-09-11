//! Optional EWMH basic resize synchronization. Queries are asynchronous and
//! rate-limited; never use XSync Await (which can stall the shared X connection).
use super::{
    Error, Result,
    association::XWindow,
    requests::{Completion, Importance, ReplyKind, Requests},
    transport::Transport,
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use x11rb_protocol::{
    protocol::sync,
    x11_utils::{Request, TryParse},
};
const TIMEOUT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeSyncStatus {
    Unsupported,
    Initializing,
    Idle,
    Waiting,
    Acknowledged,
    TimedOut,
    InvalidCounter,
}
struct Entry {
    counter: u32,
    epoch: u64,
    value: i64,
    status: ResizeSyncStatus,
    deadline: Instant,
    next_poll: Instant,
    inflight: bool,
    reset: bool,
}
#[derive(Clone, Copy)]
struct Pending {
    window: XWindow,
    epoch: u64,
    value: i64,
    reset: bool,
}
#[derive(Default)]
pub(crate) struct ResizeSync {
    entries: BTreeMap<XWindow, Entry>,
    pending: BTreeMap<u64, Pending>,
    epoch: u64,
    next_value: i64,
}
impl ResizeSync {
    pub fn capability(
        &mut self,
        window: XWindow,
        counter: Option<u32>,
        now: Instant,
    ) -> Result<()> {
        let Some(counter) = counter else {
            self.forget(window);
            return Ok(());
        };
        if self
            .entries
            .get(&window)
            .is_some_and(|entry| entry.counter == counter)
        {
            return Ok(());
        }
        if self.entries.len() >= 4096 {
            return Err(Error("resize sync window bound reached".into()));
        }
        self.epoch = self
            .epoch
            .checked_add(1)
            .ok_or_else(|| Error("resize sync epoch exhausted".into()))?;
        self.entries.insert(
            window,
            Entry {
                counter,
                epoch: self.epoch,
                value: 0,
                status: ResizeSyncStatus::Initializing,
                deadline: now + TIMEOUT,
                next_poll: now,
                inflight: false,
                reset: true,
            },
        );
        Ok(())
    }
    pub fn forget(&mut self, window: XWindow) {
        self.entries.remove(&window);
    }
    pub fn cancel(&mut self, window: XWindow) {
        if let Some(e) = self.entries.get_mut(&window) {
            if e.status == ResizeSyncStatus::Waiting {
                e.status = ResizeSyncStatus::Idle;
            }
        }
    }
    pub fn status(&self, window: XWindow) -> ResizeSyncStatus {
        self.entries
            .get(&window)
            .map_or(ResizeSyncStatus::Unsupported, |e| e.status)
    }
    pub fn waiting(&self, window: XWindow) -> bool {
        matches!(
            self.status(window),
            ResizeSyncStatus::Initializing | ResizeSyncStatus::Waiting
        )
    }
    /// Called immediately before the ordered SendEvent/ConfigureWindow pair.
    pub fn begin(&mut self, window: XWindow, now: Instant) -> Option<i64> {
        let e = self.entries.get_mut(&window)?;
        if !matches!(
            e.status,
            ResizeSyncStatus::Idle | ResizeSyncStatus::Acknowledged
        ) {
            return None;
        }
        let Some(value) = self.next_value.checked_add(1) else {
            e.status = ResizeSyncStatus::InvalidCounter;
            return None;
        };
        self.next_value = value;
        e.value = value;
        e.status = ResizeSyncStatus::Waiting;
        e.inflight = false;
        e.deadline = now + TIMEOUT;
        e.next_poll = now;
        Some(value)
    }
    pub fn deadline(&self) -> Option<Instant> {
        self.entries
            .values()
            .filter(|e| {
                matches!(
                    e.status,
                    ResizeSyncStatus::Initializing | ResizeSyncStatus::Waiting
                )
            })
            .map(|e| {
                if e.inflight {
                    e.deadline
                } else {
                    e.next_poll.min(e.deadline)
                }
            })
            .min()
    }
    pub fn schedule(
        &mut self,
        opcode: u8,
        transport: &mut Transport,
        requests: &mut Requests,
        now: Instant,
        work_deadline: Instant,
    ) -> Result<bool> {
        let mut count = 0;
        for (&window, e) in &mut self.entries {
            if !matches!(
                e.status,
                ResizeSyncStatus::Initializing | ResizeSyncStatus::Waiting
            ) {
                continue;
            }
            if now >= e.deadline {
                e.status = ResizeSyncStatus::TimedOut;
                eprintln!(
                    "telorgon-xwayland: resize repaint acknowledgement timed out for generation {} window {}; using buffer-only fallback",
                    window.generation, window.xid
                );
                continue;
            }
            if e.inflight || now < e.next_poll {
                continue;
            }
            if count == 16
                || Instant::now() >= work_deadline
                || self.pending.len() + if e.reset { 2 } else { 1 } > 128
                || requests.available_slots() < 2
            {
                break;
            }
            if e.reset {
                // EWMH requires initialization: the client's initial counter is undefined.
                let (bytes, _) = Request::serialize(
                    sync::SetCounterRequest {
                        counter: e.counter,
                        value: sync::Int64 { hi: 0, lo: 0 },
                    },
                    opcode,
                );
                let id = requests.queue(
                    transport,
                    bytes,
                    ReplyKind::Void,
                    Importance::Optional,
                    e.deadline,
                )?;
                self.pending.insert(
                    id.sequence,
                    Pending {
                        window,
                        epoch: e.epoch,
                        value: e.value,
                        reset: true,
                    },
                );
                e.reset = false;
            }
            let (bytes, _) =
                Request::serialize(sync::QueryCounterRequest { counter: e.counter }, opcode);
            let id = requests.queue(
                transport,
                bytes,
                ReplyKind::Reply,
                Importance::Optional,
                e.deadline,
            )?;
            self.pending.insert(
                id.sequence,
                Pending {
                    window,
                    epoch: e.epoch,
                    value: e.value,
                    reset: false,
                },
            );
            e.inflight = true;
            count += 1;
        }
        Ok(self.pending.len() < 128
            && requests.available_slots() >= 2
            && self.entries.values().any(|e| {
                matches!(
                    e.status,
                    ResizeSyncStatus::Initializing | ResizeSyncStatus::Waiting
                ) && !e.inflight
                    && e.next_poll <= now
            }))
    }
    pub fn completion(&mut self, completion: &Completion, now: Instant) -> bool {
        let id = match completion {
            Completion::Reply(id, _)
            | Completion::Checked(id)
            | Completion::Error(id, _)
            | Completion::TimedOut(id) => id,
            Completion::Event(_) => return false,
        };
        let Some(p) = self.pending.get(&id.sequence).copied() else {
            return false;
        };
        if id.generation != p.window.generation {
            return false;
        }
        self.pending.remove(&id.sequence);
        let Some(e) = self
            .entries
            .get_mut(&p.window)
            .filter(|e| e.epoch == p.epoch && e.value == p.value)
        else {
            return true;
        };
        if !matches!(
            e.status,
            ResizeSyncStatus::Initializing | ResizeSyncStatus::Waiting
        ) {
            return true;
        }
        if now >= e.deadline || matches!(completion, Completion::TimedOut(_)) {
            e.status = ResizeSyncStatus::TimedOut;
            eprintln!(
                "telorgon-xwayland: resize sync timed out for window {}; using buffer-only fallback",
                p.window.xid
            );
        } else if p.reset && matches!(completion, Completion::Checked(_)) {
            return true;
        } else if let Completion::Reply(_, bytes) = completion
            && !p.reset
        {
            e.inflight = false;
            match sync::QueryCounterReply::try_parse(bytes) {
                Ok((reply, _)) if bytes.len() == 32 && reply.length == 0 => {
                    let actual = (i64::from(reply.counter_value.hi) << 32)
                        | i64::from(reply.counter_value.lo);
                    if actual == e.value {
                        e.status = if e.status == ResizeSyncStatus::Initializing {
                            ResizeSyncStatus::Idle
                        } else {
                            ResizeSyncStatus::Acknowledged
                        };
                    } else {
                        e.next_poll = now + POLL;
                    }
                }
                _ => e.status = ResizeSyncStatus::InvalidCounter,
            }
        } else {
            e.status = ResizeSyncStatus::InvalidCounter;
        }
        if e.status == ResizeSyncStatus::InvalidCounter {
            eprintln!(
                "telorgon-xwayland: invalid resize sync counter for window {}; using buffer-only fallback",
                p.window.xid
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        discovery::tests::{flush_requests, ready, request},
        requests::RequestId,
    };
    use super::*;
    use x11rb_protocol::x11_utils::Serialize;
    fn window() -> XWindow {
        XWindow {
            generation: 1,
            xid: 10,
            incarnation: 1,
        }
    }
    fn reply(sequence: u64, value: i64) -> Completion {
        let mut bytes = sync::QueryCounterReply {
            sequence: sequence as u16,
            length: 0,
            counter_value: sync::Int64 {
                hi: (value >> 32) as i32,
                lo: value as u32,
            },
        }
        .serialize()
        .to_vec();
        bytes.resize(32, 0);
        Completion::Reply(
            RequestId {
                generation: 1,
                sequence,
            },
            bytes,
        )
    }
    fn pending(sync: &mut ResizeSync, sequence: u64) {
        let e = &sync.entries[&window()];
        sync.pending.insert(
            sequence,
            Pending {
                window: window(),
                epoch: e.epoch,
                value: e.value,
                reset: false,
            },
        );
    }
    #[test]
    fn initializes_undefined_counter_and_polls_without_busy_waiting() {
        let (mut transport, mut requests, discovered, mut peer, now) = ready();
        let opcode = discovered.extensions["SYNC"].major_opcode;
        let mut sync = ResizeSync::default();
        sync.capability(window(), Some(100), now).unwrap();
        assert!(sync.waiting(window()));
        sync.schedule(
            opcode,
            &mut transport,
            &mut requests,
            now,
            Instant::now() + TIMEOUT,
        )
        .unwrap();
        flush_requests(&mut transport);
        let set = request(&mut peer);
        assert_eq!(&set[..2], &[opcode, 3]); // SetCounter
        assert_eq!(&set[8..16], &[0; 8]);
        assert_eq!(&request(&mut peer)[..2], &[opcode, 5]); // QueryCounter
        let sequence = requests.last_sequence();
        for c in requests
            .ingest(match reply(sequence, 0) {
                Completion::Reply(_, b) => b,
                _ => unreachable!(),
            })
            .unwrap()
        {
            assert!(sync.completion(&c, now));
        }
        assert_eq!(sync.status(window()), ResizeSyncStatus::Idle);
        assert_eq!(sync.begin(window(), now), Some(1));
        sync.schedule(
            opcode,
            &mut transport,
            &mut requests,
            now,
            Instant::now() + TIMEOUT,
        )
        .unwrap();
        flush_requests(&mut transport);
        assert_eq!(&request(&mut peer)[..2], &[opcode, 5]);
        let sequence = requests.last_sequence();
        for c in requests
            .ingest(match reply(sequence, 0) {
                Completion::Reply(_, b) => b,
                _ => unreachable!(),
            })
            .unwrap()
        {
            assert!(sync.completion(&c, now));
        }
        assert_eq!(sync.status(window()), ResizeSyncStatus::Waiting);
        assert_eq!(sync.deadline(), Some(now + POLL));
        sync.schedule(
            opcode,
            &mut transport,
            &mut requests,
            now,
            Instant::now() + TIMEOUT,
        )
        .unwrap();
        assert_eq!(requests.last_sequence(), sequence);
        sync.schedule(
            opcode,
            &mut transport,
            &mut requests,
            now + POLL,
            Instant::now() + TIMEOUT,
        )
        .unwrap();
        let sequence = requests.last_sequence();
        for c in requests
            .ingest(match reply(sequence, 1) {
                Completion::Reply(_, b) => b,
                _ => unreachable!(),
            })
            .unwrap()
        {
            assert!(sync.completion(&c, now + POLL));
        }
        assert_eq!(sync.status(window()), ResizeSyncStatus::Acknowledged);
        assert_eq!(sync.deadline(), None);
        assert_eq!(sync.begin(window(), now + POLL), Some(2));
    }
    #[test]
    fn stale_counter_replies_and_reused_windows_cannot_acknowledge_a_new_resize() {
        let now = Instant::now();
        let mut sync = ResizeSync::default();
        sync.capability(window(), Some(100), now).unwrap();
        pending(&mut sync, 1);
        sync.forget(window());
        sync.capability(window(), Some(100), now).unwrap();
        assert!(sync.completion(&reply(1, 0), now));
        assert_eq!(sync.status(window()), ResizeSyncStatus::Initializing);
        pending(&mut sync, 2);
        assert!(sync.completion(&reply(2, 0), now));
        sync.begin(window(), now);
        pending(&mut sync, 3);
        assert!(sync.completion(&reply(3, 0), now));
        assert_eq!(sync.status(window()), ResizeSyncStatus::Waiting);
        pending(&mut sync, 4);
        assert!(sync.completion(&reply(4, 1), now));
        sync.begin(window(), now);
        pending(&mut sync, 5);
        assert!(sync.completion(&reply(5, 1), now));
        assert_eq!(sync.status(window()), ResizeSyncStatus::Waiting);
        sync.forget(window());
        let reused = XWindow {
            incarnation: 2,
            ..window()
        };
        sync.capability(reused, Some(100), now).unwrap();
        assert_eq!(sync.status(window()), ResizeSyncStatus::Unsupported);
        assert_eq!(sync.status(reused), ResizeSyncStatus::Initializing);
    }
    #[test]
    fn timeout_and_bad_counter_are_explicit_fallbacks_not_acknowledgements() {
        let now = Instant::now();
        let mut sync = ResizeSync::default();
        sync.capability(window(), Some(100), now).unwrap();
        pending(&mut sync, 1);
        assert!(sync.completion(&reply(1, 0), now));
        sync.begin(window(), now);
        pending(&mut sync, 2);
        assert!(sync.completion(&reply(2, 1), now + TIMEOUT));
        assert_eq!(sync.status(window()), ResizeSyncStatus::TimedOut);
        assert_eq!(sync.begin(window(), now + TIMEOUT), None);
        assert!(!sync.waiting(window()));
        sync.forget(window());
        sync.capability(window(), Some(101), now).unwrap();
        pending(&mut sync, 3);
        assert!(sync.completion(
            &Completion::Error(
                RequestId {
                    generation: 1,
                    sequence: 3
                },
                1
            ),
            now
        ));
        assert_eq!(sync.status(window()), ResizeSyncStatus::InvalidCounter);
        assert_eq!(sync.deadline(), None);
    }
}
