mod timer;

pub use crate::foundation::MonotonicInstant;
pub use timer::TimerHandle;
pub(crate) use timer::{PendingTimerStart, TimerArena, TimerStart};

/// Renderer- and event-loop-independent frame demand state for one mounted view.
#[derive(Clone, Debug)]
pub struct FrameScheduler {
    requested: bool,
    animation_active: bool,
    external_surface_dirty: bool,
    next_deadline: Option<crate::runtime::MonotonicInstant>,
}

impl Default for FrameScheduler {
    fn default() -> Self {
        Self {
            requested: true,
            animation_active: false,
            external_surface_dirty: false,
            next_deadline: None,
        }
    }
}

impl FrameScheduler {
    pub fn request(&mut self) {
        self.requested = true;
    }

    pub fn set_animation_active(&mut self, active: bool) {
        self.animation_active = active;
        if active {
            self.requested = true;
        }
    }

    pub fn external_surface_changed(&mut self) {
        self.external_surface_dirty = true;
        self.requested = true;
    }

    pub fn needs_frame(&self) -> bool {
        self.requested || self.animation_active || self.external_surface_dirty
    }

    /// Reports whether sampled theme or component motion requires successor frames.
    pub const fn animation_active(&self) -> bool {
        self.animation_active
    }

    /// Replaces the runtime-owned monotonic deadline exposed to the view host.
    pub(crate) fn set_next_deadline(&mut self, deadline: Option<crate::runtime::MonotonicInstant>) {
        self.next_deadline = deadline;
    }

    /// Returns the earliest timer/animation deadline known to this view.
    pub fn next_deadline(&self) -> Option<crate::runtime::MonotonicInstant> {
        self.next_deadline
    }

    /// Consumes current demand after the view assembly begins processor/frame preparation.
    pub fn begin_frame(&mut self) {
        self.requested = self.animation_active;
        self.external_surface_dirty = false;
    }
}
