use super::*;

/// Owner-thread observations, not client-side or hardware timestamps. Requests may be
/// cached by synchronized subsurfaces; publication and presentation are separate stages.
#[derive(Clone, Copy)]
pub(crate) enum TimingEvent {
    CommitRequested {
        surface: u32,
        current_revision: u64,
        attaches_buffer: bool,
        callbacks: usize,
    },
    CallbacksQueued {
        surface: u32,
        revision: u64,
        count: usize,
        presented: bool,
    },
}

pub(crate) type TimingObserver = Box<dyn Fn(TimingEvent)>;

impl NativeCompositor<'_> {
    pub(crate) fn set_timing_observer(&mut self, observer: Option<TimingObserver>) {
        self.state.timing_observer = observer;
    }
}
