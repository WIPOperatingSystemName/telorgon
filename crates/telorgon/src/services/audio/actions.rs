//! Shared control-thread action ordering for widgets and keyboard shortcuts.
use super::*;
use crate::integrations::{pipewire::connection::Completion, wireplumber::DefaultKind};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub enum AudioControlTarget {
    Node(ObjectHandle),
    DefaultOutput,
    DefaultInput,
}
#[derive(Clone, Copy, Debug)]
pub enum AudioSystemAction {
    /// A delta in cubic UI-volume units; repeated actions resolve after prior completion.
    AdjustVolume {
        target: AudioControlTarget,
        delta_ui: f32,
        amplification: Amplification,
    },
    ToggleMute {
        target: AudioControlTarget,
    },
    SetMute {
        target: AudioControlTarget,
        mute: bool,
    },
    /// Existing explicit-node actions participate in the same ordered queue.
    Direct(AudioAction),
}
impl AudioControls {
    fn system_action(&self, action: AudioSystemAction) -> Result<Request, MediaError> {
        let selector = match action {
            AudioSystemAction::AdjustVolume { target, .. }
            | AudioSystemAction::ToggleMute { target }
            | AudioSystemAction::SetMute { target, .. } => target,
            AudioSystemAction::Direct(action) => return self.execute(action),
        };
        let target = match selector {
            AudioControlTarget::Node(target) => target,
            AudioControlTarget::DefaultOutput => self
                .policy()
                .default_device(DefaultKind::Output)
                .ok_or(MediaError::NotReady)?,
            AudioControlTarget::DefaultInput => self
                .policy()
                .default_device(DefaultKind::Input)
                .ok_or(MediaError::NotReady)?,
        };
        match action {
            AudioSystemAction::SetMute { mute, .. } => self.set_mute(target, mute),
            AudioSystemAction::ToggleMute { .. } => {
                let node = self
                    .nodes()
                    .into_iter()
                    .find(|node| node.handle == target)
                    .ok_or(MediaError::StaleHandle)?;
                self.set_mute(
                    target,
                    !node
                        .mute
                        .ok_or(MediaError::Unsupported("observed mute state"))?,
                )
            }
            AudioSystemAction::AdjustVolume {
                delta_ui,
                amplification,
                ..
            } => {
                let node = self
                    .nodes()
                    .into_iter()
                    .find(|node| node.handle == target)
                    .ok_or(MediaError::StaleHandle)?;
                let gain = node
                    .channel_volumes
                    .iter()
                    .copied()
                    .max_by(|a, b| a.value().total_cmp(&b.value()))
                    .or(node.volume)
                    .ok_or(MediaError::Unsupported("observed volume state"))?;
                let ceiling = match amplification {
                    Amplification::Forbid => Gain::UNITY,
                    Amplification::AllowUpTo(limit) => limit,
                };
                let wanted = (gain.as_ui() + delta_ui).clamp(0.0, ceiling.as_ui());
                self.set_volume(target, Gain::ui(wanted)?, amplification)
            }
            AudioSystemAction::Direct(_) => unreachable!(),
        }
    }
}
struct Queued {
    action: AudioSystemAction,
    completion: Completion,
    epoch: u64,
    deadline: Instant,
}
struct Active {
    native: Request,
    completion: Completion,
}
/// Explicit owner, shared by a shell's widget and shortcut dispatch. Construction performs
/// no mutation or thread creation. Enqueue returns acceptance; returned Request completes
/// only after native observation. Poll on connection events and at least every 20 ms while
/// pending (native completion need not produce a registry event). No method is realtime-safe.
/// Default targets resolve at execution; explicit node handles retain stale-ID protection.
/// Disconnect/reconnect invalidates queued work rather than replaying it on a new connection.
pub struct AudioActionQueue {
    controls: AudioControls,
    queued: VecDeque<Queued>,
    active: Option<Active>,
    capacity: usize,
}
impl AudioActionQueue {
    pub fn new(controls: AudioControls, capacity: usize) -> Result<Self, MediaError> {
        if !(1..=64).contains(&capacity) {
            return Err(MediaError::InvalidArgument("audio action queue capacity"));
        }
        Ok(Self {
            controls,
            queued: VecDeque::with_capacity(capacity),
            active: None,
            capacity,
        })
    }
    pub fn pending(&self) -> usize {
        self.queued.len() + usize::from(self.active.is_some())
    }
    pub fn enqueue(&mut self, action: AudioSystemAction) -> Result<Request, MediaError> {
        let request = Request::new();
        self.enqueue_existing(action, &request)?;
        Ok(request)
    }
    pub(crate) fn enqueue_existing(
        &mut self,
        action: AudioSystemAction,
        request: &Request,
    ) -> Result<(), MediaError> {
        if self.pending() >= self.capacity {
            return Err(MediaError::QueueFull);
        }
        if let AudioSystemAction::AdjustVolume { delta_ui, .. } = action {
            if !delta_ui.is_finite() || delta_ui.abs() > 1.0 {
                return Err(MediaError::InvalidArgument(
                    "UI volume step must be within -1..1",
                ));
            }
        }
        let snapshot = self.controls.connection.snapshot();
        if snapshot.state != ConnectionState::Ready {
            return Err(MediaError::NotReady);
        }
        if snapshot.restricted {
            return Err(MediaError::PermissionDenied);
        }
        self.queued.push_back(Queued {
            action,
            completion: Completion(request.state.clone()),
            epoch: snapshot.epoch,
            deadline: Instant::now() + Duration::from_secs(5),
        });
        Ok(())
    }
    /// Advances at most eight cancelled/failed entries and starts at most one native action.
    pub fn poll(&mut self) {
        if let Some(active) = &self.active {
            match active.native.state() {
                RequestState::Complete(result) => {
                    let active = self.active.take().unwrap();
                    active.completion.finish(result);
                }
                _ => return,
            }
        }
        for _ in 0..8 {
            let Some(queued) = self.queued.pop_front() else {
                return;
            };
            if !queued.completion.begin() {
                continue;
            }
            let snapshot = self.controls.connection.snapshot();
            if snapshot.epoch != queued.epoch || snapshot.state != ConnectionState::Ready {
                queued.completion.finish(Err(MediaError::Disconnected));
                continue;
            }
            if Instant::now() > queued.deadline {
                queued.completion.finish(Err(MediaError::Timeout));
                continue;
            }
            match self.controls.system_action(queued.action) {
                Ok(native) => {
                    self.active = Some(Active {
                        native,
                        completion: queued.completion,
                    });
                    return;
                }
                Err(error) => queued.completion.finish(Err(error)),
            }
        }
    }
    pub fn controls(&self) -> &AudioControls {
        &self.controls
    }
}
impl Drop for AudioActionQueue {
    fn drop(&mut self) {
        for queued in self.queued.drain(..) {
            queued.completion.finish(Err(MediaError::Cancelled));
        }
        if let Some(active) = self.active.take() {
            // Accepted execution cannot be undone. Report lost confirmation, not cancellation,
            // when native cancellation has already crossed its execution boundary.
            let result = match active.native.state() {
                RequestState::Complete(result) => result,
                _ if active.native.cancel() => Err(MediaError::Cancelled),
                _ => Err(MediaError::Disconnected),
            };
            active.completion.finish(result);
        }
    }
}
