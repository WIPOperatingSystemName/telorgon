//! Timestamped MIDI on PipeWire DSP ports. MIDI 1.0 uses complete messages (no running
//! status), SysEx is owned and limited to 1024 bytes including F0/F7. UMP accepts one packet
//! of 1/2/4 native-endian words: utility/system/MIDI1/MIDI2/SysEx7/SysEx8/flex/stream types.
//! Reserved UMP types and fragmented byte-stream SysEx are rejected explicitly.
//! No implicit MIDI1<->MIDI2 translation is performed. Route matching representations.
mod packet;
mod sequence;
mod timing;
use crate::integrations::pipewire::{
    ConnectionHandle, ConnectionState, MediaError, ObjectHandle, ObjectKind, connection::Command,
};
pub use packet::{MAX_SYSEX_BYTES, MidiPacket, MidiRepresentation};
pub(crate) use sequence::{read_sequence, write_sequence};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiDirection {
    Input,
    Output,
}
#[derive(Clone, Debug)]
pub struct MidiConfig {
    pub name: String,
    pub direction: MidiDirection,
    pub representation: MidiRepresentation,
    pub queue_events: usize,
}
impl MidiConfig {
    pub fn new(
        name: impl Into<String>,
        direction: MidiDirection,
        representation: MidiRepresentation,
    ) -> Self {
        Self {
            name: name.into(),
            direction,
            representation,
            queue_events: 256,
        }
    }
    fn validate(&self) -> Result<(), MediaError> {
        if self.name.is_empty()
            || self.name.len() > 256
            || self.name.contains('\0')
            || !(1..=4096).contains(&self.queue_events)
        {
            Err(MediaError::InvalidArgument("MIDI configuration"))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MidiTime {
    /// Stream-local clock epoch. Schedule output using that output stream's clock;
    /// copying an input event to another stream requires rebasing its time.
    pub generation: u64,
    pub clock_id: u32,
    pub position: u64,
    pub monotonic_ns: u64,
    pub rate_num: u32,
    pub rate_denom: u32,
}
/// Input timestamps refer to the exact sample offset in a graph cycle. Output events
/// carry absolute graph positions in the same clock ID. Clock changes drop stale queued
/// events; late events are delivered at offset zero and counted separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiEvent {
    pub time: MidiTime,
    pub packet: MidiPacket,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MidiState {
    Connecting,
    Paused,
    Streaming,
    Stopped,
    Failed(MediaError),
}
#[derive(Clone, Copy, Debug, Default)]
pub struct MidiDiagnostics {
    pub delivered: u64,
    pub overflows: u64,
    pub late: u64,
    pub stale: u64,
    pub malformed: u64,
    pub discontinuities: u64,
}
pub(crate) struct MidiShared {
    pub clock_version: AtomicU64,
    pub state: Mutex<MidiState>,
    pub stop: AtomicBool,
    pub node: crate::integrations::pipewire::owned_node::OwnedNode,
    pub clock_id: AtomicU32,
    pub clock_generation: AtomicU64,
    pub position: AtomicU64,
    pub monotonic_ns: AtomicU64,
    pub rate_num: AtomicU32,
    pub rate_denom: AtomicU32,
    pub delivered: AtomicU64,
    pub overflows: AtomicU64,
    pub late: AtomicU64,
    pub stale: AtomicU64,
    pub malformed: AtomicU64,
    pub discontinuities: AtomicU64,
}
impl MidiShared {
    pub(crate) fn state(&self, state: MidiState) {
        let mut current = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(*current, MidiState::Failed(_)) {
            *current = state;
        }
    }
    pub(crate) fn publish_clock(&self, time: MidiTime) {
        // Single realtime writer. SeqCst payload atomics keep the sequence observation
        // coherent even on weakly ordered targets; readers retry a bounded number of times.
        self.clock_version.fetch_add(1, Ordering::SeqCst);
        self.clock_id.store(time.clock_id, Ordering::SeqCst);
        self.clock_generation.store(time.generation, Ordering::SeqCst);
        self.position.store(time.position, Ordering::SeqCst);
        self.monotonic_ns.store(time.monotonic_ns, Ordering::SeqCst);
        self.rate_num.store(time.rate_num, Ordering::SeqCst);
        self.rate_denom.store(time.rate_denom, Ordering::SeqCst);
        self.clock_version.fetch_add(1, Ordering::SeqCst);
    }
}
pub(crate) enum MidiEndpoint {
    Input(rtrb::Producer<MidiEvent>),
    Output(rtrb::Consumer<MidiEvent>),
}
/// Single producer/consumer endpoints own their event storage. send/receive are bounded
/// nonblocking operations; UI callers may poll at their own rate. Discovery and links use
/// the shared registry and Graph. Drop requests retirement without blocking the calling UI.
pub struct MidiStream {
    connection: ConnectionHandle,
    shared: Arc<MidiShared>,
    representation: MidiRepresentation,
    sender: Option<rtrb::Producer<MidiEvent>>,
    receiver: Option<rtrb::Consumer<MidiEvent>>,
    last_sent: Option<(u64, u32, u64)>,
}
impl MidiStream {
    pub fn open(connection: ConnectionHandle, config: MidiConfig) -> Result<Self, MediaError> {
        config.validate()?;
        connection.ensure_ready()?;
        let shared = Arc::new(MidiShared {
            clock_version: AtomicU64::new(0),
            state: Mutex::new(MidiState::Connecting),
            stop: AtomicBool::new(false),
            node: crate::integrations::pipewire::owned_node::OwnedNode::new()?,
            clock_id: AtomicU32::new(u32::MAX),
            clock_generation: AtomicU64::new(0),
            position: AtomicU64::new(0),
            monotonic_ns: AtomicU64::new(0),
            rate_num: AtomicU32::new(0),
            rate_denom: AtomicU32::new(0),
            delivered: AtomicU64::new(0),
            overflows: AtomicU64::new(0),
            late: AtomicU64::new(0),
            stale: AtomicU64::new(0),
            malformed: AtomicU64::new(0),
            discontinuities: AtomicU64::new(0),
        });
        let (producer, consumer) = rtrb::RingBuffer::new(config.queue_events);
        let (sender, receiver, endpoint) = match config.direction {
            MidiDirection::Input => (None, Some(consumer), MidiEndpoint::Input(producer)),
            MidiDirection::Output => (Some(producer), None, MidiEndpoint::Output(consumer)),
        };
        let representation = config.representation;
        connection.send(Command::CreateMidi(config, shared.clone(), endpoint))?;
        Ok(Self {
            connection,
            shared,
            representation,
            sender,
            receiver,
            last_sent: None,
        })
    }
    pub fn state(&self) -> MidiState {
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if matches!(state, MidiState::Failed(_)) { return state; }
        match self.connection.state() {
            ConnectionState::Ready => state,
            ConnectionState::Failed(error) => MidiState::Failed(error),
            _ if matches!(state, MidiState::Stopped) => state,
            _ => MidiState::Failed(MediaError::Disconnected),
        }
    }
    pub fn node(&self) -> Option<ObjectHandle> {
        if self.shared.stop.load(Ordering::Acquire)
            || matches!(self.state(), MidiState::Stopped | MidiState::Failed(_)) { return None; }
        self.shared.node.get(&self.connection)
    }
    /// Coherent latest graph cycle; None until processing starts or if the clock is
    /// changing throughout three read attempts. Compare clock IDs before aligning audio.
    pub fn clock(&self) -> Option<MidiTime> {
        if self.shared.stop.load(Ordering::Acquire) || self.state() != MidiState::Streaming {
            return None;
        }
        for _ in 0..3 {
            let version = self.shared.clock_version.load(Ordering::SeqCst);
            if version == 0 || version % 2 != 0 {
                continue;
            }
            let time = MidiTime {
                generation: self.shared.clock_generation.load(Ordering::SeqCst),
                clock_id: self.shared.clock_id.load(Ordering::SeqCst),
                position: self.shared.position.load(Ordering::SeqCst),
                monotonic_ns: self.shared.monotonic_ns.load(Ordering::SeqCst),
                rate_num: self.shared.rate_num.load(Ordering::SeqCst),
                rate_denom: self.shared.rate_denom.load(Ordering::SeqCst),
            };
            if version == self.shared.clock_version.load(Ordering::SeqCst) {
                return Some(time);
            }
        }
        None
    }
    /// FIFO nondecreasing positions per clock; rejects new events on overflow, preserving
    /// the caller's event by value in the error. Delivery can still be late or stale later.
    pub fn send(&mut self, event: MidiEvent) -> Result<(), (MediaError, MidiEvent)> {
        let state = self.state();
        if let MidiState::Failed(error) = state {
            return Err((error, event));
        }
        if self.shared.stop.load(Ordering::Acquire)
            || !matches!(state, MidiState::Paused | MidiState::Streaming)
        {
            return Err((MediaError::NotReady, event));
        }
        if event.time.generation == 0 {
            return Err((MediaError::NotReady, event));
        }
        if event.time.generation != self.shared.clock_generation.load(Ordering::SeqCst) {
            return Err((MediaError::StaleHandle, event));
        }
        if event.packet.representation() != self.representation
            || self.last_sent.is_some_and(|(generation, clock, position)| {
                generation == event.time.generation && clock == event.time.clock_id && event.time.position < position
            })
        {
            return Err((
                MediaError::InvalidArgument("MIDI representation or event ordering"),
                event,
            ));
        }
        let Some(sender) = self.sender.as_mut() else {
            return Err((MediaError::Unsupported("MIDI output"), event));
        };
        let stamp = (event.time.generation, event.time.clock_id, event.time.position);
        sender.push(event).map_err(|e| {
            self.shared.overflows.fetch_add(1, Ordering::Relaxed);
            let rtrb::PushError::Full(event) = e;
            (MediaError::QueueFull, event)
        })?;
        self.last_sent = Some(stamp);
        Ok(())
    }
    pub fn receive(&mut self) -> Result<Option<MidiEvent>, MediaError> {
        let receiver = self
            .receiver
            .as_mut()
            .ok_or(MediaError::Unsupported("MIDI input"))?;
        Ok(receiver.pop().ok())
    }
    pub fn diagnostics(&self) -> MidiDiagnostics {
        let s = &self.shared;
        MidiDiagnostics {
            delivered: s.delivered.load(Ordering::Relaxed),
            overflows: s.overflows.load(Ordering::Relaxed),
            late: s.late.load(Ordering::Relaxed),
            stale: s.stale.load(Ordering::Relaxed),
            malformed: s.malformed.load(Ordering::Relaxed),
            discontinuities: s.discontinuities.load(Ordering::Relaxed),
        }
    }
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}
impl Drop for MidiStream {
    fn drop(&mut self) {
        self.stop();
    }
}
#[derive(Clone, Debug)]
pub struct MidiPort {
    pub handle: ObjectHandle,
    pub name: String,
    pub direction: MidiDirection,
    pub representation: MidiRepresentation,
    pub node: Option<ObjectHandle>,
}
pub fn discover(connection: &ConnectionHandle) -> Vec<MidiPort> {
    let s = connection.snapshot();
    s.objects_of_kind(ObjectKind::Port)
        .filter_map(|o| {
            let representation = match o.properties.get("format.dsp")?.as_str() {
                "8 bit raw midi" => MidiRepresentation::Midi1,
                "32 bit raw UMP" => MidiRepresentation::Ump,
                _ => return None,
            };
            let direction = match o.properties.get("port.direction")?.as_str() {
                "in" => MidiDirection::Input,
                "out" => MidiDirection::Output,
                _ => return None,
            };
            Some(MidiPort {
                handle: o.handle,
                name: o.properties.get("port.name").cloned().unwrap_or_default(),
                direction,
                representation,
                node: o
                    .properties
                    .get("node.id")
                    .and_then(|v| v.parse().ok())
                    .and_then(|id| s.objects.get(&id))
                    .map(|n| n.handle),
            })
        })
        .collect()
}
