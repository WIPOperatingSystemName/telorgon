use super::{
    BootCommand, BootError, BootHostEvent, BootPhase, BootProgress, BootRequest, BootRequestId,
    BootResult, BootSnapshot, BootTarget, BootTheme,
};
use alloc::{collections::VecDeque, string::String, vec::Vec};
use core::time::Duration;

pub(crate) struct BootModel {
    pub snapshot: BootSnapshot,
    requests: VecDeque<BootRequest>,
    active: Option<BootRequestId>,
    next_request: u64,
}

impl BootModel {
    pub fn splash(targets: Vec<BootTarget>, selected: usize, theme: BootTheme) -> Self {
        let mut model = Self::new(targets, selected, theme);
        model.snapshot.phase = BootPhase::OsStarting;
        model.snapshot.status = "Starting the operating system".into();
        model.active = Some(BootRequestId(1));
        model.next_request = 2;
        model
    }
    pub fn new(targets: Vec<BootTarget>, selected: usize, theme: BootTheme) -> Self {
        Self {
            snapshot: BootSnapshot {
                theme,
                targets,
                selected,
                phase: BootPhase::Selecting,
                progress: 0.0,
                progress_kind: BootProgress::Unknown,
                status: "Choose where to start".into(),
                frame: 0,
                elapsed: Duration::ZERO,
                paused: false,
                simulation: false,
            },
            requests: VecDeque::new(),
            active: None,
            next_request: 1,
        }
    }

    pub fn dispatch(&mut self, command: BootCommand) {
        match command {
            BootCommand::Select(index)
                if self.snapshot.phase == BootPhase::Selecting
                    && index < self.snapshot.targets.len() =>
            {
                self.snapshot.selected = index
            }
            BootCommand::MoveSelection(delta)
                if self.snapshot.phase == BootPhase::Selecting
                    && !self.snapshot.targets.is_empty() =>
            {
                let count = self.snapshot.targets.len() as i64;
                self.snapshot.selected =
                    (self.snapshot.selected as i64 + i64::from(delta)).rem_euclid(count) as usize;
            }
            BootCommand::Launch if self.snapshot.phase == BootPhase::Selecting => self.launch(),
            BootCommand::Boot(id) if self.snapshot.phase == BootPhase::Selecting => {
                if let Some(index) = self
                    .snapshot
                    .targets
                    .iter()
                    .position(|target| target.id == id)
                {
                    self.snapshot.selected = index;
                    self.launch();
                }
            }
            BootCommand::Retry if self.snapshot.phase == BootPhase::Failed => self.launch(),
            BootCommand::Reset
                if matches!(
                    self.snapshot.phase,
                    BootPhase::Selecting | BootPhase::Failed | BootPhase::Complete
                ) || (self.snapshot.phase == BootPhase::Loading
                    && self
                        .requests
                        .iter()
                        .any(|request| Some(request.id) == self.active)) =>
            {
                // A queued launch can be cancelled; an executing host request cannot.
                self.requests.clear();
                self.active = None;
                self.snapshot.phase = BootPhase::Selecting;
                self.snapshot.status = "Choose where to start".into();
                self.set_progress(BootProgress::Unknown);
            }
            BootCommand::SetTheme(theme) => self.snapshot.theme = theme,
            _ => {}
        }
    }

    fn launch(&mut self) {
        let Some(target) = self.snapshot.targets.get(self.snapshot.selected).cloned() else {
            return;
        };
        let Some(source) = target.source.clone() else {
            self.snapshot.phase = BootPhase::Failed;
            self.snapshot.status = "This target has no boot source".into();
            self.set_progress(BootProgress::Unknown);
            return;
        };
        let id = BootRequestId(self.next_request);
        self.next_request = self.next_request.wrapping_add(1).max(1);
        self.active = Some(id);
        self.snapshot.phase = BootPhase::Loading;
        self.snapshot.status = "Opening the selected EFI image".into();
        self.set_progress(BootProgress::Unknown);
        self.requests.push_back(BootRequest { id, target, source });
    }

    pub fn take_request(&mut self) -> Option<BootRequest> {
        self.requests.pop_front()
    }
    pub fn active_request(&self) -> Option<BootRequestId> {
        self.active
    }

    pub fn advance(&mut self, delta: Duration) {
        self.snapshot.frame = self.snapshot.frame.wrapping_add(1);
        self.snapshot.elapsed = self.snapshot.elapsed.saturating_add(delta);
    }

    pub fn report(&mut self, event: BootHostEvent) -> BootResult<()> {
        let request = match &event {
            BootHostEvent::Loading { request, .. }
            | BootHostEvent::Handoff { request }
            | BootHostEvent::OsStarting { request, .. }
            | BootHostEvent::Complete { request }
            | BootHostEvent::Failed { request, .. } => *request,
        };
        if self.active != Some(request) {
            return Err(BootError::Host(
                "host event does not belong to the active boot request".into(),
            ));
        }
        match event {
            BootHostEvent::Loading {
                status, progress, ..
            } => {
                if self.snapshot.phase != BootPhase::Loading {
                    return Err(BootError::Host(
                        "loader updates require an active loading phase".into(),
                    ));
                }
                validate_status(&status)?;
                progress.validate()?;
                self.snapshot.status = status;
                self.set_progress(progress);
            }
            BootHostEvent::Handoff { .. } => {
                if self.snapshot.phase != BootPhase::Loading {
                    return Err(BootError::Host(
                        "handoff requires an active loading phase".into(),
                    ));
                }
                self.snapshot.phase = BootPhase::OsStarting;
                self.snapshot.status = "Passing control to the operating system".into();
                self.set_progress(BootProgress::Unknown);
            }
            BootHostEvent::OsStarting {
                status, progress, ..
            } => {
                if self.snapshot.phase != BootPhase::OsStarting {
                    return Err(BootError::Host(
                        "OS updates require a completed loader handoff".into(),
                    ));
                }
                validate_status(&status)?;
                progress.validate()?;
                self.snapshot.status = status;
                self.set_progress(progress);
            }
            BootHostEvent::Complete { .. } => {
                if self.snapshot.phase != BootPhase::OsStarting {
                    return Err(BootError::Host(
                        "completion requires an OS startup phase".into(),
                    ));
                }
                self.snapshot.phase = BootPhase::Complete;
                self.snapshot.status = "Startup complete".into();
                self.set_progress(BootProgress::Unknown);
                self.active = None;
            }
            BootHostEvent::Failed { message, .. } => {
                validate_status(&message)?;
                self.snapshot.phase = BootPhase::Failed;
                self.snapshot.status = message;
                self.set_progress(BootProgress::Unknown);
                self.active = None;
            }
        }
        Ok(())
    }

    fn set_progress(&mut self, progress: BootProgress) {
        self.snapshot.progress_kind = progress;
        self.snapshot.progress = progress.fraction().unwrap_or(0.0);
    }
}

fn validate_status(status: &String) -> BootResult<()> {
    if status.len() > 512 || status.chars().any(char::is_control) {
        Err(BootError::Host(
            "startup status must fit 512 bytes without control characters".into(),
        ))
    } else {
        Ok(())
    }
}
