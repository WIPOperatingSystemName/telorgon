//! Linux initramfs continuation of the ordinary Telorgon boot splash. The caller supplies a
//! fresh OS-owned splash session and framebuffer path; no EFI state or private table is read.

mod fd;
mod protocol;
pub use fd::run_fd;
pub use protocol::{
    MAX_MILESTONE_LINE_BYTES, MAX_MILESTONE_TEXT_BYTES, MilestoneError, SplashMilestone,
    read_milestone,
};

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::authoring::compose::Component;
use crate::boot::{
    BootController, BootError, BootHostEvent, BootPhase, BootProgress, BootRequestId, BootSession,
};
use crate::foundation::MonotonicInstant;
use crate::graphics::presentation::PresentationError;
use crate::graphics::presentation::linux_framebuffer::{
    FramebufferExit, LinuxFramebufferPresenter,
};
use crate::host::embedded::{SoftwareUiHost, SoftwareUiHostError};
use crate::runtime::CompositionDriver;

pub struct LinuxSplashConfig {
    pub framebuffer: PathBuf,
    pub scale_factor: f64,
    pub maximum_framebuffer_bytes: usize,
    pub on_ready: FramebufferExit,
    pub on_failure: FramebufferExit,
}

impl LinuxSplashConfig {
    pub fn new(framebuffer: impl AsRef<Path>) -> Self {
        Self {
            framebuffer: framebuffer.as_ref().to_owned(),
            scale_factor: 1.0,
            maximum_framebuffer_bytes: 64 * 1024 * 1024,
            on_ready: FramebufferExit::KeepLastFrame,
            on_failure: FramebufferExit::KeepLastFrame,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinuxSplashExit {
    Ready,
    Failed(String),
    /// The producer closed its stream. This does not imply OS startup completed.
    InputClosed,
}

#[derive(Debug, thiserror::Error)]
pub enum LinuxSplashError {
    #[error("{0}")]
    Boot(#[from] BootError),
    #[error("{0}")]
    Presentation(#[from] PresentationError),
    #[error("{0}")]
    Ui(#[from] SoftwareUiHostError),
    #[error("{0}")]
    Milestone(#[from] MilestoneError),
    #[error("Linux splash requires an OS-starting session with an active request")]
    InvalidSession,
    #[error("splash software surface exceeds its configured byte budget")]
    ResourceLimit,
}

pub type LinuxSplashResult<T> = Result<T, LinuxSplashError>;

/// Runs only when explicitly called. Create `session` with `into_splash_session(target_id)` so the
/// OS publishes its own milestones without launching another boot image. Arrange display ownership
/// before calling and stop this host before the graphical session acquires the display.
///
/// The reader may be locked stdin or a FIFO. This initial synchronous host paints once per record
/// and blocks awaiting the next record; it does not promise continuously animated frames between
/// records. No window, service, worker thread, mode switch, or binary stdout output is created.
pub fn run<C: Component, R: BufRead>(
    session: BootSession<C>,
    config: LinuxSplashConfig,
    milestones: &mut R,
) -> LinuxSplashResult<LinuxSplashExit> {
    let mut host = SplashRuntime::new(session, config)?;
    let result = (|| {
        host.paint()?;
        loop {
            let Some(milestone) = read_milestone(milestones)? else {
                return Ok(LinuxSplashExit::InputClosed);
            };
            let exit = host.apply(milestone)?;
            host.paint()?;
            if let Some(exit) = exit {
                return Ok(exit);
            }
        }
    })();
    host.finish(result)
}

struct SplashRuntime {
    ui: SoftwareUiHost<CompositionDriver>,
    presenter: LinuxFramebufferPresenter,
    controller: BootController,
    request: BootRequestId,
    config: LinuxSplashConfig,
    clock: Instant,
    previous: std::time::Duration,
}

impl SplashRuntime {
    fn new<C: Component>(
        session: BootSession<C>,
        config: LinuxSplashConfig,
    ) -> LinuxSplashResult<Self> {
        let request = session
            .active_request()
            .ok_or(LinuxSplashError::InvalidSession)?;
        if session.controller.snapshot().phase != BootPhase::OsStarting {
            return Err(LinuxSplashError::InvalidSession);
        }
        let presenter =
            LinuxFramebufferPresenter::open(&config.framebuffer, config.maximum_framebuffer_bytes)?;
        let metrics = presenter.metrics(config.scale_factor)?;
        let bytes = (metrics.physical_extent.width as usize)
            .checked_mul(metrics.physical_extent.height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if bytes.is_none_or(|bytes| bytes > config.maximum_framebuffer_bytes) {
            return Err(LinuxSplashError::ResourceLimit);
        }
        let ui = SoftwareUiHost::from_composed(session.component, metrics)?;
        Ok(Self {
            ui,
            presenter,
            controller: session.controller,
            request,
            config,
            clock: Instant::now(),
            previous: std::time::Duration::ZERO,
        })
    }

    fn elapsed(&self) -> std::time::Duration {
        self.clock.elapsed()
    }

    fn apply(&mut self, milestone: SplashMilestone) -> LinuxSplashResult<Option<LinuxSplashExit>> {
        apply_milestone(&self.controller, self.request, milestone)
    }

    fn paint(&mut self) -> LinuxSplashResult<()> {
        let elapsed = self.clock.elapsed();
        self.controller
            .advance(elapsed.saturating_sub(self.previous));
        self.previous = elapsed;
        let now =
            MonotonicInstant::from_nanos(u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX));
        let frame = self.ui.advance(now, false)?;
        if let Some(surface) = frame.surface() {
            self.presenter.present(surface)?;
        }
        Ok(())
    }

    fn finish(
        mut self,
        result: LinuxSplashResult<LinuxSplashExit>,
    ) -> LinuxSplashResult<LinuxSplashExit> {
        if result.is_err() {
            // Display failure is best effort; preserve the original error for the caller.
            let _ = self.controller.report(BootHostEvent::Failed {
                request: self.request,
                message: "The splash presentation or milestone stream failed".into(),
            });
            let _ = self.paint();
        }
        let policy = match result {
            Ok(LinuxSplashExit::Ready) => self.config.on_ready,
            Ok(LinuxSplashExit::InputClosed) => FramebufferExit::KeepLastFrame,
            _ => self.config.on_failure,
        };
        let shutdown = self.presenter.shutdown(policy);
        match result {
            Ok(exit) => {
                shutdown?;
                Ok(exit)
            }
            Err(error) => Err(error),
        }
    }
}

fn apply_milestone(
    controller: &BootController,
    request: BootRequestId,
    milestone: SplashMilestone,
) -> LinuxSplashResult<Option<LinuxSplashExit>> {
    match milestone {
        SplashMilestone::Status(status) => controller.report(BootHostEvent::OsStarting {
            request,
            status,
            progress: BootProgress::Unknown,
        })?,
        SplashMilestone::Progress { completed, total } => {
            controller.report(BootHostEvent::OsStarting {
                request,
                status: controller.snapshot().status,
                progress: BootProgress::Measured { completed, total },
            })?
        }
        SplashMilestone::Ready => {
            controller.report(BootHostEvent::Complete { request })?;
            return Ok(Some(LinuxSplashExit::Ready));
        }
        SplashMilestone::Failed(message) => {
            controller.report(BootHostEvent::Failed {
                request,
                message: message.clone(),
            })?;
            return Ok(Some(LinuxSplashExit::Failed(message)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::{BootApplication, BootTarget};

    #[test]
    fn milestones_update_shared_production_splash_without_an_execution_request() {
        let session = BootApplication::new()
            .target(BootTarget::linux("linux", "Linux").unwrap())
            .build()
            .unwrap()
            .into_splash_session("linux")
            .unwrap();
        let request = session.active_request().unwrap();
        assert!(session.controller.take_request().is_none());
        apply_milestone(
            &session.controller,
            request,
            SplashMilestone::Status("Mounting root".into()),
        )
        .unwrap();
        apply_milestone(
            &session.controller,
            request,
            SplashMilestone::Progress {
                completed: 2,
                total: 5,
            },
        )
        .unwrap();
        assert_eq!(
            session.controller.snapshot().progress_kind,
            BootProgress::Measured {
                completed: 2,
                total: 5
            }
        );
        assert_eq!(
            apply_milestone(&session.controller, request, SplashMilestone::Ready).unwrap(),
            Some(LinuxSplashExit::Ready)
        );
        assert_eq!(session.controller.snapshot().phase, BootPhase::Complete);
    }
}
