use super::*;
use crate::host::application::input::InputCoalescingDiagnostics;

impl<D: ComponentDriver> AppRuntimeCore<D> {
    pub fn flush_input(&mut self, timestamp: MonotonicInstant) -> InputFlushOutcome {
        #[cfg(feature = "profiler")]
        let _input_span = crate::runtime::instrumentation::span!("input.dispatch");
        self.sync_interaction();
        let frame_needed_before = self.view.scheduler().needs_frame();
        if self.scroll.advance(
            self.view.ui_mut(),
            &self.layout,
            timestamp,
            self.motion_preference,
        ) {
            self.view.scheduler_mut().request();
        }
        let external_updates_processed = if self.view.external_updates_ready() {
            let processed = self.view.process_external_updates();
            self.sync_interaction();
            processed
        } else {
            0
        };
        let timestamp_ns = timestamp.as_nanos();
        let batch = self.input.drain();
        let InputCoalescingDiagnostics {
            events_received,
            pointer_moves_received,
            pointer_moves_coalesced,
            scroll_events_received,
            scroll_events_coalesced,
            resize_events_received,
            resize_events_coalesced,
        } = batch.diagnostics;
        let mut events = batch.events;
        let mut events_dispatched = 0;
        for event in events.drain(..) {
            if !self.interaction.is_active() && !matches!(event, PlatformInput::Resize(_)) {
                continue;
            }
            events_dispatched += 1;
            match event {
                PlatformInput::Resize(size) => {
                    self.extent = size;
                    self.view.set_viewport_size(size);
                    self.view.scheduler_mut().request();
                }
                PlatformInput::Input(
                    event @ InputEvent::PointerMoved {
                        pointer, position, ..
                    },
                ) => {
                    let hit = self.layout.hit_test(self.view.ui_mut(), position);
                    let routing =
                        self.interaction
                            .pointer_moved(self.view.ui_mut(), pointer, position, hit);
                    self.apply_pointer_routing(routing, position, timestamp_ns);
                    if let Some(target) = routing.target {
                        self.dispatch_observed(
                            target,
                            UiEventKind::Input(event),
                            LISTEN_POINTER,
                            timestamp_ns,
                            Some(position),
                        );
                    }
                }
                PlatformInput::Input(
                    event @ InputEvent::PointerButton {
                        pointer,
                        button,
                        state,
                        ..
                    },
                ) => {
                    if state == crate::input::ButtonState::Pressed {
                        self.scroll.cancel();
                    }
                    let position = self
                        .interaction
                        .pointer_position(pointer)
                        .unwrap_or_default();
                    let hit = self.layout.hit_test(self.view.ui_mut(), position);
                    let routing = self.interaction.pointer_button(
                        self.view.ui_mut(),
                        pointer,
                        button,
                        state,
                        hit,
                    );
                    self.apply_pointer_routing(routing, position, timestamp_ns);
                    if let Some(target) = routing.target {
                        self.dispatch_observed(
                            target,
                            UiEventKind::Input(event),
                            LISTEN_POINTER,
                            timestamp_ns,
                            Some(position),
                        );
                    }
                }
                PlatformInput::Input(
                    event @ InputEvent::Scroll {
                        pointer,
                        delta,
                        precision,
                        ..
                    },
                ) => {
                    let position = self
                        .interaction
                        .pointer_position(pointer)
                        .unwrap_or_default();
                    let hit = self.layout.hit_test(self.view.ui_mut(), position);
                    if let Some(target) = hit {
                        if self.scroll.wheel(
                            self.view.ui_mut(),
                            &self.layout,
                            target,
                            delta,
                            precision,
                            timestamp,
                            self.motion_preference,
                        ) {
                            self.view.scheduler_mut().request();
                        }
                        self.dispatch_observed(
                            target,
                            UiEventKind::Input(event),
                            LISTEN_POINTER,
                            timestamp_ns,
                            Some(position),
                        );
                    }
                }
                PlatformInput::TextInput { target, event } => {
                    self.dispatch_text_input(target, event, timestamp_ns);
                }
                PlatformInput::Input(InputEvent::TextInput(event)) => {
                    if let Some(target) = self.interaction.focused() {
                        self.dispatch_text_input(target, event, timestamp_ns);
                    }
                }
                PlatformInput::Input(InputEvent::ModifiersChanged(modifiers)) => {
                    self.modifiers = modifiers;
                    if let Some(target) = self.interaction.focused() {
                        self.dispatch_observed(
                            target,
                            UiEventKind::Input(InputEvent::ModifiersChanged(modifiers)),
                            LISTEN_KEY,
                            timestamp_ns,
                            None,
                        );
                    }
                }
                PlatformInput::Input(InputEvent::Key(key)) => {
                    self.modifiers = key.modifiers;
                    if let Some(target) = self.interaction.focused() {
                        self.dispatch_observed(
                            target,
                            UiEventKind::Input(InputEvent::Key(key.clone())),
                            LISTEN_KEY,
                            timestamp_ns,
                            None,
                        );
                    }
                    if key.logical_key == crate::input::LogicalKey::Named(NamedKey::Tab)
                        && key.state == ButtonState::Pressed
                        && !key.repeat
                    {
                        self.move_focus(key.modifiers.contains(Modifiers::SHIFT), timestamp_ns);
                    } else {
                        let routing = self.interaction.key(self.view.ui_mut(), &key);
                        if let Some((target, activation)) = routing.activation {
                            self.view.dispatch_activation(target, activation.source);
                        }
                        if routing.changed {
                            self.view.scheduler_mut().request();
                        }
                    }
                }
            }
            self.sync_interaction();
        }
        self.input.recycle(events);
        let task_results_processed = if self.view.task_results_ready() {
            let processed = self.view.process_task_results();
            self.sync_interaction();
            processed
        } else {
            0
        };
        let timers_processed = if self.view.timers_ready(timestamp) {
            let processed = self.view.process_timers(timestamp);
            self.sync_interaction();
            processed
        } else {
            0
        };
        let outcome = InputFlushOutcome {
            events_received,
            events_dispatched,
            pointer_moves_received,
            pointer_moves_coalesced,
            scroll_events_received,
            scroll_events_coalesced,
            resize_events_received,
            resize_events_coalesced,
            external_updates_processed: external_updates_processed as u64,
            task_results_processed: task_results_processed as u64,
            timers_processed: timers_processed as u64,
            frame_needed_before,
            frame_needed_after: self.view.scheduler().needs_frame(),
        };
        #[cfg(feature = "profiler")]
        {
            crate::runtime::instrumentation::counter!(
                "input.events.received",
                outcome.events_received
            );
            crate::runtime::instrumentation::counter!(
                "input.non_pointer_events.received",
                outcome
                    .events_received
                    .saturating_sub(outcome.pointer_moves_received)
            );
            crate::runtime::instrumentation::counter!(
                "input.events.dispatched",
                outcome.events_dispatched
            );
            crate::runtime::instrumentation::counter!(
                "input.pointer_moves.coalesced",
                outcome.pointer_moves_coalesced
            );
            crate::runtime::instrumentation::counter!(
                "input.scroll_events.coalesced",
                outcome.scroll_events_coalesced
            );
            crate::runtime::instrumentation::counter!(
                "input.resize_events.coalesced",
                outcome.resize_events_coalesced
            );
            crate::runtime::instrumentation::counter!(
                "runtime.external_updates.processed",
                outcome.external_updates_processed
            );
            crate::runtime::instrumentation::counter!(
                "runtime.task_results.processed",
                outcome.task_results_processed
            );
            crate::runtime::instrumentation::counter!(
                "runtime.timers.processed",
                outcome.timers_processed
            );
        }
        outcome
    }
}
