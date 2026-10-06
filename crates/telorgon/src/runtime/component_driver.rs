use std::any::TypeId;
use std::collections::VecDeque;
use std::sync::Arc;

use crate::input::{ChangeSource, ValueChangePhase};
use crate::ui::{MountWriter, UiEvent, UiNodeId, UiRoot};

use crate::runtime::{
    Component, ComponentDiagnostics, ComponentId, LifecycleState, MonotonicInstant, RuntimeError,
    RuntimeResult,
    binding::BindingArena,
    component_arena::{ComponentArena, ComponentStores},
    context::DriverContext,
    input_route::InputRouteArena,
    observer::ObserverArena,
    read_arena::ReadArena,
    routed_action::{RoutedAction, RoutedOutput},
    scheduler::{TimerArena, TimerStart},
    state_arena::StateArena,
    structure::{StructureArena, StructureRuntime},
    task::{TaskArena, TaskStart},
    task_host::{TaskHost, UnsupportedTaskHost},
};

/// Internal component lifecycle contract consumed by [`crate::runtime::ViewRuntime`].
pub trait ComponentDriver {
    type Action: 'static;

    #[doc(hidden)]
    fn set_viewport_size(&mut self, _size: crate::SizeF) {}

    fn mount(&mut self, ui: &mut MountWriter<'_, Self::Action>) -> UiRoot;

    #[doc(hidden)]
    fn initialize(&mut self, _context: &mut DriverContext<'_>) {}

    #[doc(hidden)]
    fn dispatch_root_action(&mut self, _action: Self::Action, _context: &mut DriverContext<'_>) {}

    #[doc(hidden)]
    fn close(&mut self, _context: &mut DriverContext<'_>) {}

    #[doc(hidden)]
    fn dispatch_node_activation(
        &mut self,
        _target: UiNodeId,
        _source: ChangeSource,
        _context: &mut DriverContext<'_>,
    ) -> bool {
        false
    }

    #[doc(hidden)]
    fn dispatch_node_value(
        &mut self,
        _target: UiNodeId,
        _value: f32,
        _phase: ValueChangePhase,
        _source: ChangeSource,
        _context: &mut DriverContext<'_>,
    ) -> bool {
        false
    }

    #[doc(hidden)]
    fn dispatch_ui_route(
        &mut self,
        _event: &UiEvent,
        _listener_mask: u16,
        _context: &mut DriverContext<'_>,
    ) -> bool {
        false
    }

    #[doc(hidden)]
    fn reject_stale_node_action(&mut self, _target: UiNodeId) {}

    /// Reports externally owned reactive data waiting for a component reconciliation turn.
    #[doc(hidden)]
    fn external_updates_ready(&self) -> bool {
        false
    }

    /// Processes a bounded/coalesced set of external reactive invalidations.
    #[doc(hidden)]
    fn process_external_updates(&mut self, _context: &mut DriverContext<'_>) -> usize {
        0
    }

    #[doc(hidden)]
    fn task_results_ready(&self) -> bool {
        false
    }

    #[doc(hidden)]
    fn process_task_results(&mut self, _context: &mut DriverContext<'_>) -> usize {
        0
    }

    #[doc(hidden)]
    fn shutdown_task_host(&mut self) -> usize {
        0
    }

    #[doc(hidden)]
    fn next_deadline(&self) -> Option<MonotonicInstant> {
        None
    }

    #[doc(hidden)]
    fn timers_ready(&self, _now: MonotonicInstant) -> bool {
        false
    }

    #[doc(hidden)]
    fn process_timers(
        &mut self,
        _now: MonotonicInstant,
        _context: &mut DriverContext<'_>,
    ) -> usize {
        0
    }
}

/// Adapts a root [`Component`] into the retained `ViewRuntime` owner.
pub struct ComponentRuntimeDriver<C: Component> {
    component: Option<C>,
    components: ComponentArena,
    states: StateArena,
    reads: ReadArena,
    bindings: BindingArena,
    observers: ObserverArena,
    structures: StructureArena,
    input_routes: InputRouteArena,
    tasks: TaskArena,
    timers: TimerArena,
    task_host: Box<dyn TaskHost>,
    root: Option<ComponentId>,
    diagnostics: ComponentDiagnostics,
    last_error: Option<RuntimeError>,
    action_round_limit: u32,
    task_result_limit: usize,
    timer_result_limit: usize,
}

impl<C: Component> ComponentRuntimeDriver<C> {
    pub(crate) fn new(component: C, view: u64) -> Self {
        Self::new_with_task_host(component, view, UnsupportedTaskHost)
    }

    pub(crate) fn new_with_task_host(component: C, view: u64, task_host: impl TaskHost) -> Self {
        Self::new_with_task_host_and_wake(component, view, task_host, || {})
    }

    pub(crate) fn new_with_task_host_and_wake(
        component: C,
        view: u64,
        task_host: impl TaskHost,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let task_wake = {
            let wake = wake.clone();
            move || wake()
        };
        let timer_wake = move || wake();
        Self {
            component: Some(component),
            components: ComponentArena::new(view),
            states: StateArena::new(view),
            reads: ReadArena::new(view),
            bindings: BindingArena::default(),
            observers: ObserverArena::default(),
            structures: StructureArena::default(),
            input_routes: InputRouteArena::default(),
            tasks: TaskArena::new(view, task_wake),
            timers: TimerArena::new(timer_wake),
            task_host: Box::new(task_host),
            root: None,
            diagnostics: ComponentDiagnostics::default(),
            last_error: None,
            action_round_limit: 32,
            task_result_limit: 32,
            timer_result_limit: 32,
        }
    }

    pub fn root_component(&self) -> Option<ComponentId> {
        self.root
    }

    pub fn lifecycle(&self) -> Option<LifecycleState> {
        self.root
            .and_then(|component| self.components.lifecycle(component))
    }

    pub fn diagnostics(&self) -> ComponentDiagnostics {
        self.diagnostics
    }

    pub(crate) fn take_error(&mut self) -> Option<RuntimeError> {
        self.last_error.take()
    }

    pub(crate) fn set_action_round_limit(&mut self, limit: u32) -> RuntimeResult<()> {
        if limit == 0 {
            return Err(RuntimeError::new(
                "component action round limit must be greater than zero",
            ));
        }
        self.action_round_limit = limit;
        Ok(())
    }

    pub(crate) fn set_task_result_limit(&mut self, limit: usize) -> RuntimeResult<()> {
        if limit == 0 {
            return Err(RuntimeError::new(
                "component task result limit must be greater than zero",
            ));
        }
        self.task_result_limit = limit;
        Ok(())
    }

    pub(crate) fn set_timer_result_limit(&mut self, limit: usize) -> RuntimeResult<()> {
        if limit == 0 {
            return Err(RuntimeError::new(
                "component timer result limit must be greater than zero",
            ));
        }
        self.timer_result_limit = limit;
        Ok(())
    }

    fn sync_read_diagnostics(&mut self) {
        self.diagnostics.live_reads = self.reads.live();
        self.diagnostics.live_read_dependencies = self.reads.dependency_count();
        self.diagnostics.live_bindings = self.bindings.live();
        self.diagnostics.live_observers = self.observers.live();
        self.diagnostics.live_input_routes = self.input_routes.live();
        self.diagnostics.live_structural_containers = self.structures.live();
        self.diagnostics.live_tasks = self.tasks.live();
        self.diagnostics.live_timers = self.timers.live();
        self.diagnostics.reads_evaluated = self.reads.evaluated;
        self.diagnostics.unchanged_reads = self.reads.unchanged;
        self.diagnostics.read_cycles = self.reads.cycles;
    }

    fn reconcile_structures(&mut self, ui: &mut crate::ui::MountedUi) -> RuntimeResult<()> {
        let mut structures = std::mem::take(&mut self.structures);
        let result = structures.reconcile(&mut StructureRuntime {
            components: &mut self.components,
            ui,
            states: &mut self.states,
            reads: &mut self.reads,
            bindings: &mut self.bindings,
            observers: &mut self.observers,
            input_routes: &mut self.input_routes,
            tasks: &mut self.tasks,
            timers: &mut self.timers,
            diagnostics: &mut self.diagnostics,
        });
        self.structures = structures;
        result
    }

    fn handle_action(&mut self, action: C::Action, context: &mut DriverContext<'_>) {
        let Some(root) = self.root else {
            self.diagnostics.stale_actions += 1;
            self.last_error = Some(RuntimeError::new("root component action route is closed"));
            return;
        };
        self.handle_routed_actions(
            VecDeque::from([RoutedAction {
                target: root,
                type_id: TypeId::of::<C::Action>(),
                value: Box::new(action),
            }]),
            context,
        );
    }

    fn handle_outputs(
        &mut self,
        outputs: impl IntoIterator<Item = RoutedOutput>,
        context: &mut DriverContext<'_>,
    ) {
        let mut actions = VecDeque::new();
        for output in outputs {
            match output {
                RoutedOutput::Action(action) => actions.push_back(action),
                RoutedOutput::Command(command) => context.command(command),
            }
        }
        self.handle_routed_actions(actions, context);
    }

    fn handle_routed_actions(
        &mut self,
        mut actions: VecDeque<RoutedAction>,
        context: &mut DriverContext<'_>,
    ) {
        let mut rounds = 0_u32;
        while let Some(action) = actions.pop_front() {
            if self.components.lifecycle(action.target) != Some(LifecycleState::Mounted) {
                self.diagnostics.stale_actions += 1;
                continue;
            }
            if rounds >= self.action_round_limit {
                self.diagnostics.action_round_limit_hits += 1;
                self.last_error = Some(RuntimeError::new(
                    "component observer/action round limit reached",
                ));
                break;
            }
            rounds += 1;
            self.diagnostics.transactions += 1;
            match self.components.action_erased(
                action.target,
                action.type_id,
                action.value,
                &mut self.states,
                self.task_host.as_ref(),
                |transaction, states| self.structures.validate(transaction, &self.reads, states),
            ) {
                Ok((commit, task_starts, timer_starts)) => {
                    self.diagnostics.actions_handled += 1;
                    self.diagnostics.state_writes_staged += commit.staged;
                    self.diagnostics.state_writes_coalesced += commit.coalesced;
                    self.diagnostics.state_writes_committed += commit.committed;
                    self.diagnostics.equal_writes_suppressed += commit.equal_suppressed;
                    self.reads.invalidate_states(&commit.changed);
                    for task_start in task_starts {
                        match self
                            .tasks
                            .start(action.target, task_start, self.task_host.as_mut())
                        {
                            Ok(TaskStart::Started) => self.diagnostics.tasks_started += 1,
                            Ok(TaskStart::Cancelled) => self.diagnostics.tasks_cancelled += 1,
                            Err(error) => {
                                self.diagnostics.task_starts_failed += 1;
                                self.last_error = Some(error);
                            }
                        }
                    }
                    for timer_start in timer_starts {
                        match self.timers.start(action.target, timer_start) {
                            TimerStart::Started => self.diagnostics.timers_started += 1,
                            TimerStart::Cancelled => self.diagnostics.timers_cancelled += 1,
                        }
                    }
                    if let Err(error) = self.reconcile_structures(context.ui) {
                        self.last_error = Some(error);
                    }
                    match self
                        .bindings
                        .apply_current(&mut self.reads, &self.states, context.ui)
                    {
                        Ok(patched) => {
                            self.diagnostics.bindings_patched += patched as u64;
                            if patched > 0 {
                                *context.frame_requested = true;
                            }
                        }
                        Err(error) => self.last_error = Some(error),
                    }
                    match self.observers.collect(&mut self.reads, &self.states, true) {
                        Ok(emitted) => {
                            self.diagnostics.observer_actions += emitted.len() as u64;
                            for output in emitted {
                                match output {
                                    RoutedOutput::Action(action) => actions.push_back(action),
                                    RoutedOutput::Command(command) => context.command(command),
                                }
                            }
                        }
                        Err(error) => self.last_error = Some(error),
                    }
                }
                Err(error) => {
                    self.diagnostics.rejected_transactions += 1;
                    self.last_error = Some(error);
                }
            }
        }
        self.sync_read_diagnostics();
    }

    fn process_ready_tasks(&mut self, context: &mut DriverContext<'_>) -> usize {
        self.tasks.begin_turn();
        let cancelled = self.tasks.cancel_requested();
        let drain = self.tasks.drain(self.task_result_limit);
        self.diagnostics.tasks_cancelled += cancelled as u64;
        self.diagnostics.tasks_completed += drain.completed as u64;
        self.diagnostics.task_results_delivered += drain.delivered as u64;
        self.diagnostics.stale_task_results += drain.stale as u64;
        self.diagnostics.task_queue_high_water = self
            .diagnostics
            .task_queue_high_water
            .max(drain.queue_depth);
        let processed = cancelled + drain.delivered + drain.stale;
        self.handle_routed_actions(drain.actions, context);
        self.tasks.finish_turn();
        processed
    }

    fn shutdown_tasks(&mut self) -> usize {
        let cancelled = self.tasks.cancel_all();
        self.task_host = Box::new(UnsupportedTaskHost);
        self.diagnostics.tasks_cancelled += cancelled as u64;
        self.diagnostics.task_host_shutdowns += 1;
        self.sync_read_diagnostics();
        cancelled
    }

    fn process_ready_timers(
        &mut self,
        now: MonotonicInstant,
        context: &mut DriverContext<'_>,
    ) -> usize {
        self.timers.begin_turn();
        let drain = self.timers.drain_due(now, self.timer_result_limit);
        self.diagnostics.timers_fired += drain.fired as u64;
        self.diagnostics.timer_actions_delivered += drain.actions.len() as u64;
        self.diagnostics.timers_cancelled += drain.cancelled as u64;
        self.diagnostics.stale_timer_deadlines += drain.stale as u64;
        self.diagnostics.missed_timer_intervals += drain.missed_intervals;
        self.diagnostics.timer_queue_high_water = self
            .diagnostics
            .timer_queue_high_water
            .max(drain.queue_depth);
        let processed = drain.fired + drain.cancelled + drain.stale;
        self.handle_routed_actions(drain.actions, context);
        self.timers.finish_turn(now);
        processed
    }

    fn unmount(&mut self, context: &mut DriverContext<'_>) -> RuntimeResult<()> {
        let Some(root) = self.root.take() else {
            return Ok(());
        };
        let mut stores = ComponentStores {
            states: &mut self.states,
            reads: &mut self.reads,
            bindings: &mut self.bindings,
            observers: &mut self.observers,
            structures: &mut self.structures,
            input_routes: &mut self.input_routes,
            tasks: &mut self.tasks,
            timers: &mut self.timers,
            diagnostics: &mut self.diagnostics,
        };
        self.components.unmount(root, context.ui, &mut stores)?;
        self.sync_read_diagnostics();
        *context.frame_requested = true;
        Ok(())
    }
}

impl<C: Component> ComponentDriver for ComponentRuntimeDriver<C> {
    type Action = C::Action;

    fn mount(&mut self, ui: &mut MountWriter<'_, Self::Action>) -> UiRoot {
        let component = self
            .component
            .take()
            .expect("a component adapter mounts its root exactly once");
        let mut stores = ComponentStores {
            states: &mut self.states,
            reads: &mut self.reads,
            bindings: &mut self.bindings,
            observers: &mut self.observers,
            structures: &mut self.structures,
            input_routes: &mut self.input_routes,
            tasks: &mut self.tasks,
            timers: &mut self.timers,
            diagnostics: &mut self.diagnostics,
        };
        let (id, root) = self.components.mount_root(component, ui, &mut stores);
        self.root = Some(id);
        self.sync_read_diagnostics();
        root
    }

    fn initialize(&mut self, context: &mut DriverContext<'_>) {
        if let Err(error) = self.reconcile_structures(context.ui) {
            self.last_error = Some(error);
        }
        match self
            .bindings
            .apply_current(&mut self.reads, &self.states, context.ui)
        {
            Ok(patched) => {
                self.diagnostics.bindings_patched += patched as u64;
                if patched > 0 {
                    *context.frame_requested = true;
                }
                if let Err(error) = self.observers.collect(&mut self.reads, &self.states, false) {
                    self.last_error = Some(error);
                }
                self.sync_read_diagnostics();
            }
            Err(error) => self.last_error = Some(error),
        }
    }

    fn dispatch_root_action(&mut self, action: Self::Action, context: &mut DriverContext<'_>) {
        self.handle_action(action, context);
    }

    fn close(&mut self, context: &mut DriverContext<'_>) {
        if let Err(error) = self.unmount(context) {
            self.last_error = Some(error);
        }
    }

    fn dispatch_node_activation(
        &mut self,
        target: UiNodeId,
        source: ChangeSource,
        context: &mut DriverContext<'_>,
    ) -> bool {
        let (matched, output) =
            self.input_routes
                .activate(target, source, &mut self.reads, &self.states);
        match output {
            Ok(Some(output)) => self.handle_outputs([output], context),
            Ok(None) => {}
            Err(error) => self.last_error = Some(error),
        }
        self.sync_read_diagnostics();
        matched
    }

    fn dispatch_node_value(
        &mut self,
        target: UiNodeId,
        value: f32,
        phase: ValueChangePhase,
        source: ChangeSource,
        context: &mut DriverContext<'_>,
    ) -> bool {
        let (matched, output) = self.input_routes.change_value(target, value, phase, source);
        if let Some(output) = output {
            self.handle_outputs([output], context);
        }
        self.sync_read_diagnostics();
        matched
    }

    fn dispatch_ui_route(
        &mut self,
        event: &UiEvent,
        listener_mask: u16,
        context: &mut DriverContext<'_>,
    ) -> bool {
        let (matched, outputs) = self.input_routes.dispatch(event, listener_mask);
        self.handle_outputs(outputs, context);
        matched
    }

    fn reject_stale_node_action(&mut self, _target: UiNodeId) {
        self.diagnostics.stale_actions += 1;
    }

    fn task_results_ready(&self) -> bool {
        self.tasks.is_ready()
    }

    fn process_task_results(&mut self, context: &mut DriverContext<'_>) -> usize {
        self.process_ready_tasks(context)
    }

    fn shutdown_task_host(&mut self) -> usize {
        self.shutdown_tasks()
    }

    fn next_deadline(&self) -> Option<MonotonicInstant> {
        self.timers.next_deadline()
    }

    fn timers_ready(&self, now: MonotonicInstant) -> bool {
        self.timers.is_ready(now)
    }

    fn process_timers(&mut self, now: MonotonicInstant, context: &mut DriverContext<'_>) -> usize {
        self.process_ready_timers(now, context)
    }
}

impl<C: Component> Drop for ComponentRuntimeDriver<C> {
    fn drop(&mut self) {
        if let Some(root) = self.root.take() {
            let mut stores = ComponentStores {
                states: &mut self.states,
                reads: &mut self.reads,
                bindings: &mut self.bindings,
                observers: &mut self.observers,
                structures: &mut self.structures,
                input_routes: &mut self.input_routes,
                tasks: &mut self.tasks,
                timers: &mut self.timers,
                diagnostics: &mut self.diagnostics,
            };
            let _ = self.components.unmount_for_drop(root, &mut stores);
        }
    }
}
