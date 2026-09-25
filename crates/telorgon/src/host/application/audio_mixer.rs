//! Shared desktop mixer owner. Reconnects observation without replaying mutations.
use super::desktop_audio::{DesktopAudio, DesktopAudioSnapshot};
use crate::{
    authoring::compose::{Signal, SignalWriter},
    integrations::{
        pipewire::{connection::Completion, *},
        wireplumber::DefaultKind,
    },
    services::audio::{mixer::*, *},
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub struct MixerOperationState {
    pub pending: bool,
    pub preview_volume: Option<f32>,
    pub error: Option<MediaError>,
    pub applied: usize,
    pub failed: usize,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MixerSnapshot {
    pub generation: u64,
    pub state: ConnectionState,
    pub nodes: Vec<AudioNode>,
    pub applications: Vec<MixerApplication>,
    pub devices: Vec<AudioDevice>,
    pub default_output: Option<ObjectHandle>,
    pub default_input: Option<ObjectHandle>,
    /// Observed direct graph destinations. Empty means unknown, not the default device.
    pub destinations: BTreeMap<ObjectHandle, Vec<ObjectHandle>>,
    pub operations: BTreeMap<MixerTarget, MixerOperationState>,
}
impl MixerSnapshot {
    pub fn targets(&self, target: &MixerTarget) -> Vec<AudioNode> {
        match target {
            MixerTarget::Node(id) => self
                .nodes
                .iter()
                .filter(|n| n.handle == *id)
                .cloned()
                .collect(),
            MixerTarget::DefaultOutput => self
                .default_output
                .map(|id| self.targets(&MixerTarget::Node(id)))
                .unwrap_or_default(),
            MixerTarget::DefaultInput => self
                .default_input
                // WirePlumber may use an output monitor as the default source when no
                // microphone exists. Its node volume is the output volume, not input gain.
                .map(|id| {
                    self.targets(&MixerTarget::Node(id))
                        .into_iter()
                        .filter(|node| node.media_class == "Audio/Source")
                        .collect()
                })
                .unwrap_or_default(),
            MixerTarget::Application(id, direction) => self
                .applications
                .iter()
                .find(|a| &a.id == id)
                .map(|a| a.streams(*direction).to_vec())
                .unwrap_or_default(),
        }
    }
    pub fn volume(&self, target: &MixerTarget) -> Option<f32> {
        self.operations
            .get(target)
            .and_then(|s| s.preview_volume)
            .or_else(|| group_volume(&self.targets(target)))
    }
}
fn empty(generation: u64, state: ConnectionState) -> MixerSnapshot {
    MixerSnapshot {
        generation,
        state,
        nodes: vec![],
        applications: vec![],
        devices: vec![],
        default_output: None,
        default_input: None,
        destinations: BTreeMap::new(),
        operations: BTreeMap::new(),
    }
}
#[derive(Clone, Debug)]
pub enum MixerAction {
    SetVolume {
        target: MixerTarget,
        volume: f32,
    },
    ToggleMute(MixerTarget),
    SetMute {
        target: MixerTarget,
        muted: bool,
    },
    SetDefault {
        device: ObjectHandle,
        input: bool,
    },
    MoveStream {
        stream: ObjectHandle,
        destination: ObjectHandle,
    },
    System(AudioSystemAction),
    Device(AudioAction),
}
struct Command {
    target: MixerTarget,
    generation: u64,
    deadline: Instant,
    preview: Option<f32>,
    toggle: bool,
    actions: VecDeque<AudioSystemAction>,
    completion: Completion,
}
fn resolve_toggle(command: &mut Command, snapshot: &MixerSnapshot) {
    if !command.toggle {
        return;
    }
    let members: Vec<_> = command
        .actions
        .iter()
        .filter_map(|action| match action {
            AudioSystemAction::Direct(AudioAction::Mute { target, .. }) => Some(*target),
            _ => None,
        })
        .collect();
    let nodes: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|node| members.contains(&node.handle))
        .cloned()
        .collect();
    let wanted = group_mute(&nodes) != MuteState::Muted;
    for action in &mut command.actions {
        if let AudioSystemAction::Direct(AudioAction::Mute { mute, .. }) = action {
            *mute = wanted;
        }
    }
}
struct Active {
    command: Command,
    request: Option<Request>,
    error: Option<MediaError>,
    applied: usize,
    failed: usize,
}
struct Shared {
    commands: Mutex<VecDeque<Command>>,
    baselines: Mutex<BTreeMap<MixerTarget, (u64, Vec<AudioNode>)>>,
    stopped: AtomicBool,
    wake: Mutex<Option<thread::Thread>>,
}
#[derive(Clone)]
pub struct AudioMixerHandle {
    signal: Signal<MixerSnapshot>,
    shared: Arc<Shared>,
}
impl PartialEq for AudioMixerHandle {
    fn eq(&self, other: &Self) -> bool {
        self.signal == other.signal
    }
}
impl AudioMixerHandle {
    pub fn signal(&self) -> Signal<MixerSnapshot> {
        self.signal.clone()
    }
    /// Slider writes replace undispatched values for the same target. Superseded requests
    /// finish Cancelled. Group requests finish after all members, returning the first error.
    pub fn execute(&self, action: MixerAction) -> Result<Request, MediaError> {
        if self.shared.stopped.load(Ordering::Acquire) {
            return Err(MediaError::Disconnected);
        }
        let snapshot = self.signal.snapshot();
        if snapshot.state != ConnectionState::Ready {
            return Err(MediaError::NotReady);
        }
        let toggle = matches!(&action, MixerAction::ToggleMute(_));
        let mut preview = None;
        let mut actions = VecDeque::new();
        let target = match action {
            MixerAction::SetVolume { target, volume } => {
                let nodes = snapshot.targets(&target);
                if nodes.is_empty() {
                    return Err(MediaError::StaleHandle);
                }
                let mut baselines = self
                    .shared
                    .baselines
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let baseline = baselines
                    .entry(target.clone())
                    .or_insert_with(|| (snapshot.generation, nodes.clone()));
                let default_changed = matches!(
                    target,
                    MixerTarget::DefaultOutput | MixerTarget::DefaultInput
                ) && baseline.1.first().map(|n| n.handle)
                    != nodes.first().map(|n| n.handle);
                if baseline.0 != snapshot.generation || default_changed {
                    *baseline = (snapshot.generation, nodes);
                }
                for (node, value) in relative_volumes(&baseline.1, volume)? {
                    actions.push_back(AudioSystemAction::Direct(AudioAction::Volume {
                        target: node,
                        gain: Gain::ui(value)?,
                        amplification: Amplification::Forbid,
                    }));
                }
                preview = Some(volume);
                target
            }
            MixerAction::SetMute { target, muted } => {
                mute_actions(&snapshot, &target, muted, &mut actions)?;
                target
            }
            MixerAction::ToggleMute(target) => {
                let muted = group_mute(&snapshot.targets(&target)) != MuteState::Muted;
                mute_actions(&snapshot, &target, muted, &mut actions)?;
                target
            }
            MixerAction::SetDefault { device, input } => {
                actions.push_back(AudioSystemAction::Direct(AudioAction::Default {
                    target: device,
                    kind: if input {
                        DefaultKind::Input
                    } else {
                        DefaultKind::Output
                    },
                }));
                MixerTarget::Node(device)
            }
            MixerAction::MoveStream {
                stream,
                destination,
            } => {
                actions.push_back(AudioSystemAction::Direct(AudioAction::MoveStream {
                    stream,
                    target: destination,
                }));
                MixerTarget::Node(stream)
            }
            MixerAction::Device(action) => {
                let target = match action {
                    AudioAction::Profile { target, .. }
                    | AudioAction::Route { target, .. }
                    | AudioAction::Balance { target, .. } => target,
                    _ => {
                        return Err(MediaError::InvalidArgument(
                            "device profile, route or balance action",
                        ));
                    }
                };
                actions.push_back(AudioSystemAction::Direct(action));
                MixerTarget::Node(target)
            }
            MixerAction::System(action) => {
                let target = match action {
                    AudioSystemAction::AdjustVolume { target, .. }
                    | AudioSystemAction::ToggleMute { target }
                    | AudioSystemAction::SetMute { target, .. } => match target {
                        AudioControlTarget::DefaultOutput => MixerTarget::DefaultOutput,
                        AudioControlTarget::DefaultInput => MixerTarget::DefaultInput,
                        AudioControlTarget::Node(node) => MixerTarget::Node(node),
                    },
                    AudioSystemAction::Direct(_) => {
                        return Err(MediaError::InvalidArgument("use typed mixer action"));
                    }
                };
                actions.push_back(action);
                target
            }
        };
        if actions.len() > 256 {
            return Err(MediaError::ResourceLimit("mixer group size"));
        }
        let mut commands = self
            .shared
            .commands
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.shared.stopped.load(Ordering::Acquire) {
            return Err(MediaError::Disconnected);
        }
        if preview.is_some() {
            // Never move a slider write across a mute/default/routing action.
            if let Some(last) = commands.back() {
                if last.target == target && last.preview.is_some() {
                    if let Some(old) = commands.pop_back() {
                        old.completion.finish(Err(MediaError::Cancelled));
                    }
                }
            }
        }
        if commands.len() >= 64 {
            return Err(MediaError::QueueFull);
        }
        let request = Request::new();
        commands.push_back(Command {
            target,
            generation: snapshot.generation,
            deadline: Instant::now() + Duration::from_secs(10),
            preview,
            toggle,
            actions,
            completion: Completion(request.state.clone()),
        });
        drop(commands);
        if let Some(worker) = self
            .shared
            .wake
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            worker.unpark();
        }
        Ok(request)
    }
}
fn mute_actions(
    snapshot: &MixerSnapshot,
    target: &MixerTarget,
    muted: bool,
    actions: &mut VecDeque<AudioSystemAction>,
) -> Result<(), MediaError> {
    let nodes = snapshot.targets(target);
    if nodes.is_empty() {
        return Err(MediaError::StaleHandle);
    }
    if nodes.iter().any(|n| !n.can_set_volume || n.mute.is_none()) {
        return Err(MediaError::Unsupported("mute control"));
    }
    for node in nodes {
        actions.push_back(AudioSystemAction::Direct(AudioAction::Mute {
            target: node.handle,
            mute: muted,
        }));
    }
    Ok(())
}
/// Explicit owner for one session connection and DesktopAudio. No audio capture is opened.
pub struct AudioMixer {
    handle: AudioMixerHandle,
    worker: Option<JoinHandle<()>>,
}
impl AudioMixer {
    pub fn start() -> Result<Self, MediaError> {
        let (signal, writer) = Signal::new(empty(0, ConnectionState::Connecting));
        let shared = Arc::new(Shared {
            commands: Mutex::new(VecDeque::new()),
            baselines: Mutex::new(BTreeMap::new()),
            stopped: AtomicBool::new(false),
            wake: Mutex::new(None),
        });
        let handle = AudioMixerHandle {
            signal,
            shared: shared.clone(),
        };
        let worker = thread::Builder::new()
            .name("telorgon-mixer".into())
            .spawn(move || run(shared, writer))
            .map_err(|e| MediaError::Native(e.to_string()))?;
        *handle.shared.wake.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(worker.thread().clone());
        Ok(Self {
            handle,
            worker: Some(worker),
        })
    }
    pub fn handle(&self) -> AudioMixerHandle {
        self.handle.clone()
    }
    pub fn shutdown(&mut self) {
        self.handle.shared.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}
impl Drop for AudioMixer {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn run(shared: Arc<Shared>, writer: SignalWriter<MixerSnapshot>) {
    let mut generation = 0;
    let mut retry = Instant::now();
    let mut session: Option<(Connection, DesktopAudio)> = None;
    let mut active: Option<Active> = None;
    let mut snapshot = empty(0, ConnectionState::Connecting);
    while !shared.stopped.load(Ordering::Acquire) {
        for state in snapshot.operations.values_mut() {
            state.pending = false;
            state.preview_volume = None;
        }
        if let Some(work) = &active {
            snapshot
                .operations
                .entry(work.command.target.clone())
                .and_modify(|state| {
                    state.pending = true;
                    state.preview_volume = work.command.preview;
                });
        }
        if session.is_none() && Instant::now() >= retry {
            generation += 1;
            let result = Connection::connect(ConnectionConfig::default(), Remote::Default)
                .and_then(
                    |mut connection| match DesktopAudio::start(connection.handle()) {
                        Ok(audio) => Ok((connection, audio)),
                        Err(error) => {
                            let _ = connection.shutdown();
                            Err(error)
                        }
                    },
                );
            match result {
                Ok(value) => {
                    snapshot = empty(generation, ConnectionState::Connecting);
                    session = Some(value);
                }
                Err(error) => {
                    snapshot = empty(generation, ConnectionState::Failed(error));
                    retry = Instant::now() + Duration::from_secs(2);
                }
            }
        }
        if let Some((connection, audio)) = &mut session {
            let observed = audio.handle().signal().snapshot();
            snapshot.state = observed.state.clone();
            snapshot.nodes = observed.nodes.clone();
            snapshot.devices = observed.devices.clone();
            snapshot.applications = applications(&snapshot.nodes);
            snapshot.default_input = observed.default_input;
            snapshot.default_output = observed.default_output;
            snapshot.destinations = destinations(&connection.handle().snapshot(), &observed);
            if snapshot.state == ConnectionState::Ready {
                if active.is_none() {
                    let command = shared
                        .commands
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .pop_front();
                    if let Some(mut command) = command {
                        if command.generation != generation {
                            command.completion.finish(Err(MediaError::Disconnected));
                        } else if Instant::now() >= command.deadline {
                            command.completion.finish(Err(MediaError::Timeout));
                        } else if command.completion.begin() {
                            resolve_toggle(&mut command, &snapshot);
                            snapshot.operations.insert(
                                command.target.clone(),
                                MixerOperationState {
                                    pending: true,
                                    preview_volume: command.preview,
                                    error: None,
                                    applied: 0,
                                    failed: 0,
                                },
                            );
                            active = Some(Active {
                                command,
                                request: None,
                                error: None,
                                applied: 0,
                                failed: 0,
                            });
                        }
                    }
                }
                if let Some(work) = &mut active {
                    if let Some(request) = &work.request {
                        if let RequestState::Complete(result) = request.state() {
                            match result {
                                Ok(()) => work.applied += 1,
                                Err(error) => {
                                    work.failed += 1;
                                    work.error.get_or_insert(error);
                                }
                            }
                            work.request = None;
                        }
                    }
                    if Instant::now() >= work.command.deadline {
                        work.failed += work.command.actions.len();
                        work.command.actions.clear();
                        work.error.get_or_insert(MediaError::Timeout);
                    }
                    if work.request.is_none() {
                        if let Some(action) = work.command.actions.pop_front() {
                            match audio.handle().execute(action) {
                                Ok(request) => work.request = Some(request),
                                Err(error) => {
                                    work.failed += 1;
                                    work.error.get_or_insert(error);
                                }
                            }
                        } else {
                            let done = active.take().unwrap();
                            if let Some(error) = &done.error {
                                eprintln!("audio-mixer: {:?}: {} applied, {} failed: {error}",
                                    done.command.target, done.applied, done.failed);
                            }
                            let queued = shared
                                .commands
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .iter()
                                .any(|c| c.target == done.command.target);
                            if !queued {
                                shared
                                    .baselines
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .remove(&done.command.target);
                            }
                            snapshot.operations.insert(
                                done.command.target.clone(),
                                MixerOperationState {
                                    pending: false,
                                    preview_volume: None,
                                    error: done.error.clone(),
                                    applied: done.applied,
                                    failed: done.failed,
                                },
                            );
                            done.command
                                .completion
                                .finish(done.error.map_or(Ok(()), Err));
                        }
                    }
                }
            } else if matches!(
                snapshot.state,
                ConnectionState::Failed(_) | ConnectionState::Stopped
            ) {
                if let Some(work) = active.take() {
                    work.command
                        .completion
                        .finish(Err(MediaError::Disconnected));
                }
                shared
                    .commands
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
                let (mut connection, mut audio) = session.take().unwrap();
                audio.shutdown();
                let _ = connection.shutdown();
                shared
                    .baselines
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
                snapshot = empty(generation, snapshot.state.clone());
                retry = Instant::now() + Duration::from_secs(2);
            }
        }
        // Queued slider previews follow the latest desired value, never a stale active write.
        for command in shared
            .commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            snapshot.operations.insert(
                command.target.clone(),
                MixerOperationState {
                    pending: true,
                    preview_volume: command.preview,
                    error: None,
                    applied: 0,
                    failed: 0,
                },
            );
        }
        snapshot.operations.retain(|target, state| {
            state.pending
                || match target {
                    MixerTarget::Node(id) => {
                        snapshot.nodes.iter().any(|n| n.handle == *id)
                            || snapshot.devices.iter().any(|n| n.handle == *id)
                    }
                    MixerTarget::Application(id, _) => {
                        snapshot.applications.iter().any(|a| &a.id == id)
                    }
                    _ => true,
                }
        });
        let targets: std::collections::BTreeSet<_> = shared
            .commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|command| command.target.clone())
            .chain(active.iter().map(|work| work.command.target.clone()))
            .collect();
        shared
            .baselines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|target, _| targets.contains(target));
        writer.publish_if_changed(snapshot.clone());
        thread::park_timeout(Duration::from_millis(if targets.is_empty() {
            250
        } else {
            8
        }));
    }
    shared
        .commands
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    drop(active);
    if let Some((mut connection, mut audio)) = session {
        audio.shutdown();
        let _ = connection.shutdown();
    }
    writer.publish(empty(generation, ConnectionState::Stopped));
}
fn destinations(
    registry: &RegistrySnapshot,
    audio: &DesktopAudioSnapshot,
) -> BTreeMap<ObjectHandle, Vec<ObjectHandle>> {
    let mut result: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for link in registry.objects_of_kind(ObjectKind::Link) {
        let output = link
            .properties
            .get("link.output.node")
            .and_then(|v| v.parse::<u32>().ok());
        let input = link
            .properties
            .get("link.input.node")
            .and_then(|v| v.parse::<u32>().ok());
        for stream in audio
            .nodes
            .iter()
            .filter(|n| n.media_class.starts_with("Stream/"))
        {
            let target = match stream.media_class.as_str() {
                "Stream/Output/Audio" if output == Some(stream.handle.id()) => input,
                "Stream/Input/Audio" if input == Some(stream.handle.id()) => output,
                _ => None,
            };
            if let Some(device) = audio.nodes.iter().find(|n| {
                Some(n.handle.id()) == target
                    && matches!(n.media_class.as_str(), "Audio/Sink" | "Audio/Source")
            }) {
                let list = result.entry(stream.handle).or_default();
                if !list.contains(&device.handle) {
                    list.push(device.handle);
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::audio::mixer::tests::node;
    fn fixture() -> (AudioMixerHandle, SignalWriter<MixerSnapshot>) {
        let mut state = empty(1, ConnectionState::Ready);
        state.nodes = vec![
            node(1, Some("app"), None, 0.8),
            node(2, Some("app"), None, 0.4),
        ];
        state.applications = applications(&state.nodes);
        state.default_output = Some(state.nodes[0].handle);
        let (signal, writer) = Signal::new(state);
        (
            AudioMixerHandle {
                signal,
                shared: Arc::new(Shared {
                    commands: Mutex::new(VecDeque::new()),
                    baselines: Mutex::new(BTreeMap::new()),
                    stopped: AtomicBool::new(false),
                    wake: Mutex::new(None),
                }),
            },
            writer,
        )
    }
    #[test]
    fn input_monitor_alias_cannot_change_output_volume() {
        let (handle, writer) = fixture();
        let mut snapshot = (*handle.signal.snapshot()).clone();
        snapshot.nodes[0].media_class = "Audio/Sink".into();
        snapshot.default_input = snapshot.default_output;
        writer.publish(snapshot);
        assert!(handle.signal.snapshot().targets(&MixerTarget::DefaultInput).is_empty());
        assert!(matches!(handle.execute(MixerAction::SetVolume {
            target: MixerTarget::DefaultInput, volume: 0.3,
        }), Err(MediaError::StaleHandle)));
        assert!(handle.shared.commands.lock().unwrap().is_empty());
        assert_eq!(handle.signal.snapshot().volume(&MixerTarget::DefaultOutput), Some(0.8));
    }

    #[test]
    fn mixer_coalesces_drag_values_without_crossing_mute_actions() {
        let (handle, _writer) = fixture();
        let target = MixerTarget::DefaultOutput;
        let first = handle
            .execute(MixerAction::SetVolume {
                target: target.clone(),
                volume: 0.2,
            })
            .unwrap();
        let latest = handle
            .execute(MixerAction::SetVolume {
                target: target.clone(),
                volume: 0.6,
            })
            .unwrap();
        assert_eq!(
            first.state(),
            RequestState::Complete(Err(MediaError::Cancelled))
        );
        assert_eq!(latest.state(), RequestState::Queued);
        assert_eq!(handle.shared.commands.lock().unwrap().len(), 1);
        handle
            .execute(MixerAction::ToggleMute(target.clone()))
            .unwrap();
        handle
            .execute(MixerAction::SetVolume {
                target,
                volume: 0.7,
            })
            .unwrap();
        assert_eq!(handle.shared.commands.lock().unwrap().len(), 3);
    }
    #[test]
    fn mixer_drag_preserves_baseline_during_partial_group_updates() {
        let (handle, writer) = fixture();
        let target = MixerTarget::Application(
            ApplicationGroupId::Application("app".into()),
            StreamDirection::Playback,
        );
        handle
            .execute(MixerAction::SetVolume {
                target: target.clone(),
                volume: 0.4,
            })
            .unwrap();
        let mut partial = (*handle.signal.snapshot()).clone();
        partial.nodes[0].volume = Some(Gain::ui(0.4).unwrap());
        partial.applications = applications(&partial.nodes);
        writer.publish(partial);
        handle
            .execute(MixerAction::SetVolume {
                target,
                volume: 0.6,
            })
            .unwrap();
        let commands = handle.shared.commands.lock().unwrap();
        let values: Vec<_> = commands
            .back()
            .unwrap()
            .actions
            .iter()
            .map(|action| match action {
                AudioSystemAction::Direct(AudioAction::Volume { gain, .. }) => gain.as_ui(),
                _ => panic!("volume expected"),
            })
            .collect();
        assert!((values[0] - 0.6).abs() < 0.0001);
        assert!((values[1] - 0.3).abs() < 0.0001);
    }
    #[test]
    fn mixer_rejects_disconnected_and_stopped_admission() {
        let (handle, writer) = fixture();
        writer.publish(empty(2, ConnectionState::Connecting));
        assert!(matches!(
            handle.execute(MixerAction::ToggleMute(MixerTarget::DefaultOutput)),
            Err(MediaError::NotReady)
        ));
        handle.shared.stopped.store(true, Ordering::Release);
        assert!(matches!(
            handle.execute(MixerAction::ToggleMute(MixerTarget::DefaultOutput)),
            Err(MediaError::Disconnected)
        ));
    }
    #[test]
    fn mixer_new_default_does_not_reuse_the_previous_device_drag() {
        let (handle, writer) = fixture();
        handle
            .execute(MixerAction::SetVolume {
                target: MixerTarget::DefaultOutput,
                volume: 0.3,
            })
            .unwrap();
        let mut observed = (*handle.signal.snapshot()).clone();
        let new_default = observed.nodes[1].handle;
        observed.default_output = Some(new_default);
        writer.publish(observed);
        handle
            .execute(MixerAction::SetVolume {
                target: MixerTarget::DefaultOutput,
                volume: 0.7,
            })
            .unwrap();
        let commands = handle.shared.commands.lock().unwrap();
        assert!(
            matches!(commands.back().unwrap().actions.front(), Some(AudioSystemAction::Direct(AudioAction::Volume { target, .. })) if *target == new_default)
        );
    }
    #[test]
    fn mixer_toggle_uses_observed_state_at_dispatch() {
        let (handle, writer) = fixture();
        handle
            .execute(MixerAction::ToggleMute(MixerTarget::DefaultOutput))
            .unwrap();
        let mut observed = (*handle.signal.snapshot()).clone();
        observed.nodes[0].mute = Some(true);
        writer.publish(observed.clone());
        let mut command = handle.shared.commands.lock().unwrap().pop_front().unwrap();
        resolve_toggle(&mut command, &observed);
        assert!(matches!(
            command.actions.front(),
            Some(AudioSystemAction::Direct(AudioAction::Mute {
                mute: false,
                ..
            }))
        ));
    }
    #[test]
    #[cfg(any(feature = "application-software", feature = "shell-wayland-linux"))]
    fn mixer_panel_mounts_without_a_live_audio_server() {
        let (handle, writer) = fixture();
        let mut snapshot = (*handle.signal.snapshot()).clone();
        let mut output = node(3, None, None, 0.65);
        output.media_class = "Audio/Sink".into();
        output.description = "Speakers".into();
        let mut input = node(4, None, None, 0.8);
        input.media_class = "Audio/Source".into();
        input.description = "Microphone".into();
        snapshot.default_output = Some(output.handle);
        snapshot.default_input = Some(input.handle);
        snapshot.nodes.extend([output, input]);
        let ready = snapshot.clone();
        snapshot.state = ConnectionState::Connecting;
        writer.publish(snapshot);
        let mut runtime = crate::host::application::AppRuntimeCore::from_composed_with_extent(
            crate::components::shell::AudioMixerPanel::new(handle.clone()),
            crate::SizeI {
                width: 380,
                height: 540,
            },
        )
        .unwrap();
        runtime
            .prepare_frame(crate::MonotonicInstant::ZERO, false)
            .unwrap();
        writer.publish(ready);
        runtime
            .prepare_frame(crate::MonotonicInstant::from_nanos(1), false)
            .unwrap();
        assert!(!runtime.ui().nodes.alive().is_empty());
        let sliders: Vec<_> = runtime
            .ui()
            .nodes
            .alive()
            .iter()
            .copied()
            .filter(|n| runtime.ui().kinds.get(*n) == Some(&crate::NodeKind::Slider))
            .collect();
        let buttons: Vec<_> = runtime
            .ui()
            .nodes
            .alive()
            .iter()
            .copied()
            .filter(|n| runtime.ui().kinds.get(*n) == Some(&crate::NodeKind::Button))
            .collect();
        assert_eq!(sliders.len(), 3);
        for slider in &sliders {
            let rect = runtime.layout().computed(*slider).unwrap().border_rect;
            assert!(
                rect.width >= 300.0,
                "slider input region must contain its whole track"
            );
            for button in &buttons {
                let other = runtime.layout().computed(*button).unwrap().border_rect;
                assert!(
                    rect.y + rect.height <= other.y
                        || other.y + other.height <= rect.y
                        || rect.x + rect.width <= other.x
                        || other.x + other.width <= rect.x,
                    "mixer controls overlap: {rect:?}, {other:?}"
                );
            }
        }

        // Wheel events over a child control must reach the overflowing mixer.
        let viewport = runtime
            .ui()
            .nodes
            .alive()
            .iter()
            .copied()
            .find(|n| runtime.ui().kinds.get(*n) == Some(&crate::NodeKind::Scroll))
            .unwrap();
        let rect = runtime.layout().computed(sliders[0]).unwrap().border_rect;
        runtime.queue_input(crate::InputEvent::mouse_moved(crate::PointF {
            x: rect.x + 5.0,
            y: rect.y + 5.0,
        }));
        runtime.queue_input(crate::InputEvent::mouse_scroll(crate::PointF {
            x: 0.0,
            y: -40.0,
        }));
        runtime.flush_input(crate::MonotonicInstant::from_nanos(2));
        runtime
            .prepare_frame(crate::MonotonicInstant::from_nanos(2), false)
            .unwrap();
        assert!(runtime.ui().layouts.get(viewport).unwrap().scroll_offset.y > 0.0);
        assert!(runtime.layout().computed(sliders[0]).unwrap().border_rect.y < rect.y);
        // Live audio updates must preserve the user's scroll position.
        let offset = runtime.ui().layouts.get(viewport).unwrap().scroll_offset;
        let mut update = (*handle.signal.snapshot()).clone();
        update.nodes[0].mute = Some(true);
        writer.publish(update);
        runtime
            .prepare_frame(crate::MonotonicInstant::from_nanos(3), false)
            .unwrap();
        assert_eq!(
            runtime.ui().layouts.get(viewport).unwrap().scroll_offset,
            offset
        );
        runtime.queue_input(crate::InputEvent::mouse_scroll(crate::PointF {
            x: 0.0,
            y: 1000.0,
        }));
        runtime.flush_input(crate::MonotonicInstant::from_nanos(4));
        runtime
            .prepare_frame(crate::MonotonicInstant::from_nanos(4), false)
            .unwrap();
        assert_eq!(
            runtime.ui().layouts.get(viewport).unwrap().scroll_offset.y,
            0.0
        );

        if let Ok(path) = std::env::var("TELORGON_MIXER_PREVIEW") {
            use crate::graphics::render::{
                RenderBackend, RenderRequest, RenderTargetInfo, TargetLoad, TargetStore,
            };
            use crate::graphics::renderers::software::{
                SoftwareRenderer, SoftwareSurface, SoftwareTarget,
            };
            let renderer = SoftwareRenderer;
            let mut scene = renderer.create_scene().unwrap();
            while let Some(delta) = runtime.pop_scene_delta() {
                renderer.apply_scene_delta(&mut scene, &delta).unwrap();
            }
            let mut surface = SoftwareSurface::default();
            let target = SoftwareTarget::new(RenderTargetInfo::full(crate::SizeI {
                width: 380,
                height: 540,
            }));
            renderer
                .render(
                    &mut scene,
                    &mut surface.begin_frame(),
                    &target,
                    &RenderRequest {
                        force: true,
                        load: TargetLoad::Clear(crate::ColorRgba8::rgba(32, 33, 39, 255)),
                        store: TargetStore::Store,
                        region: None,
                    },
                )
                .unwrap();
            image::save_buffer(
                path,
                surface.pixels_rgba8(),
                380,
                540,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        let row = runtime.ui().nodes.children(sliders[0]).next().unwrap();
        let track = runtime.ui().nodes.children(row).nth(1).unwrap();
        let rect = runtime.layout().computed(track).unwrap().border_rect;
        let point = crate::PointF {
            x: rect.x + rect.width - 1.0,
            y: rect.y + rect.height * 0.5,
        };
        runtime.queue_input(crate::InputEvent::mouse_moved(point));
        runtime.queue_input(crate::InputEvent::mouse_button(
            crate::input::PointerButton::PRIMARY,
            crate::input::ButtonState::Pressed,
        ));
        runtime.flush_input(crate::MonotonicInstant::from_nanos(1_000_000));
        let commands = handle.shared.commands.lock().unwrap();
        let command = commands
            .back()
            .expect("clicking the far end of the track submits volume");
        assert!(command.preview.unwrap() > 0.9);
    }
}
