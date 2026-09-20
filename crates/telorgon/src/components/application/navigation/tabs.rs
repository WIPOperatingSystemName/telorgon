//! Controlled application tabs over the shared navigation and composite owners.
//!
//! Tabs retain transient focus only. The [`NavigationController`] remains the selected-route
//! owner, and route content/keep-alive state remains the responsibility of the later route host.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::input::{
    ChangeSource, CompositeChange, CompositeEdgeBehavior, CompositeError, CompositeItem,
    CompositeNavigationCommand, CompositeNavigationPolicy, CompositeOrientation,
    CompositeSelectionBehavior, CompositeStateMachine, DisabledItemPolicy, WritingDirection,
};
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, BoxStyle, ControlHandle, CornerRadii, Flow, LayoutStyle, Property, SemanticActions,
    SemanticName, SemanticNode, SemanticRelationship, SemanticRelationshipKind, SemanticRole,
    SemanticState, SizeRule, SizeRule2D, UiNodeId,
};

use crate::components::application::{DensityClass, DensityMetrics, NavigationController};

/// One stable typed route represented by a tab and a matching empty panel slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab<R> {
    route: R,
    label: String,
    enabled: bool,
}

impl<R> Tab<R> {
    pub fn new(route: R, label: impl Into<String>) -> Result<Self, TabError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TabError::MissingAccessibleName);
        }
        Ok(Self {
            route,
            label,
            enabled: true,
        })
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub const fn route(&self) -> &R {
        &self.route
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabError {
    MissingAccessibleName,
}

impl fmt::Display for TabError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("tab accessible name is empty")
    }
}

impl std::error::Error for TabError {}

/// Whether focus movement immediately proposes route selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabActivationPolicy {
    /// Immediate selection is valid only when every panel is local and latency-free.
    #[default]
    AutomaticLocal,
    /// Arrows move focus only; Enter, Space, pointer, or semantic activation requests selection.
    Manual,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/// Fixed tab-list navigation policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabPolicy {
    pub activation: TabActivationPolicy,
    pub orientation: TabOrientation,
    pub edge_behavior: CompositeEdgeBehavior,
}

impl Default for TabPolicy {
    fn default() -> Self {
        Self {
            activation: TabActivationPolicy::AutomaticLocal,
            orientation: TabOrientation::Horizontal,
            edge_behavior: CompositeEdgeBehavior::Wrap,
        }
    }
}

/// Source-preserving controlled proposal emitted by tabs.
///
/// The application decides whether this route should be pushed, replaced, or selected through its
/// [`NavigationController`]. Constructing the proposal never mutates navigation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabSelectionRequest<R> {
    route: R,
    source: ChangeSource,
}

impl<R> TabSelectionRequest<R> {
    pub const fn route(&self) -> &R {
        &self.route
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub fn into_route(self) -> R {
        self.route
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabNavigationKind {
    FocusMoved,
    Boundary,
    Ignored,
    Unchanged,
}

/// Result of one arrow/Home/End operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabNavigation<R> {
    kind: TabNavigationKind,
    previous_focus: Option<R>,
    focused: Option<R>,
    selection: Option<TabSelectionRequest<R>>,
}

impl<R> TabNavigation<R> {
    pub const fn kind(&self) -> TabNavigationKind {
        self.kind
    }

    pub const fn previous_focus(&self) -> Option<&R> {
        self.previous_focus.as_ref()
    }

    pub const fn focused(&self) -> Option<&R> {
        self.focused.as_ref()
    }

    pub const fn selection(&self) -> Option<&TabSelectionRequest<R>> {
        self.selection.as_ref()
    }

    pub fn into_selection(self) -> Option<TabSelectionRequest<R>> {
        self.selection
    }
}

/// The only transient focus owner used by a tab list.
#[derive(Clone, Debug)]
pub struct TabBehavior<R> {
    tabs: Vec<Tab<R>>,
    policy: TabPolicy,
    composite: CompositeStateMachine<usize>,
}

impl<R> TabBehavior<R>
where
    R: Clone + Eq,
{
    fn new(tabs: &[Tab<R>], selected: &R, policy: TabPolicy) -> Result<Self, TabsError<R>> {
        let Some(selected_index) = tabs.iter().position(|tab| &tab.route == selected) else {
            return Err(TabsError::SelectedRouteMissing(selected.clone()));
        };
        let composite_policy = CompositeNavigationPolicy {
            orientation: match policy.orientation {
                TabOrientation::Horizontal => CompositeOrientation::Horizontal,
                TabOrientation::Vertical => CompositeOrientation::Vertical,
            },
            edge_behavior: policy.edge_behavior,
            disabled_items: DisabledItemPolicy::Skip,
            selection: match policy.activation {
                TabActivationPolicy::AutomaticLocal => CompositeSelectionBehavior::FollowsHighlight,
                TabActivationPolicy::Manual => CompositeSelectionBehavior::Independent,
            },
        };
        let mut composite = CompositeStateMachine::new(composite_policy);
        composite
            .update_items(tabs.iter().enumerate().map(|(key, tab)| CompositeItem {
                key,
                enabled: tab.enabled,
            }))
            .map_err(TabsError::Composite)?;
        composite
            .enter(Some(selected_index))
            .map_err(TabsError::Composite)?;
        Ok(Self {
            tabs: tabs.to_vec(),
            policy,
            composite,
        })
    }

    pub const fn policy(&self) -> TabPolicy {
        self.policy
    }

    pub fn focused_route(&self) -> Option<&R> {
        self.composite
            .active_descendant()
            .map(|index| &self.tabs[index].route)
    }

    pub fn navigate(
        &mut self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<TabNavigation<R>, TabsError<R>> {
        let previous_focus = self.focused_route().cloned();
        let change = self
            .composite
            .navigate(command, direction)
            .map_err(TabsError::Composite)?;
        let focused = self.focused_route().cloned();
        let selection = match change {
            CompositeChange::Highlighted {
                selection_request: Some(request),
                ..
            } => Some(TabSelectionRequest {
                route: self.tabs[request.key].route.clone(),
                source: request.source,
            }),
            _ => None,
        };
        let kind = match change {
            CompositeChange::Highlighted { .. }
            | CompositeChange::Entered { .. }
            | CompositeChange::Left { .. }
            | CompositeChange::Rooted { .. } => TabNavigationKind::FocusMoved,
            CompositeChange::Boundary { .. } => TabNavigationKind::Boundary,
            CompositeChange::Ignored { .. } => TabNavigationKind::Ignored,
            CompositeChange::Unchanged => TabNavigationKind::Unchanged,
        };
        Ok(TabNavigation {
            kind,
            previous_focus,
            focused,
            selection,
        })
    }

    pub fn request_focused_selection(
        &mut self,
        source: ChangeSource,
    ) -> Result<TabSelectionRequest<R>, TabsError<R>> {
        let request = self
            .composite
            .request_active_selection(source)
            .map_err(TabsError::Composite)?;
        Ok(TabSelectionRequest {
            route: self.tabs[request.key].route.clone(),
            source: request.source,
        })
    }

    pub fn request_route_selection(
        &mut self,
        route: &R,
        source: ChangeSource,
    ) -> Result<TabSelectionRequest<R>, TabsError<R>> {
        let Some(index) = self.tabs.iter().position(|tab| &tab.route == route) else {
            return Err(TabsError::UnknownRoute(route.clone()));
        };
        if !self.tabs[index].enabled {
            return Err(TabsError::DisabledRoute(route.clone()));
        }
        self.composite
            .set_active_descendant(index)
            .map_err(TabsError::Composite)?;
        Ok(TabSelectionRequest {
            route: route.clone(),
            source,
        })
    }
}

/// Typed mount-time visual slots for a tab list and its empty panel targets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabsStyle {
    pub container: BoxStyle,
    pub tab_list: BoxStyle,
    pub tab: BoxStyle,
    pub selected_tab: BoxStyle,
    pub panel: BoxStyle,
    pub label_color: ColorRgba8,
    pub selected_label_color: ColorRgba8,
    pub disabled_label_color: ColorRgba8,
    pub label_size: f32,
    pub gap: f32,
    pub panel_gap: f32,
}

impl Default for TabsStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            tab_list: BoxStyle::default(),
            tab: BoxStyle {
                padding: EdgeInsets {
                    top: 6.0,
                    right: 12.0,
                    bottom: 6.0,
                    left: 12.0,
                },
                decoration: crate::ui::BoxDecoration {
                    corner_radii: CornerRadii::all(5.0),
                    ..crate::ui::BoxDecoration::default()
                },
                ..BoxStyle::default()
            },
            selected_tab: BoxStyle {
                padding: EdgeInsets {
                    top: 6.0,
                    right: 12.0,
                    bottom: 6.0,
                    left: 12.0,
                },
                decoration: crate::ui::BoxDecoration {
                    background: Background::Color(ColorRgba8::rgba(62, 83, 122, 180)),
                    corner_radii: CornerRadii::all(5.0),
                    ..crate::ui::BoxDecoration::default()
                },
                ..BoxStyle::default()
            },
            panel: BoxStyle::default(),
            label_color: ColorRgba8::rgba(210, 216, 228, 255),
            selected_label_color: ColorRgba8::rgba(247, 249, 252, 255),
            disabled_label_color: ColorRgba8::rgba(137, 145, 160, 255),
            label_size: 14.0,
            gap: 4.0,
            panel_gap: 8.0,
        }
    }
}

/// Immutable tab-list configuration. Selected route is always read from navigation at behavior
/// creation or mount time and is never copied into this configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct Tabs<R> {
    label: String,
    tabs: Vec<Tab<R>>,
    policy: TabPolicy,
    density: DensityMetrics,
    style: TabsStyle,
}

impl<R> Tabs<R>
where
    R: Clone + Eq + 'static,
{
    pub fn new(
        label: impl Into<String>,
        tabs: impl IntoIterator<Item = Tab<R>>,
    ) -> Result<Self, TabsError<R>> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TabsError::MissingAccessibleName);
        }
        let tabs: Vec<_> = tabs.into_iter().collect();
        if tabs.is_empty() {
            return Err(TabsError::Empty);
        }
        for (index, tab) in tabs.iter().enumerate() {
            if tabs[..index].iter().any(|other| other.route == tab.route) {
                return Err(TabsError::DuplicateRoute(tab.route.clone()));
            }
        }
        Ok(Self {
            label,
            tabs,
            policy: TabPolicy::default(),
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: TabsStyle::default(),
        })
    }

    pub fn policy(mut self, policy: TabPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: TabsStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn tabs(&self) -> &[Tab<R>] {
        &self.tabs
    }

    pub fn behavior(
        &self,
        navigation: &NavigationController<R>,
    ) -> Result<TabBehavior<R>, TabsError<R>> {
        TabBehavior::new(&self.tabs, navigation.current(), self.policy)
    }

    /// Mounts tabs plus empty panel slots. The later route host owns panel content and keep-alive.
    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        navigation: &NavigationController<R>,
        map: Map,
    ) -> RuntimeResult<TabsRef<R>>
    where
        Action: 'static,
        Map: Fn(TabSelectionRequest<R>) -> Action + 'static,
    {
        let selected = navigation.current();
        let behavior = Rc::new(RefCell::new(
            self.behavior(navigation)
                .map_err(|_| RuntimeError::new("invalid tabs state"))?,
        ));
        let active = behavior.borrow().focused_route().cloned();
        let minimum = self.density.effective_minimum();
        let flow = match self.policy.orientation {
            TabOrientation::Horizontal => Flow::Horizontal,
            TabOrientation::Vertical => Flow::Vertical,
        };
        let tab_items = self.tabs.clone();
        let panel_items = self.tabs.clone();
        let mut mounted_tabs = Vec::with_capacity(tab_items.len());
        let mut mounted_panels = Vec::with_capacity(panel_items.len());
        let root = ui
            .foundation()
            .button_node_under(host, self.style.container, |writer| {
                writer.container(
                    self.style.tab_list,
                    LayoutStyle {
                        flow,
                        gap: self.style.gap,
                        ..LayoutStyle::default()
                    },
                    |writer| {
                        for tab in tab_items {
                            let is_selected = &tab.route == selected;
                            let mut style = if is_selected {
                                self.style.selected_tab
                            } else {
                                self.style.tab
                            };
                            style.min_size = SizeRule2D {
                                width: SizeRule::Logical(minimum.width()),
                                height: SizeRule::Logical(minimum.height()),
                            };
                            let color = if !tab.enabled {
                                self.style.disabled_label_color
                            } else if is_selected {
                                self.style.selected_label_color
                            } else {
                                self.style.label_color
                            };
                            let label = tab.label.clone();
                            let control = writer.action_node(style, false, |writer| {
                                writer.text(label, color, self.style.label_size);
                            });
                            mounted_tabs.push((tab, control, is_selected));
                        }
                    },
                );
                writer.container(
                    BoxStyle::default(),
                    LayoutStyle {
                        gap: self.style.panel_gap,
                        ..LayoutStyle::default()
                    },
                    |writer| {
                        for tab in panel_items {
                            let is_selected = &tab.route == selected;
                            let control = writer.layer(
                                is_selected,
                                self.style.panel,
                                LayoutStyle::default(),
                                |_| {},
                            );
                            mounted_panels.push((tab.route, control, is_selected));
                        }
                    },
                );
            })
            .ok_or_else(|| RuntimeError::new("application tabs host is stale"))?;

        let map: Rc<dyn Fn(TabSelectionRequest<R>) -> Action> = Rc::new(map);
        let mut tab_refs = Vec::with_capacity(mounted_tabs.len());
        let mut panel_refs = Vec::with_capacity(mounted_panels.len());
        for (index, ((tab, control, is_selected), (route, panel, panel_selected))) in
            mounted_tabs.iter().zip(mounted_panels.iter()).enumerate()
        {
            debug_assert_eq!(is_selected, panel_selected);
            let tab_name = ui.foundation().intern(&tab.label);
            let mut actions = SemanticActions::NONE;
            if tab.enabled {
                actions |= SemanticActions::ACTIVATE | SemanticActions::SELECT;
            }
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        role: SemanticRole::Tab,
                        name: SemanticName::Text(tab_name),
                        state: SemanticState {
                            disabled: !tab.enabled,
                            selected: Some(*is_selected),
                            ..SemanticState::default()
                        },
                        actions,
                        relationships: vec![SemanticRelationship {
                            kind: SemanticRelationshipKind::Controls,
                            target: panel.node,
                        }],
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| RuntimeError::new(format!("invalid tab semantics: {error:?}")))?;
            ui.foundation()
                .semantic_node(
                    panel.node,
                    SemanticNode {
                        role: SemanticRole::TabPanel,
                        state: SemanticState {
                            hidden: !*panel_selected,
                            selected: Some(*panel_selected),
                            ..SemanticState::default()
                        },
                        relationships: vec![SemanticRelationship {
                            kind: SemanticRelationshipKind::LabelledBy,
                            target: control.node,
                        }],
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid tab-panel semantics: {error:?}"))
                })?;
            if !tab.enabled {
                ui.foundation().disabled(control.node, true);
            }
            if *is_selected {
                ui.foundation().selected(control.node, true);
            }
            if active.as_ref() == Some(&tab.route) {
                ui.foundation().highlighted(control.node, true);
            }
            if tab.enabled {
                let route = tab.route.clone();
                let route_behavior = behavior.clone();
                let route_map = map.clone();
                ui.route_activation_fallible(control.node, move |activation| {
                    let request = route_behavior
                        .borrow_mut()
                        .request_route_selection(&route, activation.source)
                        .map_err(|_| RuntimeError::new("tab activation failed"))?;
                    Ok(route_map(request))
                })?;
            }
            tab_refs.push(TabRef {
                route: tab.route.clone(),
                control: *control,
                panel: panel.node,
                enabled: tab.enabled,
                selected: *is_selected,
            });
            panel_refs.push(TabPanelRef {
                route: route.clone(),
                control: *panel,
                selected: *panel_selected,
            });
            debug_assert_eq!(index, tab_refs.len() - 1);
        }

        let any_enabled = self.tabs.iter().any(|tab| tab.enabled);
        let name = ui.foundation().intern(&self.label);
        let mut relationships = Vec::with_capacity(tab_refs.len() + panel_refs.len() + 1);
        relationships.extend(tab_refs.iter().map(|tab| SemanticRelationship {
            kind: SemanticRelationshipKind::Owns,
            target: tab.control.node,
        }));
        relationships.extend(panel_refs.iter().map(|panel| SemanticRelationship {
            kind: SemanticRelationshipKind::Owns,
            target: panel.control.node,
        }));
        if let Some(active) = active.as_ref()
            && let Some(tab) = tab_refs.iter().find(|tab| &tab.route == active)
        {
            relationships.push(SemanticRelationship {
                kind: SemanticRelationshipKind::ActiveDescendant,
                target: tab.control.node,
            });
        }
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Generic,
                    name: SemanticName::Text(name),
                    state: SemanticState {
                        disabled: !any_enabled,
                        focusable: any_enabled,
                        ..SemanticState::default()
                    },
                    actions: if any_enabled {
                        SemanticActions::FOCUS | SemanticActions::ACTIVATE
                    } else {
                        SemanticActions::NONE
                    },
                    relationships,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| RuntimeError::new(format!("invalid tab-list semantics: {error:?}")))?;

        if any_enabled {
            let route_behavior = behavior.clone();
            let route_map = map.clone();
            ui.route_activation_fallible(root.node, move |activation| {
                let request = route_behavior
                    .borrow_mut()
                    .request_focused_selection(activation.source)
                    .map_err(|_| RuntimeError::new("focused tab activation failed"))?;
                Ok(route_map(request))
            })?;
        }

        Ok(TabsRef {
            root,
            tabs: tab_refs,
            panels: panel_refs,
            behavior,
        })
    }
}

#[derive(Clone, Debug)]
pub struct TabsRef<R> {
    root: ControlHandle,
    tabs: Vec<TabRef<R>>,
    panels: Vec<TabPanelRef<R>>,
    behavior: Rc<RefCell<TabBehavior<R>>>,
}

impl<R> TabsRef<R>
where
    R: Clone + Eq + 'static,
{
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub fn tabs(&self) -> &[TabRef<R>] {
        &self.tabs
    }

    pub fn panels(&self) -> &[TabPanelRef<R>] {
        &self.panels
    }

    pub fn selected_panel(&self) -> Option<&TabPanelRef<R>> {
        self.panels.iter().find(|panel| panel.selected)
    }

    pub fn focused_route(&self) -> Option<R> {
        self.behavior.borrow().focused_route().cloned()
    }

    pub fn navigate(
        &self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<TabNavigation<R>, TabsError<R>> {
        self.behavior.borrow_mut().navigate(command, direction)
    }

    pub fn request_focused_selection(
        &self,
        source: ChangeSource,
    ) -> Result<TabSelectionRequest<R>, TabsError<R>> {
        self.behavior.borrow_mut().request_focused_selection(source)
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Debug)]
pub struct TabRef<R> {
    route: R,
    control: ControlHandle,
    panel: UiNodeId,
    enabled: bool,
    selected: bool,
}

impl<R> TabRef<R> {
    pub const fn route(&self) -> &R {
        &self.route
    }

    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn panel(&self) -> UiNodeId {
        self.panel
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub const fn is_selected(&self) -> bool {
        self.selected
    }
}

#[derive(Clone, Debug)]
pub struct TabPanelRef<R> {
    route: R,
    control: ControlHandle,
    selected: bool,
}

impl<R> TabPanelRef<R> {
    pub const fn route(&self) -> &R {
        &self.route
    }

    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn is_selected(&self) -> bool {
        self.selected
    }

    pub const fn visible(&self) -> Property<bool> {
        self.control.visible
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabsError<R> {
    MissingAccessibleName,
    Empty,
    DuplicateRoute(R),
    SelectedRouteMissing(R),
    UnknownRoute(R),
    DisabledRoute(R),
    Composite(CompositeError<usize>),
}

impl<R: fmt::Debug> fmt::Display for TabsError<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "tabs operation failed: {self:?}")
    }
}

impl<R: fmt::Debug> std::error::Error for TabsError<R> {}

#[cfg(test)]
mod tests;
