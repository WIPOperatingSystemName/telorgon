use crate::boot::{
    BootProgress, BootTarget, BootTargetKind, PreviewCommand, PreviewPhase, PreviewScenario,
    PreviewSnapshot, PreviewTheme,
};
use alloc::{string::String, vec::Vec};
use core::time::Duration;

pub(crate) struct PreviewModel {
    pub snapshot: PreviewSnapshot,
    phase_elapsed: Duration,
}

impl PreviewModel {
    pub fn new(
        targets: Vec<BootTarget>,
        selected: usize,
        theme: PreviewTheme,
        scenario: PreviewScenario,
    ) -> Self {
        let mut model = Self {
            snapshot: PreviewSnapshot {
                targets,
                selected,
                theme,
                phase: PreviewPhase::Selecting,
                progress: 0.0,
                status: String::new(),
                frame: 0,
                elapsed: Duration::ZERO,
                paused: false,
                simulation: true,
                progress_kind: BootProgress::Unknown,
            },
            phase_elapsed: Duration::ZERO,
        };
        model.scenario(scenario);
        model
    }

    pub fn dispatch(&mut self, command: PreviewCommand) {
        match command {
            PreviewCommand::Select(index)
                if self.snapshot.phase == PreviewPhase::Selecting
                    && index < self.snapshot.targets.len() =>
            {
                self.snapshot.selected = index;
            }
            PreviewCommand::MoveSelection(delta)
                if self.snapshot.phase == PreviewPhase::Selecting =>
            {
                let count = self.snapshot.targets.len() as i64;
                self.snapshot.selected =
                    (self.snapshot.selected as i64 + i64::from(delta)).rem_euclid(count) as usize;
            }
            PreviewCommand::Launch if self.snapshot.phase == PreviewPhase::Selecting => {
                self.begin_loading();
            }
            PreviewCommand::Boot(id) if self.snapshot.phase == PreviewPhase::Selecting => {
                if let Some(index) = self
                    .snapshot
                    .targets
                    .iter()
                    .position(|target| target.id == id)
                {
                    self.snapshot.selected = index;
                    self.begin_loading();
                }
            }
            PreviewCommand::Retry if self.snapshot.phase == PreviewPhase::Failed => {
                self.begin_loading()
            }
            PreviewCommand::Reset => self.scenario(PreviewScenario::Selecting),
            PreviewCommand::SetTheme(theme) => self.snapshot.theme = theme,
            PreviewCommand::TogglePause => self.snapshot.paused = !self.snapshot.paused,
            PreviewCommand::Fail => self.scenario(PreviewScenario::Failed),
            PreviewCommand::ContinueOs if self.snapshot.phase == PreviewPhase::Loading => {
                self.scenario(PreviewScenario::OsStarting);
                self.phase_elapsed = Duration::ZERO;
                self.snapshot.progress = 0.0;
            }
            PreviewCommand::SetScenario(scenario) => self.scenario(scenario),
            _ => {}
        }
        self.refresh_status();
    }

    pub fn tick(&mut self, delta: Duration) {
        if self.snapshot.paused {
            return;
        }
        self.snapshot.frame = self.snapshot.frame.wrapping_add(1);
        self.snapshot.elapsed = self.snapshot.elapsed.saturating_add(delta);
        if matches!(
            self.snapshot.phase,
            PreviewPhase::Loading | PreviewPhase::OsStarting
        ) {
            self.phase_elapsed = self.phase_elapsed.saturating_add(delta);
            let stage_duration = Duration::from_secs(4);
            // Carry remaining time through the simulated handoff, even after a slow frame.
            if self.snapshot.phase == PreviewPhase::Loading && self.phase_elapsed >= stage_duration
            {
                self.snapshot.phase = PreviewPhase::OsStarting;
                self.phase_elapsed = self.phase_elapsed.saturating_sub(stage_duration);
            }
            if self.snapshot.phase == PreviewPhase::OsStarting
                && self.phase_elapsed >= stage_duration
            {
                self.snapshot.phase = PreviewPhase::Complete;
                self.phase_elapsed = stage_duration;
            }
            self.snapshot.progress = (self.phase_elapsed.as_secs_f32() / 4.0).clamp(0.0, 1.0);
        }
        self.refresh_status();
    }

    fn scenario(&mut self, scenario: PreviewScenario) {
        self.snapshot.phase = match scenario {
            PreviewScenario::Startup if self.snapshot.targets.len() == 1 => PreviewPhase::Loading,
            PreviewScenario::Startup => PreviewPhase::Selecting,
            PreviewScenario::Selecting => PreviewPhase::Selecting,
            PreviewScenario::Loading => PreviewPhase::Loading,
            PreviewScenario::OsStarting => PreviewPhase::OsStarting,
            PreviewScenario::Complete => PreviewPhase::Complete,
            PreviewScenario::Failed => PreviewPhase::Failed,
        };
        self.phase_elapsed = match scenario {
            PreviewScenario::Loading => Duration::from_millis(1400),
            PreviewScenario::OsStarting => Duration::from_millis(1200),
            PreviewScenario::Complete => Duration::from_secs(4),
            _ => Duration::ZERO,
        };
        // Startup and launch begin at zero; explicit phase previews start mid-phase.
        self.snapshot.progress = self.phase_elapsed.as_secs_f32() / 4.0;
        self.snapshot.paused = false;
        self.refresh_status();
    }

    fn begin_loading(&mut self) {
        self.scenario(PreviewScenario::Loading);
        self.phase_elapsed = Duration::ZERO;
        self.snapshot.progress = 0.0;
    }

    fn refresh_status(&mut self) {
        self.snapshot.progress_kind = if matches!(
            self.snapshot.phase,
            PreviewPhase::Selecting | PreviewPhase::Failed
        ) {
            BootProgress::Unknown
        } else {
            BootProgress::Measured {
                completed: (self.snapshot.progress * 4000.0) as u64,
                total: 4000,
            }
        };
        self.snapshot.status = match self.snapshot.phase {
            PreviewPhase::Selecting => "Choose where to start".into(),
            PreviewPhase::Loading if self.snapshot.progress < 0.33 => {
                "Reading the boot image".into()
            }
            PreviewPhase::Loading if self.snapshot.progress < 0.7 => "Preparing the kernel".into(),
            PreviewPhase::Loading => "Passing control to the operating system".into(),
            PreviewPhase::OsStarting => match self.snapshot.targets[self.snapshot.selected].kind {
                BootTargetKind::Linux => "Linux startup companion is drawing this splash".into(),
                BootTargetKind::Custom => "Your kernel is drawing this splash".into(),
                BootTargetKind::Unknown => "The operating system is starting".into(),
                BootTargetKind::Windows => "Windows is now drawing its own startup screen".into(),
            },
            PreviewPhase::Complete => "Startup simulation complete".into(),
            PreviewPhase::Failed => "The selected boot image could not be loaded".into(),
        };
    }
}
