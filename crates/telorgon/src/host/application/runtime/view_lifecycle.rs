use super::*;

impl<D: ComponentDriver> AppRuntimeCore<D> {
    /// Cancels all transient interaction state when the containing native view deactivates.
    pub fn deactivate_view(&mut self, timestamp: MonotonicInstant) {
        for (pointer, position) in self.input.discard_interaction() {
            self.interaction.observe_pointer_position(pointer, position);
        }
        self.scroll.cancel();
        self.modifiers = Modifiers::empty();
        self.view.scheduler_mut().request();
        let old_focus = self.interaction.focused();
        if self.interaction.view_deactivated(self.view.ui_mut()) {
            self.view.scheduler_mut().request();
        }
        self.dispatch_focus_change(
            FocusChange {
                old: old_focus,
                new: None,
            },
            timestamp.as_nanos(),
        );
    }

    /// Restores a still-eligible control after native window activation.
    pub fn activate_view(&mut self, timestamp: MonotonicInstant) {
        let change = self.interaction.view_activated(self.view.ui_mut());
        self.dispatch_focus_change(change, timestamp.as_nanos());
        self.sync_interaction();
    }
}
