//! Explicit desktop-control assembly shared by GUI components and compositor shortcuts.
use crate::{
    authoring::compose::{Signal, SignalWriter},
    input::NamedKey,
    integrations::{
        pipewire::{connection::Completion, *},
        wireplumber::DefaultKind,
    },
    services::audio::*,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
const CAPACITY: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopAudioSnapshot {
    pub state: ConnectionState,
    pub nodes: Vec<AudioNode>,
    pub devices: Vec<AudioDevice>,
    pub default_output: Option<ObjectHandle>,
    pub default_input: Option<ObjectHandle>,
    pub pending: usize,
    pub last_error: Option<MediaError>,
}
struct Command {
    action: AudioSystemAction,
    completion: Completion,
    epoch: u64,
}
#[derive(Clone)]
pub struct DesktopAudioHandle {
    signal: Signal<DesktopAudioSnapshot>,
    commands: SyncSender<Command>,
    wake: SyncSender<()>,
    stopped: Arc<AtomicBool>,
    pending: Arc<AtomicUsize>,
    connection: ConnectionHandle,
}
impl PartialEq for DesktopAudioHandle {
    fn eq(&self, other: &Self) -> bool {
        self.signal == other.signal
    }
}
impl DesktopAudioHandle {
    pub fn signal(&self) -> Signal<DesktopAudioSnapshot> {
        self.signal.clone()
    }
    /// Bounded acceptance, not native completion. One Request works from widgets and
    /// shortcuts alike, including cancellation before the ordered service dispatches it.
    pub fn execute(&self, action: AudioSystemAction) -> Result<Request, MediaError> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(MediaError::Disconnected);
        }
        let snapshot = self.connection.snapshot();
        if snapshot.state != ConnectionState::Ready {
            return Err(MediaError::NotReady);
        }
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < CAPACITY).then_some(count + 1)
            })
            .map_err(|_| MediaError::QueueFull)?;
        let request = Request::new();
        let command = Command {
            action,
            completion: Completion(request.state.clone()),
            epoch: snapshot.epoch,
        };
        if let Err(error) = self.commands.try_send(command) {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            return Err(match error {
                mpsc::TrySendError::Full(_) => MediaError::QueueFull,
                mpsc::TrySendError::Disconnected(_) => MediaError::Disconnected,
            });
        }
        let _ = self.wake.try_send(());
        Ok(request)
    }
    /// Call only for key presses the host has chosen to handle (and permitted repeats).
    /// This does not intercept keys, run on releases, or override firmware handling.
    pub fn media_key(&self, key: NamedKey, step_ui: f32) -> Result<Option<Request>, MediaError> {
        if !step_ui.is_finite() || !(0.0..=1.0).contains(&step_ui) {
            return Err(MediaError::InvalidArgument("volume key step"));
        }
        let (target, delta) = match key {
            NamedKey::AudioVolumeUp => (AudioControlTarget::DefaultOutput, step_ui),
            NamedKey::AudioVolumeDown => (AudioControlTarget::DefaultOutput, -step_ui),
            NamedKey::MicrophoneVolumeUp => (AudioControlTarget::DefaultInput, step_ui),
            NamedKey::MicrophoneVolumeDown => (AudioControlTarget::DefaultInput, -step_ui),
            NamedKey::AudioVolumeMute => {
                return self
                    .execute(AudioSystemAction::ToggleMute {
                        target: AudioControlTarget::DefaultOutput,
                    })
                    .map(Some);
            }
            NamedKey::MicrophoneToggle | NamedKey::MicrophoneVolumeMute => {
                return self
                    .execute(AudioSystemAction::ToggleMute {
                        target: AudioControlTarget::DefaultInput,
                    })
                    .map(Some);
            }
            _ => return Ok(None),
        };
        self.execute(AudioSystemAction::AdjustVolume {
            target,
            delta_ui: delta,
            amplification: Amplification::Forbid,
        })
        .map(Some)
    }
}
/// Owns one non-realtime control worker, not the connection. Keep the Connection owner
/// alive. Construction starts observation only; controls require explicit handle actions.
/// Drop requests shutdown without joining. shutdown joins on the caller's control thread.
pub struct DesktopAudio {
    handle: DesktopAudioHandle,
    worker: Option<JoinHandle<()>>,
}
impl DesktopAudio {
    pub fn start(connection: ConnectionHandle) -> Result<Self, MediaError> {
        let controls = AudioControls::new(connection.clone())?;
        let queue = AudioActionQueue::new(controls, CAPACITY)?;
        let (commands, receiver) = mpsc::sync_channel(CAPACITY);
        let (wake, wake_receiver) = mpsc::sync_channel(1);
        let sender = wake.clone();
        let subscription = connection.subscribe(64, move || {
            let _ = sender.try_send(());
        })?;
        let stopped = Arc::new(AtomicBool::new(false));
        let pending = Arc::new(AtomicUsize::new(0));
        let initial = DesktopAudioSnapshot {
            state: connection.state(),
            nodes: Vec::new(),
            devices: Vec::new(),
            default_output: None,
            default_input: None,
            pending: 0,
            last_error: None,
        };
        let (signal, publish) = Signal::new(initial);
        let handle = DesktopAudioHandle {
            signal,
            commands,
            wake,
            stopped: stopped.clone(),
            pending: pending.clone(),
            connection: connection.clone(),
        };
        let worker = thread::Builder::new()
            .name("telorgon-desktop-audio".into())
            .spawn(move || {
                run(
                    connection,
                    queue,
                    receiver,
                    wake_receiver,
                    subscription,
                    stopped,
                    pending,
                    publish,
                );
            })
            .map_err(|error| MediaError::Native(error.to_string()))?;
        Ok(Self {
            handle,
            worker: Some(worker),
        })
    }
    pub fn handle(&self) -> DesktopAudioHandle {
        self.handle.clone()
    }
    pub fn stop(&self) {
        self.handle.stopped.store(true, Ordering::Release);
        let _ = self.handle.wake.try_send(());
    }
    pub fn shutdown(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for DesktopAudio {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(
    connection: ConnectionHandle,
    mut queue: AudioActionQueue,
    receiver: mpsc::Receiver<Command>,
    wake: mpsc::Receiver<()>,
    events: Subscription,
    stopped: Arc<AtomicBool>,
    pending: Arc<AtomicUsize>,
    publish: SignalWriter<DesktopAudioSnapshot>,
) {
    let mut receipts: Vec<Completion> = Vec::with_capacity(CAPACITY);
    let mut last_error = None;
    while !stopped.load(Ordering::Acquire) {
        for _ in 0..64 {
            if events.try_recv().is_none() {
                break;
            }
        }
        for _ in 0..CAPACITY {
            let Ok(command) = receiver.try_recv() else {
                break;
            };
            let request = Request {
                state: command.completion.0.clone(),
            };
            if connection.snapshot().epoch != command.epoch {
                command.completion.finish(Err(MediaError::Disconnected));
            } else if request.state() == RequestState::Queued {
                if let Err(error) = queue.enqueue_existing(command.action, &request) {
                    command.completion.finish(Err(error));
                }
            }
            receipts.push(command.completion);
        }
        queue.poll();
        receipts.retain(|completion| {
            match (Request {
                state: completion.0.clone(),
            })
            .state()
            {
                RequestState::Complete(result) => {
                    match result {
                        Ok(()) => last_error = None,
                        Err(MediaError::Cancelled) => {}
                        Err(error) => last_error = Some(error),
                    }
                    pending.fetch_sub(1, Ordering::AcqRel);
                    false
                }
                _ => true,
            }
        });
        let policy = queue.controls().policy();
        publish.publish_if_changed(DesktopAudioSnapshot {
            state: connection.state(),
            nodes: queue.controls().nodes(),
            devices: queue.controls().devices(),
            default_output: policy.default_device(DefaultKind::Output),
            default_input: policy.default_device(DefaultKind::Input),
            pending: pending.load(Ordering::Acquire),
            last_error: last_error.clone(),
        });
        let _ = wake.recv_timeout(if pending.load(Ordering::Acquire) > 0 {
            Duration::from_millis(8)
        } else {
            Duration::from_millis(250)
        });
    }
    drop(queue);
    drop(receiver); // Completes accepted but undispatched commands through Completion::drop.
    drop(receipts);
    // Admission may still be unwinding a failed send; do not reset its atomic counter.
    publish.publish(DesktopAudioSnapshot {
        state: ConnectionState::Stopped,
        nodes: Vec::new(),
        devices: Vec::new(),
        default_output: None,
        default_input: None,
        pending: 0,
        last_error,
    });
}
