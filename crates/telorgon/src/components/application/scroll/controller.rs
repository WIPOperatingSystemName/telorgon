//! Application-domain command owner over the shared neutral scroll transition engine.

use std::fmt;
use std::time::Duration;

use crate::foundation::{PointF, SizeF};
pub use crate::ui::layout::{
    RevealAlignment, RevealRequest, ScrollActivity, ScrollAnchorMode, ScrollCancelReason,
    ScrollChangeSource, ScrollDiagnostics, ScrollError, ScrollExtentAnchor, ScrollInputSource,
    ScrollMetrics, ScrollMotionId, ScrollMotionRequest, ScrollPhysics, ScrollState, ScrollUpdate,
};

/// One application request routed to a [`ScrollController`].
///
/// Motion steps carry caller-supplied elapsed time. Routing a command never starts a timer, captures
/// input, changes layout, or schedules a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollControllerCommand {
    SetExtents {
        viewport: SizeF,
        content: SizeF,
        anchor: ScrollExtentAnchor,
    },
    ScrollBy {
        delta: PointF,
        source: ScrollInputSource,
    },
    ScrollTo {
        offset: PointF,
        source: ScrollInputSource,
    },
    Reveal(RevealRequest),
    BeginDrag,
    DragBy {
        delta: PointF,
    },
    EndDrag {
        velocity: PointF,
        physics: ScrollPhysics,
        reduced_motion: bool,
    },
    StepMotion {
        id: ScrollMotionId,
        elapsed: Duration,
    },
    Cancel {
        reason: ScrollCancelReason,
    },
}

/// Typed application outcome retaining the exact neutral transition record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollControllerOutcome {
    ExtentsChanged(ScrollUpdate),
    Scrolled(ScrollUpdate),
    Revealed(ScrollUpdate),
    DragBegan(ScrollUpdate),
    Dragged(ScrollUpdate),
    DragEnded(ScrollUpdate),
    MotionStepped(ScrollUpdate),
    Cancelled(ScrollUpdate),
}

impl ScrollControllerOutcome {
    pub const fn update(self) -> ScrollUpdate {
        match self {
            Self::ExtentsChanged(update)
            | Self::Scrolled(update)
            | Self::Revealed(update)
            | Self::DragBegan(update)
            | Self::Dragged(update)
            | Self::DragEnded(update)
            | Self::MotionStepped(update)
            | Self::Cancelled(update) => update,
        }
    }

    pub fn changed(self) -> bool {
        self.update().changed()
    }

    pub const fn motion(self) -> ScrollMotionRequest {
        self.update().motion
    }
}

/// One application-owned controller around exactly one neutral [`ScrollState`].
#[derive(Clone, Debug, Default)]
pub struct ScrollController {
    state: ScrollState,
}

impl ScrollController {
    pub fn new(viewport: SizeF, content: SizeF) -> Result<Self, ScrollControllerError> {
        Ok(Self {
            state: ScrollState::new(viewport, content)?,
        })
    }

    pub const fn from_state(state: ScrollState) -> Self {
        Self { state }
    }

    pub const fn state(&self) -> &ScrollState {
        &self.state
    }

    pub fn into_state(self) -> ScrollState {
        self.state
    }

    pub const fn metrics(&self) -> ScrollMetrics {
        self.state.metrics()
    }

    pub const fn activity(&self) -> ScrollActivity {
        self.state.activity()
    }

    pub fn velocity(&self) -> PointF {
        self.state.velocity()
    }

    pub const fn diagnostics(&self) -> ScrollDiagnostics {
        self.state.diagnostics()
    }

    /// Computes reveal geometry without changing offset, activity, motion, or diagnostics.
    pub fn reveal_target(&self, request: RevealRequest) -> Result<PointF, ScrollControllerError> {
        self.state
            .reveal_target(request)
            .map_err(ScrollControllerError::Scroll)
    }

    /// Routes exactly one transition and returns its unapplied layout and scheduler effects.
    pub fn route(
        &mut self,
        command: ScrollControllerCommand,
    ) -> Result<ScrollControllerOutcome, ScrollControllerError> {
        Ok(match command {
            ScrollControllerCommand::SetExtents {
                viewport,
                content,
                anchor,
            } => ScrollControllerOutcome::ExtentsChanged(
                self.state.set_extents(viewport, content, anchor)?,
            ),
            ScrollControllerCommand::ScrollBy { delta, source } => {
                ScrollControllerOutcome::Scrolled(self.state.scroll_by(delta, source)?)
            }
            ScrollControllerCommand::ScrollTo { offset, source } => {
                ScrollControllerOutcome::Scrolled(self.state.scroll_to(offset, source)?)
            }
            ScrollControllerCommand::Reveal(request) => {
                ScrollControllerOutcome::Revealed(self.state.reveal(request)?)
            }
            ScrollControllerCommand::BeginDrag => {
                ScrollControllerOutcome::DragBegan(self.state.begin_drag())
            }
            ScrollControllerCommand::DragBy { delta } => {
                ScrollControllerOutcome::Dragged(self.state.drag_by(delta)?)
            }
            ScrollControllerCommand::EndDrag {
                velocity,
                physics,
                reduced_motion,
            } => ScrollControllerOutcome::DragEnded(self.state.end_drag(
                velocity,
                physics,
                reduced_motion,
            )?),
            ScrollControllerCommand::StepMotion { id, elapsed } => {
                ScrollControllerOutcome::MotionStepped(self.state.step_motion(id, elapsed)?)
            }
            ScrollControllerCommand::Cancel { reason } => {
                ScrollControllerOutcome::Cancelled(self.state.cancel(reason))
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollControllerError {
    Scroll(ScrollError),
}

impl From<ScrollError> for ScrollControllerError {
    fn from(error: ScrollError) -> Self {
        Self::Scroll(error)
    }
}

impl fmt::Display for ScrollControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "application scroll transition failed: {self:?}")
    }
}

impl std::error::Error for ScrollControllerError {}

#[cfg(test)]
mod tests {
    use crate::foundation::RectF;

    use super::*;

    fn size(width: f32, height: f32) -> SizeF {
        SizeF { width, height }
    }

    fn controller() -> ScrollController {
        ScrollController::new(size(100.0, 100.0), size(300.0, 500.0)).unwrap()
    }

    #[test]
    fn discrete_commands_preserve_consumed_unconsumed_and_source_details() {
        let mut controller = controller();
        let outcome = controller
            .route(ScrollControllerCommand::ScrollBy {
                delta: PointF { x: 250.0, y: 450.0 },
                source: ScrollInputSource::Wheel,
            })
            .unwrap();
        let ScrollControllerOutcome::Scrolled(update) = outcome else {
            panic!("scroll-by must return a scrolled outcome")
        };
        assert_eq!(update.after.offset, PointF { x: 200.0, y: 400.0 });
        assert_eq!(update.consumed_delta, PointF { x: 200.0, y: 400.0 });
        assert_eq!(update.unconsumed_delta, PointF { x: 50.0, y: 50.0 });
        assert_eq!(
            update.source,
            ScrollChangeSource::Input(ScrollInputSource::Wheel)
        );
        assert_eq!(controller.metrics(), update.after);
        assert_eq!(controller.diagnostics().boundary_hits, 1);
    }

    #[test]
    fn invalid_requests_leave_metrics_and_activity_atomic() {
        let mut controller = controller();
        let before = controller.metrics();
        let activity = controller.activity();
        assert_eq!(
            controller.route(ScrollControllerCommand::ScrollTo {
                offset: PointF {
                    x: f32::NAN,
                    y: 10.0,
                },
                source: ScrollInputSource::Programmatic,
            }),
            Err(ScrollControllerError::Scroll(ScrollError::InvalidOffset))
        );
        assert_eq!(controller.metrics(), before);
        assert_eq!(controller.activity(), activity);
        assert_eq!(controller.diagnostics().invalid_requests, 1);
    }

    #[test]
    fn extent_anchor_and_reveal_delegate_to_the_single_neutral_owner() {
        let mut controller = controller();
        controller
            .route(ScrollControllerCommand::ScrollTo {
                offset: PointF { x: 0.0, y: 350.0 },
                source: ScrollInputSource::Programmatic,
            })
            .unwrap();
        let extent = controller
            .route(ScrollControllerCommand::SetExtents {
                viewport: size(100.0, 100.0),
                content: size(300.0, 600.0),
                anchor: ScrollExtentAnchor {
                    horizontal: ScrollAnchorMode::Clamp,
                    vertical: ScrollAnchorMode::PreserveEndDistance,
                },
            })
            .unwrap();
        assert_eq!(extent.update().after.offset.y, 450.0);

        let request = RevealRequest::nearest(RectF {
            x: 0.0,
            y: 180.0,
            width: 20.0,
            height: 20.0,
        });
        assert_eq!(controller.reveal_target(request).unwrap().y, 180.0);
        let reveal = controller
            .route(ScrollControllerCommand::Reveal(request))
            .unwrap();
        assert!(matches!(reveal, ScrollControllerOutcome::Revealed(_)));
        assert_eq!(controller.metrics().offset.y, 180.0);
    }

    #[test]
    fn caller_timed_motion_keeps_generation_checks_and_scheduler_handoffs() {
        let mut controller = controller();
        controller
            .route(ScrollControllerCommand::BeginDrag)
            .unwrap();
        controller
            .route(ScrollControllerCommand::DragBy {
                delta: PointF { x: 0.0, y: 20.0 },
            })
            .unwrap();
        let ended = controller
            .route(ScrollControllerCommand::EndDrag {
                velocity: PointF { x: 0.0, y: 200.0 },
                physics: ScrollPhysics::new(200.0, 0.0).unwrap(),
                reduced_motion: false,
            })
            .unwrap();
        let ScrollMotionRequest::Start(id) = ended.motion() else {
            panic!("ending a moving drag must request caller scheduling")
        };
        assert_eq!(controller.activity(), ScrollActivity::Ballistic(id));

        let stale = ScrollMotionId::from_raw(id.generation() + 1).unwrap();
        let before = controller.metrics();
        assert_eq!(
            controller.route(ScrollControllerCommand::StepMotion {
                id: stale,
                elapsed: Duration::from_millis(100),
            }),
            Err(ScrollControllerError::Scroll(ScrollError::StaleMotion {
                expected: id,
                received: stale,
            }))
        );
        assert_eq!(controller.metrics(), before);
        assert_eq!(controller.activity(), ScrollActivity::Ballistic(id));

        let stepped = controller
            .route(ScrollControllerCommand::StepMotion {
                id,
                elapsed: Duration::from_millis(100),
            })
            .unwrap();
        assert!(matches!(
            stepped.motion(),
            ScrollMotionRequest::Continue(_) | ScrollMotionRequest::Stop(_)
        ));
        assert_eq!(
            controller.diagnostics().stale_motion_steps,
            1,
            "stale generations remain observable without corrupting motion"
        );
    }

    #[test]
    fn reduced_motion_and_cancellation_return_activity_without_scheduling() {
        let mut controller = controller();
        controller
            .route(ScrollControllerCommand::BeginDrag)
            .unwrap();
        let ended = controller
            .route(ScrollControllerCommand::EndDrag {
                velocity: PointF { x: 0.0, y: 500.0 },
                physics: ScrollPhysics::default(),
                reduced_motion: true,
            })
            .unwrap();
        assert_eq!(ended.motion(), ScrollMotionRequest::None);
        assert_eq!(controller.activity(), ScrollActivity::Idle);

        controller
            .route(ScrollControllerCommand::BeginDrag)
            .unwrap();
        let cancelled = controller
            .route(ScrollControllerCommand::Cancel {
                reason: ScrollCancelReason::ViewDeactivated,
            })
            .unwrap();
        assert!(matches!(cancelled, ScrollControllerOutcome::Cancelled(_)));
        assert_eq!(controller.activity(), ScrollActivity::Idle);
    }
}
