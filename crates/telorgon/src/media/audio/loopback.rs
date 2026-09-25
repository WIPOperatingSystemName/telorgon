//! Explicit graph-clock loopback with owned links and optional bounded feedback delay.
use super::{AudioFilter, FilterConfig, FilterCycle, FilterProcessor, FilterState};
use crate::integrations::pipewire::{
    graph::{Feedback, Graph, LinkLease, LinkState},
    *,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct LoopbackConfig {
    pub name: String,
    /// Per-channel (source output port, destination input port), using fresh registry handles.
    pub routes: Vec<(ObjectHandle, ObjectHandle)>,
    pub max_quantum: usize,
    pub delay_frames: usize,
    pub feedback: Feedback,
    pub start_paused: bool,
}
impl LoopbackConfig {
    pub fn new(routes: Vec<(ObjectHandle, ObjectHandle)>) -> Self {
        Self {
            name: "Telorgon loopback".into(),
            routes,
            max_quantum: 8192,
            delay_frames: 0,
            feedback: Feedback::Reject,
            start_paused: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoopbackState {
    Connecting,
    Paused,
    Active,
    Stopped,
    Failed(MediaError),
}
struct CopyProcessor {
    delay: Vec<f32>,
    delay_frames: usize,
    position: usize,
    gain: Arc<AtomicU32>,
    last_rate: Option<(u32, u32)>,
}
impl FilterProcessor for CopyProcessor {
    fn process(&mut self, mut cycle: FilterCycle<'_>) {
        let rate = (cycle.clock.rate_num, cycle.clock.rate_denom);
        if cycle.clock.discontinuity || self.last_rate.is_some_and(|previous| previous != rate) {
            self.delay.fill(0.0);
            self.position = 0;
        }
        self.last_rate = Some(rate);
        let gain = f32::from_bits(self.gain.load(Ordering::Acquire));
        for channel in 0..cycle.inputs.channels() {
            let Some(input) = cycle.inputs.channel(channel) else {
                continue;
            };
            let Some(output) = cycle.outputs.channel(channel) else {
                continue;
            };
            for (index, (source, destination)) in input.iter().zip(output).enumerate() {
                let source = if source.is_finite() { *source } else { 0.0 };
                let sample = if self.delay_frames == 0 {
                    source
                } else {
                    let slot =
                        channel * self.delay_frames + (self.position + index) % self.delay_frames;
                    let old = self.delay[slot];
                    self.delay[slot] = source;
                    old
                } * gain;
                *destination = if sample.is_finite() { sample } else { 0.0 };
            }
        }
        if self.delay_frames > 0 {
            self.position = (self.position + cycle.clock.frames) % self.delay_frames;
        }
    }
}
/// Owns one filter and at most 128 links. Call poll on registry changes and periodically
/// while connecting; it never waits. Construction starts no device other than the requested
/// graph routing. Creation is asynchronous, and partial setup stays paused until every link
/// exists. Failure/stop/drop withdraw only this owner's links/filter. No automatic reconnect.
pub struct AudioLoopback {
    connection: ConnectionHandle,
    config: LoopbackConfig,
    filter: AudioFilter,
    links: Vec<LinkLease>,
    gain: Arc<AtomicU32>,
    state: LoopbackState,
    deadline: Instant,
    desired_active: bool,
    applied_active: bool,
    activity: Option<(bool, Request)>,
    wired: bool,
}
impl AudioLoopback {
    pub fn open(connection: ConnectionHandle, config: LoopbackConfig) -> Result<Self, MediaError> {
        if !(1..=64).contains(&config.routes.len())
            || !(1..=32768).contains(&config.max_quantum)
            || config.delay_frames > 65536
            || (config.feedback == Feedback::Allow && config.delay_frames < config.max_quantum)
        {
            return Err(MediaError::InvalidArgument(
                "loopback routes/quantum/delay; feedback needs at least one maximum quantum of delay",
            ));
        }
        let snapshot = connection.snapshot();
        for &(source, destination) in &config.routes {
            validate_port(&snapshot, source, "out")?;
            validate_port(&snapshot, destination, "in")?;
        }
        let channels = config.routes.len();
        let gain = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let filter = AudioFilter::open(
            connection.clone(),
            FilterConfig {
                name: config.name.clone(),
                inputs: (0..channels)
                    .map(|index| format!("input_{index}"))
                    .collect(),
                outputs: (0..channels)
                    .map(|index| format!("output_{index}"))
                    .collect(),
                max_quantum: config.max_quantum,
                latency_frames: config.delay_frames as u32,
                start_paused: true,
            },
            CopyProcessor {
                delay: vec![0.0; channels * config.delay_frames],
                delay_frames: config.delay_frames,
                position: 0,
                gain: gain.clone(),
                last_rate: None,
            },
        )?;
        Ok(Self {
            connection,
            desired_active: !config.start_paused,
            config,
            filter,
            links: Vec::with_capacity(channels * 2),
            gain,
            state: LoopbackState::Connecting,
            deadline: Instant::now() + Duration::from_secs(5),
            applied_active: false,
            activity: None,
            wired: false,
        })
    }
    pub fn state(&self) -> LoopbackState {
        self.state.clone()
    }
    pub fn node(&self) -> Option<ObjectHandle> {
        self.filter.node()
    }
    pub fn frames_processed(&self) -> u64 {
        self.filter.frames_processed()
    }
    /// Application-local linear gain; publication takes effect on a following graph cycle.
    pub fn set_gain(&self, gain: f32) -> Result<(), MediaError> {
        if !gain.is_finite() || !(0.0..=16.0).contains(&gain) {
            return Err(MediaError::InvalidArgument("loopback gain"));
        }
        self.gain.store(gain.to_bits(), Ordering::Release);
        Ok(())
    }
    /// Changes intent; poll submits/observes activation in order. State is confirmed, not optimistic.
    pub fn set_active(&mut self, active: bool) {
        self.desired_active = active;
    }
    pub fn stop(&mut self) {
        self.links.clear();
        self.filter.stop();
        if let Some((_, request)) = self.activity.take() {
            request.cancel();
        }
        self.state = LoopbackState::Stopped;
    }
    fn fail(&mut self, error: MediaError) {
        self.stop();
        self.state = LoopbackState::Failed(error);
    }
    pub fn poll(&mut self) {
        if matches!(
            self.state,
            LoopbackState::Stopped | LoopbackState::Failed(_)
        ) {
            return;
        }
        if let Err(error) = self.advance() {
            self.fail(error);
        }
    }
    fn advance(&mut self) -> Result<(), MediaError> {
        let snapshot = self.connection.snapshot();
        if snapshot.state != ConnectionState::Ready {
            return Err(MediaError::Disconnected);
        }
        for &(source, destination) in &self.config.routes {
            validate_port(&snapshot, source, "out")?;
            validate_port(&snapshot, destination, "in")?;
        }
        match self.filter.state() {
            FilterState::Failed(error) => return Err(error),
            FilterState::Stopped => return Err(MediaError::Disconnected),
            _ => {}
        }
        if !self.wired {
            if Instant::now() >= self.deadline {
                return Err(MediaError::Timeout);
            }
            let Some(node) = self.filter.node() else {
                return Ok(());
            };
            let mut pairs = Vec::with_capacity(self.config.routes.len() * 2);
            for (index, &(source, destination)) in self.config.routes.iter().enumerate() {
                let find = |name: String, direction: &str| {
                    snapshot
                        .objects
                        .values()
                        .find(|object| {
                            object.kind == ObjectKind::Port
                                && object
                                    .properties
                                    .get("node.id")
                                    .and_then(|id| id.parse::<u32>().ok())
                                    == Some(node.id())
                                && object.properties.get("port.name") == Some(&name)
                                && object.properties.get("port.direction").map(String::as_str)
                                    == Some(direction)
                        })
                        .map(|object| object.handle)
                };
                let Some(input) = find(format!("input_{index}"), "in") else {
                    return Ok(());
                };
                let Some(output) = find(format!("output_{index}"), "out") else {
                    return Ok(());
                };
                pairs.push((source, input));
                pairs.push((output, destination));
            }
            let graph = Graph::new(self.connection.clone());
            for (output, input) in pairs {
                self.links
                    .push(graph.link(output, input, self.config.feedback)?);
            }
            self.wired = true;
        }
        let mut connected = true;
        for link in &self.links {
            match link.state() {
                LinkState::Active | LinkState::Paused => {}
                LinkState::Connecting => connected = false,
                LinkState::Failed(error) => return Err(error),
                LinkState::Removed => return Err(MediaError::StaleHandle),
            }
        }
        if !connected {
            if Instant::now() >= self.deadline {
                return Err(MediaError::Timeout);
            }
            return Ok(());
        }
        if let Some((active, request)) = &self.activity {
            if let RequestState::Complete(result) = request.state() {
                result?;
                self.applied_active = *active;
                self.activity = None;
            }
        }
        if self.activity.is_none() && self.desired_active != self.applied_active {
            self.activity = Some((
                self.desired_active,
                self.filter.set_active(self.desired_active)?,
            ));
        }
        self.state = if self.applied_active && self.filter.state() == FilterState::Processing {
            LoopbackState::Active
        } else {
            LoopbackState::Paused
        };
        Ok(())
    }
}
impl Drop for AudioLoopback {
    fn drop(&mut self) {
        self.stop();
    }
}
fn validate_port(
    snapshot: &RegistrySnapshot,
    handle: ObjectHandle,
    direction: &str,
) -> Result<(), MediaError> {
    let port = snapshot.resolve(handle)?;
    if port.kind != ObjectKind::Port
        || port.properties.get("port.direction").map(String::as_str) != Some(direction)
        || port.properties.get("format.dsp").map(String::as_str) != Some("32 bit float mono audio")
    {
        return Err(MediaError::Unsupported(
            "loopback requires matching mono-f32 DSP ports",
        ));
    }
    Ok(())
}
