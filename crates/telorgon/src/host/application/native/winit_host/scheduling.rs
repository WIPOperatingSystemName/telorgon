use super::*;

impl<S: NativeRuntimeSource, P: NativePresentation> NativeHost<S, P> {
    pub(super) fn redraw_eligible(&self) -> bool {
        self.drawable && !self.occluded && !self.suspended
    }

    pub(super) fn request_redraw_once(&mut self) {
        if !self.redraw_eligible() || !self.redraw.has_demand() {
            return;
        }
        if !self.redraw.queue_native_request() {
            self.diagnostics.redraw_requests_suppressed = self
                .diagnostics
                .redraw_requests_suppressed
                .saturating_add(1);
            return;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
            self.diagnostics.redraw_requests = self.diagnostics.redraw_requests.saturating_add(1);
        } else {
            self.redraw.cancel_native_request();
        }
    }

    pub(super) fn emit_host_diagnostics(&self) {
        #[cfg(feature = "profiler")]
        {
            crate::runtime::instrumentation::counter!(
                "host.pointer_moves.received",
                self.diagnostics.native_pointer_moves
            );
            crate::runtime::instrumentation::counter!(
                "host.input_turns",
                self.diagnostics.input_turns
            );
            crate::runtime::instrumentation::counter!(
                "host.input_turns.clean",
                self.diagnostics.clean_input_turns
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_requests",
                self.diagnostics.redraw_requests
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_requests.suppressed",
                self.diagnostics.redraw_requests_suppressed
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_callbacks",
                self.diagnostics.redraw_callbacks
            );
            crate::runtime::instrumentation::counter!(
                "host.presentations.idle",
                self.diagnostics.presentations_idle
            );
            crate::runtime::instrumentation::counter!(
                "host.presentations.submitted",
                self.diagnostics.presentations_submitted
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.runtime",
                self.diagnostics.redraw_reasons[RedrawReason::Runtime as usize]
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.input",
                self.diagnostics.redraw_reasons[RedrawReason::Input as usize]
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.command",
                self.diagnostics.redraw_reasons[RedrawReason::Command as usize]
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.external_wake",
                self.diagnostics.redraw_reasons[RedrawReason::ExternalWake as usize]
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.animation",
                self.diagnostics.redraw_reasons[RedrawReason::Animation as usize]
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.timer",
                self.diagnostics.redraw_reasons[RedrawReason::Timer as usize]
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.startup",
                self.diagnostics.redraw_reasons[RedrawReason::Startup as usize]
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.resize",
                self.diagnostics.redraw_reasons[RedrawReason::Resize as usize]
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.expose",
                self.diagnostics.redraw_reasons[RedrawReason::Expose as usize]
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.recovery",
                self.diagnostics.redraw_reasons[RedrawReason::Recovery as usize]
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.os",
                self.diagnostics.redraw_reasons[RedrawReason::OperatingSystem as usize]
            );
            crate::runtime::instrumentation::counter!(
                "host.redraw_reason.pointer_move",
                self.diagnostics.redraw_reasons[RedrawReason::PointerMove as usize]
            );
        }
    }

    pub(super) fn refresh_frame_interval(&self) -> Duration {
        let refresh_millihertz = self
            .window
            .as_ref()
            .and_then(|window| window.current_monitor())
            .and_then(|monitor| monitor.refresh_rate_millihertz());
        frame_interval_for_refresh_rate(refresh_millihertz)
    }

    pub(super) fn resize_signal(
        &self,
    ) -> Option<crate::host::application::native::resize::ResizeSignalSnapshot> {
        self.window
            .as_deref()
            .and_then(|window| self.resize_signals.snapshot(window))
    }

    pub(super) fn finalize_live_resize(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let signal = self.resize_signal();
        if !self.live_resize.needs_finalization(signal) {
            return true;
        }
        if self.pending_resize.is_pending() {
            self.mark_redraw(RedrawReason::Resize);
            self.redraw(event_loop, RedrawSource::SynchronousResize);
            return self.failure.is_none();
        }
        let policy = self.presentation.resize_policy();
        let Some(update) = self.live_resize.finalize(signal, policy) else {
            return true;
        };
        if let Err(error) = self.presentation.resize(update) {
            self.fail(event_loop, error);
            return false;
        }
        self.mark_redraw(RedrawReason::Resize);
        true
    }

    pub(super) fn complete_live_resize_release(
        &mut self,
        event_loop: &ActiveEventLoop,
        signal: crate::host::application::native::resize::ResizeSignalSnapshot,
        _observed_at: Instant,
    ) -> bool {
        // A native request made during the modal sizing loop may never deliver its callback until
        // well after WM_EXITSIZEMOVE. The synchronous release frame satisfies that request, so it
        // must no longer suppress subsequent animation or input redraws.
        self.redraw.cancel_native_request();
        // The direct Win32 stream is authoritative during the modal transaction. Re-read the
        // client size at release so a skipped nested callback or delayed Winit event cannot make
        // the final transaction commit an intermediate extent.
        if let Some(window) = self.window.as_ref() {
            let size = window.inner_size();
            let current_extent = SizeI {
                width: size.width as i32,
                height: size.height as i32,
            };
            if self.pending_resize.queue(current_extent) {
                self.mark_redraw(RedrawReason::Resize);
            }
        }
        #[cfg(feature = "profiler")]
        let submissions_before = self.diagnostics.presentations_submitted;

        if self.pending_resize.is_pending() {
            self.mark_redraw(RedrawReason::Resize);
            self.redraw(event_loop, RedrawSource::SynchronousResize);
        } else {
            let policy = self.presentation.resize_policy();
            if let Some(update) = self.live_resize.finalize(Some(signal), policy) {
                if let Err(error) = self.presentation.resize(update) {
                    self.fail(event_loop, error);
                    return false;
                }
            }
            // Resize release is a mandatory frame boundary even when the final WM_SIZE was already
            // consumed. This publishes the committed surface and resumes animation immediately.
            self.mark_redraw(RedrawReason::Resize);
            self.redraw(event_loop, RedrawSource::SynchronousResize);
        }
        if self.failure.is_some() {
            return false;
        }

        #[cfg(feature = "profiler")]
        if self.diagnostics.presentations_submitted > submissions_before {
            crate::runtime::instrumentation::instant!(
                "responsiveness.resize.final_frame_submitted"
            );
            crate::runtime::instrumentation::counter!(
                "responsiveness.resize.release_to_submit_ms",
                _observed_at.elapsed().as_secs_f64() * 1_000.0
            );
        }

        self.apply_post_turn_schedule(event_loop, Instant::now())
    }

    pub(super) fn flush_commands(&mut self) {
        #[cfg(feature = "profiler")]
        let _span = crate::runtime::instrumentation::span!("commands.flush");
        loop {
            let command = self
                .runtime
                .as_mut()
                .and_then(|runtime| runtime.pop_command());
            let Some(command) = command else { break };
            match command {
                Command::RequestFrame => self.mark_redraw(RedrawReason::Command),
            }
        }
    }

    pub(super) fn process_runtime_turn(&mut self, timestamp: MonotonicInstant) {
        let pending = self
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.has_pending_runtime_turn(timestamp));
        if !pending {
            return;
        }
        let outcome = {
            #[cfg(feature = "profiler")]
            let _span = crate::runtime::instrumentation::span!("input.flush");
            self.runtime
                .as_mut()
                .expect("pending runtime work requires a mounted runtime")
                .flush_input(timestamp)
        };
        self.sync_text_input();
        self.diagnostics.input_turns = self.diagnostics.input_turns.saturating_add(1);
        if outcome.processed_work() && !outcome.frame_became_needed() {
            self.diagnostics.clean_input_turns =
                self.diagnostics.clean_input_turns.saturating_add(1);
        }
        if outcome.frame_became_needed() {
            if outcome.timers_processed != 0 {
                self.mark_redraw(RedrawReason::Timer);
            } else if outcome.pointer_move_only_frame_became_needed() {
                self.mark_redraw(RedrawReason::PointerMove);
            } else if outcome.events_dispatched != 0 {
                self.mark_redraw(RedrawReason::Input);
            } else {
                self.mark_redraw(RedrawReason::ExternalWake);
            }
        }
    }

    pub(super) fn mark_runtime_frame_demand(&mut self) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        if runtime.needs_frame() {
            let reason = if runtime.animation_active() {
                RedrawReason::Animation
            } else {
                RedrawReason::Runtime
            };
            self.mark_redraw(reason);
        }
    }

    pub(super) fn apply_post_turn_schedule(
        &mut self,
        event_loop: &ActiveEventLoop,
        native_now: Instant,
    ) -> bool {
        let timestamp = self.timestamp_at(native_now);
        #[cfg(feature = "profiler")]
        let _pointer_profile_suppression =
            (!crate::runtime::instrumentation::pointer_move_events_enabled()
                && self.runtime.as_ref().is_some_and(|runtime| {
                    runtime.pending_runtime_turn_is_pointer_move_only(timestamp)
                }))
            .then(crate::runtime::instrumentation::suppress_current_thread);
        self.process_runtime_turn(timestamp);
        self.flush_commands();
        self.mark_runtime_frame_demand();
        if !self.refresh_pointer(event_loop, native_now) {
            return false;
        }

        let runtime_pending = self
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.has_pending_runtime_turn(timestamp));
        let next_deadline = self
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.next_deadline());
        let live_resize_active = cfg!(target_os = "windows")
            && self.resize_signal().is_some_and(
                crate::host::application::native::resize::ResizeSignalSnapshot::is_active,
            );
        let redraw_eligible = self.redraw_eligible();
        let redraw_demanded = self.redraw.has_demand();
        let frame_interval = (live_resize_active || (redraw_eligible && redraw_demanded))
            .then(|| self.refresh_frame_interval());
        let resize_throttled = live_resize_active
            && self.pending_resize.is_pending()
            && !self.pending_resize.is_due(
                native_now,
                frame_interval.expect("live resize needs a frame interval"),
            );
        let redraw_pacing_deadline = if redraw_eligible
            && redraw_demanded
            && !self.redraw.requires_immediate_presentation()
            && !resize_throttled
        {
            self.frame_pacer.throttle_deadline(
                native_now,
                frame_interval.expect("redraw demand needs a frame interval"),
            )
        } else {
            None
        };
        let redraw_throttled = resize_throttled || redraw_pacing_deadline.is_some();
        let redraw_view = (redraw_eligible && redraw_demanded && !redraw_throttled)
            .then_some(self.view)
            .flatten();
        let redraw_views = redraw_view.as_slice();
        let schedule = match PostTurnSchedule::new(
            RemainingWork::new(runtime_pending, false, false, false),
            redraw_views,
            next_deadline,
            PendingHostFacts::new(self.host_wake_pending, false),
        ) {
            Ok(schedule) => schedule,
            Err(error) => {
                self.fail(
                    event_loop,
                    format!("failed to publish post-turn schedule: {error}"),
                );
                return false;
            }
        };
        let plan = match interpret_schedule(
            &schedule,
            &self.views,
            WinitClockObservation::new(timestamp, native_now),
        ) {
            Ok(plan) => plan,
            Err(error) => {
                self.fail(
                    event_loop,
                    format!("failed to interpret Winit schedule: {error}"),
                );
                return false;
            }
        };

        if plan.wake_intent() == WinitWakeIntent::RequestWake {
            if self.event_proxy.send_event(HostEvent::RuntimeWake).is_err() {
                self.fail(
                    event_loop,
                    "failed to request the next managed runtime turn",
                );
                return false;
            }
            self.host_wake_pending = true;
        }

        let mut control_flow = plan.control_flow();
        if live_resize_active
            && let Some(resize_deadline) = self
                .pending_resize
                .next_due_at(frame_interval.expect("live resize needs a frame interval"))
        {
            control_flow = earlier_wait_deadline(control_flow, resize_deadline);
        }
        if let Some(redraw_deadline) = redraw_pacing_deadline {
            control_flow = earlier_wait_deadline(control_flow, redraw_deadline);
        }
        if let Some(pointer_deadline) = self
            .pointer
            .as_ref()
            .and_then(ManagedPointer::next_deadline)
        {
            control_flow = earlier_wait_deadline(control_flow, pointer_deadline);
        }
        event_loop.set_control_flow(control_flow);

        if !plan.redraw_targets().is_empty() {
            debug_assert_eq!(plan.redraw_targets().len(), 1);
            self.request_redraw_once();
        }
        self.emit_host_diagnostics();
        true
    }

    #[cfg(target_os = "windows")]
    pub(super) fn native_live_resize_tick(
        &mut self,
        extent: SizeI,
        observed_at: Instant,
        synchronize_present: bool,
        repeat_extent: bool,
    ) {
        if self.failure.is_some()
            || !self.resize_signal().is_some_and(
                crate::host::application::native::resize::ResizeSignalSnapshot::is_active,
            )
        {
            return;
        }

        self.drawable = extent.width > 0 && extent.height > 0;
        let queued = if repeat_extent {
            self.pending_resize.queue_for_barrier(extent)
        } else {
            self.pending_resize.queue(extent)
        };
        if queued {
            self.mark_redraw(RedrawReason::Resize);
        }
        let frame_interval = self.refresh_frame_interval();
        let resize_due =
            synchronize_present || self.pending_resize.is_due(observed_at, frame_interval);
        let redraw_due = synchronize_present
            || (self.redraw.has_demand()
                && self
                    .frame_pacer
                    .throttle_deadline(observed_at, frame_interval)
                    .is_none());
        if (resize_due || redraw_due)
            && let Err(error) = self.try_redraw(if synchronize_present {
                RedrawSource::SynchronousResizeBarrier
            } else {
                RedrawSource::SynchronousResize
            })
        {
            self.record_failure(error);
            // The proxy message may be buffered until the native loop exits, but it guarantees
            // the ordinary host path observes the failure and terminates afterward.
            let _ = self.event_proxy.send_event(HostEvent::RuntimeWake);
        }
    }
}
