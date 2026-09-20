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
    Settings,
}

fn tabs(policy: TabActivationPolicy) -> Tabs<Route> {
    Tabs::new(
        "Primary sections",
        [
            Tab::new(Route::Home, "Home").unwrap(),
            Tab::new(Route::Library, "Library").unwrap(),
            Tab::new(Route::Settings, "Settings").unwrap(),
        ],
    )
    .unwrap()
    .policy(TabPolicy {
        activation: policy,
        ..TabPolicy::default()
    })
}

#[test]
fn construction_and_selected_route_validation_are_atomic() {
    assert_eq!(
        Tab::new(Route::Home, " ").unwrap_err(),
        TabError::MissingAccessibleName
    );
    assert_eq!(
        Tabs::<Route>::new(" ", [Tab::new(Route::Home, "Home").unwrap()]).unwrap_err(),
        TabsError::MissingAccessibleName
    );
    assert_eq!(
        Tabs::<Route>::new("Tabs", []).unwrap_err(),
        TabsError::Empty
    );
    assert_eq!(
        Tabs::new(
            "Tabs",
            [
                Tab::new(Route::Home, "First").unwrap(),
                Tab::new(Route::Home, "Second").unwrap(),
            ],
        )
        .unwrap_err(),
        TabsError::DuplicateRoute(Route::Home)
    );
    let navigation = NavigationController::new(Route::Settings, None);
    let home_only = Tabs::new("Tabs", [Tab::new(Route::Home, "Home").unwrap()]).unwrap();
    assert!(matches!(
        home_only.behavior(&navigation),
        Err(TabsError::SelectedRouteMissing(Route::Settings))
    ));
}

#[test]
fn automatic_and_manual_navigation_keep_focus_distinct_from_navigation_selection() {
    let navigation = NavigationController::new(Route::Home, None);
    let mut automatic = tabs(TabActivationPolicy::AutomaticLocal)
        .behavior(&navigation)
        .unwrap();
    let moved = automatic
        .navigate(
            CompositeNavigationCommand::Right,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    assert_eq!(moved.kind(), TabNavigationKind::FocusMoved);
    assert_eq!(moved.previous_focus(), Some(&Route::Home));
    assert_eq!(moved.focused(), Some(&Route::Library));
    assert_eq!(moved.selection().unwrap().route(), &Route::Library);
    assert_eq!(
        moved.selection().unwrap().source(),
        ChangeSource::Directional
    );
    assert_eq!(navigation.current(), &Route::Home);

    let mut manual = tabs(TabActivationPolicy::Manual)
        .behavior(&navigation)
        .unwrap();
    let focused = manual
        .navigate(
            CompositeNavigationCommand::Left,
            WritingDirection::RightToLeft,
        )
        .unwrap();
    assert_eq!(focused.focused(), Some(&Route::Library));
    assert!(focused.selection().is_none());
    manual
        .navigate(
            CompositeNavigationCommand::End,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    let requested = manual
        .request_focused_selection(ChangeSource::Keyboard)
        .unwrap();
    assert_eq!(requested.route(), &Route::Settings);
    assert_eq!(requested.source(), ChangeSource::Keyboard);
    assert_eq!(navigation.current(), &Route::Home);
}

#[derive(Debug)]
enum MountedAction {
    Requested(TabSelectionRequest<Route>),
}

struct MountedTabs {
    mounted: Rc<RefCell<Option<TabsRef<Route>>>>,
    requests: Rc<RefCell<Vec<TabSelectionRequest<Route>>>>,
}

struct MountedState {
    navigation: NavigationController<Route>,
    tabs: Tabs<Route>,
}

impl Component for MountedTabs {
    type State = MountedState;
    type Action = MountedAction;

    fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
        let mut navigation = NavigationController::new(Route::Home, None);
        navigation
            .push(Route::Library, None, ChangeSource::Programmatic)
            .unwrap();
        let tabs = tabs(TabActivationPolicy::Manual)
            .density(DensityMetrics::baseline(DensityClass::Touch));
        MountedState { navigation, tabs }
    }

    fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let root = ui
            .foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
        self.mounted.replace(Some(
            state
                .tabs
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
        match action {
            MountedAction::Requested(request) => self.requests.borrow_mut().push(request),
        }
    }
}

struct Harness {
    runtime: ViewRuntime<ComponentRuntimeDriver<MountedTabs>>,
    mounted: Rc<RefCell<Option<TabsRef<Route>>>>,
    requests: Rc<RefCell<Vec<TabSelectionRequest<Route>>>>,
}

fn mounted() -> Harness {
    let mounted = Rc::new(RefCell::new(None));
    let requests = Rc::new(RefCell::new(Vec::new()));
    let runtime = ViewRuntime::from_component(MountedTabs {
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
fn mounted_tabs_have_one_focus_entry_tab_panel_relationships_and_touch_density() {
    let harness = mounted();
    let mounted = harness.mounted.borrow();
    let mounted = mounted.as_ref().unwrap();
    assert!(
        harness
            .runtime
            .ui()
            .interactions
            .get(mounted.node())
            .unwrap()
            .focusable
    );
    assert!(mounted.tabs().iter().all(|tab| {
        !harness
            .runtime
            .ui()
            .interactions
            .get(tab.node())
            .is_some_and(|interaction| interaction.focusable)
    }));
    assert_eq!(mounted.selected_panel().unwrap().route(), &Route::Library);
    for (tab, panel) in mounted.tabs().iter().zip(mounted.panels()) {
        let tab_semantics = harness.runtime.ui().semantics.get(tab.node()).unwrap();
        assert_eq!(tab_semantics.role, SemanticRole::Tab);
        assert_eq!(tab_semantics.state.selected, Some(tab.is_selected()));
        assert_eq!(
            tab_semantics.relationships[0].kind,
            SemanticRelationshipKind::Controls
        );
        assert_eq!(tab_semantics.relationships[0].target, panel.node());
        let panel_semantics = harness.runtime.ui().semantics.get(panel.node()).unwrap();
        assert_eq!(panel_semantics.role, SemanticRole::TabPanel);
        assert_eq!(panel_semantics.state.hidden, !panel.is_selected());
        assert_eq!(
            panel_semantics.relationships[0].kind,
            SemanticRelationshipKind::LabelledBy
        );
        assert_eq!(panel_semantics.relationships[0].target, tab.node());
        assert_eq!(
            harness
                .runtime
                .ui()
                .box_styles
                .get(tab.node())
                .unwrap()
                .min_size,
            SizeRule2D {
                width: SizeRule::Logical(44.0),
                height: SizeRule::Logical(44.0),
            }
        );
    }
    assert!(
        harness
            .runtime
            .ui()
            .semantics
            .get(mounted.node())
            .unwrap()
            .actions
            .contains(SemanticAction::Focus)
    );
}

#[test]
fn mounted_item_and_focused_activation_preserve_sources() {
    let mut harness = mounted();
    let (root, home) = {
        let mounted = harness.mounted.borrow();
        let mounted = mounted.as_ref().unwrap();
        (mounted.node(), mounted.tabs()[0].node())
    };
    assert!(
        harness
            .runtime
            .dispatch_activation(home, ChangeSource::Pointer)
    );
    assert!(
        harness
            .runtime
            .dispatch_activation(root, ChangeSource::Accessibility)
    );
    assert_eq!(
        &*harness.requests.borrow(),
        &[
            TabSelectionRequest {
                route: Route::Home,
                source: ChangeSource::Pointer,
            },
            TabSelectionRequest {
                route: Route::Home,
                source: ChangeSource::Accessibility,
            },
        ]
    );
}
