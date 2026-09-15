use crate::core::{MonotonicInstant, PointF, SizeF, SizeI};
#[cfg(all(test, feature = "application-software"))]
use crate::input::PointerButton;
use crate::input::{
    ButtonState, ChangeSource, CompetingGesture, InputEvent, Modifiers, NamedKey, PointerId,
    ValueChangePhase,
};
use crate::layout::LayoutEngine;
use crate::render::{
    ImageId, ImageResource, ImageResourceUpdate, MaterialId, MaterialResource, RenderScene,
    RenderSceneDelta, SceneCompiler,
};
use crate::runtime::{
    Component, ComponentDiagnostics, ComponentDriver, ComponentRuntimeDriver,
    CompositionDiagnostics, CompositionDriver, ViewRuntime,
};
use crate::scene::NodeId;
use crate::text::RetainedTextSystem;
use crate::theme::{
    CompiledTheme, MotionPreference, ThemeDomain, ThemeReplacement, ThemeResult, ThemeRuntime,
    ThemeScope,
};
use crate::ui::{MountedUi, UiEventKind, ValueAxis};

use crate::application_host::{
    AppError, AppResult, Command, FrameDiagnostics, FrameScheduler, LISTEN_FOCUS, LISTEN_KEY,
    LISTEN_POINTER, PlatformInput, SceneDeltaQueue,
    input::{InputCoalescer, InputCoalescingDiagnostics},
    interaction::{FocusChange, InteractionDiagnostics, InteractionRouter},
};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PreparedFrame {
    pub changed: bool,
    pub scene_epoch: u64,
    pub diagnostics: FrameDiagnostics,
}

/// Aggregate result of one bounded owner-thread input/runtime turn.
///
/// Native hosts use this result to distinguish input delivery from paint demand. Receiving input
/// does not itself imply that a frame is needed.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct InputFlushOutcome {
    pub events_received: u64,
    pub events_dispatched: u64,
    pub pointer_moves_received: u64,
    pub pointer_moves_coalesced: u64,
    pub scroll_events_received: u64,
    pub scroll_events_coalesced: u64,
    pub resize_events_received: u64,
    pub resize_events_coalesced: u64,
    pub external_updates_processed: u64,
    pub task_results_processed: u64,
    pub timers_processed: u64,
    pub frame_needed_before: bool,
    pub frame_needed_after: bool,
}

impl InputFlushOutcome {
    /// Reports whether this turn processed any queued runtime or input work.
    pub const fn processed_work(self) -> bool {
        self.events_dispatched != 0
            || self.external_updates_processed != 0
            || self.task_results_processed != 0
            || self.timers_processed != 0
    }

    /// Reports whether this turn changed frame demand from clean to dirty.
    pub const fn frame_became_needed(self) -> bool {
        !self.frame_needed_before && self.frame_needed_after
    }

    /// Reports that pointer movement was the only work that newly requested this frame.
    pub const fn pointer_move_only_frame_became_needed(self) -> bool {
        self.frame_became_needed()
            && self.events_received != 0
            && self.events_received == self.pointer_moves_received
            && self.external_updates_processed == 0
            && self.task_results_processed == 0
            && self.timers_processed == 0
    }
}

pub struct AppRuntimeCore<D: ComponentDriver> {
    view: ViewRuntime<D>,
    layout: LayoutEngine,
    text: RetainedTextSystem,
    scene: RenderScene,
    compiler: SceneCompiler,
    deltas: SceneDeltaQueue,
    input: InputCoalescer,
    extent: SizeF,
    scene_epoch: u64,
    interaction: InteractionRouter,
    theme: ThemeRuntime,
    motion_preference: MotionPreference,
}

pub type AppRuntime<C> = AppRuntimeCore<ComponentRuntimeDriver<C>>;
pub type ComposedAppRuntime = AppRuntimeCore<CompositionDriver>;

impl<C: Component> AppRuntimeCore<ComponentRuntimeDriver<C>> {
    pub fn new(component: C) -> AppResult<Self> {
        Self::with_extent(
            component,
            SizeI {
                width: 1280,
                height: 800,
            },
        )
    }

    pub fn with_extent(component: C, extent: SizeI) -> AppResult<Self> {
        Self::with_theme_and_extent(
            component,
            extent,
            ThemeRuntime::default(),
            ThemeDomain::Application,
        )
    }

    pub fn with_theme(component: C, theme: ThemeRuntime) -> AppResult<Self> {
        Self::with_theme_and_extent(
            component,
            SizeI {
                width: 1280,
                height: 800,
            },
            theme,
            ThemeDomain::Application,
        )
    }

    pub fn with_theme_and_extent(
        component: C,
        extent: SizeI,
        theme: ThemeRuntime,
        domain: ThemeDomain,
    ) -> AppResult<Self> {
        let view = ViewRuntime::from_component(component)?;
        Self::from_view(view, extent, theme, domain)
    }

    pub fn component_diagnostics(&self) -> ComponentDiagnostics {
        self.view.component_diagnostics()
    }

    pub fn send_component_action(&mut self, action: C::Action) -> AppResult<()> {
        self.view.send_component_action(action)?;
        self.sync_interaction();
        Ok(())
    }

    pub fn close(&mut self) -> AppResult<()> {
        self.view.unmount_component()?;
        self.sync_interaction();
        Ok(())
    }
}

impl AppRuntimeCore<CompositionDriver> {
    pub fn from_composed<C: crate::compose::Component>(component: C) -> AppResult<Self> {
        Self::from_composed_with_extent(
            component,
            SizeI {
                width: 1280,
                height: 800,
            },
        )
    }

    pub fn from_composed_with_extent<C: crate::compose::Component>(
        component: C,
        extent: SizeI,
    ) -> AppResult<Self> {
        Self::from_composed_with_theme_and_extent(
            component,
            extent,
            ThemeRuntime::default(),
            ThemeDomain::Application,
        )
    }

    pub fn from_composed_with_theme_and_extent<C: crate::compose::Component>(
        component: C,
        extent: SizeI,
        theme: ThemeRuntime,
        domain: ThemeDomain,
    ) -> AppResult<Self> {
        let view = ViewRuntime::from_composed(component)?;
        Self::from_view(view, extent, theme, domain)
    }

    #[cfg(any(
        feature = "application-software",
        feature = "shell-wayland-linux",
        all(feature = "application-vulkan-windows", target_os = "windows")
    ))]
    pub(crate) fn from_composition_driver(
        driver: CompositionDriver,
        extent: SizeI,
    ) -> AppResult<Self> {
        let view = ViewRuntime::new(driver)?;
        Self::from_view(
            view,
            extent,
            ThemeRuntime::default(),
            ThemeDomain::Application,
        )
    }

    pub fn composition_diagnostics(&self) -> CompositionDiagnostics {
        self.view.composition_diagnostics()
    }

    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    pub(crate) fn update_composition_root(
        &mut self,
        candidate: Box<dyn crate::compose::ErasedComponent>,
    ) -> AppResult<bool> {
        let changed = self.view.update_composition_root(candidate)?;
        if changed {
            self.sync_interaction();
        }
        Ok(changed)
    }

    pub(crate) fn shell_input(&mut self, event: crate::input::InputEvent) -> AppResult<()> {
        self.view.shell_input(event)?;
        self.sync_interaction();
        Ok(())
    }
    pub(crate) fn dismiss_shell_widget(
        &mut self,
        reason: crate::compose::ShellDismissReason,
    ) -> AppResult<()> {
        self.view.dismiss_shell_widget(reason)?;
        self.sync_interaction();
        Ok(())
    }
    pub fn close_composition(&mut self) -> AppResult<()> {
        self.view.unmount_composition()?;
        self.sync_interaction();
        Ok(())
    }
}

impl<D: ComponentDriver> AppRuntimeCore<D> {
    fn from_view(
        mut view: ViewRuntime<D>,
        extent: SizeI,
        theme: ThemeRuntime,
        domain: ThemeDomain,
    ) -> AppResult<Self> {
        let scope = ThemeRuntime::root_scope(domain);
        view.ui_mut().set_theme_domain(domain.id(), scope.id());
        let runtime = Self {
            view,
            layout: LayoutEngine::default(),
            text: RetainedTextSystem::new(100_000)
                .map_err(|error| AppError::new(error.to_string()))?,
            scene: RenderScene::default(),
            compiler: SceneCompiler::default(),
            deltas: SceneDeltaQueue::new(3),
            input: InputCoalescer::default(),
            extent: SizeF {
                width: extent.width.max(1) as f32,
                height: extent.height.max(1) as f32,
            },
            scene_epoch: 0,
            interaction: InteractionRouter::default(),
            theme,
            motion_preference: MotionPreference::Full,
        };
        Ok(runtime)
    }

    pub fn ui(&self) -> &MountedUi {
        self.view.ui()
    }

    pub fn interaction_diagnostics(&self) -> InteractionDiagnostics {
        self.interaction.diagnostics()
    }

    pub fn theme_runtime(&self) -> &ThemeRuntime {
        &self.theme
    }

    pub fn replace_theme(
        &mut self,
        scope: ThemeScope,
        replacement: CompiledTheme,
    ) -> ThemeResult<ThemeReplacement> {
        let result = self.theme.replace_theme(scope, replacement)?;
        if !result.changed_styles.is_empty() {
            self.view.scheduler_mut().request();
        }
        Ok(result)
    }

    pub fn set_motion_preference(&mut self, preference: MotionPreference) {
        if self.motion_preference != preference {
            self.motion_preference = preference;
            self.view.scheduler_mut().request();
        }
    }

    pub(crate) fn motion_preference(&self) -> MotionPreference {
        self.motion_preference
    }
    pub(crate) fn shell_hit_test(&mut self, point: PointF) -> bool {
        self.layout.hit_test(self.view.ui_mut(), point).is_some()
    }
    pub fn layout(&self) -> &LayoutEngine {
        &self.layout
    }

    pub fn extent(&self) -> SizeF {
        self.extent
    }

    /// Change glyph raster density while preserving logical layout and input coordinates.
    pub fn set_raster_scale(&mut self, scale: crate::platform::ScaleFactor) {
        if self.text.raster_scale() != scale {
            self.text.set_raster_scale(scale);
            self.compiler = SceneCompiler::default();
            self.view.scheduler_mut().request();
        }
    }

    pub fn resize(&mut self, extent: SizeI) -> AppResult<()> {
        if extent.width <= 0 || extent.height <= 0 {
            return Err(AppError::new("runtime extent must be positive"));
        }
        self.extent = SizeF {
            width: extent.width as f32,
            height: extent.height as f32,
        };
        self.view.scheduler_mut().request();
        Ok(())
    }

    pub fn ui_mut(&mut self) -> &mut MountedUi {
        self.view.ui_mut()
    }

    pub fn scheduler_mut(&mut self) -> &mut FrameScheduler {
        self.view.scheduler_mut()
    }

    pub fn drain_commands(&mut self) -> impl Iterator<Item = Command> + '_ {
        self.view.drain_commands()
    }

    pub fn pop_command(&mut self) -> Option<Command> {
        self.view.pop_command()
    }

    pub fn needs_frame(&self) -> bool {
        self.view.scheduler().needs_frame() || self.view.external_updates_ready()
    }

    /// Reports whether native input is waiting for the next bounded owner-thread turn.
    pub fn has_pending_input(&self) -> bool {
        self.input.has_pending()
    }

    /// Reports whether input, reactive updates, task results, or due timers need an owner turn.
    pub fn has_pending_runtime_turn(&self, now: MonotonicInstant) -> bool {
        self.input.has_pending()
            || self.view.external_updates_ready()
            || self.view.task_results_ready()
            || self.view.timers_ready(now)
    }

    /// Reports that the pending owner turn contains only coalescible pointer movement.
    ///
    /// Native hosts use this before dispatch to suppress profiler production when the live viewer
    /// has excluded high-rate pointer details. The input is still processed normally.
    #[cfg(any(feature = "profiler", all(test, feature = "application-software")))]
    pub(crate) fn pending_runtime_turn_is_pointer_move_only(&self, now: MonotonicInstant) -> bool {
        self.input.has_only_pending_pointer_moves()
            && !self.view.external_updates_ready()
            && !self.view.task_results_ready()
            && !self.view.timers_ready(now)
    }

    /// Returns the earliest runtime-owned timer deadline in the host's monotonic domain.
    pub fn next_deadline(&self) -> Option<MonotonicInstant> {
        self.view.scheduler().next_deadline()
    }

    /// Reports whether the current view has motion that needs successor frames.
    pub fn animation_active(&self) -> bool {
        self.view.scheduler().animation_active()
    }

    pub fn queue_input(&mut self, event: impl Into<PlatformInput>) {
        self.input.push(event.into());
    }

    /// Cancels an in-flight pointer interaction without producing an activation.
    pub fn cancel_pointer(&mut self, pointer: PointerId) {
        if self.interaction.cancel_pointer(self.view.ui_mut(), pointer) {
            self.view.scheduler_mut().request();
        }
    }

    /// Reports that native pointer capture was lost and clears the owning control's transient state.
    pub fn pointer_capture_lost(&mut self, pointer: PointerId) {
        if self.interaction.capture_lost(self.view.ui_mut(), pointer) {
            self.view.scheduler_mut().request();
        }
    }

    /// Hands an armed pointer to a competing gesture without allowing the later release to click.
    pub fn pointer_gesture_claimed(&mut self, pointer: PointerId, gesture: CompetingGesture) {
        if self
            .interaction
            .gesture_claimed(self.view.ui_mut(), pointer, gesture)
        {
            self.view.scheduler_mut().request();
        }
    }

    /// Cancels all transient interaction state when the containing native view deactivates.
    pub fn deactivate_view(&mut self, timestamp: MonotonicInstant) {
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

    pub fn flush_input(&mut self, timestamp: MonotonicInstant) -> InputFlushOutcome {
        #[cfg(feature = "profiler")]
        let _input_span = crate::profiler::span!("input.dispatch");
        let frame_needed_before = self.view.scheduler().needs_frame();
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
        let events_dispatched = events.len() as u64;
        for event in events.drain(..) {
            match event {
                PlatformInput::Resize(size) => {
                    self.extent = size;
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
                        self.view.dispatch_ui(
                            target,
                            UiEventKind::Input(event),
                            LISTEN_POINTER,
                            timestamp_ns,
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
                        self.view.dispatch_ui(
                            target,
                            UiEventKind::Input(event),
                            LISTEN_POINTER,
                            timestamp_ns,
                        );
                    }
                }
                PlatformInput::Input(event @ InputEvent::Scroll { pointer, .. }) => {
                    let position = self
                        .interaction
                        .pointer_position(pointer)
                        .unwrap_or_default();
                    let hit = self.layout.hit_test(self.view.ui_mut(), position);
                    if let Some(target) = hit {
                        self.view.dispatch_ui(
                            target,
                            UiEventKind::Input(event),
                            LISTEN_POINTER,
                            timestamp_ns,
                        );
                    }
                }
                PlatformInput::Input(InputEvent::Key(key)) => {
                    if let Some(target) = self.interaction.focused() {
                        self.view.dispatch_ui(
                            target,
                            UiEventKind::Input(InputEvent::Key(key.clone())),
                            LISTEN_KEY,
                            timestamp_ns,
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
            crate::profiler::counter!("input.events.received", outcome.events_received);
            crate::profiler::counter!(
                "input.non_pointer_events.received",
                outcome
                    .events_received
                    .saturating_sub(outcome.pointer_moves_received)
            );
            crate::profiler::counter!("input.events.dispatched", outcome.events_dispatched);
            crate::profiler::counter!(
                "input.pointer_moves.coalesced",
                outcome.pointer_moves_coalesced
            );
            crate::profiler::counter!(
                "input.scroll_events.coalesced",
                outcome.scroll_events_coalesced
            );
            crate::profiler::counter!(
                "input.resize_events.coalesced",
                outcome.resize_events_coalesced
            );
            crate::profiler::counter!(
                "runtime.external_updates.processed",
                outcome.external_updates_processed
            );
            crate::profiler::counter!(
                "runtime.task_results.processed",
                outcome.task_results_processed
            );
            crate::profiler::counter!("runtime.timers.processed", outcome.timers_processed);
        }
        outcome
    }

    pub fn prepare_frame(
        &mut self,
        now: MonotonicInstant,
        force: bool,
    ) -> AppResult<PreparedFrame> {
        #[cfg(feature = "profiler")]
        let _prepare_span = crate::profiler::span!("frame.prepare");
        if self.view.external_updates_ready() {
            self.view.process_external_updates();
            self.sync_interaction();
        }
        if !force && !self.view.scheduler().needs_frame() {
            return Ok(PreparedFrame {
                scene_epoch: self.scene_epoch,
                ..PreparedFrame::default()
            });
        }
        let theme = {
            #[cfg(feature = "profiler")]
            let _span = crate::profiler::span!("theme.resolve");
            self.theme
                .update_styles(self.view.ui_mut(), now, self.motion_preference)
        };
        self.view
            .scheduler_mut()
            .set_animation_active(theme.active_animations);
        if theme.changed {
            self.view.scheduler_mut().request();
        }
        self.view.scheduler_mut().begin_frame();
        let layout = {
            #[cfg(feature = "profiler")]
            let _span = crate::profiler::span!("layout.update");
            self.layout
                .update(self.view.ui_mut(), &mut self.text, self.extent, 1.0)
        };
        let compile = {
            #[cfg(feature = "profiler")]
            let _span = crate::profiler::span!("scene.compile");
            self.compiler.compile(
                self.view.ui_mut(),
                &self.layout,
                &mut self.text,
                &mut self.scene,
                self.extent,
                crate::core::ColorRgba8::default(),
            )
        };
        let delta = {
            #[cfg(feature = "profiler")]
            let _span = crate::profiler::span!("scene.delta.take");
            self.scene.take_delta()
        };
        let changed = if let Some(delta) = delta {
            #[cfg(feature = "profiler")]
            let _span = crate::profiler::span!("scene.delta.enqueue");
            self.scene_epoch = delta.epoch;
            self.deltas.push(delta);
            true
        } else {
            false
        };
        #[cfg(feature = "profiler")]
        {
            crate::profiler::counter!("scene.delta.queue.high_water", self.deltas.high_water());
            crate::profiler::counter!("scene.epoch", self.scene_epoch);
        }
        Ok(PreparedFrame {
            changed,
            scene_epoch: self.scene_epoch,
            diagnostics: FrameDiagnostics {
                layout,
                compile,
                theme: theme.diagnostics,
                delta_queue_high_water: self.deltas.high_water(),
            },
        })
    }

    pub fn pop_scene_delta(&mut self) -> Option<RenderSceneDelta> {
        #[cfg(feature = "profiler")]
        let _span = crate::profiler::span!("transport.dequeue");
        self.deltas.pop()
    }

    pub fn set_image_resource(&mut self, resource: ImageResource) -> AppResult<()> {
        self.scene
            .set_image_resource(resource)
            .map_err(|error| AppError::new(error.to_string()))?;
        self.view.scheduler_mut().request();
        Ok(())
    }

    pub fn update_image_resource_region(&mut self, update: ImageResourceUpdate) -> AppResult<()> {
        self.scene
            .update_image_resource_region(update)
            .map_err(|error| AppError::new(error.to_string()))?;
        self.view.scheduler_mut().request();
        Ok(())
    }

    pub fn remove_image_resource(&mut self, image: ImageId) -> bool {
        let removed = self.scene.remove_image_resource(image);
        if removed {
            self.view.scheduler_mut().request();
        }
        removed
    }

    pub fn set_material_resource(&mut self, resource: MaterialResource) {
        self.scene.set_material_resource(resource);
        self.view.scheduler_mut().request();
    }

    pub fn remove_material_resource(&mut self, material: MaterialId) -> bool {
        let removed = self.scene.remove_material_resource(material);
        if removed {
            self.view.scheduler_mut().request();
        }
        removed
    }

    pub fn scene_snapshot(&mut self) -> RenderSceneDelta {
        let atlas = self.text.atlas();
        let atlas_extent = SizeI {
            width: atlas.width_px,
            height: atlas.height_px,
        };
        let delta = self
            .scene
            .snapshot_delta(atlas_extent, vec![self.text.atlas_snapshot()]);
        self.scene_epoch = delta.epoch;
        delta
    }

    fn apply_pointer_routing(
        &mut self,
        routing: crate::application_host::interaction::PointerRouting,
        position: PointF,
        timestamp: u64,
    ) {
        if let Some(focus) = routing.focus {
            self.dispatch_focus_change(focus, timestamp);
        }
        if let Some((target, phase)) = routing.value {
            self.dispatch_pointer_value(target, position, phase, ChangeSource::Pointer);
        }
        if let Some((target, activation)) = routing.activation {
            self.view.dispatch_activation(target, activation.source);
        }
        if routing.changed || routing.activation.is_some() || routing.value.is_some() {
            self.view.scheduler_mut().request();
        }
    }

    fn move_focus(&mut self, backwards: bool, timestamp: u64) {
        let order = self.layout.focus_order(self.view.ui_mut());
        let target = if order.is_empty() {
            None
        } else {
            let next = self
                .interaction
                .focused()
                .and_then(|focused| order.iter().position(|node| *node == focused))
                .map(|index| {
                    if backwards {
                        order[(index + order.len() - 1) % order.len()]
                    } else {
                        order[(index + 1) % order.len()]
                    }
                })
                .unwrap_or_else(|| {
                    if backwards {
                        *order.last().expect("focus order is nonempty")
                    } else {
                        order[0]
                    }
                });
            Some(next)
        };
        let change = self.interaction.set_focus(self.view.ui_mut(), target, true);
        self.dispatch_focus_change(change, timestamp);
        self.view.scheduler_mut().request();
    }

    fn dispatch_focus_change(&mut self, change: FocusChange, timestamp: u64) {
        if change.old == change.new {
            return;
        }
        if let Some(old) = change.old {
            self.view
                .dispatch_ui(old, UiEventKind::Focus(false), LISTEN_FOCUS, timestamp);
        }
        if let Some(new) = change.new {
            self.view
                .dispatch_ui(new, UiEventKind::Focus(true), LISTEN_FOCUS, timestamp);
        }
    }

    fn dispatch_pointer_value(
        &mut self,
        node: NodeId,
        position: PointF,
        phase: ValueChangePhase,
        source: ChangeSource,
    ) -> bool {
        let Some(interaction) = self.view.ui().interactions.get(node) else {
            return false;
        };
        let (Some(track), Some(axis)) = (interaction.value_track, interaction.value_axis) else {
            return false;
        };
        let Some(layout) = self.layout.computed(track) else {
            return false;
        };
        let rect = layout.border_rect;
        let mut value = match axis {
            ValueAxis::Horizontal { .. } if rect.width > 0.0 => (position.x - rect.x) / rect.width,
            ValueAxis::Vertical { .. } if rect.height > 0.0 => (position.y - rect.y) / rect.height,
            ValueAxis::Horizontal { .. } | ValueAxis::Vertical { .. } => return false,
        }
        .clamp(0.0, 1.0);
        if matches!(
            axis,
            ValueAxis::Horizontal { inverted: true } | ValueAxis::Vertical { inverted: true }
        ) {
            value = 1.0 - value;
        }
        self.view.dispatch_value(node, value, phase, source)
    }

    fn sync_interaction(&mut self) {
        if self.interaction.sync(self.view.ui_mut()) {
            self.view.scheduler_mut().request();
        }
    }
}

#[cfg(all(test, feature = "application-software"))]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::core::{ColorRgba8, RectI};
    use crate::input::{KeyEvent, LogicalKey, PhysicalKey, PhysicalKeyCode};
    use crate::render::{
        ReadbackFormat, ReadbackRequest, RenderBackend, RenderRequest, RenderStats,
        RenderTargetInfo, TargetLoad, TargetStore,
    };
    use crate::renderer_software::{
        SoftwareRenderer, SoftwareScene, SoftwareSurface, SoftwareTarget,
    };
    use crate::runtime::{CreateContext, Ui, UpdateContext};
    use crate::ui::{
        Background, BoxStyle, ControlHandle, EventPhase, LayoutStyle, SizeRule, UiEvent, UiRoot,
    };

    #[derive(Clone)]
    struct TestComponent {
        button: Rc<Cell<Option<ControlHandle>>>,
        actions: Rc<Cell<usize>>,
    }

    impl Component for TestComponent {
        type State = ();
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &(), ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui.foundation().root(
                BoxStyle {
                    width: SizeRule::Fill(1.0),
                    height: SizeRule::Fill(1.0),
                    ..BoxStyle::default()
                },
                LayoutStyle::default(),
                |_| {},
            );
            let button = ui
                .button(
                    root.0,
                    || (),
                    BoxStyle {
                        width: SizeRule::Logical(80.0),
                        height: SizeRule::Logical(30.0),
                        decoration: crate::ui::BoxDecoration {
                            background: Background::Color(ColorRgba8::rgba(10, 20, 30, 255)),
                            ..crate::ui::BoxDecoration::default()
                        },
                        ..BoxStyle::default()
                    },
                    |writer| {
                        writer.text("Click", ColorRgba8::rgba(245, 247, 252, 255), 14.0);
                    },
                )
                .unwrap();
            self.button.set(Some(button));
            root
        }

        fn action(
            &self,
            _state: &mut (),
            _action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            self.actions.set(self.actions.get() + 1);
        }
    }

    struct SoftwareHarness {
        runtime: AppRuntime<TestComponent>,
        renderer: SoftwareRenderer,
        scene: SoftwareScene,
        surface: SoftwareSurface,
        render_calls: usize,
        now: u64,
    }

    impl SoftwareHarness {
        fn new(component: TestComponent) -> Self {
            let renderer = SoftwareRenderer;
            Self {
                runtime: AppRuntime::new(component).unwrap(),
                scene: renderer.create_scene().unwrap(),
                renderer,
                surface: SoftwareSurface::default(),
                render_calls: 0,
                now: 0,
            }
        }

        fn frame(&mut self, force: bool) -> (PreparedFrame, RenderStats) {
            let prepared = self
                .runtime
                .prepare_frame(MonotonicInstant::from_nanos(self.now), force)
                .unwrap();
            self.now = self.now.saturating_add(200_000_000);
            while let Some(delta) = self.runtime.pop_scene_delta() {
                self.renderer
                    .apply_scene_delta(&mut self.scene, &delta)
                    .unwrap();
            }
            if !force && !prepared.changed {
                return (
                    prepared,
                    RenderStats {
                        epoch: prepared.scene_epoch,
                        ..RenderStats::default()
                    },
                );
            }
            let logical_extent = self.runtime.extent();
            let extent = SizeI {
                width: logical_extent.width.ceil().max(1.0) as i32,
                height: logical_extent.height.ceil().max(1.0) as i32,
            };
            let target = SoftwareTarget::new(RenderTargetInfo::full(extent));
            let clear = self.scene.background();
            let mut frame = self.surface.begin_frame();
            let stats = self
                .renderer
                .render(
                    &mut self.scene,
                    &mut frame,
                    &target,
                    &RenderRequest {
                        force,
                        load: TargetLoad::Clear(clear),
                        store: TargetStore::Store,
                        region: None,
                    },
                )
                .unwrap();
            self.render_calls += 1;
            (prepared, stats)
        }

        fn pixels(&self) -> Vec<u8> {
            let extent = self.surface.framebuffer_extent();
            self.surface
                .readback(&ReadbackRequest {
                    region: RectI {
                        x: 0,
                        y: 0,
                        width: extent.width,
                        height: extent.height,
                    },
                    format: ReadbackFormat::Rgba8,
                })
                .unwrap()
                .pixels
        }
    }

    type Fixture = (
        TestComponent,
        Rc<Cell<Option<ControlHandle>>>,
        Rc<Cell<usize>>,
    );

    fn fixture() -> Fixture {
        let button = Rc::new(Cell::new(None));
        let actions = Rc::new(Cell::new(0));
        (
            TestComponent {
                button: button.clone(),
                actions: actions.clone(),
            },
            button,
            actions,
        )
    }

    #[test]
    fn root_component_mounts_once_and_receives_activation() {
        let (component, _, actions) = fixture();
        let mut harness = SoftwareHarness::new(component);
        harness.frame(true);
        let nodes = harness.runtime.ui().nodes.alive().len();
        harness
            .runtime
            .queue_input(InputEvent::mouse_moved(PointF { x: 10.0, y: 10.0 }));
        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Pressed,
        ));
        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Released,
        ));
        harness.runtime.flush_input(MonotonicInstant::from_nanos(1));
        harness.frame(false);
        assert_eq!(harness.runtime.ui().nodes.alive().len(), nodes);
        assert_eq!(actions.get(), 1);
    }

    #[test]
    fn focused_controls_activate_from_enter_and_space_keyboard_defaults() {
        let (component, button, actions) = fixture();
        let mut harness = SoftwareHarness::new(component);
        harness.frame(true);
        harness
            .runtime
            .queue_input(InputEvent::mouse_moved(PointF { x: 10.0, y: 10.0 }));
        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Pressed,
        ));
        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Released,
        ));
        harness.runtime.flush_input(MonotonicInstant::from_nanos(1));
        assert_eq!(actions.get(), 1);
        let node = button.get().unwrap().node;
        assert!(
            !harness
                .runtime
                .ui()
                .interactions
                .get(node)
                .unwrap()
                .flags
                .contains(crate::ui::InteractionFlags::FOCUS_VISIBLE)
        );

        harness.runtime.queue_input(InputEvent::Key(
            KeyEvent::new(
                PhysicalKey::from_code(PhysicalKeyCode::Enter),
                ButtonState::Pressed,
            )
            .with_logical_key(LogicalKey::Named(NamedKey::Enter)),
        ));
        harness.runtime.flush_input(MonotonicInstant::from_nanos(2));
        assert_eq!(actions.get(), 2);
        assert!(
            harness
                .runtime
                .ui()
                .interactions
                .get(node)
                .unwrap()
                .flags
                .contains(crate::ui::InteractionFlags::FOCUS_VISIBLE)
        );

        for state in [ButtonState::Pressed, ButtonState::Released] {
            harness.runtime.queue_input(InputEvent::Key(
                KeyEvent::new(PhysicalKey::from_code(PhysicalKeyCode::Space), state)
                    .with_logical_key(LogicalKey::Named(NamedKey::Space)),
            ));
        }
        harness.runtime.flush_input(MonotonicInstant::from_nanos(3));
        assert_eq!(actions.get(), 3);
    }

    #[test]
    fn hover_press_focus_and_disabled_states_repaint_button_pixels() {
        let (component, button, _) = fixture();
        let mut harness = SoftwareHarness::new(component);
        harness.frame(true);
        let framebuffer = harness.surface.pixels_rgba8().as_ptr();
        let normal = harness.pixels();

        harness
            .runtime
            .queue_input(InputEvent::mouse_moved(PointF { x: 10.0, y: 10.0 }));
        harness.runtime.flush_input(MonotonicInstant::from_nanos(1));
        // The first requested frame establishes the transition at its exact start value.
        harness.frame(false);
        let (_, hover_stats) = harness.frame(false);
        let hovered = harness.pixels();
        assert!(hover_stats.recorded);
        assert!(hover_stats.damage_area < 10_000.0);
        assert!(!harness.surface.presented_damage().full);
        assert_eq!(harness.surface.pixels_rgba8().as_ptr(), framebuffer);
        assert_ne!(hovered, normal);

        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Pressed,
        ));
        harness.runtime.flush_input(MonotonicInstant::from_nanos(2));
        harness.frame(false);
        let (_, press_stats) = harness.frame(false);
        let pressed = harness.pixels();
        assert!(press_stats.recorded);
        assert_ne!(pressed, hovered);

        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Released,
        ));
        harness.runtime.flush_input(MonotonicInstant::from_nanos(3));
        harness.frame(false);
        harness.frame(false);
        let focused = harness.pixels();
        // Pointer focus is deliberately not focus-visible and therefore preserves hover paint.
        assert_eq!(focused, hovered);

        let button = button.get().unwrap();
        let interaction = harness.runtime.ui().interactions.get(button.node).unwrap();
        assert!(
            interaction
                .flags
                .contains(crate::ui::InteractionFlags::FOCUSED)
        );
        assert!(
            !interaction
                .flags
                .contains(crate::ui::InteractionFlags::FOCUS_VISIBLE)
        );
        harness
            .runtime
            .ui_mut()
            .transaction(|transaction| transaction.set(button.enabled, false));
        harness.frame(true);
        harness.frame(false);
        assert_ne!(harness.pixels(), focused);
    }

    #[test]
    fn idle_runtime_prepares_and_records_no_frame() {
        let (component, _, _) = fixture();
        let mut harness = SoftwareHarness::new(component);
        harness.frame(false);
        let render_calls = harness.render_calls;
        let (prepared, stats) = harness.frame(false);
        assert!(!prepared.changed);
        assert!(!stats.recorded);
        assert_eq!(harness.render_calls, render_calls);
    }

    #[test]
    fn clean_pointer_bursts_coalesce_without_requesting_a_frame() {
        let (component, _, _) = fixture();
        let mut harness = SoftwareHarness::new(component);
        harness.frame(true);

        harness
            .runtime
            .queue_input(InputEvent::mouse_moved(PointF { x: 10.0, y: 10.0 }));
        let hover = harness.runtime.flush_input(MonotonicInstant::from_nanos(1));
        assert!(hover.frame_became_needed());
        assert!(hover.pointer_move_only_frame_became_needed());
        harness.frame(false);
        harness.frame(false);
        assert!(!harness.runtime.needs_frame());

        for value in 0..10_000 {
            harness.runtime.queue_input(InputEvent::mouse_moved(PointF {
                x: 10.0 + (value % 2) as f32,
                y: 10.0,
            }));
        }
        assert!(
            harness
                .runtime
                .pending_runtime_turn_is_pointer_move_only(MonotonicInstant::from_nanos(2))
        );
        let outcome = harness.runtime.flush_input(MonotonicInstant::from_nanos(2));

        assert_eq!(outcome.events_received, 10_000);
        assert_eq!(outcome.events_dispatched, 1);
        assert_eq!(outcome.pointer_moves_coalesced, 9_999);
        assert!(!outcome.frame_became_needed());
        assert!(!outcome.pointer_move_only_frame_became_needed());
        assert!(!outcome.frame_needed_after);
        assert!(!harness.runtime.needs_frame());
    }

    #[test]
    fn releasing_pointer_outside_capture_does_not_activate() {
        let (component, _, actions) = fixture();
        let mut harness = SoftwareHarness::new(component);
        harness.frame(true);
        harness
            .runtime
            .queue_input(InputEvent::mouse_moved(PointF { x: 10.0, y: 10.0 }));
        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Pressed,
        ));
        harness
            .runtime
            .queue_input(InputEvent::mouse_moved(PointF { x: 150.0, y: 150.0 }));
        harness.runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Released,
        ));
        harness.runtime.flush_input(MonotonicInstant::from_nanos(2));
        assert_eq!(actions.get(), 0);
    }

    struct RoutedComponent {
        routes: Rc<RefCell<Vec<EventPhase>>>,
    }

    impl Component for RoutedComponent {
        type State = ();
        type Action = EventPhase;

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &(), ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let parent = ui
                .foundation()
                .container_node_under(root.0, BoxStyle::default(), LayoutStyle::default(), |_| {})
                .unwrap()
                .node;
            let target = ui
                .foundation()
                .container_node_under(
                    parent,
                    BoxStyle {
                        width: SizeRule::Logical(80.0),
                        height: SizeRule::Logical(30.0),
                        ..BoxStyle::default()
                    },
                    LayoutStyle::default(),
                    |_| {},
                )
                .unwrap()
                .node;
            ui.listen(parent, LISTEN_POINTER, |event: &UiEvent| event.phase)
                .unwrap();
            ui.listen(target, LISTEN_POINTER, |event: &UiEvent| event.phase)
                .unwrap();
            root
        }

        fn action(
            &self,
            _state: &mut (),
            action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            self.routes.borrow_mut().push(action);
        }
    }

    #[test]
    fn capture_target_and_bubble_report_component_routes() {
        let routes = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = AppRuntime::new(RoutedComponent {
            routes: routes.clone(),
        })
        .unwrap();
        runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
        runtime.queue_input(InputEvent::mouse_moved(PointF { x: 10.0, y: 10.0 }));
        runtime.flush_input(MonotonicInstant::from_nanos(1));
        routes.borrow_mut().clear();
        runtime.queue_input(InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Pressed,
        ));
        runtime.flush_input(MonotonicInstant::from_nanos(3));
        assert_eq!(
            *routes.borrow(),
            vec![EventPhase::Capture, EventPhase::Target, EventPhase::Bubble]
        );
    }
}
