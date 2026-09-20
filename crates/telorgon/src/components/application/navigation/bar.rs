//! Controlled horizontal application navigation destinations.
//!
//! The bar delegates transient focus to the neutral composite and reads selected route from
//! [`NavigationController`]. It emits route proposals but owns neither history nor route content.

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
    SemanticCollection, SemanticName, SemanticNode, SemanticRelationship, SemanticRelationshipKind,
    SemanticRole, SemanticState, SizeRule, SizeRule2D, UiNodeId,
};

use crate::components::application::{DensityClass, DensityMetrics, NavigationController};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationBarDestination<R> {
    route: R,
    label: String,
    enabled: bool,
}

impl<R> NavigationBarDestination<R> {
    pub fn new(route: R, label: impl Into<String>) -> Result<Self, NavigationBarDestinationError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(NavigationBarDestinationError::MissingAccessibleName);
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
pub enum NavigationBarDestinationError {
    MissingAccessibleName,
}

impl fmt::Display for NavigationBarDestinationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("navigation-bar destination accessible name is empty")
    }
}

impl std::error::Error for NavigationBarDestinationError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavigationBarPolicy {
    pub edge_behavior: CompositeEdgeBehavior,
}

impl Default for NavigationBarPolicy {
    fn default() -> Self {
        Self {
            edge_behavior: CompositeEdgeBehavior::Stop,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationBarSelectionRequest<R> {
    route: R,
    source: ChangeSource,
}

impl<R> NavigationBarSelectionRequest<R> {
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
pub enum NavigationBarNavigationKind {
    FocusMoved,
    Boundary,
    Ignored,
    Unchanged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationBarNavigation<R> {
    kind: NavigationBarNavigationKind,
    previous_focus: Option<R>,
    focused: Option<R>,
}

impl<R> NavigationBarNavigation<R> {
    pub const fn kind(&self) -> NavigationBarNavigationKind {
        self.kind
    }

    pub const fn previous_focus(&self) -> Option<&R> {
        self.previous_focus.as_ref()
    }

    pub const fn focused(&self) -> Option<&R> {
        self.focused.as_ref()
    }
}

/// Bar-specific wrapper over the single neutral composite focus owner.
#[derive(Clone, Debug)]
pub struct NavigationBarBehavior<R> {
    destinations: Vec<NavigationBarDestination<R>>,
    composite: CompositeStateMachine<usize>,
}

impl<R> NavigationBarBehavior<R>
where
    R: Clone + Eq,
{
    fn new(
        destinations: &[NavigationBarDestination<R>],
        selected: &R,
        policy: NavigationBarPolicy,
    ) -> Result<Self, NavigationBarError<R>> {
        let Some(selected_index) = destinations
            .iter()
            .position(|destination| &destination.route == selected)
        else {
            return Err(NavigationBarError::SelectedRouteMissing(selected.clone()));
        };
        let mut composite = CompositeStateMachine::new(CompositeNavigationPolicy {
            orientation: CompositeOrientation::Horizontal,
            edge_behavior: policy.edge_behavior,
            disabled_items: DisabledItemPolicy::Skip,
            selection: CompositeSelectionBehavior::Independent,
        });
        composite
            .update_items(
                destinations
                    .iter()
                    .enumerate()
                    .map(|(key, destination)| CompositeItem {
                        key,
                        enabled: destination.enabled,
                    }),
            )
            .map_err(NavigationBarError::Composite)?;
        composite
            .enter(Some(selected_index))
            .map_err(NavigationBarError::Composite)?;
        Ok(Self {
            destinations: destinations.to_vec(),
            composite,
        })
    }

    pub fn focused_route(&self) -> Option<&R> {
        self.composite
            .active_descendant()
            .map(|index| &self.destinations[index].route)
    }

    pub fn navigate(
        &mut self,
        command: CompositeNavigationCommand,
        writing_direction: WritingDirection,
    ) -> Result<NavigationBarNavigation<R>, NavigationBarError<R>> {
        let previous_focus = self.focused_route().cloned();
        let change = self
            .composite
            .navigate(command, writing_direction)
            .map_err(NavigationBarError::Composite)?;
        let focused = self.focused_route().cloned();
        let kind = match change {
            CompositeChange::Highlighted { .. }
            | CompositeChange::Entered { .. }
            | CompositeChange::Left { .. }
            | CompositeChange::Rooted { .. } => NavigationBarNavigationKind::FocusMoved,
            CompositeChange::Boundary { .. } => NavigationBarNavigationKind::Boundary,
            CompositeChange::Ignored { .. } => NavigationBarNavigationKind::Ignored,
            CompositeChange::Unchanged => NavigationBarNavigationKind::Unchanged,
        };
        Ok(NavigationBarNavigation {
            kind,
            previous_focus,
            focused,
        })
    }

    pub fn request_focused_selection(
        &mut self,
        source: ChangeSource,
    ) -> Result<NavigationBarSelectionRequest<R>, NavigationBarError<R>> {
        let request = self
            .composite
            .request_active_selection(source)
            .map_err(NavigationBarError::Composite)?;
        Ok(NavigationBarSelectionRequest {
            route: self.destinations[request.key].route.clone(),
            source: request.source,
        })
    }

    pub fn request_route_selection(
        &mut self,
        route: &R,
        source: ChangeSource,
    ) -> Result<NavigationBarSelectionRequest<R>, NavigationBarError<R>> {
        let Some(index) = self
            .destinations
            .iter()
            .position(|destination| &destination.route == route)
        else {
            return Err(NavigationBarError::UnknownRoute(route.clone()));
        };
        if !self.destinations[index].enabled {
            return Err(NavigationBarError::DisabledRoute(route.clone()));
        }
        self.composite
            .set_active_descendant(index)
            .map_err(NavigationBarError::Composite)?;
        Ok(NavigationBarSelectionRequest {
            route: route.clone(),
            source,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavigationBarStyle {
    pub container: BoxStyle,
    pub destination: BoxStyle,
    pub selected_destination: BoxStyle,
    pub label_color: ColorRgba8,
    pub selected_label_color: ColorRgba8,
    pub disabled_label_color: ColorRgba8,
    pub label_size: f32,
    pub gap: f32,
}

impl Default for NavigationBarStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            destination: BoxStyle {
                padding: EdgeInsets {
                    top: 5.0,
                    right: 8.0,
                    bottom: 5.0,
                    left: 8.0,
                },
                decoration: crate::ui::BoxDecoration {
                    corner_radii: CornerRadii::all(6.0),
                    ..crate::ui::BoxDecoration::default()
                },
                ..BoxStyle::default()
            },
            selected_destination: BoxStyle {
                padding: EdgeInsets {
                    top: 5.0,
                    right: 8.0,
                    bottom: 5.0,
                    left: 8.0,
                },
                decoration: crate::ui::BoxDecoration {
                    background: Background::Color(ColorRgba8::rgba(61, 84, 128, 180)),
                    corner_radii: CornerRadii::all(6.0),
                    ..crate::ui::BoxDecoration::default()
                },
                ..BoxStyle::default()
            },
            label_color: ColorRgba8::rgba(213, 220, 233, 255),
            selected_label_color: ColorRgba8::rgba(248, 250, 253, 255),
            disabled_label_color: ColorRgba8::rgba(137, 145, 160, 255),
            label_size: 13.0,
            gap: 3.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavigationBar<R> {
    label: String,
    destinations: Vec<NavigationBarDestination<R>>,
    policy: NavigationBarPolicy,
    density: DensityMetrics,
    style: NavigationBarStyle,
}

impl<R> NavigationBar<R>
where
    R: Clone + Eq + 'static,
{
    pub fn new(
        label: impl Into<String>,
        destinations: impl IntoIterator<Item = NavigationBarDestination<R>>,
    ) -> Result<Self, NavigationBarError<R>> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(NavigationBarError::MissingAccessibleName);
        }
        let destinations: Vec<_> = destinations.into_iter().collect();
        if destinations.is_empty() {
            return Err(NavigationBarError::Empty);
        }
        for (index, destination) in destinations.iter().enumerate() {
            if destinations[..index]
                .iter()
                .any(|other| other.route == destination.route)
            {
                return Err(NavigationBarError::DuplicateRoute(
                    destination.route.clone(),
                ));
            }
        }
        Ok(Self {
            label,
            destinations,
            policy: NavigationBarPolicy::default(),
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: NavigationBarStyle::default(),
        })
    }

    pub fn policy(mut self, policy: NavigationBarPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: NavigationBarStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn destinations(&self) -> &[NavigationBarDestination<R>] {
        &self.destinations
    }

    pub fn behavior(
        &self,
        navigation: &NavigationController<R>,
    ) -> Result<NavigationBarBehavior<R>, NavigationBarError<R>> {
        NavigationBarBehavior::new(&self.destinations, navigation.current(), self.policy)
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        navigation: &NavigationController<R>,
        map: Map,
    ) -> RuntimeResult<NavigationBarRef<R>>
    where
        Action: 'static,
        Map: Fn(NavigationBarSelectionRequest<R>) -> Action + 'static,
    {
        let selected = navigation.current();
        let behavior =
            Rc::new(RefCell::new(self.behavior(navigation).map_err(|_| {
                RuntimeError::new("invalid navigation bar state")
            })?));
        let active = behavior.borrow().focused_route().cloned();
        let minimum = self.density.effective_minimum();
        let item_count = u32::try_from(self.destinations.len())
            .map_err(|_| RuntimeError::new("navigation bar exceeds semantic item capacity"))?;
        let display = self.destinations.clone();
        let mut mounted = Vec::with_capacity(display.len());
        let root = ui
            .foundation()
            .button_node_under(host, self.style.container, |writer| {
                writer.container(
                    BoxStyle::default(),
                    LayoutStyle {
                        flow: Flow::Horizontal,
                        gap: self.style.gap,
                        ..LayoutStyle::default()
                    },
                    |writer| {
                        for destination in display {
                            let is_selected = &destination.route == selected;
                            let mut style = if is_selected {
                                self.style.selected_destination
                            } else {
                                self.style.destination
                            };
                            style.min_size = SizeRule2D {
                                width: SizeRule::Logical(minimum.width()),
                                height: SizeRule::Logical(minimum.height()),
                            };
                            let color = if !destination.enabled {
                                self.style.disabled_label_color
                            } else if is_selected {
                                self.style.selected_label_color
                            } else {
                                self.style.label_color
                            };
                            let label = destination.label.clone();
                            let control = writer.action_node(style, false, |writer| {
                                writer.text(label, color, self.style.label_size);
                            });
                            mounted.push((destination, control, is_selected));
                        }
                    },
                );
            })
            .ok_or_else(|| RuntimeError::new("application navigation-bar host is stale"))?;

        let map: Rc<dyn Fn(NavigationBarSelectionRequest<R>) -> Action> = Rc::new(map);
        let mut destination_refs = Vec::with_capacity(mounted.len());
        for (index, (destination, control, is_selected)) in mounted.iter().enumerate() {
            let name = ui.foundation().intern(&destination.label);
            let mut actions = SemanticActions::NONE;
            if destination.enabled {
                actions |= SemanticActions::ACTIVATE | SemanticActions::SELECT;
            }
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        role: SemanticRole::Link,
                        name: SemanticName::Text(name),
                        state: SemanticState {
                            disabled: !destination.enabled,
                            selected: Some(*is_selected),
                            ..SemanticState::default()
                        },
                        actions,
                        collection: Some(SemanticCollection {
                            item_index: u32::try_from(index).ok(),
                            item_count: Some(item_count),
                            position_in_set: u32::try_from(index + 1).ok(),
                            set_size: Some(item_count),
                            ..SemanticCollection::default()
                        }),
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!(
                        "invalid navigation-bar destination semantics: {error:?}"
                    ))
                })?;
            if !destination.enabled {
                ui.foundation().disabled(control.node, true);
            }
            if *is_selected {
                ui.foundation().selected(control.node, true);
            }
            if active.as_ref() == Some(&destination.route) {
                ui.foundation().highlighted(control.node, true);
            }
            if destination.enabled {
                let route = destination.route.clone();
                let route_behavior = behavior.clone();
                let route_map = map.clone();
                ui.route_activation_fallible(control.node, move |activation| {
                    let request = route_behavior
                        .borrow_mut()
                        .request_route_selection(&route, activation.source)
                        .map_err(|_| {
                            RuntimeError::new("navigation-bar destination activation failed")
                        })?;
                    Ok(route_map(request))
                })?;
            }
            destination_refs.push(NavigationBarDestinationRef {
                route: destination.route.clone(),
                control: *control,
                enabled: destination.enabled,
                selected: *is_selected,
            });
        }

        let any_enabled = self
            .destinations
            .iter()
            .any(|destination| destination.enabled);
        let name = ui.foundation().intern(&self.label);
        let mut relationships: Vec<_> = destination_refs
            .iter()
            .map(|destination| SemanticRelationship {
                kind: SemanticRelationshipKind::Owns,
                target: destination.control.node,
            })
            .collect();
        if let Some(active) = active.as_ref()
            && let Some(destination) = destination_refs
                .iter()
                .find(|destination| &destination.route == active)
        {
            relationships.push(SemanticRelationship {
                kind: SemanticRelationshipKind::ActiveDescendant,
                target: destination.control.node,
            });
        }
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::List,
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
                    collection: Some(SemanticCollection {
                        item_count: Some(item_count),
                        set_size: Some(item_count),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid navigation-bar semantics: {error:?}"))
            })?;

        if any_enabled {
            let route_behavior = behavior.clone();
            let route_map = map.clone();
            ui.route_activation_fallible(root.node, move |activation| {
                let request = route_behavior
                    .borrow_mut()
                    .request_focused_selection(activation.source)
                    .map_err(|_| RuntimeError::new("focused navigation-bar destination failed"))?;
                Ok(route_map(request))
            })?;
        }

        Ok(NavigationBarRef {
            root,
            destinations: destination_refs,
            behavior,
        })
    }
}

#[derive(Clone, Debug)]
pub struct NavigationBarRef<R> {
    root: ControlHandle,
    destinations: Vec<NavigationBarDestinationRef<R>>,
    behavior: Rc<RefCell<NavigationBarBehavior<R>>>,
}

impl<R> NavigationBarRef<R>
where
    R: Clone + Eq,
{
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub fn destinations(&self) -> &[NavigationBarDestinationRef<R>] {
        &self.destinations
    }

    pub fn focused_route(&self) -> Option<R> {
        self.behavior.borrow().focused_route().cloned()
    }

    pub fn navigate(
        &self,
        command: CompositeNavigationCommand,
        writing_direction: WritingDirection,
    ) -> Result<NavigationBarNavigation<R>, NavigationBarError<R>> {
        self.behavior
            .borrow_mut()
            .navigate(command, writing_direction)
    }

    pub fn request_focused_selection(
        &self,
        source: ChangeSource,
    ) -> Result<NavigationBarSelectionRequest<R>, NavigationBarError<R>> {
        self.behavior.borrow_mut().request_focused_selection(source)
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Debug)]
pub struct NavigationBarDestinationRef<R> {
    route: R,
    control: ControlHandle,
    enabled: bool,
    selected: bool,
}

impl<R> NavigationBarDestinationRef<R> {
    pub const fn route(&self) -> &R {
        &self.route
    }

    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub const fn is_selected(&self) -> bool {
        self.selected
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavigationBarError<R> {
    MissingAccessibleName,
    Empty,
    DuplicateRoute(R),
    SelectedRouteMissing(R),
    UnknownRoute(R),
    DisabledRoute(R),
    Composite(CompositeError<usize>),
}

impl<R: fmt::Debug> fmt::Display for NavigationBarError<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "navigation-bar operation failed: {self:?}")
    }
}

impl<R: fmt::Debug> std::error::Error for NavigationBarError<R> {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::runtime::{
        Component, ComponentRuntimeDriver, CreateContext, UpdateContext, ViewRuntime,
    };
    use crate::ui::{SemanticAction, UiRoot};

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Route {
        Home,
        Library,
        Disabled,
        Settings,
    }

    fn bar() -> NavigationBar<Route> {
        NavigationBar::new(
            "Primary compact navigation",
            [
                NavigationBarDestination::new(Route::Home, "Home").unwrap(),
                NavigationBarDestination::new(Route::Library, "Library").unwrap(),
                NavigationBarDestination::new(Route::Disabled, "Disabled")
                    .unwrap()
                    .enabled(false),
                NavigationBarDestination::new(Route::Settings, "Settings").unwrap(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn construction_and_selected_route_validation_are_typed() {
        assert_eq!(
            NavigationBarDestination::new(Route::Home, " ").unwrap_err(),
            NavigationBarDestinationError::MissingAccessibleName
        );
        assert_eq!(
            NavigationBar::<Route>::new(
                " ",
                [NavigationBarDestination::new(Route::Home, "Home").unwrap()]
            )
            .unwrap_err(),
            NavigationBarError::MissingAccessibleName
        );
        assert_eq!(
            NavigationBar::<Route>::new("Bar", []).unwrap_err(),
            NavigationBarError::Empty
        );
        assert_eq!(
            NavigationBar::new(
                "Bar",
                [
                    NavigationBarDestination::new(Route::Home, "Home").unwrap(),
                    NavigationBarDestination::new(Route::Home, "Again").unwrap(),
                ],
            )
            .unwrap_err(),
            NavigationBarError::DuplicateRoute(Route::Home)
        );
        let missing = NavigationController::new(Route::Settings, None);
        let home_only = NavigationBar::new(
            "Bar",
            [NavigationBarDestination::new(Route::Home, "Home").unwrap()],
        )
        .unwrap();
        assert!(matches!(
            home_only.behavior(&missing),
            Err(NavigationBarError::SelectedRouteMissing(Route::Settings))
        ));
    }

    #[test]
    fn horizontal_navigation_is_rtl_aware_skips_disabled_and_does_not_select() {
        let navigation = NavigationController::new(Route::Home, None);
        let mut behavior = bar().behavior(&navigation).unwrap();
        let library = behavior
            .navigate(
                CompositeNavigationCommand::Right,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(library.kind(), NavigationBarNavigationKind::FocusMoved);
        assert_eq!(library.focused(), Some(&Route::Library));
        let settings = behavior
            .navigate(
                CompositeNavigationCommand::Left,
                WritingDirection::RightToLeft,
            )
            .unwrap();
        assert_eq!(settings.focused(), Some(&Route::Settings));
        assert_eq!(navigation.current(), &Route::Home);
        let ignored = behavior
            .navigate(
                CompositeNavigationCommand::Down,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(ignored.kind(), NavigationBarNavigationKind::Ignored);
        behavior
            .navigate(
                CompositeNavigationCommand::Home,
                WritingDirection::RightToLeft,
            )
            .unwrap();
        assert_eq!(behavior.focused_route(), Some(&Route::Home));
        behavior
            .navigate(
                CompositeNavigationCommand::End,
                WritingDirection::RightToLeft,
            )
            .unwrap();
        let request = behavior
            .request_focused_selection(ChangeSource::Keyboard)
            .unwrap();
        assert_eq!(request.route(), &Route::Settings);
        assert_eq!(request.source(), ChangeSource::Keyboard);
        assert_eq!(navigation.current(), &Route::Home);
    }

    #[derive(Debug)]
    enum MountedAction {
        Requested(NavigationBarSelectionRequest<Route>),
    }

    struct MountedBar {
        mounted: Rc<RefCell<Option<NavigationBarRef<Route>>>>,
        requests: Rc<RefCell<Vec<NavigationBarSelectionRequest<Route>>>>,
    }

    struct MountedState {
        navigation: NavigationController<Route>,
        bar: NavigationBar<Route>,
    }

    impl Component for MountedBar {
        type State = MountedState;
        type Action = MountedAction;

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
            MountedState {
                navigation: NavigationController::new(Route::Library, None),
                bar: bar().density(DensityMetrics::baseline(DensityClass::Touch)),
            }
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.mounted.replace(Some(
                state
                    .bar
                    .mount(ui, root.0, &state.navigation, MountedAction::Requested)
                    .unwrap(),
            ));
            root
        }

        fn action(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            let MountedAction::Requested(request) = action;
            self.requests.borrow_mut().push(request);
        }
    }

    struct Harness {
        runtime: ViewRuntime<ComponentRuntimeDriver<MountedBar>>,
        mounted: Rc<RefCell<Option<NavigationBarRef<Route>>>>,
        requests: Rc<RefCell<Vec<NavigationBarSelectionRequest<Route>>>>,
    }

    fn mounted() -> Harness {
        let mounted = Rc::new(RefCell::new(None));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let runtime = ViewRuntime::from_component(MountedBar {
            mounted: mounted.clone(),
            requests: requests.clone(),
        })
        .unwrap();
        Harness {
            runtime,
            mounted,
            requests,
        }
    }

    #[test]
    fn mounted_bar_is_horizontal_one_focus_entry_selected_and_touch_sized() {
        let harness = mounted();
        let mounted = harness.mounted.borrow();
        let mounted = mounted.as_ref().unwrap();
        let root = harness.runtime.ui().semantics.get(mounted.node()).unwrap();
        assert_eq!(root.role, SemanticRole::List);
        assert_eq!(root.collection.unwrap().item_count, Some(4));
        assert!(root.actions.contains(SemanticAction::Focus));
        assert!(
            harness
                .runtime
                .ui()
                .interactions
                .get(mounted.node())
                .unwrap()
                .focusable
        );
        for destination in mounted.destinations() {
            assert!(
                !harness
                    .runtime
                    .ui()
                    .interactions
                    .get(destination.node())
                    .is_some_and(|interaction| interaction.focusable)
            );
            let semantic = harness
                .runtime
                .ui()
                .semantics
                .get(destination.node())
                .unwrap();
            assert_eq!(semantic.role, SemanticRole::Link);
            assert_eq!(semantic.state.selected, Some(destination.is_selected()));
            assert_eq!(semantic.state.disabled, !destination.is_enabled());
            assert_eq!(
                harness
                    .runtime
                    .ui()
                    .box_styles
                    .get(destination.node())
                    .unwrap()
                    .min_size,
                SizeRule2D {
                    width: SizeRule::Logical(44.0),
                    height: SizeRule::Logical(44.0),
                }
            );
        }
        let disabled = harness
            .runtime
            .ui()
            .semantics
            .get(mounted.destinations()[2].node())
            .unwrap();
        assert!(disabled.effective_actions().is_empty());
    }

    #[test]
    fn mounted_destination_and_root_activation_preserve_sources() {
        let mut harness = mounted();
        let (root, settings) = {
            let mounted = harness.mounted.borrow();
            let mounted = mounted.as_ref().unwrap();
            (mounted.node(), mounted.destinations()[3].node())
        };
        assert!(
            harness
                .runtime
                .dispatch_activation(settings, ChangeSource::Pointer)
        );
        assert!(
            harness
                .runtime
                .dispatch_activation(root, ChangeSource::Accessibility)
        );
        assert_eq!(
            &*harness.requests.borrow(),
            &[
                NavigationBarSelectionRequest {
                    route: Route::Settings,
                    source: ChangeSource::Pointer,
                },
                NavigationBarSelectionRequest {
                    route: Route::Settings,
                    source: ChangeSource::Accessibility,
                },
            ]
        );
    }
}
