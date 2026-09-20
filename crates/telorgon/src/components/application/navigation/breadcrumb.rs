//! Typed, controller-validated application breadcrumb trails.
//!
//! A breadcrumb is a stateless presentation of one [`NavigationController`] stack snapshot. It
//! never owns route history, changes the current route, or invokes a URL/native navigation service.

use std::fmt;
use std::rc::Rc;

use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::input::ChangeSource;
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, BoxStyle, ControlHandle, CornerRadii, Flow, LayoutStyle, Property, SemanticActions,
    SemanticCollection, SemanticName, SemanticNode, SemanticParticipation, SemanticRelationship,
    SemanticRelationshipKind, SemanticRole, SemanticState, SizeRule, SizeRule2D, UiNodeId,
};

use crate::components::application::{DensityClass, DensityMetrics, NavigationController};

/// One labelled route in a root-to-current breadcrumb trail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreadcrumbItem<R> {
    route: R,
    label: String,
}

impl<R> BreadcrumbItem<R> {
    pub fn new(route: R, label: impl Into<String>) -> Result<Self, BreadcrumbItemError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(BreadcrumbItemError::MissingAccessibleName);
        }
        Ok(Self { route, label })
    }

    pub const fn route(&self) -> &R {
        &self.route
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreadcrumbItemError {
    MissingAccessibleName,
}

impl fmt::Display for BreadcrumbItemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("breadcrumb item accessible name is empty")
    }
}

impl std::error::Error for BreadcrumbItemError {}

/// Source-preserving ancestor-route proposal. The application applies it through the existing
/// navigation owner; creating this value is nonmutating.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreadcrumbSelectionRequest<R> {
    route: R,
    source: ChangeSource,
}

impl<R> BreadcrumbSelectionRequest<R> {
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

/// Typed visual slots for the trail, ancestor links, current item, and decorative separator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BreadcrumbStyle {
    pub container: BoxStyle,
    pub ancestor: BoxStyle,
    pub current: BoxStyle,
    pub ancestor_color: ColorRgba8,
    pub current_color: ColorRgba8,
    pub separator_color: ColorRgba8,
    pub label_size: f32,
    pub separator_size: f32,
    pub gap: f32,
}

impl Default for BreadcrumbStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            ancestor: BoxStyle {
                padding: EdgeInsets::all(6.0),
                decoration: crate::ui::BoxDecoration {
                    corner_radii: CornerRadii::all(4.0),
                    ..crate::ui::BoxDecoration::default()
                },
                ..BoxStyle::default()
            },
            current: BoxStyle {
                padding: EdgeInsets::all(6.0),
                decoration: crate::ui::BoxDecoration {
                    background: Background::Color(ColorRgba8::rgba(65, 75, 94, 96)),
                    corner_radii: CornerRadii::all(4.0),
                    ..crate::ui::BoxDecoration::default()
                },
                ..BoxStyle::default()
            },
            ancestor_color: ColorRgba8::rgba(177, 202, 247, 255),
            current_color: ColorRgba8::rgba(242, 245, 250, 255),
            separator_color: ColorRgba8::rgba(135, 145, 163, 255),
            label_size: 14.0,
            separator_size: 14.0,
            gap: 2.0,
        }
    }
}

/// Immutable labels and styling for a navigation-controller trail.
#[derive(Clone, Debug, PartialEq)]
pub struct Breadcrumb<R> {
    label: String,
    items: Vec<BreadcrumbItem<R>>,
    density: DensityMetrics,
    style: BreadcrumbStyle,
}

impl<R> Breadcrumb<R>
where
    R: Clone + Eq + 'static,
{
    pub fn new(
        label: impl Into<String>,
        items: impl IntoIterator<Item = BreadcrumbItem<R>>,
    ) -> Result<Self, BreadcrumbError<R>> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(BreadcrumbError::MissingAccessibleName);
        }
        let items: Vec<_> = items.into_iter().collect();
        if items.is_empty() {
            return Err(BreadcrumbError::Empty);
        }
        for (index, item) in items.iter().enumerate() {
            if items[..index].iter().any(|other| other.route == item.route) {
                return Err(BreadcrumbError::DuplicateRoute(item.route.clone()));
            }
        }
        Ok(Self {
            label,
            items,
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: BreadcrumbStyle::default(),
        })
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: BreadcrumbStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn items(&self) -> &[BreadcrumbItem<R>] {
        &self.items
    }

    /// Proves that this full displayed trail exactly describes the controller's root-to-current
    /// stack snapshot.
    pub fn validate(&self, navigation: &NavigationController<R>) -> Result<(), BreadcrumbError<R>> {
        if self.items.len() != navigation.entries().len() {
            return Err(BreadcrumbError::TrailLengthMismatch {
                expected: navigation.entries().len(),
                actual: self.items.len(),
            });
        }
        for (index, (item, entry)) in self.items.iter().zip(navigation.entries()).enumerate() {
            if item.route != *entry.route() {
                return Err(BreadcrumbError::TrailRouteMismatch {
                    index,
                    expected: entry.route().clone(),
                    actual: item.route.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn request_ancestor(
        &self,
        navigation: &NavigationController<R>,
        route: &R,
        source: ChangeSource,
    ) -> Result<BreadcrumbSelectionRequest<R>, BreadcrumbError<R>> {
        self.validate(navigation)?;
        let Some(index) = self.items.iter().position(|item| &item.route == route) else {
            return Err(BreadcrumbError::UnknownRoute(route.clone()));
        };
        if index + 1 == self.items.len() {
            return Err(BreadcrumbError::CurrentRoute(route.clone()));
        }
        Ok(BreadcrumbSelectionRequest {
            route: route.clone(),
            source,
        })
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        navigation: &NavigationController<R>,
        map: Map,
    ) -> RuntimeResult<BreadcrumbRef<R>>
    where
        Action: 'static,
        Map: Fn(BreadcrumbSelectionRequest<R>) -> Action + 'static,
    {
        self.validate(navigation)
            .map_err(|_| RuntimeError::new("breadcrumb trail does not match navigation"))?;
        let minimum = self.density.effective_minimum();
        let item_count = u32::try_from(self.items.len())
            .map_err(|_| RuntimeError::new("breadcrumb trail exceeds semantic item capacity"))?;
        let display = self.items.clone();
        let mut mounted_items = Vec::with_capacity(display.len());
        let mut separators = Vec::with_capacity(display.len().saturating_sub(1));
        let root = ui
            .foundation()
            .container_node_under(
                host,
                self.style.container,
                LayoutStyle {
                    flow: Flow::Horizontal,
                    gap: self.style.gap,
                    ..LayoutStyle::default()
                },
                |writer| {
                    let display_len = display.len();
                    for (index, item) in display.into_iter().enumerate() {
                        let is_current = index + 1 == display_len;
                        let label = item.label.clone();
                        let node = if is_current {
                            writer.container(self.style.current, LayoutStyle::default(), |writer| {
                                writer.text(label, self.style.current_color, self.style.label_size);
                            })
                        } else {
                            let mut style = self.style.ancestor;
                            style.min_size = SizeRule2D {
                                width: SizeRule::Logical(minimum.width()),
                                height: SizeRule::Logical(minimum.height()),
                            };
                            writer
                                .action_node(style, true, |writer| {
                                    writer.text(
                                        label,
                                        self.style.ancestor_color,
                                        self.style.label_size,
                                    );
                                })
                                .node
                        };
                        mounted_items.push((item, node, is_current));
                        if !is_current {
                            separators.push(
                                writer
                                    .text(
                                        "›",
                                        self.style.separator_color,
                                        self.style.separator_size,
                                    )
                                    .node,
                            );
                        }
                    }
                },
            )
            .ok_or_else(|| RuntimeError::new("application breadcrumb host is stale"))?;

        let map: Rc<dyn Fn(BreadcrumbSelectionRequest<R>) -> Action> = Rc::new(map);
        let mut item_refs = Vec::with_capacity(mounted_items.len());
        for (index, (item, node, is_current)) in mounted_items.iter().enumerate() {
            let name = ui.foundation().intern(&item.label);
            let semantic = if *is_current {
                SemanticNode {
                    role: SemanticRole::ListItem,
                    name: SemanticName::Text(name),
                    state: SemanticState {
                        selected: Some(true),
                        ..SemanticState::default()
                    },
                    collection: Some(SemanticCollection {
                        item_index: u32::try_from(index).ok(),
                        item_count: Some(item_count),
                        position_in_set: u32::try_from(index + 1).ok(),
                        set_size: Some(item_count),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                }
            } else {
                SemanticNode {
                    role: SemanticRole::Link,
                    name: SemanticName::Text(name),
                    state: SemanticState {
                        focusable: true,
                        ..SemanticState::default()
                    },
                    actions: SemanticActions::ACTIVATE,
                    collection: Some(SemanticCollection {
                        item_index: u32::try_from(index).ok(),
                        item_count: Some(item_count),
                        position_in_set: u32::try_from(index + 1).ok(),
                        set_size: Some(item_count),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                }
            };
            ui.foundation()
                .semantic_node(*node, semantic)
                .map_err(|error| {
                    RuntimeError::new(format!("invalid breadcrumb item semantics: {error:?}"))
                })?;
            if !is_current {
                let route = item.route.clone();
                let route_map = map.clone();
                ui.route_activation(*node, move |activation| {
                    route_map(BreadcrumbSelectionRequest {
                        route: route.clone(),
                        source: activation.source,
                    })
                })?;
            }
            item_refs.push(BreadcrumbItemRef {
                route: item.route.clone(),
                node: *node,
                current: *is_current,
            });
        }
        for separator in &separators {
            ui.foundation()
                .semantic_node(
                    *separator,
                    SemanticNode {
                        participation: SemanticParticipation::Exclude,
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid breadcrumb separator semantics: {error:?}"))
                })?;
        }

        let name = ui.foundation().intern(&self.label);
        let relationships = item_refs
            .iter()
            .map(|item| SemanticRelationship {
                kind: SemanticRelationshipKind::Owns,
                target: item.node,
            })
            .collect();
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::List,
                    name: SemanticName::Text(name),
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
                RuntimeError::new(format!("invalid breadcrumb trail semantics: {error:?}"))
            })?;

        Ok(BreadcrumbRef {
            root,
            items: item_refs,
            separators,
        })
    }
}

#[derive(Clone, Debug)]
pub struct BreadcrumbRef<R> {
    root: ControlHandle,
    items: Vec<BreadcrumbItemRef<R>>,
    separators: Vec<UiNodeId>,
}

impl<R> BreadcrumbRef<R>
where
    R: Clone + Eq,
{
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub fn items(&self) -> &[BreadcrumbItemRef<R>] {
        &self.items
    }

    pub fn separators(&self) -> &[UiNodeId] {
        &self.separators
    }

    pub fn current(&self) -> &BreadcrumbItemRef<R> {
        self.items
            .last()
            .expect("validated breadcrumb always has a current item")
    }

    pub fn request_ancestor(
        &self,
        route: &R,
        source: ChangeSource,
    ) -> Result<BreadcrumbSelectionRequest<R>, BreadcrumbError<R>> {
        let Some(item) = self.items.iter().find(|item| &item.route == route) else {
            return Err(BreadcrumbError::UnknownRoute(route.clone()));
        };
        if item.current {
            return Err(BreadcrumbError::CurrentRoute(route.clone()));
        }
        Ok(BreadcrumbSelectionRequest {
            route: route.clone(),
            source,
        })
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Debug)]
pub struct BreadcrumbItemRef<R> {
    route: R,
    node: UiNodeId,
    current: bool,
}

impl<R> BreadcrumbItemRef<R> {
    pub const fn route(&self) -> &R {
        &self.route
    }

    pub const fn node(&self) -> UiNodeId {
        self.node
    }

    pub const fn is_current(&self) -> bool {
        self.current
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BreadcrumbError<R> {
    MissingAccessibleName,
    Empty,
    DuplicateRoute(R),
    TrailLengthMismatch {
        expected: usize,
        actual: usize,
    },
    TrailRouteMismatch {
        index: usize,
        expected: R,
        actual: R,
    },
    UnknownRoute(R),
    CurrentRoute(R),
}

impl<R: fmt::Debug> fmt::Display for BreadcrumbError<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "breadcrumb operation failed: {self:?}")
    }
}

impl<R: fmt::Debug> std::error::Error for BreadcrumbError<R> {}

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
        Projects,
        Document,
    }

    fn navigation() -> NavigationController<Route> {
        let mut navigation = NavigationController::new(Route::Home, None);
        navigation
            .push(Route::Projects, None, ChangeSource::Programmatic)
            .unwrap();
        navigation
            .push(Route::Document, None, ChangeSource::Programmatic)
            .unwrap();
        navigation
    }

    fn breadcrumb() -> Breadcrumb<Route> {
        Breadcrumb::new(
            "Document location",
            [
                BreadcrumbItem::new(Route::Home, "Home").unwrap(),
                BreadcrumbItem::new(Route::Projects, "Projects").unwrap(),
                BreadcrumbItem::new(Route::Document, "Document").unwrap(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn construction_and_controller_trail_validation_are_typed() {
        assert_eq!(
            BreadcrumbItem::new(Route::Home, " ").unwrap_err(),
            BreadcrumbItemError::MissingAccessibleName
        );
        assert_eq!(
            Breadcrumb::<Route>::new(" ", [BreadcrumbItem::new(Route::Home, "Home").unwrap()])
                .unwrap_err(),
            BreadcrumbError::MissingAccessibleName
        );
        assert_eq!(
            Breadcrumb::<Route>::new("Trail", []).unwrap_err(),
            BreadcrumbError::Empty
        );
        assert_eq!(
            Breadcrumb::new(
                "Trail",
                [
                    BreadcrumbItem::new(Route::Home, "Home").unwrap(),
                    BreadcrumbItem::new(Route::Home, "Again").unwrap(),
                ],
            )
            .unwrap_err(),
            BreadcrumbError::DuplicateRoute(Route::Home)
        );
        let navigation = navigation();
        breadcrumb().validate(&navigation).unwrap();
        let reordered = Breadcrumb::new(
            "Trail",
            [
                BreadcrumbItem::new(Route::Home, "Home").unwrap(),
                BreadcrumbItem::new(Route::Document, "Document").unwrap(),
                BreadcrumbItem::new(Route::Projects, "Projects").unwrap(),
            ],
        )
        .unwrap();
        assert!(matches!(
            reordered.validate(&navigation),
            Err(BreadcrumbError::TrailRouteMismatch { index: 1, .. })
        ));
    }

    #[test]
    fn only_ancestors_propose_source_preserving_nonmutating_selection() {
        let navigation = navigation();
        let breadcrumb = breadcrumb();
        let revision = navigation.revision();
        let request = breadcrumb
            .request_ancestor(&navigation, &Route::Projects, ChangeSource::Keyboard)
            .unwrap();
        assert_eq!(request.route(), &Route::Projects);
        assert_eq!(request.source(), ChangeSource::Keyboard);
        assert_eq!(navigation.current(), &Route::Document);
        assert_eq!(navigation.revision(), revision);
        assert_eq!(
            breadcrumb
                .request_ancestor(&navigation, &Route::Document, ChangeSource::Accessibility,),
            Err(BreadcrumbError::CurrentRoute(Route::Document))
        );
    }

    #[derive(Debug)]
    enum MountedAction {
        Requested(BreadcrumbSelectionRequest<Route>),
    }

    struct MountedBreadcrumb {
        mounted: Rc<RefCell<Option<BreadcrumbRef<Route>>>>,
        requests: Rc<RefCell<Vec<BreadcrumbSelectionRequest<Route>>>>,
    }

    struct MountedState {
        navigation: NavigationController<Route>,
        breadcrumb: Breadcrumb<Route>,
    }

    impl Component for MountedBreadcrumb {
        type State = MountedState;
        type Action = MountedAction;

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
            MountedState {
                navigation: navigation(),
                breadcrumb: breadcrumb().density(DensityMetrics::baseline(DensityClass::Touch)),
            }
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.mounted.replace(Some(
                state
                    .breadcrumb
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
        runtime: ViewRuntime<ComponentRuntimeDriver<MountedBreadcrumb>>,
        mounted: Rc<RefCell<Option<BreadcrumbRef<Route>>>>,
        requests: Rc<RefCell<Vec<BreadcrumbSelectionRequest<Route>>>>,
    }

    fn mounted() -> Harness {
        let mounted = Rc::new(RefCell::new(None));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let runtime = ViewRuntime::from_component(MountedBreadcrumb {
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
    fn mounted_trail_has_ordered_semantics_decorative_separators_and_no_current_action() {
        let harness = mounted();
        let mounted = harness.mounted.borrow();
        let mounted = mounted.as_ref().unwrap();
        let root = harness.runtime.ui().semantics.get(mounted.node()).unwrap();
        assert_eq!(root.role, SemanticRole::List);
        assert_eq!(root.collection.unwrap().item_count, Some(3));
        assert_eq!(root.relationships.len(), 3);
        for (index, item) in mounted.items().iter().enumerate() {
            let semantic = harness.runtime.ui().semantics.get(item.node()).unwrap();
            assert_eq!(
                semantic.collection.unwrap().position_in_set,
                Some(u32::try_from(index + 1).unwrap())
            );
            if item.is_current() {
                assert_eq!(semantic.role, SemanticRole::ListItem);
                assert_eq!(semantic.state.selected, Some(true));
                assert!(semantic.actions.is_empty());
                assert!(harness.runtime.ui().interactions.get(item.node()).is_none());
            } else {
                assert_eq!(semantic.role, SemanticRole::Link);
                assert!(semantic.actions.contains(SemanticAction::Activate));
                assert!(
                    harness
                        .runtime
                        .ui()
                        .interactions
                        .get(item.node())
                        .unwrap()
                        .focusable
                );
                assert_eq!(
                    harness
                        .runtime
                        .ui()
                        .box_styles
                        .get(item.node())
                        .unwrap()
                        .min_size,
                    SizeRule2D {
                        width: SizeRule::Logical(44.0),
                        height: SizeRule::Logical(44.0),
                    }
                );
            }
        }
        assert!(mounted.separators().iter().all(|separator| {
            harness
                .runtime
                .ui()
                .semantics
                .get(*separator)
                .is_some_and(|semantic| semantic.participation == SemanticParticipation::Exclude)
        }));
    }

    #[test]
    fn mounted_ancestor_routes_sources_and_current_route_does_not_dispatch() {
        let mut harness = mounted();
        let (ancestor, current) = {
            let mounted = harness.mounted.borrow();
            let mounted = mounted.as_ref().unwrap();
            (mounted.items()[1].node(), mounted.current().node())
        };
        assert!(
            harness
                .runtime
                .dispatch_activation(ancestor, ChangeSource::Pointer)
        );
        assert!(
            !harness
                .runtime
                .dispatch_activation(current, ChangeSource::Accessibility)
        );
        assert_eq!(
            &*harness.requests.borrow(),
            &[BreadcrumbSelectionRequest {
                route: Route::Projects,
                source: ChangeSource::Pointer,
            }]
        );
    }
}
