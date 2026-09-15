use telorgon::MountedComponent;
#[cfg(feature = "application-software")]
use telorgon::renderer_software::{SoftwareRenderer, SoftwareSurface, SoftwareTarget};
use telorgon::ui::UiRoot;
use telorgon::{
    ActivationInput, ActivationStateMachine, ActivationTransition, ActivityIndicator,
    ActivityMotionPreference, ActivityState, AdaptiveScaffold, AdmittedRequest,
    ApplicationOverlayController, ApplicationOverlayHost, ApplicationPopupPlacementRequest,
    AvoidRegion, AvoidRegionKind, Button, ButtonState, CapabilityDescriptor, CapabilityLimit,
    ChangePhase, ChangeSource, CheckCyclePolicy, CheckState, Checkbox, CloseRequest,
    CloseRequestDecision, CloseRequestReason, CoalescingMetadata, CollapsedEventCount, ColorRgba8,
    CompositeItem, CompositeNavigationCommand, CompositeNavigationPolicy, CompositeStateMachine,
    CoordinateSpace, CreateContext, DataOfferId, DensityMetrics, Dialog, DialogInitialFocus,
    DisplayColorSpace, DisplayProperties, DisplayTransform, EditHistoryCommand, EditHistoryKind,
    EditHistoryPolicy, EnvironmentReadBinding, EnvironmentReads, EnvironmentSnapshot,
    EnvironmentState, EnvironmentValues, EventStamp, EventStampStream, ExecutionRequirement,
    FieldMetadata, FieldSemanticSupport, FieldValidation, FocusCandidate, FocusIndicatorPolicy,
    FocusScopeId, FocusStateMachine, FocusTraversalDirection, FocusTraversalEdge,
    ForcedDestruction, ForcedDestructionPhase, Form, FormSubmission, GestureArena, GestureInput,
    HdrState, IconButton, ImageId, InputEvent, KeyEvent, LayoutEngine, LifecycleError, Link,
    LinkCommandKind, LinkDestination, MAX_REDRAW_VIEWS, Meter, MeterBand, MeterBands, MeterLevel,
    MetricInsets, MetricsCitation, MetricsRevision, Modifiers, MonotonicClock, MonotonicClockError,
    MonotonicClockState, MonotonicInstant, MountedUi, NativeSurfaceGeneration, NativeSurfaceState,
    NumericField, OutsidePressPolicy, OverlayHost, OverlayOpenRequest, PendingHostFacts,
    PermissionState, PhysicalExtent, PhysicalKey, PlatformError, PlatformErrorKind,
    PlatformErrorSource, PlatformEvent, PlatformResult, PointF, PointerButton, PointerId, Popup,
    PopupAnchor, PopupOverflowPolicy, PopupPlacementAlignment, PopupPlacementCandidate,
    PopupPlacementRequest, PostTurnSchedule, ProgressIndicator, ProgressValue, RadioGroup,
    RadioItem, RangeFormat, RangeMark, RangeModel, RectF, RemainingWork, RenderScene,
    RequestAdmission, RequestCompletion, RequestId, RequestOutcome, RetainedTextSystem, Scaffold,
    ScaffoldSlot, ScaffoldSlotSpec, ScaleFactor, SceneCompiler, ScheduleError, ScrollInputSource,
    ScrollState, SearchField, SecureField, SemanticAction, SemanticActions, SemanticName,
    SemanticNode, SemanticRole, ServiceKey, ServiceLookup, ServiceRegistration, ServiceRegistry,
    ServiceRemoval, ServiceReplacement, ServiceUnavailable, ShortcutBinding, ShortcutChord,
    ShortcutMatcher, ShortcutScopeId, SizeF, Slider, State, Support, Switch, TapRecognizer,
    TextAffinity, TextArea, TextAreaReturnPolicy, TextBuffer, TextCompositionCommand,
    TextController, TextEdit, TextEditBatch, TextField, TextFieldMode, TextInputConfiguration,
    TextInputSession, TextNavigationDirection, TextNavigationUnit, TextOffset, TextRange,
    TextSelection, TextSelectionAdjustment, TextSessionId, ToggleButton, Ui, UnavailableReason,
    UpdateContext, UserGestureRequirement, ValidationResult, ValidationSummary, ValueChange,
    ViewId, ViewLifecycle, ViewLifetime, ViewMetrics, ViewMetricsError, ViewMetricsSnapshot,
    ViewMetricsState, ViewRevision, ViewRuntime, ViewSnapshot, ViewState, ViewStateError,
    VisibilityState, WritingDirection,
};
#[cfg(any(
    feature = "application-software",
    all(feature = "application-vulkan-windows", target_os = "windows"),
    all(feature = "desktop-wayland-linux", target_os = "linux")
))]
use telorgon::{AppRuntime, WindowOptions};
#[cfg(feature = "application-software")]
use telorgon::{
    HeadlessRuntime, RenderBackend, RenderRequest, RenderResult, RenderSceneDelta,
    RenderTargetInfo, SizeI, TargetLoad, TargetStore,
};

#[cfg(any(
    feature = "application-software",
    all(feature = "application-vulkan-windows", target_os = "windows"),
    all(feature = "desktop-wayland-linux", target_os = "linux")
))]
struct ManagedFixture;

#[telorgon::component]
struct ComposedFixture {}

impl telorgon::Component for ComposedFixture {
    fn view(&self) -> impl telorgon::View {
        telorgon::text("Compile fixture")
    }
}

struct CompileClock(MonotonicInstant);

impl MonotonicClock for CompileClock {
    fn now(&mut self) -> MonotonicInstant {
        self.0
    }
}

enum CompileService {}

impl ServiceKey for CompileService {
    type Handle = u32;
}

fn compile_request_admission(request: RequestId) -> RequestAdmission<u32, ()> {
    Ok(AdmittedRequest::new(request))
}

struct ComponentFixture;
struct ComponentFixtureState(
    State<f32>,
    State<bool>,
    State<CheckState>,
    State<Option<u32>>,
    State<bool>,
    State<ProgressValue<f32>>,
);
struct ComponentFixtureAction(Box<f32>);

fn assert_environment_read_paths(
    context: &mut CreateContext<'_>,
    snapshot: EnvironmentSnapshot,
) -> EnvironmentReads {
    EnvironmentReadBinding::new(context, snapshot)
        .expect("validated environment snapshot binds to its creating component")
        .reads()
}

impl MountedComponent for ComponentFixture {
    type State = ComponentFixtureState;
    type Action = ComponentFixtureAction;

    fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
        ComponentFixtureState(
            context.state(1.0),
            context.state(false),
            context.state(CheckState::Unchecked),
            context.state(Some(1_u32)),
            context.state(true),
            context.state(ProgressValue::Determinate(1.0)),
        )
    }

    fn mount(&self, state: &Self::State, _ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let _toggle = ToggleButton::new("Compile toggle", state.1.read())
            .expect("toggle button has an accessible name");
        let _checkbox = Checkbox::new("Compile checkbox", state.2.read())
            .expect("checkbox has an accessible name")
            .cycle(CheckCyclePolicy::two_state());
        let _radio = RadioGroup::new(
            "Compile radio group",
            state.3.read(),
            [
                RadioItem::new(1_u32, "First").expect("radio item has an accessible name"),
                RadioItem::new(2_u32, "Second").expect("radio item has an accessible name"),
            ],
        )
        .expect("radio group has unique keys and an accessible name");
        let _switch =
            Switch::new("Compile switch", state.4.read()).expect("switch has an accessible name");
        let _slider = Slider::new(
            "Compile slider",
            state.0.read(),
            RangeModel::new(0.0_f32, 10.0, 1.0, 5.0).expect("valid slider range"),
        )
        .expect("slider has an accessible name");
        let _progress = ProgressIndicator::new(
            "Compile progress",
            state.5.read(),
            RangeModel::new(0.0_f32, 10.0, 1.0, 5.0).expect("valid progress range"),
        )
        .expect("progress has an accessible name");
        let _activity = ActivityIndicator::new("Compile activity", state.1.read())
            .expect("activity indicator has an accessible name")
            .motion_preference(ActivityMotionPreference::Reduced);
        let scaffold = Scaffold::new(
            "Compile application",
            [
                ScaffoldSlotSpec::new(ScaffoldSlot::Content, "Compile content")
                    .expect("scaffold content has an accessible name"),
            ],
        )
        .expect("scaffold has one named content slot");
        let _adaptive_scaffold = AdaptiveScaffold::new(scaffold);
        let meter_model = RangeModel::new(0.0_f32, 10.0, 1.0, 5.0).expect("valid meter range");
        let meter_bands =
            MeterBands::new(&meter_model, [MeterBand::new(10.0, MeterLevel::Neutral)])
                .expect("meter bands cover the range");
        let _meter = Meter::new("Compile meter", state.0.read(), meter_model, meter_bands)
            .expect("meter has an accessible name and matching bands");
        let mut text_controller = TextController::from_text("compile text")
            .expect("compile fixture text fits the neutral buffer");
        let _text_update = text_controller
            .replace_text(text_controller.revision(), "updated compile text")
            .expect("compile fixture replacement cites the current revision");
        text_controller
            .enable_edit_history(EditHistoryPolicy::default())
            .expect("ordinary compile fixture can enable bounded history");
        let _history_availability = text_controller.edit_history_availability();
        let _history_command = EditHistoryCommand::Undo;
        let _history_kind = EditHistoryKind::ProgrammaticReplacement;
        let _text_field = TextField::new(text_controller, "Compile field", TextFieldMode::Editable)
            .expect("compile fixture field has an accessible label");
        let _text_area = TextArea::new(
            TextController::from_text("compile\narea")
                .expect("compile fixture multiline text fits the neutral buffer"),
            "Compile area",
            TextFieldMode::Editable,
        )
        .expect("compile fixture area has an accessible label")
        .return_policy(TextAreaReturnPolicy::InsertNewline)
        .expect("newline is the valid default text-area return policy");
        let _search_field = SearchField::new(
            TextController::from_text("compile query")
                .expect("compile fixture search text fits the neutral buffer"),
            "Compile search",
            TextFieldMode::Editable,
        )
        .expect("compile fixture search field has an accessible label");
        let _numeric_field = NumericField::new(
            TextController::from_text("1.5")
                .expect("compile fixture numeric text fits the neutral buffer"),
            "Compile number",
            TextFieldMode::Editable,
            RangeModel::new(-10.0_f64, 10.0, 0.5, 2.0)
                .expect("compile fixture numeric constraints are valid"),
        )
        .expect("compile fixture numeric field has an accessible label and ordinary mode");
        let _secure_field = SecureField::new(
            TextController::from_text("compile credential")
                .expect("compile fixture secure text fits the neutral buffer"),
            "Compile password",
        )
        .expect("compile fixture secure field has an accessible label");
        unimplemented!("compile-only fixture")
    }

    fn action(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        context: &mut UpdateContext<'_, Self>,
    ) {
        let _ = context.set(state.0, *action.0);
        let _handle = context.spawn(async { ComponentFixtureAction(Box::new(1.0)) });
        let _progress = context.spawn_send_with_sender(1, |sender| async move {
            let _ = sender.try_send(ComponentFixtureAction(Box::new(0.5)));
            ComponentFixtureAction(Box::new(1.0))
        });
        let _timer = context.timer_at(
            telorgon::MonotonicInstant::from_nanos(1),
            ComponentFixtureAction(Box::new(1.0)),
        );
    }
}

#[cfg(any(
    feature = "application-software",
    all(feature = "application-vulkan-windows", target_os = "windows"),
    all(feature = "desktop-wayland-linux", target_os = "linux")
))]
impl MountedComponent for ManagedFixture {
    type State = ();
    type Action = ();

    fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

    fn mount(&self, _state: &Self::State, _ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        unimplemented!("compile-only fixture")
    }

    fn action(
        &self,
        _state: &mut Self::State,
        _action: (),
        _context: &mut UpdateContext<'_, Self>,
    ) {
    }
}

#[cfg(feature = "application-software")]
fn compile_headless_software_path() {
    let mut runtime = HeadlessRuntime::default();
    let _frame = runtime.run_composed_once(
        ComposedFixture::default(),
        SizeI {
            width: 640,
            height: 480,
        },
    );
}

fn compile_scene_path(
    compiler: &mut SceneCompiler,
    ui: &mut MountedUi,
    layout: &LayoutEngine,
    text: &mut RetainedTextSystem,
    scene: &mut RenderScene,
) {
    let _stats = compiler.compile(
        ui,
        layout,
        text,
        scene,
        SizeF {
            width: 640.0,
            height: 480.0,
        },
        ColorRgba8::rgba(0, 0, 0, 255),
    );
}

#[cfg(any(
    feature = "application-software",
    all(feature = "application-vulkan-windows", target_os = "windows"),
    all(feature = "desktop-wayland-linux", target_os = "linux")
))]
fn compile_renderer_free_runtime_path(runtime: &mut AppRuntime<ManagedFixture>) {
    let _needs_frame = runtime.needs_frame();
    let _prepared = runtime.prepare_frame(telorgon::MonotonicInstant::ZERO, false);
    let _delta = runtime.pop_scene_delta();
    let _snapshot = runtime.scene_snapshot();
}

fn compile_component_runtime_path() -> telorgon::runtime::RuntimeResult<
    ViewRuntime<telorgon::runtime::ComponentRuntimeDriver<ComponentFixture>>,
> {
    ViewRuntime::from_component(ComponentFixture)
}

#[cfg(feature = "application-software")]
fn compile_software_backend_path(delta: &RenderSceneDelta) -> RenderResult<()> {
    let backend = SoftwareRenderer;
    let mut scene = backend.create_scene()?;
    backend.apply_scene_delta(&mut scene, delta)?;
    let extent = SizeI {
        width: 640,
        height: 480,
    };
    let target = SoftwareTarget::new(RenderTargetInfo::full(extent));
    let mut surface = SoftwareSurface::default();
    let mut frame = surface.begin_frame();
    backend.render(
        &mut scene,
        &mut frame,
        &target,
        &RenderRequest {
            force: false,
            load: TargetLoad::Clear(ColorRgba8::default()),
            store: TargetStore::Store,
            region: None,
        },
    )?;
    Ok(())
}

fn assert_composed_component<C: telorgon::Component>() {}

fn compile_command_model_path(owner: telorgon::ComponentId, enabled: telorgon::Read<bool>) {
    struct NonCloneCommandAction(telorgon::ChangeSource);

    let factory = telorgon::ActionFactory::new(owner, NonCloneCommandAction);
    let shortcut = telorgon::CommandShortcut::new(
        telorgon::ShortcutChord::pressed(
            telorgon::PhysicalKey::new(7),
            telorgon::Modifiers::CONTROL,
        ),
        telorgon::ShortcutDisplayBinding::new("Ctrl+S").expect("visible shortcut display"),
    );
    let command = telorgon::CommandSpec::new(9_u64, "Save", enabled, factory)
        .expect("command reads and factory share an owner")
        .shortcuts(telorgon::ShortcutSet::single(shortcut));
    let invocation: telorgon::CommandInvocation<NonCloneCommandAction> = command.invoke(
        telorgon::ResolvedCommandState::new(true, None),
        telorgon::ChangeSource::Keyboard,
    );
    if let Some(action) = invocation.into_action() {
        let _source = action.0;
    }
    let scope = telorgon::ShortcutScopeId::from_raw(1, 1).expect("nonzero scope identity");
    let _registration: telorgon::CommandShortcutRegistration<u32, u64> = command
        .shortcut_registration(1, scope, 0)
        .expect("declared shortcut index");
    let _shortcut_scope: telorgon::CommandShortcutScope<u32, u64> =
        telorgon::CommandShortcutScope::new();
    let _toolbar: telorgon::Toolbar<u64, NonCloneCommandAction> =
        telorgon::Toolbar::new("Document actions", [command.clone()])
            .expect("named nonempty toolbar");
    let _menus: telorgon::MenuController<u64> = telorgon::MenuController::new();
    let _menu_request = telorgon::MenuOpenRequest::root(
        telorgon::OverlayAnchor::Point(telorgon::PointF { x: 4.0, y: 8.0 }),
        [telorgon::CompositeItem {
            key: *command.id(),
            enabled: true,
        }],
    );
    let _menu: telorgon::Menu<u64, NonCloneCommandAction> = telorgon::Menu::new(
        "Document menu",
        [telorgon::MenuItem::command(command.clone())],
    )
    .expect("named menu with unique shared commands");
    let _context_menu: telorgon::ContextMenu<u64> = telorgon::ContextMenu::new();
    let _context_request = telorgon::ContextMenuOpenRequest::programmatic(
        telorgon::OverlayAnchor::Point(telorgon::PointF { x: 4.0, y: 8.0 }),
        [telorgon::CompositeItem {
            key: *command.id(),
            enabled: true,
        }],
    );
    let _focused_palette: telorgon::application_components::command::CommandPalette<
        u64,
        NonCloneCommandAction,
    > = telorgon::application_components::command::CommandPalette::new(
        "Document commands",
        [command],
    )
    .expect("Tier B palette stays on its focused module path");
}

fn compile_navigation_controller_path() {
    let root_key =
        telorgon::NavigationRestorationKey::from_raw(1).expect("nonzero restoration key");
    let mut navigation = telorgon::NavigationController::new("home", Some(root_key));
    let _transition: telorgon::NavigationTransition<&str> = navigation
        .push("details", None, telorgon::ChangeSource::Programmatic)
        .expect("unique route can be pushed");
    let request: telorgon::NavigationSelectionRequest<&str> = navigation
        .request_selection("home", telorgon::ChangeSource::Accessibility)
        .expect("retained route can be selected");
    let _transition = navigation
        .select(request)
        .expect("validated route remains retained");
    let _diagnostics: telorgon::NavigationDiagnostics = navigation.diagnostics();
    let tabs = telorgon::Tabs::new(
        "Document sections",
        [
            telorgon::Tab::new("home", "Home").expect("named tab"),
            telorgon::Tab::new("details", "Details").expect("named tab"),
        ],
    )
    .expect("named tabs with unique routes")
    .policy(telorgon::TabPolicy {
        activation: telorgon::TabActivationPolicy::Manual,
        ..telorgon::TabPolicy::default()
    });
    let mut behavior: telorgon::TabBehavior<&str> = tabs
        .behavior(&navigation)
        .expect("selected navigation route has a matching tab");
    let _selection: telorgon::TabSelectionRequest<&str> = behavior
        .request_focused_selection(telorgon::ChangeSource::Keyboard)
        .expect("focused enabled tab can request selection");
    let breadcrumb = telorgon::Breadcrumb::new(
        "Document location",
        [telorgon::BreadcrumbItem::new("home", "Home").expect("named breadcrumb item")],
    )
    .expect("named nonempty breadcrumb");
    breadcrumb
        .validate(&navigation)
        .expect("breadcrumb matches the current controller trail");
    let rail = telorgon::NavigationRail::new(
        "Primary navigation",
        [
            telorgon::NavigationRailDestination::new("home", "Home")
                .expect("named rail destination"),
            telorgon::NavigationRailDestination::new("details", "Details")
                .expect("named rail destination"),
        ],
    )
    .expect("named rail with unique routes");
    let mut rail_behavior: telorgon::NavigationRailBehavior<&str> = rail
        .behavior(&navigation)
        .expect("selected navigation route has a matching rail destination");
    let _rail_selection: telorgon::NavigationRailSelectionRequest<&str> = rail_behavior
        .request_focused_selection(telorgon::ChangeSource::Keyboard)
        .expect("focused enabled rail destination can request selection");
    let bar = telorgon::NavigationBar::new(
        "Primary compact navigation",
        [
            telorgon::NavigationBarDestination::new("home", "Home").expect("named bar destination"),
            telorgon::NavigationBarDestination::new("details", "Details")
                .expect("named bar destination"),
        ],
    )
    .expect("named bar with unique routes");
    let mut bar_behavior: telorgon::NavigationBarBehavior<&str> = bar
        .behavior(&navigation)
        .expect("selected navigation route has a matching bar destination");
    let _bar_navigation: telorgon::NavigationBarNavigation<&str> = bar_behavior
        .navigate(
            telorgon::CompositeNavigationCommand::Home,
            telorgon::WritingDirection::RightToLeft,
        )
        .expect("bar navigation accepts explicit writing direction");
    let route_host: telorgon::application_components::navigation::RouteHost<&str> =
        telorgon::application_components::navigation::RouteHost::new(
            "Document content",
            [
                telorgon::application_components::navigation::RouteHostRegistration::new(
                    "home",
                    "Home content",
                )
                .expect("named route content"),
                telorgon::application_components::navigation::RouteHostRegistration::new(
                    "details",
                    "Details content",
                )
                .expect("named route content"),
            ],
        )
        .expect("named route host with stable unique registrations");
    let _route_plan: telorgon::application_components::navigation::RouteHostPlan<&str> = route_host
        .plan(&navigation)
        .expect("current controller route has registered content");
}

fn compile_selection_model_path() {
    let mut selection: telorgon::SelectionModel<&str> = telorgon::SelectionModel::new(
        telorgon::SelectionMode::Multiple,
        telorgon::SelectionFollowsFocus::Enabled,
        ["one", "two", "three"],
        ["one"],
        Some("one"),
    )
    .expect("stable unique collection keys and valid selection");
    let proposal: telorgon::SelectionProposal<&str> = selection
        .propose_focus(&"two", telorgon::ChangeSource::Directional)
        .expect("known focus key")
        .expect("enabled focus-selection policy proposes a change");
    let _transition: telorgon::SelectionTransition<&str> = selection
        .apply(proposal)
        .expect("current proposal applies atomically");
}

fn compile_list_view_path() {
    let mut list: telorgon::ListView<&str> = telorgon::ListView::new(
        "Recent documents",
        [
            telorgon::ListViewItem::new("one", "One").expect("named stable-key row"),
            telorgon::ListViewItem::new("two", "Two").expect("named stable-key row"),
        ],
    )
    .expect("named list with unique stable keys");
    let _update: telorgon::ListViewUpdate<&str> = list
        .update_items([
            telorgon::ListViewItem::new("two", "Two").expect("named stable-key row"),
            telorgon::ListViewItem::new("three", "Three").expect("named stable-key row"),
        ])
        .expect("controlled keyed snapshot updates atomically");
}

fn compile_virtual_list_path() {
    let list: telorgon::VirtualListView<&str> = telorgon::VirtualListView::new(
        "Results",
        [
            telorgon::ListViewItem::new("one", "One").expect("named stable-key row"),
            telorgon::ListViewItem::new("two", "Two").expect("named stable-key row"),
        ],
        telorgon::VirtualListTotal::Known(2),
        telorgon::VirtualListPolicy::new(44.0, 88.0, 4).expect("valid bounded policy"),
    )
    .expect("named virtual list with matching known total");
    let _plan: telorgon::VirtualListPlan<&str> =
        list.plan(telorgon::VirtualListViewport::new(0.0, 44.0).expect("valid explicit viewport"));
}

fn compile_listbox_path() {
    let selection = telorgon::SelectionModel::new(
        telorgon::SelectionMode::Single,
        telorgon::SelectionFollowsFocus::Disabled,
        ["one", "two"],
        ["one"],
        Some("one"),
    )
    .expect("valid controlled selection owner");
    let mut listbox: telorgon::ListBox<&str> = telorgon::ListBox::new(
        "Options",
        [
            telorgon::ListBoxOption::new("one", "One").expect("named option"),
            telorgon::ListBoxOption::new("two", "Two").expect("named option"),
        ],
        selection,
    )
    .expect("matching stable option and selection keys");
    let _transition: telorgon::ListBoxTransition<&str> = listbox
        .navigate(
            telorgon::CompositeNavigationCommand::End,
            telorgon::WritingDirection::LeftToRight,
        )
        .expect("neutral composite navigation");
}

fn compile_table_path() {
    let table: telorgon::Table<&str, &str> = telorgon::Table::new(
        "Services",
        [
            telorgon::TableColumn::new("name", "Name").expect("named column header"),
            telorgon::TableColumn::new("status", "Status").expect("named column header"),
        ],
        [telorgon::TableRow::new(
            "api",
            "API",
            [
                telorgon::TableCell::new("name", "Gateway"),
                telorgon::TableCell::new("status", "Ready"),
            ],
        )
        .expect("named row header and rectangular cells")],
    )
    .expect("stable unique rectangular table descriptors");
    let _cell: Option<&telorgon::TableCell<&str>> = table.cell(&"api", &"status");
}

fn compile_data_grid_path() {
    let table: telorgon::Table<&str, &str> = telorgon::Table::new(
        "Services",
        [telorgon::TableColumn::new("name", "Name").expect("named column header")],
        [
            telorgon::TableRow::new("api", "API", [telorgon::TableCell::new("name", "Gateway")])
                .expect("rectangular row"),
        ],
    )
    .expect("validated table descriptors");
    let cells = telorgon::DataGrid::cells(&table);
    let selection = telorgon::SelectionModel::new(
        telorgon::SelectionMode::Single,
        telorgon::SelectionFollowsFocus::Disabled,
        cells,
        [telorgon::DataGridCell::new("api", "name")],
        None,
    )
    .expect("controlled cell selection");
    let mut grid: telorgon::DataGrid<&str, &str> =
        telorgon::DataGrid::new(table, selection).expect("matching grid cells");
    let _navigation: telorgon::DataGridNavigation<&str, &str> = grid
        .navigate(
            telorgon::CompositeNavigationCommand::Home,
            telorgon::WritingDirection::LeftToRight,
        )
        .expect("two-dimensional neutral navigation");
}

fn compile_tree_path() {
    let hierarchy = telorgon::TreeHierarchy::new(
        [
            telorgon::TreeItem::new("root", "Root", None).expect("named tree root"),
            telorgon::TreeItem::new("child", "Child", Some("root")).expect("named tree child"),
        ],
        [],
    )
    .expect("canonical preorder hierarchy");
    let selection = telorgon::SelectionModel::new(
        telorgon::SelectionMode::Single,
        telorgon::SelectionFollowsFocus::Enabled,
        ["root", "child"],
        ["root"],
        None,
    )
    .expect("controlled row selection");
    let mut tree: telorgon::TreeView<&str> =
        telorgon::TreeView::new("Files", hierarchy, selection).expect("valid tree view");
    let _navigation: telorgon::TreeViewNavigation<&str> = tree
        .navigate(
            telorgon::CompositeNavigationCommand::Right,
            telorgon::WritingDirection::LeftToRight,
        )
        .expect("hierarchical navigation");
}

fn compile_tree_grid_path() {
    let hierarchy = telorgon::TreeHierarchy::new(
        [
            telorgon::TreeItem::new("root", "Root", None).expect("named tree root"),
            telorgon::TreeItem::new("child", "Child", Some("root")).expect("named tree child"),
        ],
        ["root"],
    )
    .expect("expanded hierarchy");
    let table = telorgon::Table::new(
        "Files",
        [telorgon::TableColumn::new("name", "Name").expect("named column")],
        [
            telorgon::TableRow::new("root", "Root", [telorgon::TableCell::new("name", "Root")])
                .expect("root row"),
            telorgon::TableRow::new(
                "child",
                "Child",
                [telorgon::TableCell::new("name", "Child")],
            )
            .expect("child row"),
        ],
    )
    .expect("rectangular tree-grid table");
    let cells = telorgon::DataGrid::cells(&table);
    let selection = telorgon::SelectionModel::new(
        telorgon::SelectionMode::Single,
        telorgon::SelectionFollowsFocus::Enabled,
        cells,
        [telorgon::DataGridCell::new("root", "name")],
        None,
    )
    .expect("controlled tree-grid cell selection");
    let grid = telorgon::DataGrid::new(table, selection).expect("matching grid cells");
    let mut tree_grid: telorgon::TreeGrid<&str, &str> =
        telorgon::TreeGrid::new(hierarchy, grid, "name").expect("valid tree grid");
    let _navigation: telorgon::TreeGridNavigation<&str, &str> = tree_grid
        .navigate(
            telorgon::CompositeNavigationCommand::Right,
            telorgon::WritingDirection::LeftToRight,
        )
        .expect("tree-grid navigation");
}

fn compile_form_field_path() {
    let metadata = FieldMetadata::new("email", "Email")
        .expect("field has a visible accessible label")
        .help("Used for account recovery")
        .expect("field has visible help text")
        .required(true);
    let validation = FieldValidation::new(
        "email",
        ValidationResult::pending("Checking address").expect("pending state has visible text"),
    );
    let _semantics = metadata
        .decorate_semantics(
            SemanticNode::new(SemanticRole::TextInput),
            telorgon::StringId(1),
            &validation,
            FieldSemanticSupport::new(
                Some(telorgon::NodeId::new(2, 1)),
                Some(telorgon::NodeId::new(3, 1)),
            ),
        )
        .expect("stable field validation association");
}

fn compile_form_path() {
    let mut form = Form::new(
        [
            FieldMetadata::new("name", "Name").expect("named field"),
            FieldMetadata::new("email", "Email").expect("named field"),
        ],
        [
            FieldValidation::new("name", ValidationResult::Valid),
            FieldValidation::new(
                "email",
                ValidationResult::invalid("Enter an email").expect("visible invalid message"),
            ),
        ],
    )
    .expect("unique fields and exact validation snapshot");
    let _submission: FormSubmission<&str> = form.submit();
}

fn compile_validation_summary_path() {
    let form = Form::new(
        [FieldMetadata::new("email", "Email").expect("named field")],
        [FieldValidation::new(
            "email",
            ValidationResult::invalid("Enter an email").expect("visible invalid message"),
        )],
    )
    .expect("exact controlled form snapshot");
    let summary = ValidationSummary::new("Review fields", &form).expect("named summary");
    let _entry_count = summary.entries().len();
}

fn compile_range_slider_path() {
    let behavior: telorgon::application_components::range::RangeSliderBehavior<f64> =
        telorgon::application_components::range::RangeSliderBehavior::new(
            telorgon::RangeModel::new(0.0, 10.0, 1.0, 5.0).expect("valid range model"),
            telorgon::application_components::range::RangeSliderCrossingPolicy::Clamp,
            telorgon::SliderOrientation::Horizontal,
            telorgon::WritingDirection::LeftToRight,
            false,
            true,
        )
        .expect("valid focused range-slider behavior");
    let _proposal: telorgon::application_components::range::RangeSliderProposal<f64> = behavior
        .request(
            telorgon::application_components::range::RangeSliderValue::new(2.0, 8.0),
            telorgon::application_components::range::RangeSliderThumb::Lower,
            telorgon::SliderCommand::Increment,
            telorgon::ChangeSource::Keyboard,
        )
        .expect("valid controlled range")
        .expect("lower thumb can increment");
}

fn compile_split_view_path() {
    use telorgon::application_components::scroll::{
        SplitViewBehavior, SplitViewCollapsePolicy, SplitViewCommand, SplitViewConstraints,
        SplitViewOrientation, SplitViewProposal, SplitViewValue,
    };

    let behavior = SplitViewBehavior::new(
        SplitViewConstraints::new(800.0, 200.0, 240.0, 20.0, 100.0)
            .expect("valid split constraints"),
        SplitViewCollapsePolicy::Secondary,
        SplitViewOrientation::Horizontal,
        true,
    )
    .expect("valid focused split-view behavior");
    let _proposal: SplitViewProposal = behavior
        .request(
            SplitViewValue::expanded(360.0),
            SplitViewCommand::Increment,
            telorgon::ChangeSource::Accessibility,
        )
        .expect("valid controlled split")
        .expect("divider can increment");
}

fn compile_scroll_controller_path() {
    use telorgon::application_components::scroll::{
        ScrollController, ScrollControllerCommand, ScrollControllerOutcome, ScrollInputSource,
    };

    let mut controller = ScrollController::new(
        telorgon::SizeF {
            width: 100.0,
            height: 100.0,
        },
        telorgon::SizeF {
            width: 100.0,
            height: 500.0,
        },
    )
    .expect("valid application scroll extents");
    let _outcome: ScrollControllerOutcome = controller
        .route(ScrollControllerCommand::ScrollBy {
            delta: telorgon::PointF { x: 0.0, y: 40.0 },
            source: ScrollInputSource::Keyboard,
        })
        .expect("valid application scroll command");
}

fn compile_scroll_view_path() {
    use telorgon::application_components::scroll::{
        ScrollController, ScrollView, ScrollViewAxis, ScrollViewBehavior, ScrollViewCommand,
    };

    let controller = ScrollController::new(
        telorgon::SizeF {
            width: 100.0,
            height: 100.0,
        },
        telorgon::SizeF {
            width: 100.0,
            height: 500.0,
        },
    )
    .expect("valid application scroll extents");
    let behavior = ScrollViewBehavior::from_controller(&controller, ScrollViewAxis::Vertical, true);
    let _command: telorgon::ScrollControllerCommand = behavior
        .request(ScrollViewCommand::Forward)
        .expect("the controller snapshot can scroll forward");
    let _view: telorgon::ScrollView =
        ScrollView::new("Compile viewport", &controller).expect("named scroll view");
}

fn compile_scrollbar_path() {
    use telorgon::application_components::scroll::{
        ScrollBar, ScrollBarBehavior, ScrollBarCommand, ScrollBarTrackGeometry, ScrollController,
        ScrollViewAxis,
    };

    let controller = ScrollController::new(
        telorgon::SizeF {
            width: 100.0,
            height: 100.0,
        },
        telorgon::SizeF {
            width: 100.0,
            height: 500.0,
        },
    )
    .expect("valid application scroll extents");
    let behavior =
        ScrollBarBehavior::from_controller(&controller, ScrollViewAxis::Vertical, 20.0, true)
            .expect("valid focused scrollbar behavior");
    let _command: telorgon::ScrollControllerCommand = behavior
        .request(
            ScrollBarCommand::PageForward,
            telorgon::ScrollInputSource::Keyboard,
        )
        .expect("valid scrollbar request")
        .expect("the scrollbar snapshot can move forward");
    let track =
        ScrollBarTrackGeometry::new(0.0, 160.0, 24.0).expect("valid scrollbar track geometry");
    let _thumb: telorgon::ScrollBarThumbGeometry = track.project(behavior.model());
    let _scrollbar: telorgon::ScrollBar =
        ScrollBar::new("Compile scrollbar", &controller).expect("named scrollbar");
}

fn compile_separator_path() {
    use telorgon::application_components::structure::{
        Separator, SeparatorGeometry, SeparatorOrientation, SeparatorSemanticPolicy,
    };

    let geometry = SeparatorGeometry::new(120.0, 1.0).expect("valid separator geometry");
    let separator = Separator::named(
        "Compile separation",
        SeparatorOrientation::Horizontal,
        geometry,
    )
    .expect("named separator");
    let _policy: telorgon::SeparatorSemanticPolicy = separator.semantic_policy();
    assert_eq!(separator.semantic_policy(), SeparatorSemanticPolicy::Named);
}

fn compile_image_view_path() {
    use telorgon::application_components::content::{
        ImageView, ImageViewContent, ImageViewSemanticPolicy,
    };

    let content = ImageViewContent::new(telorgon::ImageId(73), 12);
    let image = ImageView::described(content, "Compile image content").expect("described image");
    let _image: telorgon::ImageView = image.clone();
    let _policy: telorgon::ImageViewSemanticPolicy = image.semantic_policy();
    assert_eq!(image.semantic_policy(), ImageViewSemanticPolicy::Described);
    assert_eq!(image.content(), content);
}

fn compile_label_path() {
    use telorgon::application_components::text::{Label, LabelContent, LabelStyle, LabelTextStyle};

    let content = LabelContent::new("Compile visible label", 74).expect("valid label content");
    let label = Label::from_content(content).style(LabelStyle {
        text: LabelTextStyle::default(),
        ..LabelStyle::default()
    });
    let _label: telorgon::Label = label.clone();
    let _content: &telorgon::LabelContent = label.content();
    assert_eq!(label.content().revision(), 74);
}

fn compile_selectable_text_path() {
    use telorgon::application_components::text::{
        LabelContent, SelectableText, SelectableTextBehavior,
    };

    fn component(selection: telorgon::Read<telorgon::TextSelection>) -> SelectableText {
        SelectableText::new(
            LabelContent::new("Compile selectable text", 75).expect("valid visible text"),
            selection,
        )
        .expect("bounded selectable text")
    }

    let behavior = SelectableTextBehavior::new(
        LabelContent::new("Compile behavior", 75).expect("valid visible text"),
    )
    .expect("bounded selectable text");
    let _behavior: telorgon::SelectableTextBehavior = behavior;
    let _component =
        component as fn(telorgon::Read<telorgon::TextSelection>) -> telorgon::SelectableText;
}

fn compile_menu_button_path() {
    use telorgon::application_components::command::{
        MenuButton, MenuButtonOpenRequest, MenuOpeningFocus,
    };

    let button = MenuButton::new(
        "Compile menu button",
        [telorgon::CompositeItem {
            key: 76_u8,
            enabled: true,
        }],
    )
    .expect("valid menu button")
    .opening_focus(MenuOpeningFocus::None);
    let request: telorgon::MenuButtonOpenRequest<u8> = button.open_request(
        telorgon::NodeId::new(76, 1),
        telorgon::ChangeSource::Programmatic,
    );
    let _request: &MenuButtonOpenRequest<u8> = &request;
    assert_eq!(request.source(), telorgon::ChangeSource::Programmatic);
}

fn compile_application_structure_primitive_paths() {
    use telorgon::application_primitives::prelude::{
        ApplicationRegion, ApplicationRoot, ApplicationUiExt,
    };

    fn mount<Action: 'static>(
        ui: &mut telorgon::Ui<'_, '_, Action>,
        host: telorgon::NodeId,
    ) -> telorgon::runtime::RuntimeResult<()> {
        let root = ui.mount_application_root(
            host,
            &ApplicationRoot::new("Compile application root").expect("named root"),
        )?;
        let region = ui.mount_application_region(
            root.content_node(),
            &ApplicationRegion::content("Compile content").expect("named region"),
        )?;
        let _root: telorgon::ApplicationRootRef = root;
        let _region: telorgon::ApplicationRegionRef = region;
        Ok(())
    }

    let _mount = mount::<()>;
}

fn compile_application_viewport_primitive_paths() {
    use telorgon::application_primitives::prelude::{
        ApplicationUiExt, HudCoordinateSpace, HudHitTestPolicy, HudLayer, HudSemanticPolicy,
        ViewportOverlay, ViewportOverlayPlacement, WorldAnchor, WorldAnchorProjection,
        WorldAnchorVisibility,
    };

    fn mount<Action: 'static>(
        ui: &mut telorgon::Ui<'_, '_, Action>,
        host: telorgon::NodeId,
    ) -> telorgon::runtime::RuntimeResult<()> {
        let hud = HudLayer::new(
            HudCoordinateSpace::HostLogical,
            HudHitTestPolicy::PassThrough,
            HudSemanticPolicy::IncludeContent,
        )
        .expect("valid HUD policy");
        let hud = ui.mount_hud_layer(host, &hud)?;
        let overlay = ViewportOverlay::new(
            ViewportOverlayPlacement::new(
                telorgon::RectF {
                    width: 800.0,
                    height: 600.0,
                    ..telorgon::RectF::ZERO
                },
                telorgon::PointF { x: 0.5, y: 0.5 },
                telorgon::PointF::default(),
            )
            .expect("valid viewport placement"),
        );
        let overlay = ui.mount_viewport_overlay(hud.node(), &overlay)?;
        let anchor = WorldAnchor::new(
            WorldAnchorProjection::new(
                telorgon::Transform2D::default(),
                WorldAnchorVisibility::Visible,
                0.0,
            )
            .expect("valid host projection"),
        );
        let anchor = ui.mount_world_anchor(overlay.content_node(), &anchor)?;
        let _hud: telorgon::HudLayerRef = hud;
        let _overlay: telorgon::ViewportOverlayRef = overlay;
        let _anchor: telorgon::WorldAnchorRef = anchor;
        Ok(())
    }

    let _mount = mount::<()>;
}

fn compile_application_host_content_primitive_paths() {
    use telorgon::application_primitives::prelude::{
        ApplicationPrimitiveDiagnosticCollector, ApplicationUiExt, RenderTargetToken,
        RenderTargetView, RenderTargetViewContent, VideoColorMetadata, VideoFit, VideoProtection,
        VideoSurface, VideoSurfaceContent, VideoSurfaceToken,
    };

    fn mount<Action: 'static>(
        ui: &mut telorgon::Ui<'_, '_, Action>,
        host: telorgon::NodeId,
    ) -> telorgon::runtime::RuntimeResult<()> {
        let target = RenderTargetView::decorative(
            RenderTargetViewContent::new(RenderTargetToken::new(83).unwrap(), 1)
                .expect("valid host target content"),
        );
        let target = ui.mount_render_target_view(host, &target)?;
        let video = VideoSurface::decorative(
            VideoSurfaceContent::new(
                VideoSurfaceToken::new(84).unwrap(),
                1,
                telorgon::SizeI {
                    width: 1920,
                    height: 1080,
                },
                VideoColorMetadata::default(),
                VideoProtection::Unprotected,
            )
            .expect("valid host video content"),
            VideoFit::Contain,
        );
        let video = ui.mount_video_surface(host, &video)?;
        let _target: telorgon::RenderTargetViewRef = target;
        let _video: telorgon::VideoSurfaceRef = video;
        Ok(())
    }

    let diagnostics = ApplicationPrimitiveDiagnosticCollector::default().diagnostics();
    let _diagnostics: telorgon::ApplicationPrimitiveDiagnostics = diagnostics;
    let _mount = mount::<()>;
}

fn compile_shell_model_paths() {
    use telorgon::shell::{
        ApplicationAction, ApplicationActionId, ApplicationActionKind, ApplicationEntry,
        ApplicationId, ApplicationLabel, ApplicationRevision, ApplicationStates, ExternalContentId,
        OutputColorCapabilities, OutputGeometry, OutputId, OutputRevision, OutputSnapshot,
        OutputTransform, ShellCapabilities, ShellCapabilityGrant, ShellGrantToken, ShellLayerKind,
        SurfaceAlphaMode, SurfaceBufferTransform, SurfaceCapabilities, SurfaceColorDescription,
        SurfaceContent, SurfaceContentRevision, SurfaceDamage, SurfaceGeometry, SurfaceId,
        SurfaceProtection, SurfaceRegions, SurfaceRevision, SurfaceSampling, SurfaceStates,
        WorkspaceId, WorkspaceName, WorkspaceRevision, WorkspaceSnapshot, WorkspaceSurface,
    };

    let output_id = OutputId::from_raw(86).expect("nonzero shell output identity");
    let surface_id = SurfaceId::from_raw(1).expect("nonzero shell surface identity");
    let workspace_id = WorkspaceId::from_raw(1).expect("nonzero shell workspace identity");
    let application_id = ApplicationId::from_raw(1).expect("nonzero shell application identity");
    let grant = ShellCapabilityGrant::from_host(
        ShellGrantToken::from_raw(87).expect("nonzero host grant identity"),
        output_id,
        ShellCapabilities::WORKSPACE_LAYER | ShellCapabilities::ACTIVATE_SURFACE,
    );
    let _workspace_layer = grant
        .authorize_layer(ShellLayerKind::Workspace)
        .expect("workspace layer is explicitly granted");
    let geometry = OutputGeometry::new(
        RectF {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 720.0,
        },
        RectF {
            x: 0.0,
            y: 24.0,
            width: 1280.0,
            height: 696.0,
        },
        telorgon::SizeI {
            width: 2560,
            height: 1440,
        },
        2.0,
        OutputTransform::Normal,
        telorgon::EdgeInsets::ZERO,
        OutputColorCapabilities::SRGB,
    )
    .expect("valid shell output geometry");
    let _output_snapshot = OutputSnapshot::new(
        output_id,
        OutputRevision::from_raw(88).expect("nonzero output revision"),
        geometry,
    );
    let surface_geometry = SurfaceGeometry::new(
        RectF {
            x: 20.0,
            y: 40.0,
            width: 640.0,
            height: 480.0,
        },
        telorgon::SizeI {
            width: 1280,
            height: 960,
        },
        2.0,
        SurfaceBufferTransform::Normal,
        1.0,
    )
    .expect("valid shell surface geometry");
    let surface_content = SurfaceContent::new(
        ExternalContentId::from_raw(1).expect("nonzero logical external content"),
        SurfaceContentRevision::from_raw(1).expect("nonzero surface content revision"),
        None,
        SurfaceColorDescription::default(),
        SurfaceAlphaMode::Premultiplied,
        SurfaceSampling::Linear,
        SurfaceProtection::Unprotected,
    );
    let _surface_snapshot = telorgon::shell::ClientSurfaceSnapshot::new(
        surface_id,
        SurfaceRevision::from_raw(89).expect("nonzero surface revision"),
        None,
        0,
        Some(application_id),
        None,
        surface_geometry,
        SurfaceRegions::default(),
        SurfaceDamage::default(),
        surface_content,
        SurfaceCapabilities::ACTIVATE,
        SurfaceStates::ACTIVE,
    )
    .expect("valid shell surface snapshot");
    let workspace_surface =
        WorkspaceSurface::new(surface_id, output_id, surface_geometry.logical_bounds())
            .expect("valid workspace surface placement");
    let _workspace_snapshot = WorkspaceSnapshot::new(
        workspace_id,
        WorkspaceRevision::from_raw(90).expect("nonzero workspace revision"),
        0,
        WorkspaceName::new("Compile workspace").expect("valid workspace name"),
        true,
        vec![workspace_surface],
    )
    .expect("valid workspace snapshot");
    let action_id = ApplicationActionId::from_raw(1).expect("nonzero application action");
    let launch = ApplicationAction::new(
        action_id,
        ApplicationActionKind::Launch,
        ApplicationLabel::new("Launch").expect("valid action label"),
        true,
    );
    let _application_entry = ApplicationEntry::new(
        application_id,
        ApplicationRevision::from_raw(91).expect("nonzero application revision"),
        ApplicationLabel::new("Compile application").expect("valid application label"),
        None,
        None,
        ApplicationStates::PINNED,
        Some(action_id),
        vec![launch],
    )
    .expect("valid application entry");
}

fn compile_shell_system_model_paths() {
    use telorgon::shell::{
        AccessibilityAttachmentId, AccessibilityAttachmentRevision, AccessibilityNamespaceId,
        ImportedAccessibilityAttachment, ImportedAccessibilityFocus, ImportedAccessibilityPrivacy,
        ImportedSemanticNodeId, ImportedSemanticTransform, NotificationAction,
        NotificationActionId, NotificationActionKind, NotificationId, NotificationLifecycle,
        NotificationPriority, NotificationPrivacy, NotificationRevision, NotificationSnapshot,
        NotificationText, StatusAction, StatusActionId, StatusActionKind, StatusEntry,
        StatusEntryId, StatusEntryKind, StatusPrivacy, StatusSeverity, StatusText,
        SystemStatusRevision, SystemStatusSnapshot,
    };

    let notification_action = NotificationAction::new(
        NotificationActionId::from_raw(1).expect("nonzero notification action"),
        NotificationActionKind::Open,
        NotificationText::new("Open").expect("valid notification action label"),
        true,
    );
    let _notification = NotificationSnapshot::new(
        NotificationId::from_raw(1).expect("nonzero notification identity"),
        NotificationRevision::from_raw(92).expect("nonzero notification revision"),
        None,
        NotificationText::new("Compile notification").expect("valid notification title"),
        None,
        None,
        NotificationPriority::Normal,
        NotificationPrivacy::Public,
        NotificationLifecycle::default(),
        vec![notification_action],
    )
    .expect("valid notification snapshot");
    let status_action_id = StatusActionId::from_raw(1).expect("nonzero status action");
    let status_action = StatusAction::new(
        status_action_id,
        StatusActionKind::OpenDetails,
        StatusText::new("Open status").expect("valid status action label"),
        true,
    );
    let status_entry = StatusEntry::new(
        StatusEntryId::from_raw(1).expect("nonzero status entry"),
        StatusEntryKind::Connectivity,
        StatusText::new("Network").expect("valid status label"),
        None,
        None,
        StatusSeverity::Normal,
        StatusPrivacy::Public,
        true,
        Some(status_action_id),
        vec![status_action],
    )
    .expect("valid status entry");
    let _status = SystemStatusSnapshot::new(
        SystemStatusRevision::from_raw(93).expect("nonzero system status revision"),
        vec![status_entry],
    )
    .expect("valid system status snapshot");
    let _attachment = ImportedAccessibilityAttachment::new(
        AccessibilityAttachmentId::from_raw(1).expect("nonzero accessibility attachment"),
        AccessibilityAttachmentRevision::from_raw(94)
            .expect("nonzero accessibility attachment revision"),
        telorgon::shell::SurfaceId::from_raw(1).expect("nonzero attached surface"),
        AccessibilityNamespaceId::from_raw(1).expect("nonzero accessibility namespace"),
        ImportedSemanticNodeId::from_raw(1).expect("nonzero imported semantic root"),
        ImportedSemanticTransform::IDENTITY,
        ImportedAccessibilityFocus::default(),
        ImportedAccessibilityPrivacy::Ordinary,
    );
}

fn compile_shell_request_paths() {
    use telorgon::shell::{
        AcceptedRequestId, ClientInputRequest, ContactId, InputSource, NotificationActionId,
        NotificationId, NotificationRevision, OutputEdge, OutputId, OutputRequest, OutputRevision,
        ReservedAreaExtent, ReservedAreaId, ResizeEdge, SeatId, ShellRequestResult, SurfaceId,
        SurfaceInputContact, SurfaceInputEvent, SurfaceRequest, SystemRequest, WorkspaceId,
        WorkspaceRequest, WorkspaceRevision,
    };

    let surface = SurfaceId::from_raw(1).expect("nonzero requested surface");
    let contact_id = ContactId::from_raw(1).expect("nonzero shell contact");
    let contact = SurfaceInputContact::new(
        SeatId::from_raw(1).expect("nonzero shell seat"),
        contact_id,
        InputSource::Mouse,
    )
    .expect("mouse is a contact source");
    let event = SurfaceInputEvent::moved(contact, telorgon::PointF { x: 4.0, y: 8.0 })
        .expect("finite surface-local input");
    let _client_input = ClientInputRequest::new(surface, event);
    let _surface_request = SurfaceRequest::BeginResize {
        surface,
        edge: ResizeEdge::BottomRight,
        contact: contact_id,
    };
    let _admission = ShellRequestResult::accepted(
        AcceptedRequestId::from_raw(95).expect("nonzero accepted request identity"),
    );
    let workspace = WorkspaceId::from_raw(1).expect("nonzero requested workspace");
    let _workspace_request = WorkspaceRequest::Select {
        workspace,
        revision: WorkspaceRevision::from_raw(98).expect("nonzero workspace revision"),
        source: InputSource::Keyboard,
    };
    let _output_request = OutputRequest::ProposeReservedArea {
        output: OutputId::from_raw(1).expect("nonzero requested output"),
        revision: OutputRevision::from_raw(99).expect("nonzero output revision"),
        reservation: ReservedAreaId::from_raw(1).expect("nonzero reservation identity"),
        edge: OutputEdge::Top,
        extent: ReservedAreaExtent::new(24.0).expect("positive finite reservation"),
    };
    let _system_request = SystemRequest::NotificationAction {
        notification: NotificationId::from_raw(1).expect("nonzero notification identity"),
        revision: NotificationRevision::from_raw(100).expect("nonzero notification revision"),
        action: NotificationActionId::from_raw(1).expect("nonzero notification action"),
        source: InputSource::Accessibility,
    };
}

fn compile_shell_host_paths() {
    use telorgon::shell::{
        ShellDiagnosticCollector, ShellDiagnosticKind, ShellError, ShellErrorKind, ShellHost,
        ShellSnapshot, ShellSnapshotParts, ShellSnapshotRevision, SystemStatusRevision,
        SystemStatusSnapshot,
    };

    fn assert_host<T: ShellHost>() {}

    let snapshot = ShellSnapshot::new(
        ShellSnapshotRevision::from_raw(101).expect("nonzero shell publication revision"),
        ShellSnapshotParts {
            grants: Vec::new(),
            outputs: Vec::new(),
            surfaces: Vec::new(),
            workspaces: Vec::new(),
            applications: Vec::new(),
            notifications: Vec::new(),
            system_status: SystemStatusSnapshot::new(SystemStatusRevision::INITIAL, Vec::new())
                .expect("empty system status is valid"),
            accessibility: Vec::new(),
        },
    )
    .expect("empty shell publication is valid");
    let mut diagnostics = ShellDiagnosticCollector::default();
    diagnostics.record(ShellDiagnosticKind::SnapshotPublished);
    let _diagnostics = diagnostics.diagnostics();
    let _error = ShellError::new(ShellErrorKind::HostUnavailable, "compile fixture");
    let _snapshot = snapshot;
    let _assert_host = assert_host::<CompileOnlyShellHost>;

    struct CompileOnlyShellHost;

    impl ShellHost for CompileOnlyShellHost {
        fn snapshot(&self) -> ShellSnapshot {
            panic!("compile-only host")
        }

        fn request_client_input(
            &mut self,
            _: telorgon::shell::ClientInputRequest,
        ) -> telorgon::shell::ShellRequestResult {
            telorgon::shell::ShellRequestResult::Unsupported
        }

        fn request_surface(
            &mut self,
            _: telorgon::shell::SurfaceRequest,
        ) -> telorgon::shell::ShellRequestResult {
            telorgon::shell::ShellRequestResult::Unsupported
        }

        fn request_workspace(
            &mut self,
            _: telorgon::shell::WorkspaceRequest,
        ) -> telorgon::shell::ShellRequestResult {
            telorgon::shell::ShellRequestResult::Unsupported
        }

        fn request_output(
            &mut self,
            _: telorgon::shell::OutputRequest,
        ) -> telorgon::shell::ShellRequestResult {
            telorgon::shell::ShellRequestResult::Unsupported
        }

        fn request_system(
            &mut self,
            _: telorgon::shell::SystemRequest,
        ) -> telorgon::shell::ShellRequestResult {
            telorgon::shell::ShellRequestResult::Unsupported
        }
    }
}

fn compile_shell_primitive_foundation_paths() {
    use telorgon::shell::{
        OutputColorCapabilities, OutputGeometry, OutputId, OutputRevision, OutputSnapshot,
        OutputTransform, ShellCapabilities, ShellCapabilityGrant, ShellGrantToken, ShellLayerKind,
    };
    use telorgon::shell_primitives::prelude::{OutputView, ShellLayer, ShellLayerOrder, ShellRoot};

    let output = OutputId::from_raw(1).expect("nonzero primitive output");
    let grant = ShellCapabilityGrant::from_host(
        ShellGrantToken::from_raw(1).expect("nonzero primitive grant"),
        output,
        ShellCapabilities::BACKGROUND_LAYER,
    );
    let root = ShellRoot::new("Compile shell", grant).expect("named shell root");
    let snapshot = OutputSnapshot::new(
        output,
        OutputRevision::from_raw(105).expect("nonzero output revision"),
        OutputGeometry::new(
            telorgon::RectF {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            telorgon::RectF {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            telorgon::SizeI {
                width: 100,
                height: 100,
            },
            1.0,
            OutputTransform::Normal,
            telorgon::EdgeInsets::ZERO,
            OutputColorCapabilities::SRGB,
        )
        .expect("valid primitive output geometry"),
    );
    let _view = OutputView::new(snapshot);
    let authority = root
        .authorize_layer(ShellLayerKind::Background)
        .expect("background layer granted");
    let _layer = ShellLayer::new(authority);
    let _order = ShellLayerOrder::new(output);
}

fn compile_shell_surface_primitive_paths() {
    use telorgon::shell::{
        ClientSurfaceSnapshot, OutputEdge, ReservedAreaExtent, ReservedAreaId, SurfaceId,
        SurfaceRevision,
    };
    use telorgon::shell_primitives::prelude::{
        ClientSurface, ClientSurfaceRef, DragRegion, ExclusiveRegion, ExclusiveRegionGeometry,
        OutputEdgeActivation, OutputEdgeKind, OutputEdgeThickness, ReservedArea, ResizeRegion,
        ShellPrimitiveDiagnosticCollector, ShellPrimitiveDiagnosticKind, ShellUiExt,
        SurfaceInputRegion, SurfacePlaceholder, SurfacePlaceholderReason, SurfaceSnapshot,
        SurfaceSnapshotAuthorization, SurfaceSnapshotPolicy, SurfaceSnapshotRevision,
        SurfaceSnapshotToken, SurfaceTree,
    };

    fn assert_shell_ui_ext<Action: 'static, T: ShellUiExt<Action>>() {}

    let _client_surface_constructor: fn(ClientSurfaceSnapshot) -> ClientSurface =
        ClientSurface::new;
    let _surface_tree_constructor: fn(Vec<ClientSurfaceSnapshot>) -> Result<SurfaceTree, _> =
        SurfaceTree::new;
    let surface = SurfaceId::from_raw(107).expect("nonzero primitive surface");
    let surface_revision = SurfaceRevision::from_raw(108).expect("nonzero surface revision");
    let _placeholder = SurfacePlaceholder::new(
        surface,
        surface_revision,
        SurfacePlaceholderReason::Unavailable,
    );
    let _snapshot_constructor: fn(
        ClientSurfaceSnapshot,
        SurfaceSnapshotAuthorization,
    ) -> Result<SurfaceSnapshot, _> = SurfaceSnapshot::new;
    let _snapshot_token =
        SurfaceSnapshotToken::from_raw(109).expect("nonzero snapshot authorization");
    let _snapshot_revision =
        SurfaceSnapshotRevision::from_raw(110).expect("nonzero retained revision");
    let _snapshot_policy = SurfaceSnapshotPolicy::UnprotectedOnly;
    let _reserved_area = ReservedArea::new(
        ReservedAreaId::from_raw(111).expect("nonzero reservation"),
        OutputEdge::Top,
        ReservedAreaExtent::new(24.0).expect("positive reservation extent"),
    );
    let exclusive_geometry = ExclusiveRegionGeometry::new(vec![telorgon::RectF {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 24.0,
    }])
    .expect("valid exclusive region");
    let _exclusive_region = ExclusiveRegion::new(exclusive_geometry);
    let _input_region_constructor: fn(&ClientSurfaceRef) -> SurfaceInputRegion =
        SurfaceInputRegion::from_surface;
    let _drag_region_constructor: fn(
        &ClientSurfaceRef,
        telorgon::shell::SurfaceRegion,
    ) -> Result<DragRegion, _> = DragRegion::new;
    let _resize_region_constructor: fn(
        &ClientSurfaceRef,
        telorgon::shell::ResizeEdge,
        telorgon::shell::SurfaceRegion,
    ) -> Result<ResizeRegion, _> = ResizeRegion::new;
    let _edge_kind = OutputEdgeKind::TopRight;
    let _edge_activation = OutputEdgeActivation::Accessibility;
    let _edge_thickness =
        OutputEdgeThickness::new(8.0).expect("positive finite output edge thickness");
    let mut primitive_diagnostics = ShellPrimitiveDiagnosticCollector::default();
    primitive_diagnostics.record(ShellPrimitiveDiagnosticKind::InvalidSurfaceInputMapping);
    let _primitive_diagnostics = primitive_diagnostics.diagnostics();
    let _assert_ext = assert_shell_ui_ext::<(), telorgon::Ui<'static, 'static, ()>>;
}

#[test]
fn current_public_paths_compile() {
    let _centered_content = telorgon::column()
        .justify_content(telorgon::Alignment::Center)
        .align_items(telorgon::Alignment::Center)
        .center_content();
    let _centered_text =
        telorgon::app::text("Centered").text_align(telorgon::app::Alignment::Center);
    let _centered_text_style = telorgon::TextStyle::new().text_align(telorgon::Alignment::Center);

    let mut activation = ActivationStateMachine::new(true);
    let activation_outcome = activation.handle(ActivationInput::SemanticActivate);
    assert!(matches!(
        activation_outcome.transition,
        ActivationTransition::Activated(_)
    ));
    let focus_scope = FocusScopeId::from_raw(1, 1).unwrap();
    let mut focus = FocusStateMachine::new(
        focus_scope,
        FocusTraversalEdge::Stop,
        FocusIndicatorPolicy::Automatic,
    );
    focus
        .update_candidates(vec![FocusCandidate::new((1_u32, 1_u32), focus_scope)])
        .unwrap();
    let _focus_change = focus.traverse(FocusTraversalDirection::Forward);
    let mut composite = CompositeStateMachine::new(CompositeNavigationPolicy::default());
    composite
        .update_items([CompositeItem {
            key: (1_u32, 1_u32),
            enabled: true,
        }])
        .unwrap();
    composite.enter(None).unwrap();
    let _composite_change = composite
        .navigate(
            CompositeNavigationCommand::Home,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    let environment = EnvironmentState::new(EnvironmentValues::default()).unwrap();
    let _environment_revision = environment.snapshot().revision();
    let _environment_read_factory = assert_environment_read_paths;
    let _platform_view = ViewId::from_raw(1, 1).expect("nonzero platform view identity");
    let _data_offer = DataOfferId::from_raw(1, 1).expect("nonzero platform data-offer identity");
    let _platform_request = RequestId::from_raw(1).expect("nonzero platform request identity");
    let _surface_generation =
        NativeSurfaceGeneration::from_raw(1).expect("nonzero native surface generation");
    let _shared_event_time: telorgon::platform::MonotonicInstant = MonotonicInstant::from_nanos(1);
    let _runtime_event_time: telorgon::runtime::MonotonicInstant = _shared_event_time;
    let mut _event_stamps = EventStampStream::new();
    let _event_stamp: EventStamp = _event_stamps
        .stamp(_runtime_event_time, None)
        .expect("injected monotonic event stamp");
    let mut _platform_clock = MonotonicClockState::new(CompileClock(_runtime_event_time));
    let _observed_clock_time = _platform_clock
        .observe_now()
        .expect("injected compile clock remains monotonic");
    let _clock_error_type: Option<MonotonicClockError> = None;
    let _platform_capability: Support<CapabilityDescriptor<u8, CapabilityLimit<u16>>> =
        Support::Available(CapabilityDescriptor::new(
            1,
            CapabilityLimit::Bounded(32),
            PermissionState::NotRequired,
            ExecutionRequirement::HostEventLoop,
            UserGestureRequirement::NotRequired,
        ));
    let _platform_unavailable: telorgon::platform::Support<u8> =
        Support::Unavailable(UnavailableReason::AdapterNotCompiled);
    let mut _view_lifecycle = ViewLifecycle::new();
    _view_lifecycle
        .observe_lifetime(ViewLifetime::Live)
        .expect("declared view becomes live");
    _view_lifecycle
        .observe_activity(ActivityState::Active)
        .expect("activity axis updates independently");
    _view_lifecycle
        .observe_visibility(VisibilityState::Visible)
        .expect("visibility axis updates independently");
    _view_lifecycle
        .observe_surface_available(_surface_generation)
        .expect("live view accepts native-surface generation");
    let _surface_state: NativeSurfaceState = _view_lifecycle.surface();
    let _lifecycle_error_type: Option<LifecycleError> = None;
    let mut _platform_view_state = ViewState::new(_platform_view);
    let _platform_view_update = _platform_view_state
        .observe_lifetime(ViewLifetime::Live)
        .expect("view state publishes a live snapshot");
    let _platform_view_snapshot: ViewSnapshot = _platform_view_state.snapshot();
    let _platform_view_revision: ViewRevision = _platform_view_snapshot.revision();
    let _close_request =
        CloseRequest::from_snapshot(&_platform_view_snapshot, CloseRequestReason::User);
    let _close_decision = CloseRequestDecision::Defer;
    let _forced_destruction = ForcedDestruction::from_snapshot(
        &_platform_view_snapshot,
        ForcedDestructionPhase::Destroying,
    );
    let _view_state_error_type: Option<ViewStateError> = None;
    let _metric_insets =
        MetricInsets::new(CoordinateSpace::ViewLogical, telorgon::EdgeInsets::all(4.0))
            .expect("finite logical safe insets");
    let _metric_avoid = AvoidRegion::new(
        AvoidRegionKind::Ime,
        CoordinateSpace::ViewLogical,
        RectF {
            x: 0.0,
            y: 200.0,
            width: 320.0,
            height: 40.0,
        },
    )
    .expect("finite positive IME avoid region");
    let _platform_metrics = ViewMetrics::new(
        PhysicalExtent::new(640, 480),
        ScaleFactor::new(2.0).expect("finite positive scale"),
        DisplayProperties::new(
            DisplayTransform::Identity,
            DisplayColorSpace::Srgb,
            HdrState::Unsupported,
        ),
    )
    .expect("physical extent and scale derive finite logical metrics")
    .with_safe_drawing_insets(_metric_insets)
    .expect("safe insets fit the logical extent")
    .with_avoid_regions(vec![_metric_avoid])
    .expect("bounded avoid region list");
    let mut _metrics_state = ViewMetricsState::new(_platform_metrics.clone());
    let _metrics_snapshot: ViewMetricsSnapshot = _metrics_state.snapshot();
    let _metrics_revision: MetricsRevision = _metrics_snapshot.revision();
    let _metrics_error_type: Option<ViewMetricsError> = None;
    let _platform_event = PlatformEvent::from_coalescing(
        _platform_view,
        CoalescingMetadata::coalesced(_event_stamp, CollapsedEventCount::ONE),
        MetricsCitation::converted_using(_metrics_revision),
        InputEvent::mouse_moved(PointF { x: 2.0, y: 3.0 }),
    );
    let _platform_error = PlatformError::with_source(
        PlatformErrorKind::Unavailable,
        "compile platform request",
        PlatformErrorSource::new(
            PlatformErrorKind::TransportFailure,
            "compile platform transport",
        ),
    );
    let _platform_result: PlatformResult<()> = Err(_platform_error);
    let _request_completion: RequestCompletion<u32> = compile_request_admission(_platform_request)
        .expect("compile request is admitted")
        .complete(RequestOutcome::Applied(1));
    let _post_turn_schedule = PostTurnSchedule::new(
        RemainingWork::new(true, false, false, true),
        &[_platform_view],
        Some(_observed_clock_time),
        PendingHostFacts::new(true, false),
    )
    .expect("bounded compile schedule");
    let _schedule_error_type: Option<ScheduleError> = None;
    let _maximum_redraw_views = MAX_REDRAW_VIEWS;
    let mut _services = ServiceRegistry::new();
    let _service_registration: ServiceRegistration<u32> = _services.register::<CompileService>(7);
    let _service_lookup: ServiceLookup<'_, u32> = _services.lookup::<CompileService>();
    let _service_available = _service_lookup.is_available();
    let _service_replacement: ServiceReplacement<u32> = _services.replace::<CompileService>(8);
    let _service_removal: ServiceRemoval<u32> = _services.remove::<CompileService>();
    let _service_unavailable = ServiceUnavailable::NotRegistered;
    _platform_view_state
        .observe_metrics(_platform_metrics)
        .expect("view snapshot publishes metrics atomically");
    let _density = DensityMetrics::baseline(environment.values().density);
    let _button = Button::new("Compile path")
        .expect("button has an accessible name")
        .density(_density);
    let _icon_button = IconButton::new(ImageId(1), "Compile icon action")
        .expect("icon button has an explicit accessible name")
        .density(_density);
    let _link = Link::new(
        "Compile link action",
        LinkDestination::new("compile/destination").expect("valid opaque destination"),
    )
    .expect("link has an accessible name")
    .density(_density);
    let _link_command =
        _link.context_command(LinkCommandKind::CopyDestination, ChangeSource::Programmatic);
    let _controlled_change =
        ValueChange::new(1_u32, ChangePhase::Commit, ChangeSource::Programmatic);
    let _next_check = CheckCyclePolicy::two_state()
        .next(CheckState::Unchecked)
        .expect("binary check state advances");
    let _range = RangeModel::new(0.0_f64, 100.0, 1.0, 10.0)
        .expect("valid finite range")
        .with_format(RangeFormat::new(0).expect("valid precision"))
        .with_marks([RangeMark::new(0.0), RangeMark::new(100.0)])
        .expect("ordered in-range marks");
    let pointer = PointerId::new(1);
    let mut gesture_arena = GestureArena::new();
    gesture_arena.add(pointer, "tap").unwrap();
    let _arena_decisions = gesture_arena.close(pointer).unwrap();
    let mut tap = TapRecognizer::new(8.0, true).unwrap();
    let _tap_transition = tap
        .handle(GestureInput::PointerDown {
            pointer,
            button: PointerButton::PRIMARY,
            position: PointF::default(),
        })
        .unwrap();
    let shortcut_scope = ShortcutScopeId::from_raw(1, 1).unwrap();
    let mut shortcuts = ShortcutMatcher::<u32, u32>::new();
    shortcuts
        .update_bindings([ShortcutBinding::new(
            1,
            2,
            shortcut_scope,
            ShortcutChord::pressed(PhysicalKey::new(42), Modifiers::CONTROL),
        )])
        .unwrap();
    let _shortcut_resolution = shortcuts
        .resolve(
            KeyEvent {
                physical_key: PhysicalKey::new(42),
                state: ButtonState::Pressed,
                repeat: false,
                modifiers: Modifiers::CONTROL,
                ..KeyEvent::new(PhysicalKey::new(42), ButtonState::Pressed)
            },
            [telorgon::ActiveShortcutScope::bubble(shortcut_scope)],
        )
        .unwrap();
    let mut semantics = SemanticNode::new(SemanticRole::Button);
    semantics.name = SemanticName::Contents;
    semantics.actions = SemanticActions::ACTIVATE | SemanticActions::FOCUS;
    assert!(
        semantics
            .effective_actions()
            .contains(SemanticAction::Activate)
    );
    semantics
        .validate(telorgon::NodeId::new(1, 1))
        .expect("valid mounted semantic input");
    let mut overlay_request = OverlayOpenRequest::anchored(telorgon::NodeId::new(1, 1));
    overlay_request.dismissal.outside_press = OutsidePressPolicy::DismissAndConsume;
    assert_eq!(
        overlay_request.dismissal.outside_press,
        OutsidePressPolicy::DismissAndConsume
    );
    let _overlay_host = OverlayHost::default();
    let _application_overlay_host = ApplicationOverlayHost::new();
    let _application_overlay_controller = ApplicationOverlayController::new();
    let mut popup_placement = PopupPlacementRequest::new(
        RectF {
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: 10.0,
        },
        SizeF {
            width: 40.0,
            height: 30.0,
        },
        RectF {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        },
        [PopupPlacementCandidate::below(
            PopupPlacementAlignment::Start,
        )],
    );
    popup_placement.overflow = PopupOverflowPolicy::Shift;
    let _placed_popup = telorgon::place_popup(&popup_placement).expect("popup placement fits");
    let application_environment = EnvironmentValues {
        available_size: SizeF {
            width: 100.0,
            height: 100.0,
        },
        ..EnvironmentValues::default()
    };
    let application_popup = ApplicationPopupPlacementRequest::new(
        RectF {
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: 10.0,
        },
        SizeF {
            width: 40.0,
            height: 30.0,
        },
        &application_environment,
    );
    let _application_placement = telorgon::place_application_popup(&application_popup)
        .expect("application popup placement fits");
    let _popup = Popup::new(
        PopupAnchor::rect(RectF {
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: 10.0,
        }),
        SizeF {
            width: 40.0,
            height: 30.0,
        },
    );
    let _dialog = Dialog::new(
        "Compile dialog",
        telorgon::NodeId::new(1, 1),
        RectF {
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: 10.0,
        },
        SizeF {
            width: 60.0,
            height: 40.0,
        },
        DialogInitialFocus::FirstFocusable,
    )
    .expect("dialog has an accessible name and required initial focus");
    let _sheet = telorgon::Sheet::new(
        "Compile sheet",
        telorgon::NodeId::new(1, 1),
        telorgon::SheetEdge::InlineEnd,
        telorgon::SheetExtent::new(
            SizeF {
                width: 80.0,
                height: 100.0,
            },
            SizeF {
                width: 40.0,
                height: 50.0,
            },
        ),
        telorgon::SheetMode::NonModal,
    )
    .expect("sheet has an accessible name and constrained viewport");
    let tooltip_triggers =
        telorgon::TooltipTriggerPolicy::hover(std::time::Duration::from_millis(500))
            .expect("tooltip hover delay is finite and nonzero");
    let _tooltip = telorgon::Tooltip::new(
        "Compile supplemental description",
        telorgon::TooltipAnchor::new(
            telorgon::NodeId::new(1, 1),
            RectF {
                x: 10.0,
                y: 10.0,
                width: 20.0,
                height: 10.0,
            },
        ),
        telorgon::TooltipExtent::new(
            SizeF {
                width: 80.0,
                height: 40.0,
            },
            SizeF {
                width: 40.0,
                height: 20.0,
            },
        ),
        tooltip_triggers,
    )
    .expect("tooltip has a supplemental description");
    let _toast = telorgon::Toast::new(
        "Compile toast",
        telorgon::ToastCorner::BlockEndInlineEnd,
        telorgon::ToastExtent::new(
            SizeF {
                width: 100.0,
                height: 48.0,
            },
            SizeF {
                width: 60.0,
                height: 32.0,
            },
        ),
        telorgon::ToastAnnouncementPolicy::new(telorgon::ToastAnnouncementPriority::Polite),
        telorgon::ToastLifetime::expiring(std::time::Duration::from_secs(5))
            .expect("toast expiry is finite and nonzero"),
    )
    .expect("toast has a visible message");
    let mut scroll = ScrollState::new(
        SizeF {
            width: 100.0,
            height: 100.0,
        },
        SizeF {
            width: 100.0,
            height: 500.0,
        },
    )
    .expect("valid neutral scroll extents");
    let _scroll_update = scroll
        .scroll_by(PointF { x: 0.0, y: 20.0 }, ScrollInputSource::Keyboard)
        .expect("valid neutral scroll delta");
    #[cfg(any(
        feature = "application-software",
        all(feature = "application-vulkan-windows", target_os = "windows"),
        all(feature = "desktop-wayland-linux", target_os = "linux")
    ))]
    let _window = WindowOptions::default();
    let neutral_input = InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Pressed);
    let _canonical_input: telorgon::input::InputEvent = neutral_input.clone();
    let _app_bridge: telorgon::application_host::InputEvent = neutral_input;
    let _ui_phase: telorgon::ui::EventPhase = telorgon::input::EventPhase::Target;
    let _runtime_scheduler = telorgon::runtime::FrameScheduler::default();
    let _deadline = telorgon::MonotonicInstant::from_nanos(1);
    let _app_deadline_bridge: telorgon::application_host::MonotonicInstant = _deadline;
    let _unsupported_task_host = telorgon::UnsupportedTaskHost;
    #[cfg(any(
        feature = "application-software",
        all(feature = "application-vulkan-windows", target_os = "windows"),
        all(feature = "desktop-wayland-linux", target_os = "linux")
    ))]
    {
        let _managed_executor: Option<telorgon::ManagedTaskExecutor> = None;
        let _managed_runtime: Option<telorgon::ManagedComponentRuntime<ComponentFixture>> = None;
        let _app_scheduler_bridge: telorgon::application_host::FrameScheduler =
            telorgon::FrameScheduler::default();
        let _gui_entry = telorgon::Application::gui("Compile path")
            .renderer(telorgon::Renderer::Auto)
            .window(
                telorgon::Window::new("Compile path")
                    .size(640, 480)
                    .content(ComposedFixture::default()),
            );
        let _desktop_entry = telorgon::Application::desktop_environment("Compile desktop")
            .renderer(telorgon::Renderer::Vulkan)
            .compositor(
                telorgon::Compositor::new()
                    .cursor_theme(telorgon::CursorTheme::new())
                    .background(ComposedFixture::default()),
            )
            .shell_widget(
                telorgon::ShellWidget::new("Panel")
                    .reserve_space(36.0)
                    .content(ComposedFixture::default()),
            );
    }
    let mut text = TextBuffer::from_text("compile path").expect("valid text buffer");
    let snapshot = text.snapshot();
    let selection = TextSelection {
        anchor: TextOffset::ZERO,
        active: snapshot.end(),
        affinity: TextAffinity::Downstream,
    };
    let _validated_selection = snapshot
        .validate_selection(selection)
        .expect("valid selection");
    let _bounded_chunks = snapshot
        .chunks_in(TextRange::new(TextOffset::ZERO, snapshot.end()).expect("ordered range"))
        .expect("valid range");
    let _edit_outcome = text
        .apply_edits(TextEditBatch {
            base_revision: snapshot.revision(),
            edits: vec![TextEdit {
                range: TextRange::collapsed(snapshot.end()),
                replacement: "!".to_string(),
            }],
            selection: TextSelection::collapsed(
                TextOffset(snapshot.end().bytes() + 1),
                TextAffinity::Downstream,
            ),
            composition: None,
        })
        .expect("valid atomic edit");
    let composition_start = text
        .apply_composition(TextCompositionCommand::Start {
            base_revision: text.revision(),
            edits: Vec::new(),
            selection: text.selection(),
            composition: TextRange::collapsed(text.selection().active),
        })
        .expect("valid neutral composition start");
    let _composition_commit = text
        .apply_composition(TextCompositionCommand::Commit {
            base_revision: composition_start.snapshot.revision(),
            edits: Vec::new(),
            selection: composition_start.selection,
        })
        .expect("valid neutral composition commit");
    let _logical_navigation = text
        .snapshot()
        .navigate_selection(
            text.selection(),
            TextNavigationUnit::Grapheme,
            TextNavigationDirection::Backward,
            TextSelectionAdjustment::Move,
            TextAffinity::Upstream,
        )
        .expect("valid neutral text navigation");
    let session_id = TextSessionId::from_raw(1, 1).expect("nonzero text session identity");
    let mut text_session = TextInputSession::new(session_id, TextInputConfiguration::default(), 64);
    let _open_text_session = text_session
        .open(&text)
        .expect("valid neutral text session open");
    let _close_text_session = text_session
        .close()
        .expect("valid neutral text session close");
    #[cfg(feature = "application-software")]
    let _headless_entry = compile_headless_software_path as fn();
    let _scene_entry = compile_scene_path;
    #[cfg(any(
        feature = "application-software",
        all(feature = "application-vulkan-windows", target_os = "windows"),
        all(feature = "desktop-wayland-linux", target_os = "linux")
    ))]
    let _runtime_entry = compile_renderer_free_runtime_path;
    let _component_entry = compile_component_runtime_path;
    let _command_model_entry = compile_command_model_path;
    let _navigation_controller_entry = compile_navigation_controller_path;
    let _selection_model_entry = compile_selection_model_path;
    let _list_view_entry = compile_list_view_path;
    let _virtual_list_entry = compile_virtual_list_path;
    let _listbox_entry = compile_listbox_path;
    let _table_entry = compile_table_path;
    let _data_grid_entry = compile_data_grid_path;
    let _tree_entry = compile_tree_path;
    let _tree_grid_entry = compile_tree_grid_path;
    let _form_field_entry = compile_form_field_path;
    let _form_entry = compile_form_path;
    let _validation_summary_entry = compile_validation_summary_path;
    let _range_slider_entry = compile_range_slider_path;
    let _split_view_entry = compile_split_view_path;
    let _scroll_controller_entry = compile_scroll_controller_path;
    let _scroll_view_entry = compile_scroll_view_path;
    let _scrollbar_entry = compile_scrollbar_path;
    let _separator_entry = compile_separator_path;
    let _image_view_entry = compile_image_view_path;
    let _label_entry = compile_label_path;
    let _selectable_text_entry = compile_selectable_text_path;
    let _menu_button_entry = compile_menu_button_path;
    let _application_structure_primitive_entry = compile_application_structure_primitive_paths;
    let _application_viewport_primitive_entry = compile_application_viewport_primitive_paths;
    let _application_host_content_primitive_entry =
        compile_application_host_content_primitive_paths;
    let _shell_model_entry = compile_shell_model_paths;
    let _shell_system_model_entry = compile_shell_system_model_paths;
    let _shell_request_entry = compile_shell_request_paths;
    let _shell_host_entry = compile_shell_host_paths;
    let _shell_primitive_foundation_entry = compile_shell_primitive_foundation_paths;
    let _shell_surface_primitive_entry = compile_shell_surface_primitive_paths;
    #[cfg(feature = "application-software")]
    let _backend_entry = compile_software_backend_path;
    #[cfg(feature = "application-vulkan-windows")]
    let _vulkan_policy = telorgon::renderer_vulkan::VulkanConfig::default();

    assert_composed_component::<ComposedFixture>();
}
