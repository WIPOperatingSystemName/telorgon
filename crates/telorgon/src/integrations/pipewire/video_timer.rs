//! Owned control-loop timer. Mirrors pipewire-rs's StreamRc lifetime ownership pattern.
use pipewire as pw;
pub(super) struct VideoTimer {
    timer: Option<pw::loop_::TimerSource<'static>>,
    _mainloop: pw::main_loop::MainLoopRc,
}
impl VideoTimer {
    pub fn new(mainloop: pw::main_loop::MainLoopRc, callback: impl Fn(u64) + 'static) -> Self {
        let source = mainloop.loop_().add_timer(callback);
        // SAFETY: TimerSource borrows the native Loop at its stable address. MainLoopRc
        // owns that allocation and is retained below. This type is !Send/!Sync through Rc;
        // Drop destroys the timer on its control thread before releasing the loop owner.
        let timer = unsafe {
            std::mem::transmute::<pw::loop_::TimerSource<'_>, pw::loop_::TimerSource<'static>>(
                source,
            )
        };
        Self {
            timer: Some(timer),
            _mainloop: mainloop,
        }
    }
    pub fn set_period(&self, period: Option<std::time::Duration>) -> Result<(), super::MediaError> {
        self.timer
            .as_ref()
            .unwrap()
            .update_timer(period, period)
            .into_result()
            .map(|_| ())
            .map_err(super::connection::native)
    }
}
impl Drop for VideoTimer {
    fn drop(&mut self) {
        self.timer.take();
    }
}
