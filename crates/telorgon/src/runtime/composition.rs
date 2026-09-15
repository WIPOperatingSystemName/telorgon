//! Dirty-component reconciliation from short-lived composition elements into retained UI nodes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use crate::compose::{
    ButtonElement, Component, ComponentInstanceId, ContainerElement, Element, ElementKind,
    ElementType, ErasedComponent, EventDispatch, EventHandler, ImageElement, Key, RenderedView,
    RuntimeTarget, SignalDependency, SignalSubscription, SliderElement, TextElement, ToggleElement,
    ToggleKind, ViewError,
};
use crate::core::{ColorRgba8, EdgeInsets, PointF, Transform2D};
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
        icon_node: UiNodeId,
        label_node: UiNodeId,
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
        }
    }

    /// Installs the host-turn wake used by external signals. Replacing it is safe because signal
    /// subscriptions consult this shared slot at publication time.
    pub(crate) fn connect_shell(&mut self, services: crate::compose::ShellServices) {
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
        reason: crate::compose::ShellDismissReason,
    ) {
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

    fn commit_signal_dependencies(
        &mut self,
        id: ComponentInstanceId,
        dependencies: Vec<SignalDependency>,
    ) -> Result<(), ViewError> {
        let mut subscriptions = Vec::with_capacity(dependencies.len());
        for dependency in dependencies {
            let invalidations = self.signal_invalidations.clone();
            let wake = self.wake.clone();
            subscriptions.push(dependency.subscribe(Arc::new(move || {
                let mut invalidations = invalidations
                    .lock()
                    .expect("composition invalidation queue poisoned");
                if !invalidations.contains(&id) {
                    invalidations.push(id);
                }
                drop(invalidations);
                if let Some(wake) = wake
                    .read()
                    .expect("composition wake lock poisoned")
                    .as_ref()
                    .cloned()
                {
                    wake();
                }
            })));

            // A writer can publish between the view snapshot and subscription registration.
            if dependency.changed() {
                self.signal_invalidations
                    .lock()
                    .expect("composition invalidation queue poisoned")
                    .push(id);
            }
        }
        self.arena
            .get_mut(id)
            .ok_or(ViewError::StaleParent)?
            .signal_subscriptions = subscriptions;
        Ok(())
    }

    fn signal_updates_ready(&self) -> bool {
        !self
            .signal_invalidations
            .lock()
            .expect("composition invalidation queue poisoned")
            .is_empty()
    }

    fn process_signal_updates(&mut self, context: &mut DriverContext<'_>) -> usize {
        #[cfg(feature = "instrumentation")]
        let signal_span = crate::profiler::span!("signals.drain");
        {
            let mut queue = self
                .signal_invalidations
                .lock()
                .expect("composition invalidation queue poisoned");
            std::mem::swap(&mut *queue, &mut self.signal_scratch);
        }
        self.signal_scratch.sort_unstable();
        self.signal_scratch.dedup();
        #[cfg(feature = "instrumentation")]
        drop(signal_span);
        self.diagnostics.externally_invalidated_components += self.signal_scratch.len() as u64;
        let mut invalidated = std::mem::take(&mut self.signal_scratch);
        let mut processed = 0;
        #[cfg(feature = "instrumentation")]
        let _reconcile_span = crate::profiler::span!("element.reconcile");
        for id in invalidated.drain(..) {
            if self.arena.get(id).is_none() {
                self.diagnostics.stale_events += 1;
                continue;
            }
            match self.reconcile_component(context.ui, id) {
                Ok(()) => {
                    processed += 1;
                    self.diagnostics.externally_reconciled_components += 1;
                    *context.frame_requested = true;
                }
                Err(error) => self.record_error(error),
            }
        }
        self.signal_scratch = invalidated;
        self.signal_scratch.clear();
        processed
    }

    fn mount_component(
        &mut self,
        writer: &mut MountWriter<'_, ()>,
        component: Box<dyn ErasedComponent>,
    ) -> Result<MountedElement, ViewError> {
        let type_id = component.component_type_id();
        let id = self.arena.insert(component);
        let rendered = match self.render_component(id) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.arena.remove(id);
                return Err(error);
            }
        };
        let child = match self.mount_element(writer, rendered.element, id) {
            Ok(child) => child,
            Err(error) => {
                self.arena.remove(id);
                return Err(error);
            }
        };
        self.arena.get_mut(id).expect("new component is live").child = Some(Box::new(child));
        self.commit_signal_dependencies(id, rendered.signals)?;
        let requested = self
            .arena
            .get_mut(id)
            .and_then(|slot| slot.component.as_deref_mut())
            .is_some_and(|component| component.mounted_erased(id));
        if requested {
            self.signal_invalidations
                .lock()
                .expect("composition invalidation queue poisoned")
                .push(id);
        }
        self.diagnostics.components_mounted += 1;
        Ok(MountedElement {
            key: None,
            kind: MountedKind::Component { id, type_id },
        })
    }

    fn mount_element(
        &mut self,
        writer: &mut MountWriter<'_, ()>,
        element: Element,
        owner: ComponentInstanceId,
    ) -> Result<MountedElement, ViewError> {
        let (key, kind, window_chrome_role, window_chrome_hit_spec, pointer_request) =
            element.into_parts();
        let mut mounted = match kind {
            ElementKind::Container(ContainerElement {
                style,
                layout,
                children,
            }) => {
                let mut mounted_children = Vec::with_capacity(children.len());
                let node = writer.container(style, layout, |writer| {
                    for child in children {
                        mounted_children.push(self.mount_element(writer, child, owner));
                    }
                });
                let children = mounted_children
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()?;
                MountedElement {
                    key: None,
                    kind: MountedKind::Container {
                        node,
                        style,
                        layout,
                        children,
                    },
                }
            }
            ElementKind::Text(props) => {
                let text = writer.dynamic_text(
                    props.content.clone(),
                    props.style.resolve(),
                    props.box_style,
                    props.layout,
                );
                MountedElement {
                    key: None,
                    kind: MountedKind::Text {
                        node: text.node,
                        props,
                    },
                }
            }
            ElementKind::Image(props) => {
                let image = writer.dynamic_image_tinted(
                    props.image,
                    props.content_version,
                    props.tint,
                    props.style,
                    props.layout,
                );
                if let Some(label) = &props.accessible_label {
                    let name = writer.intern(label);
                    writer
                        .semantic_node(
                            image.node,
                            SemanticNode {
                                role: SemanticRole::Image,
                                name: SemanticName::Text(name),
                                ..SemanticNode::default()
                            },
                        )
                        .map_err(|_| ViewError::MissingButtonLabel)?;
                }
                MountedElement {
                    key: None,
                    kind: MountedKind::Image {
                        node: image.node,
                        props,
                    },
                }
            }
            ElementKind::Button(props) => {
                let mut icon_node = None;
                let mut label_node = None;
                let control = writer.button_node(props.style, |writer| {
                    icon_node = Some(
                        writer
                            .dynamic_image_tinted(
                                props.icon.unwrap_or(crate::ui::ImageId(0)),
                                1,
                                props.icon_tint,
                                button_icon_style(&props),
                                crate::ui::LayoutStyle::default(),
                            )
                            .node,
                    );
                    label_node = Some(
                        writer
                            .dynamic_text(
                                props.label.clone(),
                                props.label_style,
                                button_label_box_style(&props),
                                crate::ui::LayoutStyle::default(),
                            )
                            .node,
                    );
                });
                writer.style_id(control.node, props.style_id);
                let icon_node = icon_node.expect("button always mounts its icon slot");
                let label_node = label_node.expect("button always mounts its label");
                let mut binding =
                    StyleBinding::new(control.node, ThemeScopeId::new(0, 1), props.style_id)
                        .slot(StyleSlotId::named("root"), control.node)
                        .slot(StyleSlotId::named("icon"), icon_node)
                        .slot(StyleSlotId::named("label"), label_node);
                if let Some(style) = props.inline_style.clone() {
                    binding = binding.local_style(style);
                }
                writer.style_binding(binding);
                if props.style_override != StylePropertyPatch::default() {
                    writer.style_override(
                        control.node,
                        StyleSlotId::named("root"),
                        props.style_override,
                    );
                }
                writer.disabled(control.node, !props.enabled);
                writer.busy(control.node, props.busy);
                let name = writer.intern(&props.label);
                writer
                    .semantic_node(control.node, button_semantics(name, &props))
                    .map_err(|_| ViewError::MissingButtonLabel)?;
                if let Some(handler) = &props.on_press {
                    self.handlers
                        .insert(control.node, HandlerRoute::Activate(handler.bind(owner)));
                }
                MountedElement {
                    key: None,
                    kind: MountedKind::Button {
                        node: control.node,
                        icon_node,
                        label_node,
                        props,
                    },
                }
            }
            ElementKind::Toggle(props) => self.mount_toggle(writer, props, owner)?,
            ElementKind::Slider(props) => self.mount_slider(writer, props, owner)?,
            ElementKind::Component(component) => self.mount_component(writer, component)?,
        };
        mounted.key = key;
        let root = self
            .root_node(&mounted)
            .expect("every mounted composition element has a root node");
        writer.window_chrome_role(root, window_chrome_role);
        writer.window_chrome_hit_spec(root, window_chrome_hit_spec);
        writer.pointer_request(root, pointer_request);
        self.diagnostics.elements_mounted += 1;
        Ok(mounted)
    }

    fn mount_toggle(
        &mut self,
        writer: &mut MountWriter<'_, ()>,
        props: ToggleElement,
        owner: ComponentInstanceId,
    ) -> Result<MountedElement, ViewError> {
        match props.kind {
            ToggleKind::Checkbox => {
                let styles = checkbox_styles(props.value, props.enabled);
                let mut indicator = None;
                let mut check_first = None;
                let mut check_second = None;
                let mut mixed = None;
                let mut label = None;
                let control = writer.toggle_node(styles.container, |writer| {
                    writer.container(
                        BoxStyle::default(),
                        LayoutStyle {
                            flow: Flow::Horizontal,
                            gap: 8.0,
                            ..LayoutStyle::default()
                        },
                        |writer| {
                            indicator = Some(
                                writer
                                    .container_handle(
                                        styles.indicator,
                                        LayoutStyle {
                                            flow: Flow::Overlay,
                                            ..LayoutStyle::default()
                                        },
                                        |writer| {
                                            check_first = Some(
                                                writer
                                                    .container_handle(
                                                        styles.check_first,
                                                        LayoutStyle::default(),
                                                        |_| {},
                                                    )
                                                    .node,
                                            );
                                            check_second = Some(
                                                writer
                                                    .container_handle(
                                                        styles.check_second,
                                                        LayoutStyle::default(),
                                                        |_| {},
                                                    )
                                                    .node,
                                            );
                                            mixed = Some(
                                                writer
                                                    .container_handle(
                                                        styles.mixed,
                                                        LayoutStyle::default(),
                                                        |_| {},
                                                    )
                                                    .node,
                                            );
                                        },
                                    )
                                    .node,
                            );
                            label = Some(
                                writer
                                    .dynamic_text(
                                        props.label.clone(),
                                        control_label_style(props.enabled),
                                        BoxStyle::default(),
                                        LayoutStyle::default(),
                                    )
                                    .node,
                            );
                        },
                    );
                });
                writer.style_id(
                    control.node,
                    ComponentStyleId::named(ThemeDomainId::APPLICATION, "checkbox", "default"),
                );
                writer.style_binding(
                    StyleBinding::new(
                        control.node,
                        ThemeScopeId::new(0, 1),
                        ComponentStyleId::named(ThemeDomainId::APPLICATION, "checkbox", "default"),
                    )
                    .slot(StyleSlotId::named("root"), control.node)
                    .slot(
                        StyleSlotId::named("indicator"),
                        indicator.expect("checkbox mounts its indicator"),
                    )
                    .slot(
                        StyleSlotId::named("check-start"),
                        check_first.expect("checkbox mounts its first check segment"),
                    )
                    .slot(
                        StyleSlotId::named("check-end"),
                        check_second.expect("checkbox mounts its second check segment"),
                    )
                    .slot(
                        StyleSlotId::named("mixed"),
                        mixed.expect("checkbox mounts its mixed segment"),
                    )
                    .slot(
                        StyleSlotId::named("label"),
                        label.expect("checkbox mounts its label"),
                    ),
                );
                writer.disabled(control.node, !props.enabled);
                writer.checked(control.node, props.value != SemanticCheckState::Unchecked);
                let name = writer.intern(&props.label);
                writer
                    .semantic_node(control.node, toggle_semantics(name, &props))
                    .map_err(|_| ViewError::MissingButtonLabel)?;
                if let Some(handler) = &props.on_change {
                    self.handlers.insert(
                        control.node,
                        HandlerRoute::Toggle {
                            handler: handler.bind(owner),
                            value: props.value,
                        },
                    );
                }
                Ok(MountedElement {
                    key: None,
                    kind: MountedKind::Checkbox {
                        node: control.node,
                        indicator: indicator.expect("checkbox indicator was bound"),
                        check_first: check_first.expect("checkbox first check was bound"),
                        check_second: check_second.expect("checkbox second check was bound"),
                        mixed: mixed.expect("checkbox mixed mark was bound"),
                        label: label.expect("checkbox label was bound"),
                        props,
                    },
                })
            }
            ToggleKind::Switch => {
                let styles =
                    switch_styles(props.value == SemanticCheckState::Checked, props.enabled);
                let mut track = None;
                let mut thumb = None;
                let mut label = None;
                let control = writer.toggle_node(styles.container, |writer| {
                    writer.container(
                        BoxStyle::default(),
                        LayoutStyle {
                            flow: Flow::Horizontal,
                            gap: 8.0,
                            ..LayoutStyle::default()
                        },
                        |writer| {
                            track = Some(
                                writer
                                    .container_handle(
                                        styles.track,
                                        LayoutStyle::default(),
                                        |writer| {
                                            thumb = Some(
                                                writer
                                                    .container_handle(
                                                        styles.thumb,
                                                        LayoutStyle::default(),
                                                        |_| {},
                                                    )
                                                    .node,
                                            );
                                        },
                                    )
                                    .node,
                            );
                            label = Some(
                                writer
                                    .dynamic_text(
                                        props.label.clone(),
                                        control_label_style(props.enabled),
                                        BoxStyle::default(),
                                        LayoutStyle::default(),
                                    )
                                    .node,
                            );
                        },
                    );
                });
                writer.style_id(
                    control.node,
                    ComponentStyleId::named(ThemeDomainId::APPLICATION, "switch", "default"),
                );
                writer.style_binding(
                    StyleBinding::new(
                        control.node,
                        ThemeScopeId::new(0, 1),
                        ComponentStyleId::named(ThemeDomainId::APPLICATION, "switch", "default"),
                    )
                    .slot(StyleSlotId::named("root"), control.node)
                    .slot(
                        StyleSlotId::named("track"),
                        track.expect("switch mounts its track"),
                    )
                    .slot(
                        StyleSlotId::named("thumb"),
                        thumb.expect("switch mounts its thumb"),
                    )
                    .slot(
                        StyleSlotId::named("label"),
                        label.expect("switch mounts its label"),
                    ),
                );
                writer.disabled(control.node, !props.enabled);
                writer.checked(control.node, props.value == SemanticCheckState::Checked);
                let name = writer.intern(&props.label);
                writer
                    .semantic_node(control.node, toggle_semantics(name, &props))
                    .map_err(|_| ViewError::MissingButtonLabel)?;
                if let Some(handler) = &props.on_change {
                    self.handlers.insert(
                        control.node,
                        HandlerRoute::Toggle {
                            handler: handler.bind(owner),
                            value: props.value,
                        },
                    );
                }
                Ok(MountedElement {
                    key: None,
                    kind: MountedKind::Switch {
                        node: control.node,
                        track: track.expect("switch track was bound"),
                        thumb: thumb.expect("switch thumb was bound"),
                        label: label.expect("switch label was bound"),
                        props,
                    },
                })
            }
        }
    }

    fn mount_slider(
        &mut self,
        writer: &mut MountWriter<'_, ()>,
        props: SliderElement,
        owner: ComponentInstanceId,
    ) -> Result<MountedElement, ViewError> {
        let styles = slider_styles(props.value, props.enabled);
        let mut track = None;
        let mut fill = None;
        let mut thumb = None;
        let mut label = None;
        let control = writer.slider_node(styles.container, |writer| {
            writer.container(
                BoxStyle::default(),
                LayoutStyle {
                    flow: Flow::Horizontal,
                    gap: 8.0,
                    ..LayoutStyle::default()
                },
                |writer| {
                    label = Some(
                        writer
                            .dynamic_text(
                                props.label.clone(),
                                control_label_style(props.enabled),
                                BoxStyle::default(),
                                LayoutStyle::default(),
                            )
                            .node,
                    );
                    track = Some(
                        writer
                            .container_handle(
                                styles.track,
                                LayoutStyle {
                                    flow: Flow::Overlay,
                                    ..LayoutStyle::default()
                                },
                                |writer| {
                                    fill = Some(
                                        writer
                                            .container_handle(
                                                styles.fill,
                                                LayoutStyle::default(),
                                                |_| {},
                                            )
                                            .node,
                                    );
                                    thumb = Some(
                                        writer
                                            .container_handle(
                                                styles.thumb,
                                                LayoutStyle::default(),
                                                |_| {},
                                            )
                                            .node,
                                    );
                                },
                            )
                            .node,
                    );
                },
            );
        });
        let track = track.expect("slider mounts its track");
        writer.style_id(
            control.node,
            ComponentStyleId::named(ThemeDomainId::APPLICATION, "slider", "default"),
        );
        writer.style_binding(
            StyleBinding::new(
                control.node,
                ThemeScopeId::new(0, 1),
                ComponentStyleId::named(ThemeDomainId::APPLICATION, "slider", "default"),
            )
            .slot(StyleSlotId::named("root"), control.node)
            .slot(StyleSlotId::named("track"), track)
            .slot(
                StyleSlotId::named("fill"),
                fill.expect("slider mounts its fill"),
            )
            .slot(
                StyleSlotId::named("thumb"),
                thumb.expect("slider mounts its thumb"),
            )
            .slot(
                StyleSlotId::named("label"),
                label.expect("slider mounts its label"),
            ),
        );
        writer.disabled(control.node, !props.enabled);
        writer.control_value(control.node, props.value);
        writer.value_track(
            control.node,
            track,
            ValueAxis::Horizontal { inverted: false },
        );
        let name = writer.intern(&props.label);
        let value_text = writer.intern(format!("{:.0}%", props.value * 100.0));
        writer
            .semantic_node(control.node, slider_semantics(name, value_text, &props))
            .map_err(|_| ViewError::MissingButtonLabel)?;
        if let Some(handler) = &props.on_change {
            self.handlers
                .insert(control.node, HandlerRoute::Value(handler.bind(owner)));
        }
        Ok(MountedElement {
            key: None,
            kind: MountedKind::Slider {
                node: control.node,
                track,
                fill: fill.expect("slider fill was bound"),
                thumb: thumb.expect("slider thumb was bound"),
                label: label.expect("slider label was bound"),
                props,
            },
        })
    }

    fn root_node(&self, mounted: &MountedElement) -> Option<UiNodeId> {
        match &mounted.kind {
            MountedKind::Container { node, .. }
            | MountedKind::Text { node, .. }
            | MountedKind::Image { node, .. }
            | MountedKind::Button { node, .. }
            | MountedKind::Checkbox { node, .. }
            | MountedKind::Switch { node, .. }
            | MountedKind::Slider { node, .. } => Some(*node),
            MountedKind::Component { id, .. } => self
                .arena
                .get(*id)
                .and_then(|slot| slot.child.as_deref())
                .and_then(|child| self.root_node(child)),
        }
    }

    fn update_component_candidate(
        &mut self,
        ui: &mut crate::ui::MountedUi,
        id: ComponentInstanceId,
        candidate: Box<dyn ErasedComponent>,
    ) -> Result<bool, ViewError> {
        let changed = self
            .arena
            .get_mut(id)
            .and_then(|slot| slot.component.as_deref_mut())
            .ok_or(ViewError::StaleParent)?
            .update_from(candidate)?;
        if !changed {
            self.diagnostics.components_reused += 1;
            return Ok(false);
        }
        let requested = self
            .arena
            .get_mut(id)
            .and_then(|slot| slot.component.as_deref_mut())
            .is_some_and(|component| component.inputs_changed_erased(id));
        let _ = requested;
        self.reconcile_component(ui, id)?;
        Ok(true)
    }

    fn reconcile_component(
        &mut self,
        ui: &mut crate::ui::MountedUi,
        id: ComponentInstanceId,
    ) -> Result<(), ViewError> {
        let rendered = self.render_component(id)?;
        let old = self
            .arena
            .get_mut(id)
            .and_then(|slot| slot.child.take())
            .ok_or(ViewError::StaleParent)?;
        let parent = self
            .root_node(&old)
            .and_then(|node| ui.nodes.core(node).and_then(|core| core.parent))
            .ok_or(ViewError::StaleParent)?;
        match self.reconcile_element(ui, parent, *old, rendered.element, id) {
            Ok(child) => {
                self.arena.get_mut(id).ok_or(ViewError::StaleParent)?.child = Some(Box::new(child));
                self.commit_signal_dependencies(id, rendered.signals)?;
                Ok(())
            }
            Err((old, error)) => {
                if let Some(slot) = self.arena.get_mut(id) {
                    slot.child = Some(Box::new(old));
                }
                Err(error)
            }
        }
    }

    fn reconcile_element(
        &mut self,
        ui: &mut crate::ui::MountedUi,
        parent: UiNodeId,
        mut old: MountedElement,
        candidate: Element,
        owner: ComponentInstanceId,
    ) -> Result<MountedElement, (MountedElement, ViewError)> {
        let candidate_type = candidate.kind().identity();
        let same_identity =
            old.key.as_ref() == candidate.key_ref() && old.element_type() == candidate_type;
        if !same_identity {
            let Some(old_root) = self.root_node(&old) else {
                return Err((old, ViewError::StaleParent));
            };
            let Some(mut writer) = MountWriter::under(ui, parent) else {
                return Err((old, ViewError::StaleParent));
            };
            let mounted = match self.mount_element(&mut writer, candidate, owner) {
                Ok(mounted) => mounted,
                Err(error) => return Err((old, error)),
            };
            let new_root = self
                .root_node(&mounted)
                .expect("mounted element has a root");
            ui.nodes.reparent_before(new_root, parent, Some(old_root));
            self.remove_mounted(ui, old);
            return Ok(mounted);
        }

        let (key, kind, window_chrome_role, window_chrome_hit_spec, pointer_request) =
            candidate.into_parts();
        let result = match (&mut old.kind, kind) {
            (
                MountedKind::Container {
                    node,
                    style,
                    layout,
                    children,
                },
                ElementKind::Container(candidate),
            ) => {
                ui.set_box_style(*node, candidate.style);
                ui.set_layout_style(*node, candidate.layout);
                *style = candidate.style;
                *layout = candidate.layout;
                let previous = std::mem::take(children);
                *children =
                    match self.reconcile_children(ui, *node, previous, candidate.children, owner) {
                        Ok(children) => children,
                        Err((previous, error)) => {
                            *children = previous;
                            return Err((old, error));
                        }
                    };
                Ok(())
            }
            (MountedKind::Text { node, props }, ElementKind::Text(candidate)) => {
                ui.set_dynamic_text(*node, &candidate.content);
                ui.set_text_style(*node, candidate.style.resolve());
                ui.set_box_style(*node, candidate.box_style);
                ui.set_layout_style(*node, candidate.layout);
                *props = candidate;
                Ok(())
            }
            (MountedKind::Image { node, props }, ElementKind::Image(candidate)) => {
                ui.set_image_visual_tinted(
                    *node,
                    candidate.image,
                    candidate.content_version,
                    candidate.tint,
                );
                ui.set_box_style(*node, candidate.style);
                ui.set_layout_style(*node, candidate.layout);
                match &candidate.accessible_label {
                    Some(label) => {
                        let name = ui.intern(label);
                        let _ = ui.set_semantics(
                            *node,
                            SemanticNode {
                                role: SemanticRole::Image,
                                name: SemanticName::Text(name),
                                ..SemanticNode::default()
                            },
                        );
                    }
                    None => {
                        ui.semantics.remove(*node);
                    }
                }
                *props = candidate;
                Ok(())
            }
            (
                MountedKind::Button {
                    node,
                    icon_node,
                    label_node,
                    props,
                },
                ElementKind::Button(candidate),
            ) => {
                // The mounted style includes resolved theme/inline properties. A parent
                // update must not reset those to identical authored defaults: unchanged
                // bindings will not resolve again until an interaction state changes.
                if props.style != candidate.style {
                    ui.set_box_style(*node, candidate.style);
                }
                ui.set_image_visual_tinted(
                    *icon_node,
                    candidate.icon.unwrap_or(crate::ui::ImageId(0)),
                    1,
                    candidate.icon_tint,
                );
                ui.set_box_style(*icon_node, button_icon_style(&candidate));
                ui.set_dynamic_text(*label_node, &candidate.label);
                ui.set_text_style(*label_node, candidate.label_style);
                ui.set_box_style(*label_node, button_label_box_style(&candidate));
                ui.set_disabled(*node, !candidate.enabled);
                ui.set_busy(*node, candidate.busy);
                ui.set_style_id(*node, candidate.style_id);
                ui.set_style_override(*node, StyleSlotId::named("root"), candidate.style_override);
                ui.set_local_component_style(*node, candidate.inline_style.clone());
                let name = ui.intern(&candidate.label);
                let _ = ui.set_semantics(*node, button_semantics(name, &candidate));
                match &candidate.on_press {
                    Some(handler) => {
                        self.handlers
                            .insert(*node, HandlerRoute::Activate(handler.bind(owner)));
                    }
                    None => {
                        self.handlers.remove(node);
                    }
                }
                *props = candidate;
                Ok(())
            }
            (
                MountedKind::Checkbox {
                    node,
                    indicator,
                    check_first,
                    check_second,
                    mixed,
                    label,
                    props,
                },
                ElementKind::Toggle(candidate),
            ) => {
                let styles = checkbox_styles(candidate.value, candidate.enabled);
                ui.set_box_style(*node, styles.container);
                ui.set_box_style(*indicator, styles.indicator);
                ui.set_box_style(*check_first, styles.check_first);
                ui.set_box_style(*check_second, styles.check_second);
                ui.set_box_style(*mixed, styles.mixed);
                ui.set_dynamic_text(*label, &candidate.label);
                ui.set_text_style(*label, control_label_style(candidate.enabled));
                ui.set_disabled(*node, !candidate.enabled);
                ui.set_checked(*node, candidate.value != SemanticCheckState::Unchecked);
                ui.set_mixed(*node, candidate.value == SemanticCheckState::Mixed);
                let name = ui.intern(&candidate.label);
                let _ = ui.set_semantics(*node, toggle_semantics(name, &candidate));
                match &candidate.on_change {
                    Some(handler) => {
                        self.handlers.insert(
                            *node,
                            HandlerRoute::Toggle {
                                handler: handler.bind(owner),
                                value: candidate.value,
                            },
                        );
                    }
                    None => {
                        self.handlers.remove(node);
                    }
                }
                *props = candidate;
                Ok(())
            }
            (
                MountedKind::Switch {
                    node,
                    track,
                    thumb,
                    label,
                    props,
                },
                ElementKind::Toggle(candidate),
            ) => {
                let styles = switch_styles(
                    candidate.value == SemanticCheckState::Checked,
                    candidate.enabled,
                );
                ui.set_box_style(*node, styles.container);
                ui.set_box_style(*track, styles.track);
                ui.set_box_style(*thumb, styles.thumb);
                ui.set_dynamic_text(*label, &candidate.label);
                ui.set_text_style(*label, control_label_style(candidate.enabled));
                ui.set_disabled(*node, !candidate.enabled);
                ui.set_checked(*node, candidate.value == SemanticCheckState::Checked);
                ui.set_mixed(*node, false);
                let name = ui.intern(&candidate.label);
                let _ = ui.set_semantics(*node, toggle_semantics(name, &candidate));
                match &candidate.on_change {
                    Some(handler) => {
                        self.handlers.insert(
                            *node,
                            HandlerRoute::Toggle {
                                handler: handler.bind(owner),
                                value: candidate.value,
                            },
                        );
                    }
                    None => {
                        self.handlers.remove(node);
                    }
                }
                *props = candidate;
                Ok(())
            }
            (
                MountedKind::Slider {
                    node,
                    track,
                    fill,
                    thumb,
                    label,
                    props,
                },
                ElementKind::Slider(candidate),
            ) => {
                let styles = slider_styles(candidate.value, candidate.enabled);
                ui.set_box_style(*node, styles.container);
                ui.set_box_style(*track, styles.track);
                ui.set_box_style(*fill, styles.fill);
                ui.set_box_style(*thumb, styles.thumb);
                ui.set_dynamic_text(*label, &candidate.label);
                ui.set_text_style(*label, control_label_style(candidate.enabled));
                ui.set_disabled(*node, !candidate.enabled);
                ui.set_control_value(*node, candidate.value);
                let name = ui.intern(&candidate.label);
                let value_text = ui.intern(format!("{:.0}%", candidate.value * 100.0));
                let _ = ui.set_semantics(*node, slider_semantics(name, value_text, &candidate));
                match &candidate.on_change {
                    Some(handler) => {
                        self.handlers
                            .insert(*node, HandlerRoute::Value(handler.bind(owner)));
                    }
                    None => {
                        self.handlers.remove(node);
                    }
                }
                *props = candidate;
                Ok(())
            }
            (MountedKind::Component { id, .. }, ElementKind::Component(candidate)) => self
                .update_component_candidate(ui, *id, candidate)
                .map(|_| ()),
            _ => unreachable!("equal element identities have equal payload variants"),
        };
        if let Err(error) = result {
            return Err((old, error));
        }
        let root = self
            .root_node(&old)
            .expect("every reconciled composition element has a root node");
        ui.set_window_chrome_role(root, window_chrome_role);
        ui.set_window_chrome_hit_spec(root, window_chrome_hit_spec);
        ui.set_pointer_request(root, pointer_request);
        old.key = key;
        self.diagnostics.elements_reused += 1;
        Ok(old)
    }

    fn reconcile_children(
        &mut self,
        ui: &mut crate::ui::MountedUi,
        parent: UiNodeId,
        old: Vec<MountedElement>,
        candidates: Vec<Element>,
        owner: ComponentInstanceId,
    ) -> Result<Vec<MountedElement>, (Vec<MountedElement>, ViewError)> {
        let mut old: Vec<Option<MountedElement>> = old.into_iter().map(Some).collect();
        let mut next = Vec::with_capacity(candidates.len());
        for (index, candidate) in candidates.into_iter().enumerate() {
            let candidate_type = candidate.kind().identity();
            let match_index = if let Some(key) = candidate.key_ref() {
                old.iter().position(|entry| {
                    entry.as_ref().is_some_and(|mounted| {
                        mounted.key.as_ref() == Some(key)
                            && mounted.element_type() == candidate_type
                    })
                })
            } else {
                old.get(index).and_then(|entry| {
                    entry.as_ref().and_then(|mounted| {
                        (mounted.key.is_none() && mounted.element_type() == candidate_type)
                            .then_some(index)
                    })
                })
            };
            let mounted = if let Some(match_index) = match_index {
                let mounted = old[match_index].take().expect("matched child exists");
                match self.reconcile_element(ui, parent, mounted, candidate, owner) {
                    Ok(mounted) => mounted,
                    Err((mounted, error)) => {
                        old[match_index] = Some(mounted);
                        let mut restored = next;
                        restored.extend(old.into_iter().flatten());
                        return Err((restored, error));
                    }
                }
            } else {
                let Some(mut writer) = MountWriter::under(ui, parent) else {
                    let mut restored = next;
                    restored.extend(old.into_iter().flatten());
                    return Err((restored, ViewError::StaleParent));
                };
                match self.mount_element(&mut writer, candidate, owner) {
                    Ok(mounted) => mounted,
                    Err(error) => {
                        let mut restored = next;
                        restored.extend(old.into_iter().flatten());
                        return Err((restored, error));
                    }
                }
            };
            next.push(mounted);
        }
        for removed in old.into_iter().flatten() {
            self.remove_mounted(ui, removed);
        }
        let mut before = None;
        for child in next.iter().rev() {
            let node = self.root_node(child).expect("mounted child has a root");
            ui.nodes.reparent_before(node, parent, before);
            before = Some(node);
        }
        Ok(next)
    }

    fn teardown_metadata(&mut self, mut mounted: MountedElement) {
        match &mut mounted.kind {
            MountedKind::Container { children, .. } => {
                for child in std::mem::take(children) {
                    self.teardown_metadata(child);
                }
            }
            MountedKind::Text { .. } | MountedKind::Image { .. } => {}
            MountedKind::Button { node, .. }
            | MountedKind::Checkbox { node, .. }
            | MountedKind::Switch { node, .. }
            | MountedKind::Slider { node, .. } => {
                self.handlers.remove(node);
            }
            MountedKind::Component { id, .. } => {
                if let Some(child) = self.arena.get_mut(*id).and_then(|slot| slot.child.take()) {
                    self.teardown_metadata(*child);
                }
                if let Some(component) = self
                    .arena
                    .get_mut(*id)
                    .and_then(|slot| slot.component.as_deref_mut())
                {
                    component.unmounted_erased(*id);
                }
                self.arena.remove(*id);
                self.diagnostics.components_unmounted += 1;
            }
        }
    }

    fn remove_mounted(&mut self, ui: &mut crate::ui::MountedUi, mounted: MountedElement) {
        if let Some(node) = self.root_node(&mounted) {
            self.teardown_metadata(mounted);
            ui.remove(node);
            self.diagnostics.elements_removed += 1;
        }
    }

    fn handle_activation(
        &mut self,
        target: UiNodeId,
        source: ChangeSource,
        context: &mut DriverContext<'_>,
    ) -> bool {
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
        self.process_signal_updates(context)
    }

    fn close(&mut self, context: &mut DriverContext<'_>) {
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

fn button_semantics(name: crate::ui::StringId, props: &ButtonElement) -> SemanticNode {
    let mut actions = SemanticActions::NONE;
    if props.enabled {
        actions |= SemanticActions::FOCUS;
        if !props.busy {
            actions |= SemanticActions::ACTIVATE;
        }
    }
    SemanticNode {
        role: SemanticRole::Button,
        name: SemanticName::Text(name),
        state: SemanticState {
            disabled: !props.enabled,
            busy: props.busy,
            focusable: props.enabled,
            ..SemanticState::default()
        },
        actions,
        ..SemanticNode::default()
    }
}

fn button_icon_style(props: &ButtonElement) -> BoxStyle {
    let (size, opacity) = if props.icon.is_some() {
        (props.icon_size.max(1.0), 1.0)
    } else {
        (0.0, 0.0)
    };
    BoxStyle {
        width: SizeRule::Logical(size),
        height: SizeRule::Logical(size),
        opacity,
        ..BoxStyle::default()
    }
}

fn button_label_box_style(props: &ButtonElement) -> BoxStyle {
    if props.icon.is_none() {
        BoxStyle::default()
    } else {
        BoxStyle {
            width: SizeRule::Logical(0.0),
            height: SizeRule::Logical(0.0),
            opacity: 0.0,
            ..BoxStyle::default()
        }
    }
}

fn control_label_style(enabled: bool) -> crate::ui::TextStyle {
    crate::ui::TextStyle {
        color: if enabled {
            ColorRgba8::rgba(235, 238, 244, 255)
        } else {
            ColorRgba8::rgba(153, 157, 168, 255)
        },
        size: 14.0,
        line_height: 17.5,
        family: crate::ui::StringId(1),
        weight: 400,
        align: crate::ui::TextAlign::Start,
    }
}

fn toggle_semantics(name: crate::ui::StringId, props: &ToggleElement) -> SemanticNode {
    let mut actions = SemanticActions::NONE;
    if props.enabled {
        actions |= SemanticActions::FOCUS | SemanticActions::ACTIVATE;
    }
    SemanticNode {
        role: match props.kind {
            ToggleKind::Checkbox => SemanticRole::Checkbox,
            ToggleKind::Switch => SemanticRole::Switch,
        },
        name: SemanticName::Text(name),
        state: SemanticState {
            disabled: !props.enabled,
            focusable: props.enabled,
            checked: Some(props.value),
            ..SemanticState::default()
        },
        actions,
        ..SemanticNode::default()
    }
}

fn slider_semantics(
    name: crate::ui::StringId,
    value_text: crate::ui::StringId,
    props: &SliderElement,
) -> SemanticNode {
    let actions = if props.enabled {
        SemanticActions::FOCUS
            | SemanticActions::INCREMENT
            | SemanticActions::DECREMENT
            | SemanticActions::SET_VALUE
    } else {
        SemanticActions::NONE
    };
    SemanticNode {
        role: SemanticRole::Slider,
        name: SemanticName::Text(name),
        state: SemanticState {
            disabled: !props.enabled,
            focusable: props.enabled,
            ..SemanticState::default()
        },
        value: SemanticValue::Number {
            current: f64::from(props.value),
            minimum: 0.0,
            maximum: 1.0,
            step: Some(0.01),
            value_text: Some(value_text),
        },
        actions,
        ..SemanticNode::default()
    }
}

#[derive(Clone, Copy)]
struct CheckboxStyles {
    container: BoxStyle,
    indicator: BoxStyle,
    check_first: BoxStyle,
    check_second: BoxStyle,
    mixed: BoxStyle,
}

fn checkbox_styles(value: SemanticCheckState, enabled: bool) -> CheckboxStyles {
    let opacity = if enabled { 255 } else { 180 };
    let checked = value != SemanticCheckState::Unchecked;
    let indicator = BoxStyle {
        sizing: BoxSizing::BorderBox,
        width: SizeRule::Logical(18.0),
        height: SizeRule::Logical(18.0),
        max_size: SizeRule2D {
            width: SizeRule::Logical(18.0),
            height: SizeRule::Logical(18.0),
        },
        decoration: crate::ui::BoxDecoration {
            background: if checked {
                Background::Color(ColorRgba8::rgba(54, 104, 210, opacity))
            } else {
                Background::Color(ColorRgba8::rgba(28, 31, 39, opacity))
            },
            border: Border::all(1.0, ColorRgba8::rgba(118, 127, 145, opacity)),
            corner_radii: CornerRadii::all(4.0),
            ..crate::ui::BoxDecoration::default()
        },
        ..BoxStyle::default()
    };
    let mark = ColorRgba8::rgba(255, 255, 255, opacity);
    let check_background = if value == SemanticCheckState::Checked {
        Background::Color(mark)
    } else {
        Background::None
    };
    let mixed_background = if value == SemanticCheckState::Mixed {
        Background::Color(mark)
    } else {
        Background::None
    };
    CheckboxStyles {
        container: BoxStyle {
            min_size: SizeRule2D {
                width: SizeRule::Logical(32.0),
                height: SizeRule::Logical(32.0),
            },
            padding: EdgeInsets::all(5.0),
            ..BoxStyle::default()
        },
        indicator,
        check_first: mark_segment(
            PointF { x: 20.0, y: 6.0 },
            PointF { x: 9.0, y: 17.0 },
            check_background,
        ),
        check_second: mark_segment(
            PointF { x: 9.0, y: 17.0 },
            PointF { x: 4.0, y: 12.0 },
            check_background,
        ),
        mixed: mark_segment(
            PointF { x: 5.0, y: 12.0 },
            PointF { x: 19.0, y: 12.0 },
            mixed_background,
        ),
    }
}

fn mark_segment(start: PointF, end: PointF, background: Background) -> BoxStyle {
    const SCALE: f32 = 14.0 / 24.0;
    const OFFSET: f32 = 1.0;
    let dx = (end.x - start.x) * SCALE;
    let dy = (end.y - start.y) * SCALE;
    let length = dx.hypot(dy);
    let stroke = 2.0 * SCALE;
    BoxStyle {
        width: SizeRule::Logical(length),
        height: SizeRule::Logical(stroke),
        max_size: SizeRule2D {
            width: SizeRule::Logical(length),
            height: SizeRule::Logical(stroke),
        },
        decoration: crate::ui::BoxDecoration {
            background,
            corner_radii: CornerRadii::all(stroke * 0.5),
            ..crate::ui::BoxDecoration::default()
        },
        transform: Transform2D {
            translation: PointF {
                x: OFFSET + start.x * SCALE,
                y: OFFSET + start.y * SCALE - stroke * 0.5,
            },
            rotation: dy.atan2(dx),
            origin: PointF { x: 0.0, y: 0.5 },
            ..Transform2D::default()
        },
        ..BoxStyle::default()
    }
}

#[derive(Clone, Copy)]
struct SwitchStyles {
    container: BoxStyle,
    track: BoxStyle,
    thumb: BoxStyle,
}

fn switch_styles(value: bool, enabled: bool) -> SwitchStyles {
    let opacity = if enabled { 255 } else { 180 };
    let track_color = if value {
        ColorRgba8::rgba(54, 104, 210, opacity)
    } else {
        ColorRgba8::rgba(75, 84, 102, opacity)
    };
    SwitchStyles {
        container: BoxStyle {
            min_size: SizeRule2D {
                width: SizeRule::Logical(32.0),
                height: SizeRule::Logical(32.0),
            },
            padding: EdgeInsets::all(5.0),
            ..BoxStyle::default()
        },
        track: BoxStyle {
            width: SizeRule::Logical(38.0),
            height: SizeRule::Logical(22.0),
            max_size: SizeRule2D {
                width: SizeRule::Logical(38.0),
                height: SizeRule::Logical(22.0),
            },
            padding: EdgeInsets::all(2.0),
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(track_color),
                border: Border::all(1.0, ColorRgba8::rgba(118, 127, 145, opacity)),
                corner_radii: CornerRadii::all(11.0),
                ..crate::ui::BoxDecoration::default()
            },
            ..BoxStyle::default()
        },
        thumb: BoxStyle {
            width: SizeRule::Logical(16.0),
            height: SizeRule::Logical(16.0),
            max_size: SizeRule2D {
                width: SizeRule::Logical(16.0),
                height: SizeRule::Logical(16.0),
            },
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(ColorRgba8::rgba(248, 249, 252, opacity)),
                corner_radii: CornerRadii::all(8.0),
                ..crate::ui::BoxDecoration::default()
            },
            transform: Transform2D {
                translation: PointF {
                    x: if value { 16.0 } else { 0.0 },
                    y: 0.0,
                },
                ..Transform2D::default()
            },
            ..BoxStyle::default()
        },
    }
}

#[derive(Clone, Copy)]
struct SliderStyles {
    container: BoxStyle,
    track: BoxStyle,
    fill: BoxStyle,
    thumb: BoxStyle,
}

fn slider_styles(value: f32, enabled: bool) -> SliderStyles {
    let value = value.clamp(0.0, 1.0);
    let opacity = if enabled { 255 } else { 180 };
    let accent = ColorRgba8::rgba(54, 104, 210, opacity);
    SliderStyles {
        container: BoxStyle {
            min_size: SizeRule2D {
                width: SizeRule::Logical(32.0),
                height: SizeRule::Logical(32.0),
            },
            padding: EdgeInsets::all(5.0),
            ..BoxStyle::default()
        },
        track: BoxStyle {
            width: SizeRule::Logical(160.0),
            height: SizeRule::Logical(6.0),
            max_size: SizeRule2D {
                width: SizeRule::Logical(160.0),
                height: SizeRule::Logical(6.0),
            },
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(ColorRgba8::rgba(78, 87, 105, opacity)),
                corner_radii: CornerRadii::all(3.0),
                ..crate::ui::BoxDecoration::default()
            },
            ..BoxStyle::default()
        },
        fill: BoxStyle {
            width: SizeRule::Logical(160.0 * value),
            height: SizeRule::Logical(6.0),
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(accent),
                corner_radii: CornerRadii::all(3.0),
                ..crate::ui::BoxDecoration::default()
            },
            ..BoxStyle::default()
        },
        thumb: BoxStyle {
            width: SizeRule::Logical(18.0),
            height: SizeRule::Logical(18.0),
            max_size: SizeRule2D {
                width: SizeRule::Logical(18.0),
                height: SizeRule::Logical(18.0),
            },
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(ColorRgba8::rgba(245, 247, 251, opacity)),
                border: Border::all(1.0, accent),
                corner_radii: CornerRadii::all(9.0),
                ..crate::ui::BoxDecoration::default()
            },
            transform: Transform2D {
                translation: PointF {
                    x: (160.0 - 18.0) * value,
                    y: (6.0 - 18.0) * 0.5,
                },
                ..Transform2D::default()
            },
            ..BoxStyle::default()
        },
    }
}
