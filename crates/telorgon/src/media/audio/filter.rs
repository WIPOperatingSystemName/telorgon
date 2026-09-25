//! Multiport, sample-synchronous audio filters. Ports are mono f32 DSP channels; routing
//! is explicit through the PipeWire graph. Graph rate/quantum changes arrive in each cycle.
use crate::integrations::pipewire::{
    ConnectionHandle, ConnectionState, MediaError, ObjectHandle, Request,
    connection::{Command, Completion},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
#[derive(Clone, Debug)]
pub struct FilterConfig {
    pub name: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub max_quantum: usize,
    /// Additional processing delay at the current graph sample rate, communicated to peers.
    pub latency_frames: u32,
    pub start_paused: bool,
}
impl FilterConfig {
    pub fn stereo(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            inputs: vec!["input_FL".into(), "input_FR".into()],
            outputs: vec!["output_FL".into(), "output_FR".into()],
            max_quantum: 8192,
            latency_frames: 0,
            start_paused: false,
        }
    }
    pub(crate) fn validate(&self) -> Result<(), MediaError> {
        if self.name.is_empty()
            || self.name.len() > 256
            || self.name.contains('\0')
            || self.inputs.len() > 64
            || self.outputs.len() > 64
            || self.inputs.len() + self.outputs.len() == 0
            || !(1..=32768).contains(&self.max_quantum)
            || self.latency_frames > 384000
        {
            return Err(MediaError::InvalidArgument("filter configuration"));
        }
        for ports in [&self.inputs, &self.outputs] {
            let mut unique = std::collections::BTreeSet::new();
            for name in ports {
                if name.is_empty()
                    || name.len() > 128
                    || name.contains('\0')
                    || !unique.insert(name)
                {
                    return Err(MediaError::InvalidArgument("filter port names"));
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct FilterClock {
    pub id: u32,
    pub position: u64,
    pub monotonic_ns: u64,
    pub rate_num: u32,
    pub rate_denom: u32,
    pub frames: usize,
    pub discontinuity: bool,
}
pub struct FilterInputs<'a> {
    pub(crate) storage: &'a [f32],
    pub(crate) stride: usize,
    pub(crate) frames: usize,
}
impl FilterInputs<'_> {
    pub fn channels(&self) -> usize {
        self.storage.len() / self.stride
    }
    pub fn channel(&self, index: usize) -> Option<&[f32]> {
        if index >= self.channels() {
            None
        } else {
            Some(&self.storage[index * self.stride..index * self.stride + self.frames])
        }
    }
}
pub struct FilterOutputs<'a> {
    pub(crate) storage: &'a mut [f32],
    pub(crate) stride: usize,
    pub(crate) frames: usize,
}
impl FilterOutputs<'_> {
    pub fn channels(&self) -> usize {
        self.storage.len() / self.stride
    }
    pub fn channel(&mut self, index: usize) -> Option<&mut [f32]> {
        if index >= self.channels() {
            None
        } else {
            Some(&mut self.storage[index * self.stride..index * self.stride + self.frames])
        }
    }
}
pub struct FilterCycle<'a> {
    pub clock: FilterClock,
    pub inputs: FilterInputs<'a>,
    pub outputs: FilterOutputs<'a>,
}
/// Realtime callback: prepare all storage before opening, never allocate/block/log/lock.
/// Every channel is exactly clock.frames long. Disconnected inputs and all outputs start
/// as silence. Honor configuration latency and handle clock changes without retaining slices.
pub trait FilterProcessor: Send + 'static {
    fn process(&mut self, cycle: FilterCycle<'_>);
}
impl<F> FilterProcessor for F
where
    F: for<'a> FnMut(FilterCycle<'a>) + Send + 'static,
{
    fn process(&mut self, cycle: FilterCycle<'_>) {
        self(cycle)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterState {
    Connecting,
    Paused,
    Processing,
    Stopped,
    Failed(MediaError),
}
pub(crate) struct FilterShared {
    pub state: Mutex<FilterState>,
    pub stop: AtomicBool,
    pub node: crate::integrations::pipewire::owned_node::OwnedNode,
    pub frames: AtomicU64,
    pub discontinuities: AtomicU64,
    pub failed: AtomicBool,
}
impl FilterShared {
    pub(crate) fn state(&self, state: FilterState) {
        let mut current = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(*current, FilterState::Failed(_)) {
            *current = state;
        }
    }
}
pub struct AudioFilter {
    id: u64,
    connection: ConnectionHandle,
    pub(crate) shared: Arc<FilterShared>,
}
static NEXT_FILTER: AtomicU64 = AtomicU64::new(1);
impl AudioFilter {
    pub fn open(
        connection: ConnectionHandle,
        config: FilterConfig,
        processor: impl FilterProcessor,
    ) -> Result<Self, MediaError> {
        config.validate()?;
        connection.ensure_ready()?;
        let id = NEXT_FILTER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| MediaError::ResourceLimit("filter identities"))?;
        let shared = Arc::new(FilterShared {
            state: Mutex::new(FilterState::Connecting),
            stop: AtomicBool::new(false),
            node: crate::integrations::pipewire::owned_node::OwnedNode::new()?,
            frames: AtomicU64::new(0),
            discontinuities: AtomicU64::new(0),
            failed: AtomicBool::new(false),
        });
        connection.send(Command::CreateFilter(
            id,
            config,
            shared.clone(),
            Box::new(processor),
        ))?;
        Ok(Self {
            id,
            connection,
            shared,
        })
    }
    pub fn state(&self) -> FilterState {
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if matches!(state, FilterState::Failed(_)) { return state; }
        match self.connection.state() {
            ConnectionState::Ready => state,
            ConnectionState::Failed(error) => FilterState::Failed(error),
            _ if matches!(state, FilterState::Stopped) => state,
            _ => FilterState::Failed(MediaError::Disconnected),
        }
    }
    pub fn node(&self) -> Option<ObjectHandle> {
        if self.shared.stop.load(Ordering::Acquire)
            || matches!(self.state(), FilterState::Stopped | FilterState::Failed(_)) { return None; }
        self.shared.node.get(&self.connection)
    }
    pub fn frames_processed(&self) -> u64 {
        self.shared.frames.load(Ordering::Relaxed)
    }
    pub fn discontinuities(&self) -> u64 {
        self.shared.discontinuities.load(Ordering::Relaxed)
    }
    pub fn set_active(&self, active: bool) -> Result<Request, MediaError> {
        self.connection.ensure_ready()?;
        let request = Request::new();
        self.connection.send(Command::FilterActive(
            self.id,
            active,
            Completion(request.state.clone()),
        ))?;
        Ok(request)
    }
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}
impl Drop for AudioFilter {
    fn drop(&mut self) {
        self.stop();
    }
}
