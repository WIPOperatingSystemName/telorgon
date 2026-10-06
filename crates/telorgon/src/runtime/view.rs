use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::input::{ChangeSource, ValueChangePhase};
use crate::ui::{EventPhase, MountWriter, MountedUi, UiEvent, UiEventKind, UiInputGeometry, UiNodeId};

use crate::runtime::{
    Command, Component, ComponentDiagnostics, ComponentDriver, ComponentId, ComponentRuntimeDriver,
    CompositionDiagnostics, CompositionDriver, FrameScheduler, LifecycleState, MonotonicInstant,
    RuntimeError, RuntimeResult, TaskHost, context::DriverContext,
};

static NEXT_VIEW_ID: AtomicU64 = AtomicU64::new(1);

/// One mounted application/UI-node lifecycle with no layout, renderer, platform, or event-loop
/// ownership.
pub struct ViewRuntime<A: ComponentDriver> {
    driver: A,
    ui: MountedUi,
    scheduler: FrameScheduler,
    commands: VecDeque<Command>,
    event_path: Vec<UiNodeId>,
}

impl<A: ComponentDriver> ViewRuntime<A> {
    pub fn new(mut driver: A) -> RuntimeResult<Self> {
        let mut ui = MountedUi::default();
        let mounted_root = {
            let mut builder = MountWriter::new(&mut ui);
            driver.mount(&mut builder)
        };
        if ui.root() != Some(mounted_root) {
            return Err(RuntimeError::new(
                "Component::mount must return the root created by the foundation writer",
            ));
        }
        let mut runtime = Self {
            driver,
            ui,
            scheduler: FrameScheduler::default(),
            commands: VecDeque::new(),
            event_path: Vec::with_capacity(16),
        };
        let mut frame_requested = false;
        runtime.driver.initialize(&mut DriverContext {
            ui: &mut runtime.ui,
            commands: &mut runtime.commands,
            frame_requested: &mut frame_requested,
        });
        if frame_requested {
            runtime.scheduler.request();
        }
        runtime.sync_deadline();
        Ok(runtime)
    }

    pub fn driver(&self) -> &A {
        &self.driver
    }

    pub fn driver_mut(&mut self) -> &mut A {
        &mut self.driver
    }

    pub fn ui(&self) -> &MountedUi {
        &self.ui
    }

    pub fn ui_mut(&mut self) -> &mut MountedUi {
        &mut self.ui
    }

    pub fn scheduler(&self) -> &FrameScheduler {
        &self.scheduler
    }

    pub fn scheduler_mut(&mut self) -> &mut FrameScheduler {
        &mut self.scheduler
    }

    pub fn drain_commands(&mut self) -> impl Iterator<Item = Command> + '_ {
        self.commands.drain(..)
    }

    pub fn pop_command(&mut self) -> Option<Command> {
        self.commands.pop_front()
    }

    /// Reports whether an injected task host has completed work waiting for a later UI turn.
    pub fn task_results_ready(&self) -> bool {
        self.driver.task_results_ready()
    }

    /// Reports whether a timer is due or a cancellation needs processing at `now`.
    pub fn timers_ready(&self, now: MonotonicInstant) -> bool {
        self.driver.timers_ready(now)
    }

    /// Reports whether a watched external signal invalidated one or more components.
    pub fn external_updates_ready(&self) -> bool {
        self.driver.external_updates_ready()
    }

    /// Coalesces and reconciles components invalidated by external signals.
    pub fn process_external_updates(&mut self) -> usize {
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("component.evaluate");
        let mut frame_requested = false;
        let processed = {
            let mut context = DriverContext {
                ui: &mut self.ui,
                commands: &mut self.commands,
                frame_requested: &mut frame_requested,
            };
            self.driver.process_external_updates(&mut context)
        };
        if frame_requested {
            self.scheduler.request();
        }
        processed
    }

    /// Processes a bounded batch of completed task actions on the UI writer thread.
    pub fn process_task_results(&mut self) -> usize {
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("tasks.process");
        let mut frame_requested = false;
        let processed = {
            let mut context = DriverContext {
                ui: &mut self.ui,
                commands: &mut self.commands,
                frame_requested: &mut frame_requested,
            };
            self.driver.process_task_results(&mut context)
        };
        if frame_requested {
            self.scheduler.request();
        }
        self.sync_deadline();
        processed
    }

    /// Processes a bounded batch of due timer actions on the UI writer thread.
    pub fn process_timers(&mut self, now: MonotonicInstant) -> usize {
        let mut frame_requested = false;
        let processed = {
            let mut context = DriverContext {
                ui: &mut self.ui,
                commands: &mut self.commands,
                frame_requested: &mut frame_requested,
            };
            self.driver.process_timers(now, &mut context)
        };
        if frame_requested {
            self.scheduler.request();
        }
        self.sync_deadline();
        processed
    }

    /// Cancels all live tasks and replaces the injected host with the unsupported capability.
    pub fn shutdown_task_host(&mut self) -> usize {
        let cancelled = self.driver.shutdown_task_host();
        self.sync_deadline();
        cancelled
    }

    fn dispatch_root_action(&mut self, action: A::Action) {
        let mut frame_requested = false;
        {
            let mut context = DriverContext {
                ui: &mut self.ui,
                commands: &mut self.commands,
                frame_requested: &mut frame_requested,
            };
            self.driver.dispatch_root_action(action, &mut context);
        }
        if frame_requested {
            self.scheduler.request();
        }
        self.sync_deadline();
    }

    pub(crate) fn set_viewport_size(&mut self, size: crate::SizeF) {
        self.driver.set_viewport_size(size);
        self.scheduler.request();
    }

    fn dispatch_driver_ui_route(&mut self, event: &UiEvent, listener_mask: u16) {
        let mut frame_requested = false;
        {
            let mut context = DriverContext {
                ui: &mut self.ui,
                commands: &mut self.commands,
                frame_requested: &mut frame_requested,
            };
            self.driver
                .dispatch_ui_route(event, listener_mask, &mut context);
        }
        if frame_requested {
            self.scheduler.request();
        }
        self.sync_deadline();
    }

    fn send_ui_event(&mut self, event: UiEvent, listener_mask: u16) {
        self.dispatch_driver_ui_route(&event, listener_mask);
    }

    /// Routes one neutral UI event over a stable ancestry snapshot.
    pub fn dispatch_ui(
        &mut self,
        target: UiNodeId,
        kind: UiEventKind,
        listener_mask: u16,
        timestamp: u64,
    ) {
        let modifiers = match &kind {
            UiEventKind::Input(crate::InputEvent::Key(key)) => key.modifiers,
            UiEventKind::Input(crate::InputEvent::ModifiersChanged(modifiers)) => *modifiers,
            _ => crate::input::Modifiers::empty(),
        };
        self.dispatch_ui_observed(target, kind, listener_mask, timestamp, None, modifiers);
    }

    pub(crate) fn dispatch_ui_observed(
        &mut self,
        target: UiNodeId,
        kind: UiEventKind,
        listener_mask: u16,
        timestamp: u64,
        geometry: Option<UiInputGeometry>,
        modifiers: crate::input::Modifiers,
    ) {
        if !self.ui.nodes.contains(target) {
            return;
        }
        let mut path = std::mem::take(&mut self.event_path);
        path.clear();
        let mut parent = self.ui.nodes.core(target).and_then(|core| core.parent);
        while let Some(node) = parent {
            path.push(node);
            parent = self.ui.nodes.core(node).and_then(|core| core.parent);
        }
        for index in (0..path.len()).rev() {
            let current_target = path[index];
            if self
                .ui
                .interactions
                .get(current_target)
                .is_some_and(|item| item.listener_mask & listener_mask != 0)
            {
                self.send_ui_event(
                    UiEvent {
                        target,
                        current_target,
                        kind: kind.clone(),
                        phase: EventPhase::Capture,
                        timestamp,
                        geometry,
                        modifiers,
                    },
                    listener_mask,
                );
            }
        }
        self.send_ui_event(
            UiEvent {
                target,
                current_target: target,
                kind: kind.clone(),
                phase: EventPhase::Target,
                timestamp,
                geometry,
                modifiers,
            },
            listener_mask,
        );
        for current_target in path.iter().copied() {
            if self
                .ui
                .interactions
                .get(current_target)
                .is_some_and(|item| item.listener_mask & listener_mask != 0)
            {
                self.send_ui_event(
                    UiEvent {
                        target,
                        current_target,
                        kind: kind.clone(),
                        phase: EventPhase::Bubble,
                        timestamp,
                        geometry,
                        modifiers,
                    },
                    listener_mask,
                );
            }
        }
        self.event_path = path;
        self.ui.diagnostics.events_dispatched += 1;
    }

    /// Resolves and delivers an existing typed action route directly to the application adapter.
    pub fn dispatch_action(&mut self, target: UiNodeId) -> bool {
        self.dispatch_activation(target, ChangeSource::Programmatic)
    }

    /// Resolves a completed activation while preserving the validated neutral input source.
    pub fn dispatch_activation(&mut self, target: UiNodeId, source: ChangeSource) -> bool {
        if !self.ui.nodes.contains(target) {
            self.driver.reject_stale_node_action(target);
            return false;
        }
        let mut frame_requested = false;
        let routed = {
            let mut context = DriverContext {
                ui: &mut self.ui,
                commands: &mut self.commands,
                frame_requested: &mut frame_requested,
            };
            self.driver
                .dispatch_node_activation(target, source, &mut context)
        };
        if frame_requested {
            self.scheduler.request();
        }
        self.sync_deadline();
        routed
    }

    /// Delivers one normalized continuous-value proposal to a mounted control route.
    pub fn dispatch_value(
        &mut self,
        target: UiNodeId,
        value: f32,
        phase: ValueChangePhase,
        source: ChangeSource,
    ) -> bool {
        if !self.ui.nodes.contains(target) {
            self.driver.reject_stale_node_action(target);
            return false;
        }
        let mut frame_requested = false;
        let routed = {
            let mut context = DriverContext {
                ui: &mut self.ui,
                commands: &mut self.commands,
                frame_requested: &mut frame_requested,
            };
            self.driver.dispatch_node_value(
                target,
                value.clamp(0.0, 1.0),
                phase,
                source,
                &mut context,
            )
        };
        if frame_requested {
            self.scheduler.request();
        }
        self.sync_deadline();
        routed
    }

    fn sync_deadline(&mut self) {
        self.scheduler
            .set_next_deadline(self.driver.next_deadline());
    }
}

impl<C: Component> ViewRuntime<ComponentRuntimeDriver<C>> {
    /// Mounts a normal root component into the renderer/platform-free view owner.
    pub fn from_component(component: C) -> RuntimeResult<Self> {
        let view = NEXT_VIEW_ID.fetch_add(1, Ordering::Relaxed).max(1);
        let mut runtime = Self::new(ComponentRuntimeDriver::new(component, view))?;
        match runtime.driver_mut().take_error() {
            Some(error) => Err(error),
            None => Ok(runtime),
        }
    }

    /// Mounts a component with an explicitly injected executor capability.
    pub fn from_component_with_task_host(
        component: C,
        task_host: impl TaskHost,
    ) -> RuntimeResult<Self> {
        let view = NEXT_VIEW_ID.fetch_add(1, Ordering::Relaxed).max(1);
        let mut runtime = Self::new(ComponentRuntimeDriver::new_with_task_host(
            component, view, task_host,
        ))?;
        match runtime.driver_mut().take_error() {
            Some(error) => Err(error),
            None => Ok(runtime),
        }
    }

    /// Mounts a component with an injected task host and a coalesced host-turn wake callback.
    pub fn from_component_with_task_host_and_wake(
        component: C,
        task_host: impl TaskHost,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> RuntimeResult<Self> {
        let view = NEXT_VIEW_ID.fetch_add(1, Ordering::Relaxed).max(1);
        let mut runtime = Self::new(ComponentRuntimeDriver::new_with_task_host_and_wake(
            component, view, task_host, wake,
        ))?;
        match runtime.driver_mut().take_error() {
            Some(error) => Err(error),
            None => Ok(runtime),
        }
    }

    /// Delivers one owned root action. Component actions need not implement `Clone` or `Send`.
    pub fn send_component_action(&mut self, action: C::Action) -> RuntimeResult<()> {
        self.dispatch_root_action(action);
        match self.driver_mut().take_error() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn root_component(&self) -> Option<ComponentId> {
        self.driver().root_component()
    }

    pub fn component_lifecycle(&self) -> Option<LifecycleState> {
        self.driver().lifecycle()
    }

    pub fn component_diagnostics(&self) -> ComponentDiagnostics {
        self.driver().diagnostics()
    }

    /// Sets the maximum originating-action plus observer-action transactions processed in one
    /// host turn. The default is 32.
    pub fn set_component_action_round_limit(&mut self, limit: u32) -> RuntimeResult<()> {
        self.driver_mut().set_action_round_limit(limit)
    }

    /// Sets the maximum completed task results consumed in one host turn. The default is 32.
    pub fn set_component_task_result_limit(&mut self, limit: usize) -> RuntimeResult<()> {
        self.driver_mut().set_task_result_limit(limit)
    }

    /// Sets the maximum due/cancelled/stale timer records consumed in one host turn.
    /// The default is 32.
    pub fn set_component_timer_result_limit(&mut self, limit: usize) -> RuntimeResult<()> {
        self.driver_mut().set_timer_result_limit(limit)
    }

    /// Processes task results and surfaces any component/runtime error from that turn.
    pub fn process_component_task_results(&mut self) -> RuntimeResult<usize> {
        let processed = self.process_task_results();
        match self.driver_mut().take_error() {
            Some(error) => Err(error),
            None => Ok(processed),
        }
    }

    /// Processes due component timers and surfaces any component/runtime error from that turn.
    pub fn process_component_timers(&mut self, now: MonotonicInstant) -> RuntimeResult<usize> {
        let processed = self.process_timers(now);
        match self.driver_mut().take_error() {
            Some(error) => Err(error),
            None => Ok(processed),
        }
    }

    pub fn unmount_component(&mut self) -> RuntimeResult<()> {
        let mut frame_requested = false;
        self.driver.close(&mut DriverContext {
            ui: &mut self.ui,
            commands: &mut self.commands,
            frame_requested: &mut frame_requested,
        });
        if frame_requested {
            self.scheduler.request();
        }
        self.sync_deadline();
        match self.driver_mut().take_error() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl ViewRuntime<CompositionDriver> {
    /// Creates one persistent rerender-and-reconcile component view.
    pub fn from_composed<C: crate::authoring::compose::Component>(component: C) -> RuntimeResult<Self> {
        let mut runtime = Self::new(CompositionDriver::new(component))?;
        match runtime.driver_mut().take_error() {
            Some(error) => Err(error),
            None => Ok(runtime),
        }
    }

    pub(crate) fn shell_input(&mut self, event: crate::input::InputEvent) -> RuntimeResult<()> {
        let mut requested = false;
        self.driver.shell_input(
            &mut DriverContext {
                ui: &mut self.ui,
                commands: &mut self.commands,
                frame_requested: &mut requested,
            },
            event,
        );
        if requested {
            self.scheduler.request();
        }
        match self.driver.take_error() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
    pub(crate) fn dismiss_shell_widget(
        &mut self,
        reason: crate::authoring::compose::ShellDismissReason,
    ) -> RuntimeResult<()> {
        let mut frame_requested = false;
        self.driver.dismiss_shell_widget(
            &mut DriverContext {
                ui: &mut self.ui,
                commands: &mut self.commands,
                frame_requested: &mut frame_requested,
            },
            reason,
        );
        if frame_requested {
            self.scheduler.request();
        }
        match self.driver.take_error() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
    pub fn composition_diagnostics(&self) -> CompositionDiagnostics {
        self.driver().diagnostics()
    }

    #[cfg(any(test, all(feature = "shell-wayland-linux", target_os = "linux")))]
    pub(crate) fn update_composition_root(
        &mut self,
        candidate: Box<dyn crate::authoring::compose::ErasedComponent>,
    ) -> RuntimeResult<bool> {
        let mut frame_requested = false;
        let changed = {
            let mut context = DriverContext {
                ui: &mut self.ui,
                commands: &mut self.commands,
                frame_requested: &mut frame_requested,
            };
            self.driver.update_root_candidate(&mut context, candidate)
        };
        if frame_requested {
            self.scheduler.request();
        }
        self.sync_deadline();
        match self.driver_mut().take_error() {
            Some(error) => Err(error),
            None => Ok(changed),
        }
    }

    pub fn unmount_composition(&mut self) -> RuntimeResult<()> {
        let mut frame_requested = false;
        self.driver.close(&mut DriverContext {
            ui: &mut self.ui,
            commands: &mut self.commands,
            frame_requested: &mut frame_requested,
        });
        if frame_requested {
            self.scheduler.request();
        }
        self.sync_deadline();
        match self.driver_mut().take_error() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
