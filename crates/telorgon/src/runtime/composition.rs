//! Dirty-component reconciliation from short-lived composition elements into retained UI nodes.

mod mount;
mod reconcile;
mod signals;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use crate::authoring::compose::{
    ButtonElement, Component, ComponentInstanceId, ContainerElement, Element, ElementKind,
    ElementType, ErasedComponent, EventDispatch, EventHandler, ImageElement, Key, RenderedView,
    RuntimeTarget, SignalDependency, SignalSubscription, SliderElement, TextElement, ToggleElement,
    ToggleKind, ViewError,
};
use crate::foundation::{ColorRgba8, EdgeInsets, PointF, Transform2D};
use crate::input::{ChangeSource, ValueChangePhase};
use crate::ui::{
    Background, Border, BoxSizing, BoxStyle, ComponentStyleId, CornerRadii, Flow, LayoutStyle,
    MountWriter, SemanticActions, SemanticCheckState, SemanticName, SemanticNode, SemanticRole,
    SemanticState, SemanticValue, SizeRule, SizeRule2D, StyleBinding, StylePropertyPatch,
    StyleSlotId, ThemeDomainId, ThemeScopeId, UiEvent, UiNodeId, UiRoot, ValueAxis,
};

use crate::runtime::{ComponentDriver, RuntimeError, context::DriverContext};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompositionDiagnostics {
    pub live_components: usize,
    pub view_evaluations: u64,
    pub components_mounted: u64,
    pub components_reused: u64,
    pub components_unmounted: u64,
    pub elements_mounted: u64,
    pub elements_reused: u64,
    pub elements_removed: u64,
    pub invalid_views: u64,
    pub events_delivered: u64,
    pub stale_events: u64,
    pub input_mutations_restored: u64,
    pub externally_invalidated_components: u64,
    pub externally_reconciled_components: u64,
}

struct ComponentSlot {
    generation: u32,
    component: Option<Box<dyn ErasedComponent>>,
    child: Option<Box<MountedElement>>,
    signal_subscriptions: Vec<SignalSubscription>,
}

impl Default for ComponentSlot {
    fn default() -> Self {
        Self {
            generation: 1,
            component: None,
            child: None,
            signal_subscriptions: Vec::new(),
        }
    }
}

struct CompositionArena {
    slots: Vec<ComponentSlot>,
    free: Vec<u32>,
}

impl CompositionArena {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    fn insert(&mut self, component: Box<dyn ErasedComponent>) -> ComponentInstanceId {
        let index = if let Some(index) = self.free.pop() {
            index
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(ComponentSlot::default());
            index
        };
        let slot = &mut self.slots[index as usize];
        slot.component = Some(component);
        slot.child = None;
        slot.signal_subscriptions.clear();
        ComponentInstanceId::new(index, slot.generation)
    }

    fn get(&self, id: ComponentInstanceId) -> Option<&ComponentSlot> {
        self.slots
            .get(id.index() as usize)
            .filter(|slot| slot.generation == id.generation() && slot.component.is_some())
    }

    fn get_mut(&mut self, id: ComponentInstanceId) -> Option<&mut ComponentSlot> {
        self.slots
            .get_mut(id.index() as usize)
            .filter(|slot| slot.generation == id.generation() && slot.component.is_some())
    }

    fn remove(&mut self, id: ComponentInstanceId) -> Option<Box<dyn ErasedComponent>> {
        let slot = self.get_mut(id)?;
        slot.child = None;
        slot.signal_subscriptions.clear();
        let component = slot.component.take();
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(id.index());
        component
    }

    fn live(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.component.is_some())
            .count()
    }
}

struct MountedElement {
    key: Option<Key>,
    kind: MountedKind,
}

enum MountedKind {
    Container {
        node: UiNodeId,
        style: BoxStyle,
        layout: crate::ui::LayoutStyle,
        children: Vec<MountedElement>,
    },
    Text {
        node: UiNodeId,
        props: TextElement,
    },
    Image {
        node: UiNodeId,
        props: ImageElement,
    },
    Button {
        node: UiNodeId,
        children: Vec<MountedElement>,
        props: ButtonElement,
    },
    Checkbox {
        node: UiNodeId,
        indicator: UiNodeId,
        check_first: UiNodeId,
        check_second: UiNodeId,
        mixed: UiNodeId,
        label: UiNodeId,
        props: ToggleElement,
    },
    Switch {
        node: UiNodeId,
        track: UiNodeId,
        thumb: UiNodeId,
        label: UiNodeId,
        props: ToggleElement,
    },
    Slider {
        before_thumb: UiNodeId,
        after_thumb: UiNodeId,
        node: UiNodeId,
        track: UiNodeId,
        fill: UiNodeId,
        thumb: UiNodeId,
        label: UiNodeId,
        props: SliderElement,
    },
    Component {
        id: ComponentInstanceId,
        type_id: std::any::TypeId,
    },
}

impl MountedElement {
    fn element_type(&self) -> ElementType {
        match self.kind {
            MountedKind::Container { .. } => ElementType::Container,
            MountedKind::Text { .. } => ElementType::Text,
            MountedKind::Image { .. } => ElementType::Image,
            MountedKind::Button { .. } => ElementType::Button,
            MountedKind::Checkbox { .. } => ElementType::Checkbox,
            MountedKind::Switch { .. } => ElementType::Switch,
            MountedKind::Slider { .. } => ElementType::Slider,
            MountedKind::Component { type_id, .. } => ElementType::Component(type_id),
        }
    }
}

/// Runtime driver for one persistent composition root.
pub struct CompositionDriver {
    pub(crate) typography: crate::Typography,
    pending_root: Option<Box<dyn ErasedComponent>>,
    arena: CompositionArena,
    root_component: Option<ComponentInstanceId>,
    view_root: Option<UiRoot>,
    handlers: HashMap<UiNodeId, HandlerRoute>,
    diagnostics: CompositionDiagnostics,
    last_error: Option<RuntimeError>,
    signal_invalidations: Arc<Mutex<Vec<ComponentInstanceId>>>,
    signal_scratch: Vec<ComponentInstanceId>,
    wake: Arc<RwLock<Option<Arc<dyn Fn() + Send + Sync>>>>,
    target: RuntimeTarget,
    shell_services: Option<crate::authoring::compose::ShellServices>,
    pub(crate) image_bindings: crate::authoring::compose::context::ImageBindings,
}

#[derive(Clone)]
enum HandlerRoute {
    Activate(EventHandler),
    Toggle {
        handler: EventHandler,
        value: SemanticCheckState,
    },
    Value(EventHandler),
}

impl HandlerRoute {
    fn owner(&self) -> ComponentInstanceId {
        match self {
            Self::Activate(handler) | Self::Value(handler) => handler.owner(),
            Self::Toggle { handler, .. } => handler.owner(),
        }
    }
}

impl CompositionDriver {
    pub fn new<C: Component>(component: C) -> Self {
        Self::for_target(component, RuntimeTarget::Application)
    }

    pub fn for_target<C: Component>(component: C, target: RuntimeTarget) -> Self {
        Self::from_erased_for_target(Box::new(component), target)
    }

    #[doc(hidden)]
    pub fn from_erased(component: Box<dyn ErasedComponent>) -> Self {
        Self::from_erased_for_target(component, RuntimeTarget::Application)
    }

    pub(crate) fn from_erased_for_target(
        component: Box<dyn ErasedComponent>,
        target: RuntimeTarget,
    ) -> Self {
        Self {
            typography: Default::default(),
            pending_root: Some(component),
            arena: CompositionArena::new(),
            root_component: None,
            view_root: None,
            handlers: HashMap::new(),
            diagnostics: CompositionDiagnostics::default(),
            last_error: None,
            signal_invalidations: Arc::new(Mutex::new(Vec::new())),
            signal_scratch: Vec::new(),
            wake: Arc::new(RwLock::new(None)),
            target,
            shell_services: None,
            image_bindings: Default::default(),
        }
    }

    /// Installs the host-turn wake used by external signals. Replacing it is safe because signal
    /// subscriptions consult this shared slot at publication time.
    pub(crate) fn connect_shell(&mut self, services: crate::authoring::compose::ShellServices) {
        self.shell_services = Some(services.clone());
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        if let Some(root) = self.pending_root.as_mut() {
            root.shell_connected(services);
        }
    }

    pub fn set_wake(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        *self.wake.write().expect("composition wake lock poisoned") = Some(Arc::new(wake));
    }

    pub(crate) fn shell_input(
        &mut self,
        context: &mut DriverContext<'_>,
        event: crate::input::InputEvent,
    ) {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        let Some(root) = self.root_component else {
            return;
        };
        if !self
            .arena
            .get_mut(root)
            .and_then(|s| s.component.as_deref_mut())
            .is_some_and(|c| c.shell_input(event))
        {
            return;
        }
        if let Err(error) = self.reconcile_component(context.ui, root) {
            self.record_error(error);
        }
        *context.frame_requested = true;
    }
    pub(crate) fn dismiss_shell_widget(
        &mut self,
        context: &mut DriverContext<'_>,
        reason: crate::authoring::compose::ShellDismissReason,
    ) {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        let Some(root) = self.root_component else {
            return;
        };
        if let Some(component) = self
            .arena
            .get_mut(root)
            .and_then(|s| s.component.as_deref_mut())
        {
            component.shell_dismissed(reason);
        }
        if let Err(error) = self.reconcile_component(context.ui, root) {
            self.record_error(error);
        }
        *context.frame_requested = true;
    }

    pub fn diagnostics(&self) -> CompositionDiagnostics {
        let mut diagnostics = self.diagnostics;
        diagnostics.live_components = self.arena.live();
        diagnostics
    }

    pub const fn target(&self) -> RuntimeTarget {
        self.target
    }

    pub fn take_error(&mut self) -> Option<RuntimeError> {
        self.last_error.take()
    }

    #[cfg(any(test, all(feature = "shell-wayland-linux", target_os = "linux")))]
    pub(crate) fn update_root_candidate(
        &mut self,
        context: &mut DriverContext<'_>,
        candidate: Box<dyn ErasedComponent>,
    ) -> bool {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        let Some(root) = self.root_component else {
            self.record_error("composition root is not mounted");
            return false;
        };
        let same_type = self
            .arena
            .get(root)
            .and_then(|slot| slot.component.as_deref())
            .is_some_and(|component| {
                component.component_type_id() == candidate.component_type_id()
            });
        let result = if same_type {
            self.update_component_candidate(context.ui, root, candidate)
        } else {
            self.replace_root_candidate(context.ui, root, candidate)
        };
        match result {
            Ok(changed) => {
                *context.frame_requested |= changed;
                changed
            }
            Err(error) => {
                self.record_error(error);
                false
            }
        }
    }

    #[cfg(any(test, all(feature = "shell-wayland-linux", target_os = "linux")))]
    fn replace_root_candidate(
        &mut self,
        ui: &mut crate::ui::MountedUi,
        previous: ComponentInstanceId,
        candidate: Box<dyn ErasedComponent>,
    ) -> Result<bool, ViewError> {
        let parent = self.view_root.ok_or(ViewError::StaleParent)?.0;
        let next = self.arena.insert(candidate);
        let rendered = match self.render_component(next) {
            Ok(rendered) => rendered,
            Err(error) => {
                self.arena.remove(next);
                return Err(error);
            }
        };
        let Some(mut writer) = MountWriter::under(ui, parent) else {
            self.arena.remove(next);
            return Err(ViewError::StaleParent);
        };
        let mounted = match self.mount_element(&mut writer, rendered.element, next) {
            Ok(mounted) => mounted,
            Err(error) => {
                self.arena.remove(next);
                return Err(error);
            }
        };
        if let Err(error) = self.commit_signal_dependencies(next, rendered.signals) {
            self.remove_mounted(ui, mounted);
            self.arena.remove(next);
            return Err(error);
        }
        self.arena
            .get_mut(next)
            .ok_or(ViewError::StaleParent)?
            .child = Some(Box::new(mounted));
        let requested = self
            .arena
            .get_mut(next)
            .and_then(|slot| slot.component.as_deref_mut())
            .ok_or(ViewError::StaleParent)?
            .mounted_erased(next);
        if requested {
            self.signal_invalidations
                .lock()
                .expect("composition invalidation queue poisoned")
                .push(next);
        }

        if let Some(child) = self
            .arena
            .get_mut(previous)
            .and_then(|slot| slot.child.take())
        {
            self.remove_mounted(ui, *child);
        }
        if let Some(component) = self
            .arena
            .get_mut(previous)
            .and_then(|slot| slot.component.as_deref_mut())
        {
            component.unmounted_erased(previous);
        }
        self.arena.remove(previous);
        self.root_component = Some(next);
        self.diagnostics.components_unmounted += 1;
        self.diagnostics.components_mounted += 1;
        Ok(true)
    }

    fn record_error(&mut self, error: impl ToString) {
        self.diagnostics.invalid_views += 1;
        self.last_error = Some(RuntimeError::new(error.to_string()));
    }

    fn render_component(&mut self, id: ComponentInstanceId) -> Result<RenderedView, ViewError> {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        let component = self
            .arena
            .get(id)
            .and_then(|slot| slot.component.as_deref())
            .ok_or(ViewError::StaleParent)?;
        let component_type = component.component_type_id();
        let component_name = component.component_type_name();
        let rendered = component.render(id, self.target);
        self.diagnostics.view_evaluations += 1;
        rendered
            .element
            .validate_for_component(component_type, component_name)?;
        Ok(rendered)
    }

    fn handle_activation(
        &mut self,
        target: UiNodeId,
        source: ChangeSource,
        context: &mut DriverContext<'_>,
    ) -> bool {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        let Some(route) = self.handlers.get(&target).cloned() else {
            return false;
        };
        let owner = route.owner();
        let dispatch = self
            .arena
            .get_mut(owner)
            .and_then(|slot| slot.component.as_deref_mut())
            .map(|component| match route {
                HandlerRoute::Activate(handler) => handler.dispatch(component, source),
                HandlerRoute::Toggle { handler, value } => {
                    let next = match value {
                        SemanticCheckState::Unchecked | SemanticCheckState::Mixed => {
                            SemanticCheckState::Checked
                        }
                        SemanticCheckState::Checked => SemanticCheckState::Unchecked,
                    };
                    handler.dispatch_checked(component, source, next)
                }
                HandlerRoute::Value(handler) => {
                    let current = context
                        .ui
                        .interactions
                        .get(target)
                        .map_or(0.0, |interaction| interaction.value);
                    handler.dispatch_value(
                        component,
                        source,
                        (current + 0.1).clamp(0.0, 1.0),
                        ValueChangePhase::Commit,
                    )
                }
            });
        match dispatch {
            Some(EventDispatch::Delivered { input_mutated }) => {
                self.diagnostics.events_delivered += 1;
                self.diagnostics.input_mutations_restored += u64::from(input_mutated);
                match self.reconcile_component(context.ui, owner) {
                    Ok(()) => *context.frame_requested = true,
                    Err(error) => self.record_error(error),
                }
                true
            }
            Some(EventDispatch::WrongComponentType) | None => {
                self.diagnostics.stale_events += 1;
                false
            }
        }
    }

    fn handle_value(
        &mut self,
        target: UiNodeId,
        value: f32,
        phase: ValueChangePhase,
        source: ChangeSource,
        context: &mut DriverContext<'_>,
    ) -> bool {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        let Some(HandlerRoute::Value(handler)) = self.handlers.get(&target).cloned() else {
            return false;
        };
        let owner = handler.owner();
        let dispatch = self
            .arena
            .get_mut(owner)
            .and_then(|slot| slot.component.as_deref_mut())
            .map(|component| handler.dispatch_value(component, source, value, phase));
        match dispatch {
            Some(EventDispatch::Delivered { input_mutated }) => {
                self.diagnostics.events_delivered += 1;
                self.diagnostics.input_mutations_restored += u64::from(input_mutated);
                match self.reconcile_component(context.ui, owner) {
                    Ok(()) => *context.frame_requested = true,
                    Err(error) => self.record_error(error),
                }
                true
            }
            Some(EventDispatch::WrongComponentType) | None => {
                self.diagnostics.stale_events += 1;
                false
            }
        }
    }
}

impl ComponentDriver for CompositionDriver {
    type Action = ();

    fn mount(&mut self, writer: &mut MountWriter<'_, Self::Action>) -> UiRoot {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        let component = self
            .pending_root
            .take()
            .expect("a composition driver mounts its root exactly once");
        let root_component = self.arena.insert(component);
        self.root_component = Some(root_component);
        let root_style = BoxStyle {
            width: crate::ui::SizeRule::Fill(1.0),
            height: crate::ui::SizeRule::Fill(1.0),
            ..BoxStyle::default()
        };
        let rendered = match self.render_component(root_component) {
            Ok(rendered) => Some(rendered),
            Err(error) => {
                self.record_error(error);
                None
            }
        };
        let (candidate, signal_dependencies) = rendered
            .map(|rendered| (Some(rendered.element), rendered.signals))
            .unwrap_or_default();
        let mut mounted = None;
        let mut mount_error = None;
        let root = writer.root(root_style, crate::ui::LayoutStyle::default(), |writer| {
            if let Some(candidate) = candidate {
                match self.mount_element(writer, candidate, root_component) {
                    Ok(child) => mounted = Some(child),
                    Err(error) => mount_error = Some(error),
                }
            }
        });
        if let Some(error) = mount_error {
            self.record_error(error);
        }
        self.arena
            .get_mut(root_component)
            .expect("root component is live")
            .child = mounted.map(Box::new);
        if let Err(error) = self.commit_signal_dependencies(root_component, signal_dependencies) {
            self.record_error(error);
        }
        let requested = self
            .arena
            .get_mut(root_component)
            .and_then(|slot| slot.component.as_deref_mut())
            .expect("root component is live")
            .mounted_erased(root_component);
        if requested {
            self.signal_invalidations
                .lock()
                .expect("composition invalidation queue poisoned")
                .push(root_component);
        }
        let name = writer.intern("Application");
        let _ = writer.semantic_node(
            root.0,
            SemanticNode {
                role: SemanticRole::Application,
                name: SemanticName::Text(name),
                ..SemanticNode::default()
            },
        );
        self.view_root = Some(root);
        self.diagnostics.components_mounted += 1;
        root
    }

    fn initialize(&mut self, context: &mut DriverContext<'_>) {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        self.process_signal_updates(context);
    }

    fn dispatch_node_activation(
        &mut self,
        target: UiNodeId,
        source: ChangeSource,
        context: &mut DriverContext<'_>,
    ) -> bool {
        self.handle_activation(target, source, context)
    }

    fn dispatch_node_value(
        &mut self,
        target: UiNodeId,
        value: f32,
        phase: ValueChangePhase,
        source: ChangeSource,
        context: &mut DriverContext<'_>,
    ) -> bool {
        self.handle_value(target, value, phase, source, context)
    }

    fn dispatch_ui_route(
        &mut self,
        _event: &UiEvent,
        _listener_mask: u16,
        _context: &mut DriverContext<'_>,
    ) -> bool {
        false
    }

    fn reject_stale_node_action(&mut self, _target: UiNodeId) {
        self.diagnostics.stale_events += 1;
    }

    fn external_updates_ready(&self) -> bool {
        self.signal_updates_ready()
    }

    fn process_external_updates(&mut self, context: &mut DriverContext<'_>) -> usize {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        self.process_signal_updates(context)
    }

    fn close(&mut self, context: &mut DriverContext<'_>) {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        if let Some(root_component) = self.root_component.take() {
            if let Some(child) = self
                .arena
                .get_mut(root_component)
                .and_then(|slot| slot.child.take())
            {
                self.teardown_metadata(*child);
            }
            if let Some(component) = self
                .arena
                .get_mut(root_component)
                .and_then(|slot| slot.component.as_deref_mut())
            {
                component.unmounted_erased(root_component);
            }
            self.arena.remove(root_component);
            self.diagnostics.components_unmounted += 1;
        }
        if let Some(root) = self.view_root.take() {
            context.ui.remove(root.0);
            *context.frame_requested = true;
        }
        self.handlers.clear();
    }
}

mod control_style;
use control_style::*;
