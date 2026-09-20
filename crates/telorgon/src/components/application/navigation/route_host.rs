//! Typed controlled route-content host.
//!
//! [`NavigationController`] remains the only route-history owner. This host validates stable
//! content registrations, derives an explicit bounded retention plan, mounts that snapshot, and
//! reports restoration identity without applying platform or application restoration state.

use std::fmt;

use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, MountWriter, Property, SemanticName, SemanticNode,
    SemanticRelationship, SemanticRelationshipKind, SemanticRole, SemanticState, UiNodeId,
};

use crate::components::application::{NavigationController, NavigationRestorationKey};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteHostRegistration<R> {
    route: R,
    label: String,
    retained_bytes: usize,
}

impl<R> RouteHostRegistration<R> {
    pub fn new(route: R, label: impl Into<String>) -> Result<Self, RouteHostRegistrationError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(RouteHostRegistrationError::MissingAccessibleName);
        }
        Ok(Self {
            route,
            label,
            retained_bytes: 0,
        })
    }

    /// Supplies the caller's conservative retained-state estimate for keep-alive budgeting.
    pub const fn retained_bytes(mut self, retained_bytes: usize) -> Self {
        self.retained_bytes = retained_bytes;
        self
    }

    pub const fn route(&self) -> &R {
        &self.route
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn estimated_retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteHostRegistrationError {
    MissingAccessibleName,
}

impl fmt::Display for RouteHostRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("route-host registration accessible name is empty")
    }
}

impl std::error::Error for RouteHostRegistrationError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RouteHostKeepAlivePolicy {
    /// Mount only the current route. Inactive route content has no implicit cache.
    #[default]
    None,
    /// Retain nearest inactive stack entries while both limits permit.
    Bounded {
        max_inactive_routes: usize,
        max_inactive_bytes: usize,
    },
}

impl RouteHostKeepAlivePolicy {
    pub fn bounded(
        max_inactive_routes: usize,
        max_inactive_bytes: usize,
    ) -> Result<Self, RouteHostKeepAlivePolicyError> {
        if max_inactive_routes == 0 {
            return Err(RouteHostKeepAlivePolicyError::ZeroRouteLimit);
        }
        if max_inactive_bytes == 0 {
            return Err(RouteHostKeepAlivePolicyError::ZeroByteBudget);
        }
        Ok(Self::Bounded {
            max_inactive_routes,
            max_inactive_bytes,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteHostKeepAlivePolicyError {
    ZeroRouteLimit,
    ZeroByteBudget,
}

impl fmt::Display for RouteHostKeepAlivePolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid route-host keep-alive policy: {self:?}")
    }
}

impl std::error::Error for RouteHostKeepAlivePolicyError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteHostContentState {
    Current,
    KeptAlive,
    Evicted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteHostRestorationIntent {
    Fresh,
    Restore(NavigationRestorationKey),
}

impl From<Option<NavigationRestorationKey>> for RouteHostRestorationIntent {
    fn from(restoration: Option<NavigationRestorationKey>) -> Self {
        restoration.map_or(Self::Fresh, Self::Restore)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteHostContentIntent<R> {
    route: R,
    state: RouteHostContentState,
    restoration: RouteHostRestorationIntent,
    retained_bytes: usize,
}

impl<R> RouteHostContentIntent<R> {
    pub const fn route(&self) -> &R {
        &self.route
    }

    pub const fn state(&self) -> RouteHostContentState {
        self.state
    }

    pub const fn restoration(&self) -> RouteHostRestorationIntent {
        self.restoration
    }

    pub const fn estimated_retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteHostPlan<R> {
    current: RouteHostContentIntent<R>,
    kept_alive: Vec<RouteHostContentIntent<R>>,
    evicted: Vec<RouteHostContentIntent<R>>,
    retained_bytes: usize,
}

impl<R> RouteHostPlan<R>
where
    R: Eq,
{
    pub const fn current(&self) -> &RouteHostContentIntent<R> {
        &self.current
    }

    /// Nearest inactive controller entry first.
    pub fn kept_alive(&self) -> &[RouteHostContentIntent<R>] {
        &self.kept_alive
    }

    /// Nearest budget-excluded controller entry first.
    pub fn evicted(&self) -> &[RouteHostContentIntent<R>] {
        &self.evicted
    }

    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub fn state_for(&self, route: &R) -> Option<RouteHostContentState> {
        if self.current.route() == route {
            return Some(RouteHostContentState::Current);
        }
        if self.kept_alive.iter().any(|intent| intent.route() == route) {
            return Some(RouteHostContentState::KeptAlive);
        }
        if self.evicted.iter().any(|intent| intent.route() == route) {
            return Some(RouteHostContentState::Evicted);
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RouteHostStyle {
    pub container: BoxStyle,
    pub content: BoxStyle,
    pub layout: LayoutStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RouteHost<R> {
    label: String,
    registrations: Vec<RouteHostRegistration<R>>,
    keep_alive: RouteHostKeepAlivePolicy,
    style: RouteHostStyle,
}

impl<R> RouteHost<R>
where
    R: Clone + Eq,
{
    pub fn new(
        label: impl Into<String>,
        registrations: impl IntoIterator<Item = RouteHostRegistration<R>>,
    ) -> Result<Self, RouteHostError<R>> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(RouteHostError::MissingAccessibleName);
        }
        let registrations: Vec<_> = registrations.into_iter().collect();
        if registrations.is_empty() {
            return Err(RouteHostError::Empty);
        }
        for (index, registration) in registrations.iter().enumerate() {
            if registrations[..index]
                .iter()
                .any(|other| other.route == registration.route)
            {
                return Err(RouteHostError::DuplicateRoute(registration.route.clone()));
            }
        }
        Ok(Self {
            label,
            registrations,
            keep_alive: RouteHostKeepAlivePolicy::None,
            style: RouteHostStyle::default(),
        })
    }

    pub fn keep_alive(mut self, keep_alive: RouteHostKeepAlivePolicy) -> Self {
        self.keep_alive = keep_alive;
        self
    }

    pub fn style(mut self, style: RouteHostStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn registrations(&self) -> &[RouteHostRegistration<R>] {
        &self.registrations
    }

    pub const fn keep_alive_policy(&self) -> RouteHostKeepAlivePolicy {
        self.keep_alive
    }

    pub fn plan(
        &self,
        navigation: &NavigationController<R>,
    ) -> Result<RouteHostPlan<R>, RouteHostError<R>> {
        for entry in navigation.entries() {
            if self.registration(entry.route()).is_none() {
                return Err(RouteHostError::MissingRoute(entry.route().clone()));
            }
        }
        let current_entry = navigation.current_entry();
        let current_registration = self
            .registration(current_entry.route())
            .expect("all controller entries were validated");
        let current = RouteHostContentIntent {
            route: current_entry.route().clone(),
            state: RouteHostContentState::Current,
            restoration: current_entry.restoration_key().into(),
            retained_bytes: current_registration.retained_bytes,
        };
        let mut kept_alive = Vec::new();
        let mut evicted = Vec::new();
        let mut retained_bytes = 0_usize;
        for entry in navigation.entries()[..navigation.depth() - 1].iter().rev() {
            let registration = self
                .registration(entry.route())
                .expect("all controller entries were validated");
            let fits = match self.keep_alive {
                RouteHostKeepAlivePolicy::None => false,
                RouteHostKeepAlivePolicy::Bounded {
                    max_inactive_routes,
                    max_inactive_bytes,
                } => {
                    kept_alive.len() < max_inactive_routes
                        && retained_bytes
                            .checked_add(registration.retained_bytes)
                            .is_some_and(|bytes| bytes <= max_inactive_bytes)
                }
            };
            let state = if fits {
                RouteHostContentState::KeptAlive
            } else {
                RouteHostContentState::Evicted
            };
            let intent = RouteHostContentIntent {
                route: entry.route().clone(),
                state,
                restoration: entry.restoration_key().into(),
                retained_bytes: registration.retained_bytes,
            };
            if fits {
                retained_bytes += registration.retained_bytes;
                kept_alive.push(intent);
            } else {
                evicted.push(intent);
            }
        }
        Ok(RouteHostPlan {
            current,
            kept_alive,
            evicted,
            retained_bytes,
        })
    }

    pub fn mount<'storage, Action, Content>(
        &self,
        ui: &mut Ui<'_, 'storage, Action>,
        host: UiNodeId,
        navigation: &NavigationController<R>,
        mut content: Content,
    ) -> RuntimeResult<RouteHostRef<R>>
    where
        Action: 'static,
        Content: FnMut(&R, &mut MountWriter<'storage, Action>),
    {
        let plan = self
            .plan(navigation)
            .map_err(|_| RuntimeError::new("invalid route-host navigation state"))?;
        let mut mounted = Vec::new();
        let root = ui
            .foundation()
            .container_node_under(host, self.style.container, self.style.layout, |writer| {
                for registration in &self.registrations {
                    let Some(state) = plan.state_for(&registration.route) else {
                        continue;
                    };
                    if state == RouteHostContentState::Evicted {
                        continue;
                    }
                    let visible = state == RouteHostContentState::Current;
                    let control = writer.layer(
                        visible,
                        self.style.content,
                        LayoutStyle::default(),
                        |writer| content(&registration.route, writer),
                    );
                    mounted.push((registration.clone(), control, state));
                }
            })
            .ok_or_else(|| RuntimeError::new("application route-host parent is stale"))?;

        let mut slots = Vec::with_capacity(mounted.len());
        for (registration, control, state) in mounted {
            let is_current = state == RouteHostContentState::Current;
            let name = ui.foundation().intern(&registration.label);
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        role: SemanticRole::Generic,
                        name: SemanticName::Text(name),
                        state: SemanticState {
                            inert: !is_current,
                            hidden: !is_current,
                            selected: Some(is_current),
                            ..SemanticState::default()
                        },
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid route content semantics: {error:?}"))
                })?;
            slots.push(RouteHostSlotRef {
                route: registration.route,
                control,
                state,
            });
        }

        let name = ui.foundation().intern(&self.label);
        let relationships = slots
            .iter()
            .map(|slot| SemanticRelationship {
                kind: SemanticRelationshipKind::Owns,
                target: slot.control.node,
            })
            .collect();
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Generic,
                    name: SemanticName::Text(name),
                    relationships,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid route-host semantics: {error:?}"))
            })?;

        Ok(RouteHostRef { root, slots, plan })
    }

    fn registration(&self, route: &R) -> Option<&RouteHostRegistration<R>> {
        self.registrations
            .iter()
            .find(|registration| &registration.route == route)
    }
}

#[derive(Clone, Debug)]
pub struct RouteHostRef<R> {
    root: ControlHandle,
    slots: Vec<RouteHostSlotRef<R>>,
    plan: RouteHostPlan<R>,
}

impl<R> RouteHostRef<R> {
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub fn slots(&self) -> &[RouteHostSlotRef<R>] {
        &self.slots
    }

    pub const fn plan(&self) -> &RouteHostPlan<R> {
        &self.plan
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Debug)]
pub struct RouteHostSlotRef<R> {
    route: R,
    control: ControlHandle,
    state: RouteHostContentState,
}

impl<R> RouteHostSlotRef<R> {
    pub const fn route(&self) -> &R {
        &self.route
    }

    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn state(&self) -> RouteHostContentState {
        self.state
    }

    pub const fn is_visible(&self) -> bool {
        matches!(self.state, RouteHostContentState::Current)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteHostError<R> {
    MissingAccessibleName,
    Empty,
    DuplicateRoute(R),
    MissingRoute(R),
}

impl<R: fmt::Debug> fmt::Display for RouteHostError<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "route-host operation failed: {self:?}")
    }
}

impl<R: fmt::Debug> std::error::Error for RouteHostError<R> {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::foundation::ColorRgba8;
    use crate::input::ChangeSource;
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::UiRoot;

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Route {
        Home,
        Details,
        Editor,
        Missing,
    }

    fn key(value: u64) -> NavigationRestorationKey {
        NavigationRestorationKey::from_raw(value).unwrap()
    }

    fn host() -> RouteHost<Route> {
        RouteHost::new(
            "Application content",
            [
                RouteHostRegistration::new(Route::Home, "Home content")
                    .unwrap()
                    .retained_bytes(40),
                RouteHostRegistration::new(Route::Details, "Details content")
                    .unwrap()
                    .retained_bytes(60),
                RouteHostRegistration::new(Route::Editor, "Editor content")
                    .unwrap()
                    .retained_bytes(80),
            ],
        )
        .unwrap()
    }

    fn navigation() -> NavigationController<Route> {
        let mut navigation = NavigationController::new(Route::Home, Some(key(1)));
        navigation
            .push(Route::Details, Some(key(2)), ChangeSource::Programmatic)
            .unwrap();
        navigation
            .push(Route::Editor, Some(key(3)), ChangeSource::Programmatic)
            .unwrap();
        navigation
    }

    #[test]
    fn construction_and_bounded_policy_validation_are_typed() {
        assert_eq!(
            RouteHostRegistration::new(Route::Home, " ").unwrap_err(),
            RouteHostRegistrationError::MissingAccessibleName
        );
        assert_eq!(
            RouteHost::<Route>::new(
                " ",
                [RouteHostRegistration::new(Route::Home, "Home").unwrap()]
            )
            .unwrap_err(),
            RouteHostError::MissingAccessibleName
        );
        assert_eq!(
            RouteHost::<Route>::new("Content", []).unwrap_err(),
            RouteHostError::Empty
        );
        assert_eq!(
            RouteHost::new(
                "Content",
                [
                    RouteHostRegistration::new(Route::Home, "Home").unwrap(),
                    RouteHostRegistration::new(Route::Home, "Again").unwrap(),
                ]
            )
            .unwrap_err(),
            RouteHostError::DuplicateRoute(Route::Home)
        );
        assert_eq!(
            RouteHostKeepAlivePolicy::bounded(0, 1),
            Err(RouteHostKeepAlivePolicyError::ZeroRouteLimit)
        );
        assert_eq!(
            RouteHostKeepAlivePolicy::bounded(1, 0),
            Err(RouteHostKeepAlivePolicyError::ZeroByteBudget)
        );
    }

    #[test]
    fn plan_is_bounded_nearest_first_and_reports_restoration_without_applying_it() {
        let host = host().keep_alive(RouteHostKeepAlivePolicy::bounded(2, 70).unwrap());
        let plan = host.plan(&navigation()).unwrap();
        assert_eq!(plan.current().route(), &Route::Editor);
        assert_eq!(
            plan.current().restoration(),
            RouteHostRestorationIntent::Restore(key(3))
        );
        assert_eq!(plan.kept_alive().len(), 1);
        assert_eq!(plan.kept_alive()[0].route(), &Route::Details);
        assert_eq!(plan.retained_bytes(), 60);
        assert_eq!(plan.evicted().len(), 1);
        assert_eq!(plan.evicted()[0].route(), &Route::Home);
    }

    #[test]
    fn no_cache_is_default_and_missing_controller_routes_are_diagnostic() {
        let plan = host().plan(&navigation()).unwrap();
        assert!(plan.kept_alive().is_empty());
        assert_eq!(plan.evicted().len(), 2);
        let missing = NavigationController::new(Route::Missing, None);
        assert_eq!(
            host().plan(&missing),
            Err(RouteHostError::MissingRoute(Route::Missing))
        );
    }

    struct MountedHost {
        mounted: Rc<RefCell<Option<RouteHostRef<Route>>>>,
    }

    impl Component for MountedHost {
        type State = (NavigationController<Route>, RouteHost<Route>);
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
            (
                navigation(),
                host().keep_alive(RouteHostKeepAlivePolicy::bounded(1, 100).unwrap()),
            )
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.mounted.replace(Some(
                state
                    .1
                    .mount(ui, root.0, &state.0, |route, writer| {
                        writer.text(
                            format!("{route:?}"),
                            ColorRgba8::rgba(255, 255, 255, 255),
                            14.0,
                        );
                    })
                    .unwrap(),
            ));
            root
        }

        fn action(
            &self,
            _state: &mut Self::State,
            _action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
        }
    }

    #[test]
    fn mount_builds_only_current_and_budgeted_content_with_visibility_semantics() {
        let mounted = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(MountedHost {
            mounted: mounted.clone(),
        })
        .unwrap();
        let mounted = mounted.borrow();
        let mounted = mounted.as_ref().unwrap();
        assert_eq!(mounted.slots().len(), 2);
        assert!(
            mounted
                .slots()
                .iter()
                .all(|slot| slot.route() != &Route::Home)
        );
        let current = mounted
            .slots()
            .iter()
            .find(|slot| slot.route() == &Route::Editor)
            .unwrap();
        let retained = mounted
            .slots()
            .iter()
            .find(|slot| slot.route() == &Route::Details)
            .unwrap();
        assert!(current.is_visible());
        assert!(!retained.is_visible());
        assert!(
            runtime
                .ui()
                .interactions
                .get(current.node())
                .is_none_or(|interaction| interaction.visible)
        );
        assert!(
            !runtime
                .ui()
                .interactions
                .get(retained.node())
                .unwrap()
                .visible
        );
        let current_semantics = runtime.ui().semantics.get(current.node()).unwrap();
        let retained_semantics = runtime.ui().semantics.get(retained.node()).unwrap();
        assert!(!current_semantics.state.hidden);
        assert!(!current_semantics.state.inert);
        assert!(retained_semantics.state.hidden);
        assert!(retained_semantics.state.inert);
        assert_eq!(mounted.plan().retained_bytes(), 60);
    }
}
