use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::input::{ChangeSource, ValueChangePhase};
use crate::ui::{EventPhase, MountWriter, MountedUi, UiEvent, UiEventKind, UiNodeId};

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
        let _span = crate::profiler::span!("component.evaluate");
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
        let _span = crate::profiler::span!("tasks.process");
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
    pub fn from_composed<C: crate::compose::Component>(component: C) -> RuntimeResult<Self> {
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
        reason: crate::compose::ShellDismissReason,
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
        candidate: Box<dyn crate::compose::ErasedComponent>,
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
mod tests {
    use super::*;
    use crate::ui::{BoxStyle, ControlHandle, LayoutStyle, UiRoot};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    const LISTEN_TEST: u16 = 1;

    struct ComposedRootInput {
        label: String,
        mounts: Rc<Cell<usize>>,
        unmounts: Rc<Cell<usize>>,
    }

    impl crate::compose::ComponentFields for ComposedRootInput {
        type InputSnapshot = (String,);

        fn update_inputs(&mut self, incoming: Self) -> bool {
            let changed = self.label != incoming.label;
            self.label = incoming.label;
            changed
        }

        fn capture_inputs(&self) -> Self::InputSnapshot {
            (self.label.clone(),)
        }

        fn restore_inputs(&mut self, snapshot: Self::InputSnapshot) -> bool {
            let changed = self.label != snapshot.0;
            self.label = snapshot.0;
            changed
        }
    }

    impl crate::compose::Component for ComposedRootInput {
        fn view(&self) -> impl crate::compose::View {
            crate::compose::text(self.label.clone())
        }

        fn mounted(&mut self, _cx: &mut crate::compose::MountContext<Self>) {
            self.mounts.set(self.mounts.get() + 1);
        }

        fn unmounted(&mut self, _cx: &mut crate::compose::UnmountContext<Self>) {
            self.unmounts.set(self.unmounts.get() + 1);
        }
    }

    struct AlternateComposedRoot {
        mounts: Rc<Cell<usize>>,
    }

    impl crate::compose::ComponentFields for AlternateComposedRoot {
        type InputSnapshot = ();

        fn update_inputs(&mut self, _incoming: Self) -> bool {
            false
        }

        fn capture_inputs(&self) -> Self::InputSnapshot {}

        fn restore_inputs(&mut self, _snapshot: Self::InputSnapshot) -> bool {
            false
        }
    }

    impl crate::compose::Component for AlternateComposedRoot {
        fn view(&self) -> impl crate::compose::View {
            crate::compose::text("alternate")
        }

        fn mounted(&mut self, _cx: &mut crate::compose::MountContext<Self>) {
            self.mounts.set(self.mounts.get() + 1);
        }
    }

    #[test]
    fn composed_root_inputs_reconcile_without_remounting() {
        let mounts = Rc::new(Cell::new(0));
        let unmounts = Rc::new(Cell::new(0));
        let mut runtime = ViewRuntime::from_composed(ComposedRootInput {
            label: "inactive".to_owned(),
            mounts: Rc::clone(&mounts),
            unmounts: Rc::clone(&unmounts),
        })
        .unwrap();
        assert_eq!(mounts.get(), 1);
        let mounted = runtime.composition_diagnostics().components_mounted;

        assert!(
            runtime
                .update_composition_root(Box::new(ComposedRootInput {
                    label: "active".to_owned(),
                    mounts: Rc::clone(&mounts),
                    unmounts: Rc::clone(&unmounts),
                }))
                .unwrap()
        );
        assert_eq!(mounts.get(), 1);
        assert_eq!(
            runtime.composition_diagnostics().components_mounted,
            mounted
        );
        assert!(runtime.scheduler().needs_frame());

        assert!(
            !runtime
                .update_composition_root(Box::new(ComposedRootInput {
                    label: "active".to_owned(),
                    mounts: Rc::clone(&mounts),
                    unmounts: Rc::clone(&unmounts),
                }))
                .unwrap()
        );

        let alternate_mounts = Rc::new(Cell::new(0));
        assert!(
            runtime
                .update_composition_root(Box::new(AlternateComposedRoot {
                    mounts: Rc::clone(&alternate_mounts),
                }))
                .unwrap()
        );
        assert_eq!(mounts.get(), 1);
        assert_eq!(unmounts.get(), 1);
        assert_eq!(alternate_mounts.get(), 1);
        assert_eq!(
            runtime.composition_diagnostics().components_mounted,
            mounted + 1
        );
        assert_eq!(runtime.composition_diagnostics().components_unmounted, 1);
    }

    struct Counter {
        creates: Rc<Cell<usize>>,
        mounts: Rc<Cell<usize>>,
        unmounts: Rc<Cell<usize>>,
        state: Rc<RefCell<Option<crate::runtime::State<f32>>>>,
        control: Rc<RefCell<Option<ControlHandle>>>,
    }

    struct CounterState {
        opacity: crate::runtime::State<f32>,
        display: crate::runtime::Read<f32>,
    }

    enum CounterAction {
        Set(Box<f32>),
        Foreign(crate::runtime::State<f32>),
    }

    impl Component for Counter {
        type State = CounterState;
        type Action = CounterAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            self.creates.set(self.creates.get() + 1);
            let opacity = context.state(0.4);
            let display = context.map(opacity.read(), |value| *value).unwrap();
            self.state.replace(Some(opacity));
            CounterState { opacity, display }
        }

        fn mount(
            &self,
            state: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            self.mounts.set(self.mounts.get() + 1);
            let (root, control) = {
                let writer = ui.foundation();
                let mut control = None;
                let root = writer.root(BoxStyle::default(), LayoutStyle::default(), |writer| {
                    control = Some(writer.layer(
                        true,
                        BoxStyle::default(),
                        LayoutStyle::default(),
                        |_| {},
                    ));
                });
                (root, control.unwrap())
            };
            ui.bind_read(state.display, control.opacity).unwrap();
            self.control.replace(Some(control));
            root
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            match action {
                CounterAction::Set(value) => context.transaction(|context| {
                    context.set(state.opacity, 0.2).unwrap();
                    assert_eq!(context.get(state.opacity).unwrap(), 0.2);
                    context.set(state.opacity, *value).unwrap();
                }),
                CounterAction::Foreign(foreign) => {
                    context.set(state.opacity, 0.9).unwrap();
                    let _ = context.set(foreign, 0.1);
                }
            }
        }

        fn unmount(
            &self,
            _state: &mut Self::State,
            _context: &mut crate::runtime::UnmountContext<'_>,
        ) {
            self.unmounts.set(self.unmounts.get() + 1);
        }
    }

    struct CounterFixture {
        runtime: ViewRuntime<ComponentRuntimeDriver<Counter>>,
        creates: Rc<Cell<usize>>,
        mounts: Rc<Cell<usize>>,
        unmounts: Rc<Cell<usize>>,
        state: Rc<RefCell<Option<crate::runtime::State<f32>>>>,
        control: Rc<RefCell<Option<ControlHandle>>>,
    }

    fn counter_fixture() -> CounterFixture {
        let creates = Rc::new(Cell::new(0));
        let mounts = Rc::new(Cell::new(0));
        let unmounts = Rc::new(Cell::new(0));
        let state = Rc::new(RefCell::new(None));
        let control = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(Counter {
            creates: creates.clone(),
            mounts: mounts.clone(),
            unmounts: unmounts.clone(),
            state: state.clone(),
            control: control.clone(),
        })
        .unwrap();
        CounterFixture {
            runtime,
            creates,
            mounts,
            unmounts,
            state,
            control,
        }
    }

    fn bound_opacity(fixture: &CounterFixture) -> f32 {
        let node = fixture.control.borrow().unwrap().node;
        fixture.runtime.ui().box_styles.get(node).unwrap().opacity
    }

    #[test]
    fn component_mounts_once_and_direct_binding_tracks_atomic_state_transactions() {
        let mut fixture = counter_fixture();
        assert_eq!(fixture.creates.get(), 1);
        assert_eq!(fixture.mounts.get(), 1);
        assert_eq!(bound_opacity(&fixture), 0.4);

        fixture
            .runtime
            .send_component_action(CounterAction::Set(Box::new(0.75)))
            .unwrap();
        assert_eq!(bound_opacity(&fixture), 0.75);
        assert_eq!(fixture.creates.get(), 1);
        assert_eq!(fixture.mounts.get(), 1);
        let diagnostics = fixture.runtime.component_diagnostics();
        assert_eq!(diagnostics.state_writes_staged, 2);
        assert_eq!(diagnostics.state_writes_coalesced, 1);
        assert_eq!(diagnostics.state_writes_committed, 1);
        assert_eq!(diagnostics.bindings_patched, 2);
        assert_eq!(diagnostics.reads_evaluated, 2);

        fixture
            .runtime
            .send_component_action(CounterAction::Set(Box::new(0.75)))
            .unwrap();
        let diagnostics = fixture.runtime.component_diagnostics();
        assert_eq!(diagnostics.equal_writes_suppressed, 1);
        assert_eq!(diagnostics.bindings_patched, 2);
    }

    #[test]
    fn cross_view_write_rejects_the_whole_transaction_and_unmount_closes_generation() {
        let mut first = counter_fixture();
        let second = counter_fixture();
        let foreign = second.state.borrow().unwrap();
        assert!(
            first
                .runtime
                .send_component_action(CounterAction::Foreign(foreign))
                .is_err()
        );
        assert_eq!(bound_opacity(&first), 0.4);
        assert_eq!(
            first.runtime.component_diagnostics().rejected_transactions,
            1
        );

        first.runtime.unmount_component().unwrap();
        assert_eq!(first.unmounts.get(), 1);
        assert!(first.runtime.ui().root().is_none());
        assert!(
            first
                .runtime
                .send_component_action(CounterAction::Set(Box::new(1.0)))
                .is_err()
        );
        assert_eq!(first.runtime.component_diagnostics().stale_actions, 1);
    }

    #[test]
    fn dropping_a_component_view_runs_the_unmount_hook_once() {
        let fixture = counter_fixture();
        let unmounts = fixture.unmounts.clone();
        drop(fixture);
        assert_eq!(unmounts.get(), 1);
    }

    struct ObserverComponent {
        finished: Rc<Cell<usize>>,
    }

    struct ObserverState {
        value: crate::runtime::State<u32>,
    }

    enum ObserverAction {
        Kick,
        Advance,
        Finish,
    }

    impl Component for ObserverComponent {
        type State = ObserverState;
        type Action = ObserverAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            ObserverState {
                value: context.state(0),
            }
        }

        fn mount(
            &self,
            state: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            ui.observe(state.value.read(), |value| {
                if *value < 3 {
                    ObserverAction::Advance
                } else {
                    ObserverAction::Finish
                }
            })
            .unwrap();
            ui.foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            match action {
                ObserverAction::Kick => context.set(state.value, 1).unwrap(),
                ObserverAction::Advance => {
                    let next = context.get(state.value).unwrap() + 1;
                    context.set(state.value, next).unwrap();
                }
                ObserverAction::Finish => self.finished.set(self.finished.get() + 1),
            }
        }
    }

    #[test]
    fn observers_emit_later_transactions_in_bounded_rounds() {
        let finished = Rc::new(Cell::new(0));
        let mut runtime = ViewRuntime::from_component(ObserverComponent {
            finished: finished.clone(),
        })
        .unwrap();
        runtime.send_component_action(ObserverAction::Kick).unwrap();
        assert_eq!(finished.get(), 1);
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.transactions, 4);
        assert_eq!(diagnostics.observer_actions, 3);
        assert_eq!(diagnostics.action_round_limit_hits, 0);
    }

    struct LoopComponent;

    struct LoopState {
        value: crate::runtime::State<u32>,
    }

    enum LoopAction {
        Kick,
        Again,
    }

    impl Component for LoopComponent {
        type State = LoopState;
        type Action = LoopAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            LoopState {
                value: context.state(0),
            }
        }

        fn mount(
            &self,
            state: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            ui.observe(state.value.read(), |_| LoopAction::Again)
                .unwrap();
            ui.foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        }

        fn action(
            &self,
            state: &mut Self::State,
            _action: Self::Action,
            context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            let next = context.get(state.value).unwrap() + 1;
            context.set(state.value, next).unwrap();
        }
    }

    #[test]
    fn observer_loops_stop_at_the_configured_action_round_limit() {
        let mut runtime = ViewRuntime::from_component(LoopComponent).unwrap();
        runtime.set_component_action_round_limit(3).unwrap();
        let error = runtime.send_component_action(LoopAction::Kick).unwrap_err();
        assert!(error.to_string().contains("round limit"));
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.transactions, 3);
        assert_eq!(diagnostics.action_round_limit_hits, 1);
    }

    struct ConditionalParent {
        child_creates: Rc<Cell<usize>>,
        child_mounts: Rc<Cell<usize>>,
        unmount_order: Rc<RefCell<Vec<&'static str>>>,
    }

    struct ConditionalState {
        visible: crate::runtime::State<bool>,
    }

    enum ConditionalAction {
        SetVisible(bool),
    }

    struct ConditionalChild {
        creates: Rc<Cell<usize>>,
        mounts: Rc<Cell<usize>>,
        unmount_order: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Component for ConditionalChild {
        type State = ();
        type Action = ConditionalAction;

        fn create(&self, _context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            self.creates.set(self.creates.get() + 1);
        }

        fn mount(&self, _state: &(), ui: &mut crate::runtime::Ui<'_, '_, Self::Action>) -> UiRoot {
            self.mounts.set(self.mounts.get() + 1);
            ui.foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        }

        fn action(
            &self,
            _state: &mut (),
            _action: Self::Action,
            _context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
        }

        fn unmount(&self, _state: &mut (), _context: &mut crate::runtime::UnmountContext<'_>) {
            self.unmount_order.borrow_mut().push("child");
        }
    }

    impl Component for ConditionalParent {
        type State = ConditionalState;
        type Action = ConditionalAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            ConditionalState {
                visible: context.state(true),
            }
        }

        fn mount(
            &self,
            state: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            let mut host = None;
            let root =
                ui.foundation()
                    .root(BoxStyle::default(), LayoutStyle::default(), |writer| {
                        host = Some(writer.container(
                            BoxStyle::default(),
                            LayoutStyle::default(),
                            |_| {},
                        ));
                    });
            let creates = self.child_creates.clone();
            let mounts = self.child_mounts.clone();
            let unmount_order = self.unmount_order.clone();
            ui.when(state.visible.read(), host.unwrap(), move || {
                ConditionalChild {
                    creates: creates.clone(),
                    mounts: mounts.clone(),
                    unmount_order: unmount_order.clone(),
                }
            })
            .unwrap();
            root
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            let ConditionalAction::SetVisible(visible) = action;
            context.set(state.visible, visible).unwrap();
        }

        fn unmount(
            &self,
            _state: &mut Self::State,
            _context: &mut crate::runtime::UnmountContext<'_>,
        ) {
            self.unmount_order.borrow_mut().push("parent");
        }
    }

    #[test]
    fn when_mounts_fresh_children_and_tears_down_child_first() {
        let creates = Rc::new(Cell::new(0));
        let mounts = Rc::new(Cell::new(0));
        let unmount_order = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(ConditionalParent {
            child_creates: creates.clone(),
            child_mounts: mounts.clone(),
            unmount_order: unmount_order.clone(),
        })
        .unwrap();

        assert_eq!(creates.get(), 1);
        assert_eq!(mounts.get(), 1);
        assert_eq!(runtime.ui().nodes.alive().len(), 3);

        runtime
            .send_component_action(ConditionalAction::SetVisible(false))
            .unwrap();
        assert_eq!(runtime.ui().nodes.alive().len(), 2);
        assert_eq!(&*unmount_order.borrow(), &["child"]);

        runtime
            .send_component_action(ConditionalAction::SetVisible(true))
            .unwrap();
        assert_eq!(creates.get(), 2);
        assert_eq!(mounts.get(), 2);
        assert_eq!(runtime.ui().nodes.alive().len(), 3);

        runtime.unmount_component().unwrap();
        assert_eq!(&*unmount_order.borrow(), &["child", "child", "parent"]);
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.structural_inserted, 2);
        assert_eq!(diagnostics.structural_removed, 1);
        assert_eq!(diagnostics.live_components, 0);
        assert_eq!(diagnostics.live_structural_containers, 0);
    }

    #[derive(Clone, Debug, PartialEq)]
    struct KeyedItem {
        key: u32,
        opacity: f32,
    }

    struct KeyedParent {
        host: Rc<Cell<Option<crate::ui::UiNodeId>>>,
        child_creates: Rc<Cell<usize>>,
        child_unmounts: Rc<Cell<usize>>,
        controls: Rc<RefCell<Vec<ControlHandle>>>,
    }

    struct KeyedState {
        items: crate::runtime::State<Vec<KeyedItem>>,
    }

    enum KeyedAction {
        Set(Vec<KeyedItem>),
    }

    struct KeyedChild {
        item: crate::runtime::Read<KeyedItem>,
        creates: Rc<Cell<usize>>,
        unmounts: Rc<Cell<usize>>,
        controls: Rc<RefCell<Vec<ControlHandle>>>,
    }

    impl Component for KeyedChild {
        type State = crate::runtime::Read<f32>;
        type Action = KeyedAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            self.creates.set(self.creates.get() + 1);
            context
                .map(self.item, |item| item.opacity)
                .expect("a child may derive from its explicit parent input read")
        }

        fn mount(
            &self,
            opacity: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            let mut control = None;
            let root =
                ui.foundation()
                    .root(BoxStyle::default(), LayoutStyle::default(), |writer| {
                        control = Some(writer.layer(
                            true,
                            BoxStyle::default(),
                            LayoutStyle::default(),
                            |_| {},
                        ));
                    });
            let control = control.unwrap();
            ui.bind_read(*opacity, control.opacity).unwrap();
            self.controls.borrow_mut().push(control);
            root
        }

        fn action(
            &self,
            _state: &mut Self::State,
            _action: Self::Action,
            _context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
        }

        fn unmount(
            &self,
            _state: &mut Self::State,
            _context: &mut crate::runtime::UnmountContext<'_>,
        ) {
            self.unmounts.set(self.unmounts.get() + 1);
        }
    }

    impl Component for KeyedParent {
        type State = KeyedState;
        type Action = KeyedAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            KeyedState {
                items: context.state(vec![
                    KeyedItem {
                        key: 1,
                        opacity: 0.1,
                    },
                    KeyedItem {
                        key: 2,
                        opacity: 0.2,
                    },
                ]),
            }
        }

        fn mount(
            &self,
            state: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            let mut host = None;
            let root =
                ui.foundation()
                    .root(BoxStyle::default(), LayoutStyle::default(), |writer| {
                        host = Some(writer.container(
                            BoxStyle::default(),
                            LayoutStyle::default(),
                            |_| {},
                        ));
                    });
            let host = host.unwrap();
            self.host.set(Some(host));
            let creates = self.child_creates.clone();
            let unmounts = self.child_unmounts.clone();
            let controls = self.controls.clone();
            ui.for_each_keyed(
                state.items.read(),
                host,
                |item| item.key,
                move |item| KeyedChild {
                    item,
                    creates: creates.clone(),
                    unmounts: unmounts.clone(),
                    controls: controls.clone(),
                },
            )
            .unwrap();
            root
        }

        fn action(
            &self,
            state: &mut Self::State,
            KeyedAction::Set(items): Self::Action,
            context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            context.set(state.items, items).unwrap();
        }
    }

    #[test]
    fn keyed_children_preserve_identity_reorder_and_update_item_reads() {
        let host = Rc::new(Cell::new(None));
        let creates = Rc::new(Cell::new(0));
        let unmounts = Rc::new(Cell::new(0));
        let controls = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(KeyedParent {
            host: host.clone(),
            child_creates: creates.clone(),
            child_unmounts: unmounts.clone(),
            controls: controls.clone(),
        })
        .unwrap();
        let host_node = host.get().unwrap();
        let initial_roots = runtime.ui().nodes.children(host_node).collect::<Vec<_>>();
        assert_eq!(initial_roots.len(), 2);
        assert_eq!(creates.get(), 2);

        runtime
            .send_component_action(KeyedAction::Set(vec![
                KeyedItem {
                    key: 2,
                    opacity: 0.7,
                },
                KeyedItem {
                    key: 1,
                    opacity: 0.1,
                },
                KeyedItem {
                    key: 3,
                    opacity: 0.3,
                },
            ]))
            .unwrap();
        let reordered = runtime.ui().nodes.children(host_node).collect::<Vec<_>>();
        assert_eq!(reordered[0], initial_roots[1]);
        assert_eq!(reordered[1], initial_roots[0]);
        assert_eq!(creates.get(), 3);
        let second_control = controls.borrow()[1];
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(second_control.node)
                .unwrap()
                .opacity,
            0.7
        );

        let before_duplicate = reordered;
        assert!(
            runtime
                .send_component_action(KeyedAction::Set(vec![
                    KeyedItem {
                        key: 3,
                        opacity: 0.3,
                    },
                    KeyedItem {
                        key: 3,
                        opacity: 0.4,
                    },
                ]))
                .is_err()
        );
        assert_eq!(
            runtime.ui().nodes.children(host_node).collect::<Vec<_>>(),
            before_duplicate
        );
        assert_eq!(creates.get(), 3);
        assert_eq!(unmounts.get(), 0);
        let after_duplicate = runtime.component_diagnostics();
        assert_eq!(after_duplicate.rejected_transactions, 1);
        assert_eq!(after_duplicate.state_writes_committed, 1);

        let third_root = before_duplicate[2];
        runtime
            .send_component_action(KeyedAction::Set(vec![KeyedItem {
                key: 3,
                opacity: 0.9,
            }]))
            .unwrap();
        assert_eq!(
            runtime.ui().nodes.children(host_node).collect::<Vec<_>>(),
            vec![third_root]
        );
        assert_eq!(unmounts.get(), 2);
        let third_control = controls.borrow()[2];
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(third_control.node)
                .unwrap()
                .opacity,
            0.9
        );

        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.structural_inserted, 3);
        assert_eq!(diagnostics.structural_removed, 2);
        assert_eq!(diagnostics.structural_reused, 3);
        assert_eq!(diagnostics.structural_moved, 3);
        assert_eq!(diagnostics.state_writes_committed, 2);
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    enum BranchKey {
        First,
        Second,
        Missing,
    }

    struct SwitchParent {
        host: Rc<Cell<Option<crate::ui::UiNodeId>>>,
        log: Rc<RefCell<Vec<&'static str>>>,
    }

    struct SwitchState {
        selected: crate::runtime::State<BranchKey>,
    }

    enum SwitchAction {
        Select(BranchKey),
    }

    struct FirstBranch(Rc<RefCell<Vec<&'static str>>>);
    struct SecondBranch(Rc<RefCell<Vec<&'static str>>>);

    macro_rules! branch_component {
        ($ty:ty, $create:literal, $unmount:literal) => {
            impl Component for $ty {
                type State = ();
                type Action = SwitchAction;

                fn create(&self, _context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
                    self.0.borrow_mut().push($create);
                }

                fn mount(
                    &self,
                    _state: &Self::State,
                    ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
                ) -> UiRoot {
                    ui.foundation()
                        .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
                }

                fn action(
                    &self,
                    _state: &mut Self::State,
                    _action: Self::Action,
                    _context: &mut crate::runtime::UpdateContext<'_, Self>,
                ) {
                }

                fn unmount(
                    &self,
                    _state: &mut Self::State,
                    _context: &mut crate::runtime::UnmountContext<'_>,
                ) {
                    self.0.borrow_mut().push($unmount);
                }
            }
        };
    }

    branch_component!(FirstBranch, "create-first", "unmount-first");
    branch_component!(SecondBranch, "create-second", "unmount-second");

    impl Component for SwitchParent {
        type State = SwitchState;
        type Action = SwitchAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            SwitchState {
                selected: context.state(BranchKey::First),
            }
        }

        fn mount(
            &self,
            state: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            let mut host = None;
            let root =
                ui.foundation()
                    .root(BoxStyle::default(), LayoutStyle::default(), |writer| {
                        host = Some(writer.container(
                            BoxStyle::default(),
                            LayoutStyle::default(),
                            |_| {},
                        ));
                    });
            let host = host.unwrap();
            self.host.set(Some(host));
            let first_log = self.log.clone();
            let second_log = self.log.clone();
            ui.switch(
                state.selected.read(),
                host,
                vec![
                    crate::runtime::SwitchBranch::new(BranchKey::First, move || {
                        FirstBranch(first_log.clone())
                    }),
                    crate::runtime::SwitchBranch::new(BranchKey::Second, move || {
                        SecondBranch(second_log.clone())
                    }),
                ],
            )
            .unwrap();
            root
        }

        fn action(
            &self,
            state: &mut Self::State,
            SwitchAction::Select(selected): Self::Action,
            context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            context.set(state.selected, selected).unwrap();
        }
    }

    #[test]
    fn keyed_switch_replaces_component_type_and_rejects_missing_branches_precommit() {
        let host = Rc::new(Cell::new(None));
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(SwitchParent {
            host: host.clone(),
            log: log.clone(),
        })
        .unwrap();
        let host = host.get().unwrap();
        let first_root = runtime.ui().nodes.children(host).next().unwrap();
        assert_eq!(&*log.borrow(), &["create-first"]);

        runtime
            .send_component_action(SwitchAction::Select(BranchKey::First))
            .unwrap();
        assert_eq!(runtime.ui().nodes.children(host).next(), Some(first_root));

        runtime
            .send_component_action(SwitchAction::Select(BranchKey::Second))
            .unwrap();
        let second_root = runtime.ui().nodes.children(host).next().unwrap();
        assert_ne!(second_root, first_root);
        assert_eq!(
            &*log.borrow(),
            &["create-first", "unmount-first", "create-second"]
        );

        assert!(
            runtime
                .send_component_action(SwitchAction::Select(BranchKey::Missing))
                .is_err()
        );
        assert_eq!(runtime.ui().nodes.children(host).next(), Some(second_root));
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.rejected_transactions, 1);
        assert_eq!(diagnostics.state_writes_committed, 1);
        assert_eq!(diagnostics.structural_replaced, 1);
    }

    struct RoutedParent {
        received: Rc<RefCell<Vec<u32>>>,
        child_handled: Rc<RefCell<Vec<u32>>>,
    }

    struct RoutedParentState {
        visible: crate::runtime::State<bool>,
        trigger: crate::runtime::State<bool>,
    }

    enum RoutedParentAction {
        SetTrigger(bool),
        HideAndTrigger,
        Child(Box<u32>),
    }

    struct RoutedChild {
        trigger: crate::runtime::Read<bool>,
    }

    struct RoutedChildAction(Box<u32>);

    struct SelfRoutedChild {
        trigger: crate::runtime::Read<bool>,
        handled: Rc<RefCell<Vec<u32>>>,
    }

    impl Component for RoutedChild {
        type State = ();
        type Action = RoutedChildAction;

        fn create(&self, _context: &mut crate::runtime::CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &(), ui: &mut crate::runtime::Ui<'_, '_, Self::Action>) -> UiRoot {
            ui.observe(self.trigger, |trigger| {
                RoutedChildAction(Box::new(u32::from(*trigger)))
            })
            .unwrap();
            ui.foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        }

        fn action(
            &self,
            _state: &mut (),
            _action: Self::Action,
            _context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
        }
    }

    impl Component for SelfRoutedChild {
        type State = ();
        type Action = RoutedChildAction;

        fn create(&self, _context: &mut crate::runtime::CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &(), ui: &mut crate::runtime::Ui<'_, '_, Self::Action>) -> UiRoot {
            ui.observe(self.trigger, |trigger| {
                RoutedChildAction(Box::new(u32::from(*trigger)))
            })
            .unwrap();
            ui.foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        }

        fn action(
            &self,
            _state: &mut (),
            RoutedChildAction(value): Self::Action,
            _context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            self.handled.borrow_mut().push(*value);
        }
    }

    impl Component for RoutedParent {
        type State = RoutedParentState;
        type Action = RoutedParentAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            RoutedParentState {
                visible: context.state(true),
                trigger: context.state(false),
            }
        }

        fn mount(
            &self,
            state: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            let mut hosts = Vec::new();
            let root =
                ui.foundation()
                    .root(BoxStyle::default(), LayoutStyle::default(), |writer| {
                        for _ in 0..4 {
                            hosts.push(writer.container(
                                BoxStyle::default(),
                                LayoutStyle::default(),
                                |_| {},
                            ));
                        }
                    });
            let mapped_trigger = state.trigger.read();
            ui.when_map(
                state.visible.read(),
                hosts[0],
                move || RoutedChild {
                    trigger: mapped_trigger,
                },
                |RoutedChildAction(value)| RoutedParentAction::Child(value),
            )
            .unwrap();
            let consumed_trigger = state.trigger.read();
            ui.when_consume(state.visible.read(), hosts[1], move || RoutedChild {
                trigger: consumed_trigger,
            })
            .unwrap();
            let command_trigger = state.trigger.read();
            ui.when_command(
                state.visible.read(),
                hosts[2],
                move || RoutedChild {
                    trigger: command_trigger,
                },
                |_| crate::runtime::Command::RequestFrame,
            )
            .unwrap();
            let self_trigger = state.trigger.read();
            let child_handled = self.child_handled.clone();
            ui.when(state.visible.read(), hosts[3], move || SelfRoutedChild {
                trigger: self_trigger,
                handled: child_handled.clone(),
            })
            .unwrap();
            root
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            match action {
                RoutedParentAction::SetTrigger(trigger) => {
                    context.set(state.trigger, trigger).unwrap();
                }
                RoutedParentAction::HideAndTrigger => {
                    context.set(state.visible, false).unwrap();
                    context.set(state.trigger, true).unwrap();
                }
                RoutedParentAction::Child(value) => self.received.borrow_mut().push(*value),
            }
        }
    }

    #[test]
    fn non_clone_child_observer_actions_map_or_consume_and_stale_routes_close_on_unmount() {
        let received = Rc::new(RefCell::new(Vec::new()));
        let child_handled = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(RoutedParent {
            received: received.clone(),
            child_handled: child_handled.clone(),
        })
        .unwrap();

        runtime
            .send_component_action(RoutedParentAction::SetTrigger(true))
            .unwrap();
        assert_eq!(&*received.borrow(), &[1]);
        assert_eq!(&*child_handled.borrow(), &[1]);
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.transactions, 3);
        assert_eq!(diagnostics.observer_actions, 3);
        assert_eq!(
            runtime.pop_command(),
            Some(crate::runtime::Command::RequestFrame)
        );

        runtime
            .send_component_action(RoutedParentAction::SetTrigger(false))
            .unwrap();
        received.borrow_mut().clear();
        child_handled.borrow_mut().clear();
        runtime
            .send_component_action(RoutedParentAction::HideAndTrigger)
            .unwrap();
        assert!(received.borrow().is_empty());
        assert!(child_handled.borrow().is_empty());
        assert_eq!(runtime.component_diagnostics().live_observers, 0);
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    enum InputBranch {
        Mapped,
        Consumed,
        Command,
        SelfHandled,
    }

    struct FoundationChildAction(Box<u32>);

    struct FoundationChild {
        value: u32,
        buttons: Rc<RefCell<Vec<(u32, crate::ui::UiNodeId)>>>,
        handled: Rc<RefCell<Vec<u32>>>,
    }

    impl Component for FoundationChild {
        type State = ();
        type Action = FoundationChildAction;

        fn create(&self, _context: &mut crate::runtime::CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &(), ui: &mut crate::runtime::Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let value = self.value;
            let button = ui
                .button(
                    root.0,
                    move || FoundationChildAction(Box::new(value)),
                    BoxStyle::default(),
                    |_| {},
                )
                .unwrap();
            let value = self.value;
            ui.listen(button.node, LISTEN_TEST, move |_| {
                FoundationChildAction(Box::new(value + 1))
            })
            .unwrap();
            self.buttons.borrow_mut().push((self.value, button.node));
            root
        }

        fn action(
            &self,
            _state: &mut (),
            FoundationChildAction(value): Self::Action,
            _context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            self.handled.borrow_mut().push(*value);
        }
    }

    struct FoundationRouteState {
        selected: crate::runtime::State<InputBranch>,
        items: crate::runtime::State<Vec<u32>>,
    }

    enum FoundationParentAction {
        Select(InputBranch),
        Mapped(Box<u32>),
    }

    struct FoundationRouteParent {
        buttons: Rc<RefCell<Vec<(u32, crate::ui::UiNodeId)>>>,
        received: Rc<RefCell<Vec<u32>>>,
        child_handled: Rc<RefCell<Vec<u32>>>,
    }

    impl Component for FoundationRouteParent {
        type State = FoundationRouteState;
        type Action = FoundationParentAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            FoundationRouteState {
                selected: context.state(InputBranch::Mapped),
                items: context.state(vec![1]),
            }
        }

        fn mount(
            &self,
            state: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            let mut hosts = Vec::new();
            let root =
                ui.foundation()
                    .root(BoxStyle::default(), LayoutStyle::default(), |writer| {
                        for _ in 0..4 {
                            hosts.push(writer.container(
                                BoxStyle::default(),
                                LayoutStyle::default(),
                                |_| {},
                            ));
                        }
                    });

            let mapped_buttons = self.buttons.clone();
            let mapped_handled = self.child_handled.clone();
            let consumed_buttons = self.buttons.clone();
            let consumed_handled = self.child_handled.clone();
            let command_buttons = self.buttons.clone();
            let command_handled = self.child_handled.clone();
            let self_buttons = self.buttons.clone();
            let self_handled = self.child_handled.clone();
            ui.switch(
                state.selected.read(),
                hosts[0],
                vec![
                    crate::runtime::SwitchBranch::map(
                        InputBranch::Mapped,
                        move || FoundationChild {
                            value: 10,
                            buttons: mapped_buttons.clone(),
                            handled: mapped_handled.clone(),
                        },
                        |FoundationChildAction(value)| FoundationParentAction::Mapped(value),
                    ),
                    crate::runtime::SwitchBranch::consume(InputBranch::Consumed, move || {
                        FoundationChild {
                            value: 20,
                            buttons: consumed_buttons.clone(),
                            handled: consumed_handled.clone(),
                        }
                    }),
                    crate::runtime::SwitchBranch::command(
                        InputBranch::Command,
                        move || FoundationChild {
                            value: 30,
                            buttons: command_buttons.clone(),
                            handled: command_handled.clone(),
                        },
                        |_| crate::runtime::Command::RequestFrame,
                    ),
                    crate::runtime::SwitchBranch::new(InputBranch::SelfHandled, move || {
                        FoundationChild {
                            value: 40,
                            buttons: self_buttons.clone(),
                            handled: self_handled.clone(),
                        }
                    }),
                ],
            )
            .unwrap();

            let mapped_buttons = self.buttons.clone();
            let mapped_handled = self.child_handled.clone();
            ui.for_each_keyed_map(
                state.items.read(),
                hosts[1],
                |item| *item,
                move |_| FoundationChild {
                    value: 50,
                    buttons: mapped_buttons.clone(),
                    handled: mapped_handled.clone(),
                },
                |FoundationChildAction(value)| FoundationParentAction::Mapped(value),
            )
            .unwrap();
            let consumed_buttons = self.buttons.clone();
            let consumed_handled = self.child_handled.clone();
            ui.for_each_keyed_consume(
                state.items.read(),
                hosts[2],
                |item| *item,
                move |_| FoundationChild {
                    value: 60,
                    buttons: consumed_buttons.clone(),
                    handled: consumed_handled.clone(),
                },
            )
            .unwrap();
            let command_buttons = self.buttons.clone();
            let command_handled = self.child_handled.clone();
            ui.for_each_keyed_command(
                state.items.read(),
                hosts[3],
                |item| *item,
                move |_| FoundationChild {
                    value: 70,
                    buttons: command_buttons.clone(),
                    handled: command_handled.clone(),
                },
                |_| crate::runtime::Command::RequestFrame,
            )
            .unwrap();
            root
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            match action {
                FoundationParentAction::Select(selected) => {
                    context.set(state.selected, selected).unwrap();
                }
                FoundationParentAction::Mapped(value) => {
                    self.received.borrow_mut().push(*value);
                }
            }
        }
    }

    #[test]
    fn foundation_input_routes_repeat_map_consume_command_and_close_on_keyed_replacement() {
        let buttons = Rc::new(RefCell::new(Vec::new()));
        let received = Rc::new(RefCell::new(Vec::new()));
        let child_handled = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(FoundationRouteParent {
            buttons: buttons.clone(),
            received: received.clone(),
            child_handled: child_handled.clone(),
        })
        .unwrap();
        let button = |value| {
            buttons
                .borrow()
                .iter()
                .rev()
                .find(|(candidate, _)| *candidate == value)
                .unwrap()
                .1
        };

        let mapped_switch = button(10);
        assert!(runtime.dispatch_action(mapped_switch));
        assert!(runtime.dispatch_action(mapped_switch));
        runtime.dispatch_ui(mapped_switch, UiEventKind::Focus(true), LISTEN_TEST, 11);
        assert_eq!(&*received.borrow(), &[10, 10, 11]);

        assert!(runtime.dispatch_action(button(50)));
        assert!(runtime.dispatch_action(button(60)));
        assert!(runtime.dispatch_action(button(70)));
        assert_eq!(&*received.borrow(), &[10, 10, 11, 50]);
        assert!(child_handled.borrow().is_empty());
        assert_eq!(
            runtime.pop_command(),
            Some(crate::runtime::Command::RequestFrame)
        );

        runtime
            .send_component_action(FoundationParentAction::Select(InputBranch::Consumed))
            .unwrap();
        assert!(!runtime.dispatch_action(mapped_switch));
        assert!(runtime.dispatch_action(button(20)));
        assert!(child_handled.borrow().is_empty());

        runtime
            .send_component_action(FoundationParentAction::Select(InputBranch::Command))
            .unwrap();
        assert!(runtime.dispatch_action(button(30)));
        assert_eq!(
            runtime.pop_command(),
            Some(crate::runtime::Command::RequestFrame)
        );

        runtime
            .send_component_action(FoundationParentAction::Select(InputBranch::SelfHandled))
            .unwrap();
        assert!(runtime.dispatch_action(button(40)));
        assert_eq!(&*child_handled.borrow(), &[40]);
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.stale_actions, 1);
        assert_eq!(diagnostics.live_input_routes, 8);
    }

    struct DerivedValidationChild;

    impl Component for DerivedValidationChild {
        type State = ();
        type Action = crate::runtime::NoAction;

        fn create(&self, _context: &mut crate::runtime::CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &(), ui: &mut crate::runtime::Ui<'_, '_, Self::Action>) -> UiRoot {
            ui.foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        }

        fn action(
            &self,
            _state: &mut (),
            action: Self::Action,
            _context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            match action {}
        }
    }

    struct DerivedValidationState {
        map_items: crate::runtime::State<Vec<KeyedItem>>,
        mapped_items: crate::runtime::Read<Vec<KeyedItem>>,
        zip_left: crate::runtime::State<bool>,
        zip_right: crate::runtime::State<bool>,
        zipped_branch: crate::runtime::Read<BranchKey>,
        select_bad: crate::runtime::State<bool>,
        selected_items: crate::runtime::Read<Vec<KeyedItem>>,
    }

    enum DerivedValidationAction {
        DuplicateMap,
        MissingZipBranch,
        SelectDuplicateBranch,
        Probe,
    }

    type DerivedProbe = (Vec<u32>, bool, bool, bool);

    struct DerivedValidationParent {
        hosts: Rc<RefCell<Vec<crate::ui::UiNodeId>>>,
        probes: Rc<RefCell<Vec<DerivedProbe>>>,
    }

    impl Component for DerivedValidationParent {
        type State = DerivedValidationState;
        type Action = DerivedValidationAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            let map_items = context.state(vec![
                KeyedItem {
                    key: 1,
                    opacity: 0.1,
                },
                KeyedItem {
                    key: 2,
                    opacity: 0.2,
                },
            ]);
            let mapped_items = context
                .map(map_items.read(), |items| items.clone())
                .unwrap();
            let zip_left = context.state(false);
            let zip_right = context.state(false);
            let zipped_branch = context
                .zip(zip_left.read(), zip_right.read(), |left, right| {
                    if *left && !*right {
                        BranchKey::Missing
                    } else if *right {
                        BranchKey::Second
                    } else {
                        BranchKey::First
                    }
                })
                .unwrap();
            let select_bad = context.state(false);
            let valid = context.state(vec![
                KeyedItem {
                    key: 7,
                    opacity: 0.7,
                },
                KeyedItem {
                    key: 8,
                    opacity: 0.8,
                },
            ]);
            let duplicate = context.state(vec![
                KeyedItem {
                    key: 9,
                    opacity: 0.9,
                },
                KeyedItem {
                    key: 9,
                    opacity: 1.0,
                },
            ]);
            let selected_items = context
                .select(select_bad.read(), duplicate.read(), valid.read())
                .unwrap();
            DerivedValidationState {
                map_items,
                mapped_items,
                zip_left,
                zip_right,
                zipped_branch,
                select_bad,
                selected_items,
            }
        }

        fn mount(
            &self,
            state: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            let mut hosts = Vec::new();
            let root =
                ui.foundation()
                    .root(BoxStyle::default(), LayoutStyle::default(), |writer| {
                        for _ in 0..3 {
                            hosts.push(writer.container(
                                BoxStyle::default(),
                                LayoutStyle::default(),
                                |_| {},
                            ));
                        }
                    });
            self.hosts.replace(hosts.clone());
            ui.for_each_keyed(
                state.mapped_items,
                hosts[0],
                |item| item.key,
                |_| DerivedValidationChild,
            )
            .unwrap();
            ui.switch(
                state.zipped_branch,
                hosts[1],
                vec![
                    crate::runtime::SwitchBranch::new(BranchKey::First, || DerivedValidationChild),
                    crate::runtime::SwitchBranch::new(BranchKey::Second, || DerivedValidationChild),
                ],
            )
            .unwrap();
            ui.for_each_keyed(
                state.selected_items,
                hosts[2],
                |item| item.key,
                |_| DerivedValidationChild,
            )
            .unwrap();
            root
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            match action {
                DerivedValidationAction::DuplicateMap => context
                    .set(
                        state.map_items,
                        vec![
                            KeyedItem {
                                key: 3,
                                opacity: 0.3,
                            },
                            KeyedItem {
                                key: 3,
                                opacity: 0.4,
                            },
                        ],
                    )
                    .unwrap(),
                DerivedValidationAction::MissingZipBranch => {
                    context.set(state.zip_left, true).unwrap();
                }
                DerivedValidationAction::SelectDuplicateBranch => {
                    context.set(state.select_bad, true).unwrap();
                }
                DerivedValidationAction::Probe => {
                    let keys = context
                        .get(state.map_items)
                        .unwrap()
                        .into_iter()
                        .map(|item| item.key)
                        .collect();
                    self.probes.borrow_mut().push((
                        keys,
                        context.get(state.zip_left).unwrap(),
                        context.get(state.zip_right).unwrap(),
                        context.get(state.select_bad).unwrap(),
                    ));
                }
            }
        }
    }

    #[test]
    fn derived_structural_inputs_reject_map_zip_and_select_outputs_before_publication() {
        let hosts = Rc::new(RefCell::new(Vec::new()));
        let probes = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(DerivedValidationParent {
            hosts: hosts.clone(),
            probes: probes.clone(),
        })
        .unwrap();
        let initial = hosts
            .borrow()
            .iter()
            .map(|host| runtime.ui().nodes.children(*host).collect::<Vec<_>>())
            .collect::<Vec<_>>();

        assert!(
            runtime
                .send_component_action(DerivedValidationAction::DuplicateMap)
                .is_err()
        );
        assert!(
            runtime
                .send_component_action(DerivedValidationAction::MissingZipBranch)
                .is_err()
        );
        assert!(
            runtime
                .send_component_action(DerivedValidationAction::SelectDuplicateBranch)
                .is_err()
        );
        let after = hosts
            .borrow()
            .iter()
            .map(|host| runtime.ui().nodes.children(*host).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(after, initial);

        runtime
            .send_component_action(DerivedValidationAction::Probe)
            .unwrap();
        assert_eq!(&*probes.borrow(), &[(vec![1, 2], false, false, false)]);
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.rejected_transactions, 3);
        assert_eq!(diagnostics.state_writes_committed, 0);
        assert_eq!(diagnostics.structural_removed, 0);
        assert_eq!(diagnostics.structural_replaced, 0);
    }

    struct PortalChildAction(Box<u32>);

    struct PortalChild {
        value: u32,
        buttons: Rc<RefCell<Vec<(u32, crate::ui::UiNodeId)>>>,
        components: Rc<RefCell<Vec<(u32, crate::runtime::ComponentId)>>>,
        handled: Rc<RefCell<Vec<u32>>>,
        teardown: Rc<RefCell<Vec<u32>>>,
    }

    impl Component for PortalChild {
        type State = ();
        type Action = PortalChildAction;

        fn create(&self, _context: &mut crate::runtime::CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &(), ui: &mut crate::runtime::Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let value = self.value;
            let button = ui
                .button(
                    root.0,
                    move || PortalChildAction(Box::new(value)),
                    BoxStyle::default(),
                    |_| {},
                )
                .unwrap();
            self.buttons.borrow_mut().push((self.value, button.node));
            self.components
                .borrow_mut()
                .push((self.value, ui.component()));
            root
        }

        fn action(
            &self,
            _state: &mut (),
            PortalChildAction(value): Self::Action,
            _context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            self.handled.borrow_mut().push(*value);
        }

        fn unmount(&self, _state: &mut (), _context: &mut crate::runtime::UnmountContext<'_>) {
            self.teardown.borrow_mut().push(self.value);
        }
    }

    enum PortalParentAction {
        Noop,
        Mapped(Box<u32>),
    }

    struct PortalParent {
        root: Rc<Cell<Option<crate::ui::UiNodeId>>>,
        hosts: Rc<RefCell<Vec<crate::ui::UiNodeId>>>,
        buttons: Rc<RefCell<Vec<(u32, crate::ui::UiNodeId)>>>,
        components: Rc<RefCell<Vec<(u32, crate::runtime::ComponentId)>>>,
        received: Rc<RefCell<Vec<u32>>>,
        child_handled: Rc<RefCell<Vec<u32>>>,
        teardown: Rc<RefCell<Vec<u32>>>,
    }

    impl Component for PortalParent {
        type State = ();
        type Action = PortalParentAction;

        fn create(&self, _context: &mut crate::runtime::CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &(), ui: &mut crate::runtime::Ui<'_, '_, Self::Action>) -> UiRoot {
            let mut hosts = Vec::new();
            let root =
                ui.foundation()
                    .root(BoxStyle::default(), LayoutStyle::default(), |writer| {
                        for _ in 0..4 {
                            hosts.push(writer.container(
                                BoxStyle::default(),
                                LayoutStyle::default(),
                                |_| {},
                            ));
                        }
                    });
            self.root.set(Some(root.0));
            self.hosts.replace(hosts.clone());

            let child = |value: u32,
                         buttons: Rc<RefCell<Vec<(u32, crate::ui::UiNodeId)>>>,
                         components: Rc<RefCell<Vec<(u32, crate::runtime::ComponentId)>>>,
                         handled: Rc<RefCell<Vec<u32>>>,
                         teardown: Rc<RefCell<Vec<u32>>>| PortalChild {
                value,
                buttons,
                components,
                handled,
                teardown,
            };
            let buttons = self.buttons.clone();
            let components = self.components.clone();
            let handled = self.child_handled.clone();
            let teardown = self.teardown.clone();
            ui.portal_map(
                hosts[0],
                move || {
                    child(
                        100,
                        buttons.clone(),
                        components.clone(),
                        handled.clone(),
                        teardown.clone(),
                    )
                },
                |PortalChildAction(value)| PortalParentAction::Mapped(value),
            )
            .unwrap();
            let buttons = self.buttons.clone();
            let components = self.components.clone();
            let handled = self.child_handled.clone();
            let teardown = self.teardown.clone();
            ui.portal_consume(hosts[1], move || {
                child(
                    200,
                    buttons.clone(),
                    components.clone(),
                    handled.clone(),
                    teardown.clone(),
                )
            })
            .unwrap();
            let buttons = self.buttons.clone();
            let components = self.components.clone();
            let handled = self.child_handled.clone();
            let teardown = self.teardown.clone();
            ui.portal_command(
                hosts[2],
                move || {
                    child(
                        300,
                        buttons.clone(),
                        components.clone(),
                        handled.clone(),
                        teardown.clone(),
                    )
                },
                |_| crate::runtime::Command::RequestFrame,
            )
            .unwrap();
            let buttons = self.buttons.clone();
            let components = self.components.clone();
            let handled = self.child_handled.clone();
            let teardown = self.teardown.clone();
            ui.portal(hosts[3], move || {
                child(
                    400,
                    buttons.clone(),
                    components.clone(),
                    handled.clone(),
                    teardown.clone(),
                )
            })
            .unwrap();
            root
        }

        fn action(
            &self,
            _state: &mut (),
            action: Self::Action,
            _context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            match action {
                PortalParentAction::Noop => {}
                PortalParentAction::Mapped(value) => self.received.borrow_mut().push(*value),
            }
        }

        fn unmount(&self, _state: &mut (), _context: &mut crate::runtime::UnmountContext<'_>) {
            self.teardown.borrow_mut().push(0);
        }
    }

    #[test]
    fn portals_retain_logical_children_across_visual_host_moves_and_route_actions() {
        let root = Rc::new(Cell::new(None));
        let hosts = Rc::new(RefCell::new(Vec::new()));
        let buttons = Rc::new(RefCell::new(Vec::new()));
        let components = Rc::new(RefCell::new(Vec::new()));
        let received = Rc::new(RefCell::new(Vec::new()));
        let child_handled = Rc::new(RefCell::new(Vec::new()));
        let teardown = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(PortalParent {
            root: root.clone(),
            hosts: hosts.clone(),
            buttons: buttons.clone(),
            components: components.clone(),
            received: received.clone(),
            child_handled: child_handled.clone(),
            teardown: teardown.clone(),
        })
        .unwrap();
        let hosts = hosts.borrow().clone();
        let visual_roots = hosts
            .iter()
            .map(|host| runtime.ui().nodes.children(*host).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert!(visual_roots.iter().all(|children| children.len() == 1));
        let initial_components = components.borrow().clone();
        assert_eq!(initial_components.len(), 4);
        let button = |value| {
            buttons
                .borrow()
                .iter()
                .find(|(candidate, _)| *candidate == value)
                .unwrap()
                .1
        };

        assert!(runtime.dispatch_action(button(100)));
        assert!(runtime.dispatch_action(button(100)));
        assert!(runtime.dispatch_action(button(200)));
        assert!(runtime.dispatch_action(button(300)));
        assert!(runtime.dispatch_action(button(400)));
        assert_eq!(&*received.borrow(), &[100, 100]);
        assert_eq!(&*child_handled.borrow(), &[400]);
        assert_eq!(
            runtime.pop_command(),
            Some(crate::runtime::Command::RequestFrame)
        );

        assert!(
            runtime
                .ui_mut()
                .nodes
                .reparent_before(hosts[0], root.get().unwrap(), None)
        );
        runtime
            .send_component_action(PortalParentAction::Noop)
            .unwrap();
        assert_eq!(&*components.borrow(), &initial_components);
        assert_eq!(
            runtime.ui().nodes.children(hosts[0]).collect::<Vec<_>>(),
            visual_roots[0]
        );
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.structural_inserted, 4);
        assert_eq!(diagnostics.live_components, 5);
        assert_eq!(diagnostics.live_input_routes, 4);

        runtime.unmount_component().unwrap();
        assert_eq!(&*teardown.borrow(), &[400, 300, 200, 100, 0]);
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.live_components, 0);
        assert_eq!(diagnostics.live_input_routes, 0);
        assert_eq!(diagnostics.live_structural_containers, 0);
    }

    struct StalePortalParent(crate::ui::UiNodeId);

    impl Component for StalePortalParent {
        type State = ();
        type Action = crate::runtime::NoAction;

        fn create(&self, _context: &mut crate::runtime::CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &(), ui: &mut crate::runtime::Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            ui.portal(self.0, || DerivedValidationChild).unwrap();
            root
        }

        fn action(
            &self,
            _state: &mut (),
            action: Self::Action,
            _context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            match action {}
        }
    }

    #[test]
    fn portal_rejects_a_stale_visual_host_during_initial_reconciliation() {
        let mut foreign = MountedUi::default();
        let mut stale = None;
        MountWriter::<()>::new(&mut foreign).root(
            BoxStyle::default(),
            LayoutStyle::default(),
            |writer| {
                for _ in 0..8 {
                    stale =
                        Some(writer.container(BoxStyle::default(), LayoutStyle::default(), |_| {}));
                }
            },
        );
        assert!(ViewRuntime::from_component(StalePortalParent(stale.unwrap())).is_err());
    }

    #[derive(Default)]
    struct ManualTaskControl {
        tasks: RefCell<Vec<ManualTask>>,
    }

    enum ManualFuture {
        Local(crate::runtime::LocalTask),
        Send(crate::runtime::SendTask),
    }

    struct ManualTask {
        future: ManualFuture,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    #[derive(Clone)]
    struct ManualTaskHost {
        control: Rc<ManualTaskControl>,
    }

    struct ManualCancellation(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl crate::runtime::TaskCancellation for ManualCancellation {
        fn cancel(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Release);
        }
    }

    impl crate::runtime::TaskHost for ManualTaskHost {
        fn supports_local(&self) -> bool {
            true
        }

        fn supports_send(&self) -> bool {
            true
        }

        fn spawn_local(
            &mut self,
            task: crate::runtime::LocalTask,
        ) -> crate::runtime::RuntimeResult<Box<dyn crate::runtime::TaskCancellation>> {
            let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            self.control.tasks.borrow_mut().push(ManualTask {
                future: ManualFuture::Local(task),
                cancelled: cancelled.clone(),
            });
            Ok(Box::new(ManualCancellation(cancelled)))
        }

        fn spawn_send(
            &mut self,
            task: crate::runtime::SendTask,
        ) -> crate::runtime::RuntimeResult<Box<dyn crate::runtime::TaskCancellation>> {
            let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            self.control.tasks.borrow_mut().push(ManualTask {
                future: ManualFuture::Send(task),
                cancelled: cancelled.clone(),
            });
            Ok(Box::new(ManualCancellation(cancelled)))
        }
    }

    struct NoopWake;

    impl std::task::Wake for NoopWake {
        fn wake(self: std::sync::Arc<Self>) {}
    }

    impl ManualTaskControl {
        fn poll_all(&self) {
            let waker = std::task::Waker::from(std::sync::Arc::new(NoopWake));
            let mut context = std::task::Context::from_waker(&waker);
            let tasks = std::mem::take(&mut *self.tasks.borrow_mut());
            for mut task in tasks {
                if task.cancelled.load(std::sync::atomic::Ordering::Acquire) {
                    continue;
                }
                let poll = match &mut task.future {
                    ManualFuture::Local(future) => future.as_mut().poll(&mut context),
                    ManualFuture::Send(future) => future.as_mut().poll(&mut context),
                };
                if poll.is_pending() {
                    self.tasks.borrow_mut().push(task);
                }
            }
        }
    }

    struct TaskComponent {
        observed: Rc<Cell<u32>>,
        support: TaskSupport,
    }

    #[derive(Clone, Default)]
    struct TaskSupport {
        handles: std::sync::Arc<std::sync::Mutex<Vec<crate::runtime::TaskHandle>>>,
        senders: std::sync::Arc<std::sync::Mutex<Vec<crate::runtime::TaskSender<TaskAction>>>>,
        backpressure: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    fn task_component(observed: Rc<Cell<u32>>) -> (TaskComponent, TaskSupport) {
        let support = TaskSupport::default();
        (
            TaskComponent {
                observed,
                support: support.clone(),
            },
            support,
        )
    }

    struct TaskState {
        value: crate::runtime::State<u32>,
    }

    enum TaskAction {
        StartLocal,
        StartSend,
        StartProgress,
        StartStreaming,
        StartPending,
        StartUnsupported,
        Progress(u32),
        Complete(u32),
        Inspect,
    }

    impl Component for TaskComponent {
        type State = TaskState;
        type Action = TaskAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            TaskState {
                value: context.state(0),
            }
        }

        fn mount(
            &self,
            _state: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            ui.foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            match action {
                TaskAction::StartLocal => {
                    context.spawn(async { TaskAction::Complete(1) });
                }
                TaskAction::StartSend => {
                    context.spawn_send(async { TaskAction::Complete(2) });
                }
                TaskAction::StartProgress => {
                    let senders = self.support.senders.clone();
                    let backpressure = self.support.backpressure.clone();
                    let handle = context
                        .spawn_send_with_sender(1, move |sender| {
                            senders.lock().unwrap().push(sender.clone());
                            async move {
                                assert!(sender.try_send(TaskAction::Progress(10)).is_ok());
                                if matches!(
                                    sender.try_send(TaskAction::Progress(11)),
                                    Err(crate::runtime::TaskSendError::Full(TaskAction::Progress(
                                        11
                                    )))
                                ) {
                                    backpressure.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                                TaskAction::Complete(20)
                            }
                        })
                        .unwrap();
                    self.support.handles.lock().unwrap().push(handle);
                }
                TaskAction::StartStreaming => {
                    let senders = self.support.senders.clone();
                    let handle = context
                        .spawn_send_with_sender(1, move |sender| {
                            senders.lock().unwrap().push(sender.clone());
                            async move {
                                assert!(sender.try_send(TaskAction::Progress(30)).is_ok());
                                std::future::pending::<TaskAction>().await
                            }
                        })
                        .unwrap();
                    self.support.handles.lock().unwrap().push(handle);
                }
                TaskAction::StartPending => {
                    let handle = context.spawn(std::future::pending::<TaskAction>());
                    self.support.handles.lock().unwrap().push(handle);
                }
                TaskAction::StartUnsupported => {
                    context.set(state.value, 9).unwrap();
                    context.spawn(async { TaskAction::Complete(9) });
                }
                TaskAction::Progress(value) | TaskAction::Complete(value) => {
                    context.set(state.value, value).unwrap();
                    self.observed.set(value);
                }
                TaskAction::Inspect => {
                    self.observed.set(context.get(state.value).unwrap());
                }
            }
        }
    }

    #[test]
    fn local_and_send_tasks_deliver_later_actions_with_a_fairness_budget() {
        let control = Rc::new(ManualTaskControl::default());
        let observed = Rc::new(Cell::new(0));
        let (component, _) = task_component(observed.clone());
        let mut runtime = ViewRuntime::from_component_with_task_host(
            component,
            ManualTaskHost {
                control: control.clone(),
            },
        )
        .unwrap();
        runtime.scheduler_mut().begin_frame();
        runtime
            .send_component_action(TaskAction::StartLocal)
            .unwrap();
        runtime
            .send_component_action(TaskAction::StartSend)
            .unwrap();
        assert_eq!(runtime.component_diagnostics().live_tasks, 2);
        assert!(!runtime.task_results_ready());

        control.poll_all();
        assert!(runtime.task_results_ready());
        assert_eq!(observed.get(), 0);
        runtime.set_component_task_result_limit(1).unwrap();
        assert_eq!(runtime.process_component_task_results().unwrap(), 1);
        assert_eq!(observed.get(), 1);
        assert!(runtime.task_results_ready());
        assert_eq!(runtime.process_component_task_results().unwrap(), 1);
        assert_eq!(observed.get(), 2);
        assert!(!runtime.task_results_ready());
        assert!(!runtime.scheduler().needs_frame());

        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.tasks_started, 2);
        assert_eq!(diagnostics.tasks_completed, 2);
        assert_eq!(diagnostics.live_tasks, 0);
        assert_eq!(diagnostics.task_queue_high_water, 2);
    }

    #[test]
    fn unmount_closes_a_completed_task_route_before_delivery() {
        let control = Rc::new(ManualTaskControl::default());
        let observed = Rc::new(Cell::new(0));
        let (component, _) = task_component(observed.clone());
        let mut runtime = ViewRuntime::from_component_with_task_host(
            component,
            ManualTaskHost {
                control: control.clone(),
            },
        )
        .unwrap();
        runtime
            .send_component_action(TaskAction::StartLocal)
            .unwrap();
        control.poll_all();
        assert!(runtime.task_results_ready());
        runtime.unmount_component().unwrap();
        assert_eq!(runtime.process_component_task_results().unwrap(), 1);
        assert_eq!(observed.get(), 0);

        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.tasks_cancelled, 1);
        assert_eq!(diagnostics.stale_task_results, 1);
        assert_eq!(diagnostics.live_tasks, 0);
    }

    #[test]
    fn unsupported_task_start_rejects_the_originating_transaction() {
        let observed = Rc::new(Cell::new(99));
        let (component, _) = task_component(observed.clone());
        let mut runtime = ViewRuntime::from_component(component).unwrap();
        assert!(
            runtime
                .send_component_action(TaskAction::StartUnsupported)
                .is_err()
        );
        runtime.send_component_action(TaskAction::Inspect).unwrap();
        assert_eq!(observed.get(), 0);
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.rejected_transactions, 1);
        assert_eq!(diagnostics.tasks_started, 0);
        assert_eq!(diagnostics.live_tasks, 0);
    }

    #[test]
    fn bounded_progress_sender_reports_backpressure_and_coalesces_wakes() {
        let control = Rc::new(ManualTaskControl::default());
        let observed = Rc::new(Cell::new(0));
        let (component, support) = task_component(observed.clone());
        let wakes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let wake_count = wakes.clone();
        let mut runtime = ViewRuntime::from_component_with_task_host_and_wake(
            component,
            ManualTaskHost {
                control: control.clone(),
            },
            move || {
                wake_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
        )
        .unwrap();
        runtime
            .send_component_action(TaskAction::StartProgress)
            .unwrap();
        control.poll_all();
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(
            support
                .backpressure
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        let sender = support.senders.lock().unwrap()[0].clone();
        assert!(matches!(
            sender.try_send(TaskAction::Progress(12)),
            Err(crate::runtime::TaskSendError::Closed(TaskAction::Progress(
                12
            )))
        ));

        runtime.set_component_task_result_limit(1).unwrap();
        assert_eq!(runtime.process_component_task_results().unwrap(), 1);
        assert_eq!(observed.get(), 10);
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert!(runtime.task_results_ready());
        assert_eq!(runtime.process_component_task_results().unwrap(), 1);
        assert_eq!(observed.get(), 20);
        assert!(!runtime.task_results_ready());

        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.tasks_started, 1);
        assert_eq!(diagnostics.tasks_completed, 1);
        assert_eq!(diagnostics.task_results_delivered, 2);
        assert_eq!(diagnostics.task_queue_high_water, 2);
    }

    #[test]
    fn task_handle_cancellation_wakes_and_closes_the_scope() {
        let control = Rc::new(ManualTaskControl::default());
        let observed = Rc::new(Cell::new(0));
        let (component, support) = task_component(observed);
        let wakes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let wake_count = wakes.clone();
        let mut runtime = ViewRuntime::from_component_with_task_host_and_wake(
            component,
            ManualTaskHost {
                control: control.clone(),
            },
            move || {
                wake_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
        )
        .unwrap();
        runtime
            .send_component_action(TaskAction::StartPending)
            .unwrap();
        let handle = support.handles.lock().unwrap()[0].clone();
        handle.cancel();
        handle.cancel();
        assert!(handle.is_cancelled());
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(runtime.task_results_ready());
        assert_eq!(runtime.process_component_task_results().unwrap(), 1);
        assert!(!runtime.task_results_ready());
        control.poll_all();

        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.tasks_cancelled, 1);
        assert_eq!(diagnostics.live_tasks, 0);
    }

    #[test]
    fn draining_progress_restores_capacity_and_unmount_closes_stale_senders() {
        let control = Rc::new(ManualTaskControl::default());
        let observed = Rc::new(Cell::new(0));
        let (component, support) = task_component(observed.clone());
        let mut runtime = ViewRuntime::from_component_with_task_host(
            component,
            ManualTaskHost {
                control: control.clone(),
            },
        )
        .unwrap();
        runtime
            .send_component_action(TaskAction::StartStreaming)
            .unwrap();
        control.poll_all();
        let sender = support.senders.lock().unwrap()[0].clone();
        assert!(matches!(
            sender.try_send(TaskAction::Progress(31)),
            Err(crate::runtime::TaskSendError::Full(TaskAction::Progress(
                31
            )))
        ));
        assert_eq!(runtime.process_component_task_results().unwrap(), 1);
        assert_eq!(observed.get(), 30);
        assert!(sender.try_send(TaskAction::Progress(32)).is_ok());

        runtime.unmount_component().unwrap();
        assert!(matches!(
            sender.try_send(TaskAction::Progress(33)),
            Err(crate::runtime::TaskSendError::Closed(TaskAction::Progress(
                33
            )))
        ));
        assert_eq!(runtime.process_component_task_results().unwrap(), 1);
        assert_eq!(observed.get(), 30);
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.tasks_cancelled, 1);
        assert_eq!(diagnostics.stale_task_results, 1);
        assert_eq!(diagnostics.live_tasks, 0);
    }

    #[test]
    fn executor_shutdown_cancels_scopes_and_rejects_new_starts() {
        let control = Rc::new(ManualTaskControl::default());
        let observed = Rc::new(Cell::new(0));
        let (component, support) = task_component(observed);
        let mut runtime = ViewRuntime::from_component_with_task_host(
            component,
            ManualTaskHost {
                control: control.clone(),
            },
        )
        .unwrap();
        runtime
            .send_component_action(TaskAction::StartPending)
            .unwrap();
        assert_eq!(runtime.shutdown_task_host(), 1);
        assert!(support.handles.lock().unwrap()[0].is_cancelled());
        assert!(
            runtime
                .send_component_action(TaskAction::StartLocal)
                .is_err()
        );
        control.poll_all();

        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.task_host_shutdowns, 1);
        assert_eq!(diagnostics.tasks_cancelled, 1);
        assert_eq!(diagnostics.rejected_transactions, 1);
        assert_eq!(diagnostics.live_tasks, 0);
    }

    struct TimerComponent {
        observed: Rc<Cell<u32>>,
        handles: Rc<RefCell<Vec<crate::runtime::TimerHandle>>>,
    }

    struct TimerState {
        ticks: crate::runtime::State<u32>,
    }

    enum TimerAction {
        StartOneShot,
        StartInterval,
        StartMany,
        StartInvalid,
        Tick,
        Inspect,
    }

    impl Component for TimerComponent {
        type State = TimerState;
        type Action = TimerAction;

        fn create(&self, context: &mut crate::runtime::CreateContext<'_>) -> Self::State {
            TimerState {
                ticks: context.state(0),
            }
        }

        fn mount(
            &self,
            _state: &Self::State,
            ui: &mut crate::runtime::Ui<'_, '_, Self::Action>,
        ) -> UiRoot {
            ui.foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut crate::runtime::UpdateContext<'_, Self>,
        ) {
            match action {
                TimerAction::StartOneShot => {
                    self.handles.borrow_mut().push(context.timer_at(
                        crate::runtime::MonotonicInstant::from_nanos(100),
                        TimerAction::Tick,
                    ));
                }
                TimerAction::StartInterval => {
                    let handle = context
                        .interval_at(
                            crate::runtime::MonotonicInstant::from_nanos(10),
                            std::time::Duration::from_nanos(5),
                            || TimerAction::Tick,
                        )
                        .unwrap();
                    self.handles.borrow_mut().push(handle);
                }
                TimerAction::StartMany => {
                    for _ in 0..3 {
                        context.timer_at(
                            crate::runtime::MonotonicInstant::from_nanos(100),
                            TimerAction::Tick,
                        );
                    }
                }
                TimerAction::StartInvalid => {
                    context.set(state.ticks, 99).unwrap();
                    let _ = context.interval_at(
                        crate::runtime::MonotonicInstant::from_nanos(10),
                        std::time::Duration::ZERO,
                        || TimerAction::Tick,
                    );
                }
                TimerAction::Tick => {
                    let ticks = context.get(state.ticks).unwrap() + 1;
                    context.set(state.ticks, ticks).unwrap();
                    self.observed.set(ticks);
                }
                TimerAction::Inspect => self.observed.set(context.get(state.ticks).unwrap()),
            }
        }
    }

    fn timer_component(
        observed: Rc<Cell<u32>>,
    ) -> (
        TimerComponent,
        Rc<RefCell<Vec<crate::runtime::TimerHandle>>>,
    ) {
        let handles = Rc::new(RefCell::new(Vec::new()));
        (
            TimerComponent {
                observed,
                handles: handles.clone(),
            },
            handles,
        )
    }

    #[test]
    fn one_shot_timer_routes_a_typed_action_only_at_its_deadline() {
        let observed = Rc::new(Cell::new(0));
        let (component, handles) = timer_component(observed.clone());
        let mut runtime = ViewRuntime::from_component(component).unwrap();
        runtime.scheduler_mut().begin_frame();
        runtime
            .send_component_action(TimerAction::StartOneShot)
            .unwrap();

        assert_eq!(
            runtime.scheduler().next_deadline(),
            Some(crate::runtime::MonotonicInstant::from_nanos(100))
        );
        assert!(!runtime.scheduler().needs_frame());
        assert!(!runtime.timers_ready(crate::runtime::MonotonicInstant::from_nanos(99)));
        assert_eq!(
            runtime
                .process_component_timers(crate::runtime::MonotonicInstant::from_nanos(99))
                .unwrap(),
            0
        );
        assert_eq!(observed.get(), 0);
        assert!(runtime.timers_ready(crate::runtime::MonotonicInstant::from_nanos(100)));
        assert_eq!(
            runtime
                .process_component_timers(crate::runtime::MonotonicInstant::from_nanos(100))
                .unwrap(),
            1
        );
        assert_eq!(observed.get(), 1);
        assert_eq!(runtime.scheduler().next_deadline(), None);
        assert!(handles.borrow()[0].is_cancelled());

        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.timers_started, 1);
        assert_eq!(diagnostics.timers_fired, 1);
        assert_eq!(diagnostics.timer_actions_delivered, 1);
        assert_eq!(diagnostics.live_timers, 0);
    }

    #[test]
    fn repeating_timer_coalesces_missed_periods_and_cancellation_wakes_once() {
        let observed = Rc::new(Cell::new(0));
        let (component, handles) = timer_component(observed.clone());
        let wakes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let wake_count = wakes.clone();
        let mut runtime = ViewRuntime::from_component_with_task_host_and_wake(
            component,
            crate::runtime::UnsupportedTaskHost,
            move || {
                wake_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
        )
        .unwrap();
        runtime
            .send_component_action(TimerAction::StartInterval)
            .unwrap();

        assert_eq!(
            runtime
                .process_component_timers(crate::runtime::MonotonicInstant::from_nanos(10))
                .unwrap(),
            1
        );
        assert_eq!(observed.get(), 1);
        assert_eq!(
            runtime
                .process_component_timers(crate::runtime::MonotonicInstant::from_nanos(31))
                .unwrap(),
            1
        );
        assert_eq!(observed.get(), 2);
        assert_eq!(
            runtime.scheduler().next_deadline(),
            Some(crate::runtime::MonotonicInstant::from_nanos(35))
        );
        assert_eq!(runtime.component_diagnostics().missed_timer_intervals, 3);

        let handle = handles.borrow()[0].clone();
        handle.cancel();
        handle.cancel();
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(runtime.timers_ready(crate::runtime::MonotonicInstant::from_nanos(31)));
        assert_eq!(
            runtime
                .process_component_timers(crate::runtime::MonotonicInstant::from_nanos(31))
                .unwrap(),
            1
        );
        assert_eq!(runtime.scheduler().next_deadline(), None);
        assert_eq!(runtime.component_diagnostics().timers_cancelled, 1);
    }

    #[test]
    fn due_timer_processing_obeys_its_per_turn_budget() {
        let observed = Rc::new(Cell::new(0));
        let (component, _) = timer_component(observed.clone());
        let mut runtime = ViewRuntime::from_component(component).unwrap();
        runtime
            .send_component_action(TimerAction::StartMany)
            .unwrap();
        runtime.set_component_timer_result_limit(1).unwrap();

        for expected in 1..=3 {
            assert_eq!(
                runtime
                    .process_component_timers(crate::runtime::MonotonicInstant::from_nanos(100))
                    .unwrap(),
                1
            );
            assert_eq!(observed.get(), expected);
        }
        assert!(!runtime.timers_ready(crate::runtime::MonotonicInstant::from_nanos(100)));
        assert_eq!(runtime.component_diagnostics().timer_queue_high_water, 3);
    }

    #[test]
    fn invalid_timer_start_rejects_state_and_unmount_cancels_live_deadlines() {
        let observed = Rc::new(Cell::new(0));
        let (component, handles) = timer_component(observed.clone());
        let mut runtime = ViewRuntime::from_component(component).unwrap();
        assert!(
            runtime
                .send_component_action(TimerAction::StartInvalid)
                .is_err()
        );
        runtime.send_component_action(TimerAction::Inspect).unwrap();
        assert_eq!(observed.get(), 0);
        assert_eq!(runtime.component_diagnostics().rejected_transactions, 1);

        runtime
            .send_component_action(TimerAction::StartOneShot)
            .unwrap();
        runtime.unmount_component().unwrap();
        assert!(handles.borrow()[0].is_cancelled());
        assert_eq!(runtime.scheduler().next_deadline(), None);
        let diagnostics = runtime.component_diagnostics();
        assert_eq!(diagnostics.timers_cancelled, 1);
        assert_eq!(diagnostics.live_timers, 0);
    }
}
