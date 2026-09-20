use std::cell::{Cell, RefCell};

use crate::runtime::{
    Component, ComponentRuntimeDriver, CreateContext, NoAction, Read, State, UpdateContext,
    ViewRuntime,
};
use crate::ui::{SemanticAction, UiRoot};

use crate::components::application::ActionFactory;

use super::*;

struct ReadCapture {
    captured: Rc<Cell<Option<Read<bool>>>>,
}

impl Component for ReadCapture {
    type State = State<bool>;
    type Action = NoAction;

    fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
        let state = context.state(true);
        self.captured.set(Some(state.read()));
        state
    }

    fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        ui.foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
    }

    fn action(
        &self,
        _state: &mut Self::State,
        action: Self::Action,
        _context: &mut UpdateContext<'_, Self>,
    ) {
        match action {}
    }
}

fn capture_read() -> (Read<bool>, ViewRuntime<ComponentRuntimeDriver<ReadCapture>>) {
    let captured = Rc::new(Cell::new(None));
    let runtime = ViewRuntime::from_component(ReadCapture {
        captured: captured.clone(),
    })
    .unwrap();
    (captured.get().unwrap(), runtime)
}

fn command(id: u32, enabled: Read<bool>) -> CommandSpec<u32, ()> {
    CommandSpec::new(
        id,
        format!("Command {id}"),
        enabled,
        ActionFactory::new(enabled.owner(), |_| ()),
    )
    .unwrap()
}

#[test]
fn construction_requires_name_commands_unique_ids_and_one_owner() {
    let (first, _first_runtime) = capture_read();
    let (foreign, _foreign_runtime) = capture_read();
    assert!(matches!(
        Toolbar::<u32, ()>::new(" ", [command(1, first)]),
        Err(ToolbarError::MissingAccessibleName)
    ));
    assert!(matches!(
        Toolbar::<u32, ()>::new("Edit", []),
        Err(ToolbarError::Empty)
    ));
    assert!(matches!(
        Toolbar::new("Edit", [command(1, first), command(1, first)]),
        Err(ToolbarError::DuplicateCommand(1))
    ));
    assert!(matches!(
        Toolbar::new("Edit", [command(1, first), command(2, foreign)]),
        Err(ToolbarError::OwnerMismatch { .. })
    ));
}

#[test]
fn neutral_behavior_handles_orientation_rtl_home_end_and_disabled_discovery() {
    let items = [
        CompositeItem {
            key: 1_u32,
            enabled: true,
        },
        CompositeItem {
            key: 2,
            enabled: false,
        },
        CompositeItem {
            key: 3,
            enabled: true,
        },
    ];
    let mut horizontal =
        ToolbarBehavior::new(items, ToolbarNavigationPolicy::default()).unwrap();
    assert_eq!(horizontal.active_command(), Some(1));
    horizontal
        .navigate(
            CompositeNavigationCommand::Right,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    assert_eq!(horizontal.active_command(), Some(2));
    assert_eq!(
        horizontal.request_active_command(ChangeSource::Keyboard),
        Err(CompositeError::ActiveDescendantDisabled(2))
    );
    horizontal
        .navigate(
            CompositeNavigationCommand::Right,
            WritingDirection::RightToLeft,
        )
        .unwrap();
    assert_eq!(horizontal.active_command(), Some(1));
    horizontal
        .navigate(
            CompositeNavigationCommand::End,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    assert_eq!(horizontal.active_command(), Some(3));
    horizontal
        .navigate(
            CompositeNavigationCommand::Up,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    assert_eq!(horizontal.active_command(), Some(3));

    let mut vertical = ToolbarBehavior::new(
        items,
        ToolbarNavigationPolicy {
            orientation: ToolbarOrientation::Vertical,
            ..ToolbarNavigationPolicy::default()
        },
    )
    .unwrap();
    vertical
        .navigate(
            CompositeNavigationCommand::Right,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    assert_eq!(vertical.active_command(), Some(1));
    vertical
        .navigate(
            CompositeNavigationCommand::Down,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    assert_eq!(vertical.active_command(), Some(2));
    vertical
        .navigate(
            CompositeNavigationCommand::Home,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    assert_eq!(vertical.active_command(), Some(1));
}

#[derive(Debug, PartialEq, Eq)]
struct NonCloneAction {
    command: u32,
    source: ChangeSource,
}

#[derive(Debug)]
enum MountedAction {
    Invoked(ToolbarInvocation<u32, NonCloneAction>),
}

struct MountedToolbar {
    toolbar: Rc<RefCell<Option<ToolbarRef<u32, NonCloneAction>>>>,
    actions: Rc<RefCell<Vec<NonCloneAction>>>,
}

struct MountedToolbarState {
    toolbar: Toolbar<u32, NonCloneAction>,
    _first_enabled: State<bool>,
    _second_enabled: State<bool>,
    _third_enabled: State<bool>,
    _third_checked: State<CheckState>,
}

impl Component for MountedToolbar {
    type State = MountedToolbarState;
    type Action = MountedAction;

    fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
        let first_enabled = context.state(true);
        let second_enabled = context.state(false);
        let third_enabled = context.state(true);
        let third_checked = context.state(CheckState::Mixed);
        let owner = context.component();
        let first = CommandSpec::new(
            1,
            "Cut",
            first_enabled.read(),
            ActionFactory::new(owner, |source| NonCloneAction { command: 1, source }),
        )
        .unwrap();
        let second = CommandSpec::new(
            2,
            "Copy",
            second_enabled.read(),
            ActionFactory::new(owner, |source| NonCloneAction { command: 2, source }),
        )
        .unwrap();
        let third = CommandSpec::new(
            3,
            "Bold",
            third_enabled.read(),
            ActionFactory::new(owner, |source| NonCloneAction { command: 3, source }),
        )
        .unwrap()
        .checked(third_checked.read())
        .unwrap();
        MountedToolbarState {
            toolbar: Toolbar::new("Editing", [first, second, third])
                .unwrap()
                .density(DensityMetrics::baseline(DensityClass::Touch)),
            _first_enabled: first_enabled,
            _second_enabled: second_enabled,
            _third_enabled: third_enabled,
            _third_checked: third_checked,
        }
    }

    fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let root = ui
            .foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
        let toolbar = state
            .toolbar
            .mount(ui, root.0, MountedAction::Invoked)
            .unwrap();
        self.toolbar.replace(Some(toolbar));
        root
    }

    fn action(
        &self,
        _state: &mut Self::State,
        action: Self::Action,
        _context: &mut UpdateContext<'_, Self>,
    ) {
        match action {
            MountedAction::Invoked(invocation) => {
                self.actions.borrow_mut().push(invocation.into_action())
            }
        }
    }
}

struct Harness {
    runtime: ViewRuntime<ComponentRuntimeDriver<MountedToolbar>>,
    toolbar: Rc<RefCell<Option<ToolbarRef<u32, NonCloneAction>>>>,
    actions: Rc<RefCell<Vec<NonCloneAction>>>,
}

fn mounted() -> Harness {
    let toolbar = Rc::new(RefCell::new(None));
    let actions = Rc::new(RefCell::new(Vec::new()));
    let runtime = ViewRuntime::from_component(MountedToolbar {
        toolbar: toolbar.clone(),
        actions: actions.clone(),
    })
    .unwrap();
    Harness {
        runtime,
        toolbar,
        actions,
    }
}

#[test]
fn mounted_toolbar_has_one_focus_stop_item_semantics_and_density_floor() {
    let harness = mounted();
    let toolbar = harness.toolbar.borrow();
    let toolbar = toolbar.as_ref().unwrap();
    assert!(
        harness
            .runtime
            .ui()
            .interactions
            .get(toolbar.node())
            .unwrap()
            .focusable
    );
    assert!(toolbar.items().iter().all(|item| {
        !harness
            .runtime
            .ui()
            .interactions
            .get(item.node())
            .is_some_and(|interaction| interaction.focusable)
    }));
    let semantics = harness.runtime.ui().semantics.get(toolbar.node()).unwrap();
    assert_eq!(semantics.role, SemanticRole::Toolbar);
    assert_eq!(semantics.relationships.len(), 4);
    assert_eq!(
        semantics.relationships.last().unwrap().kind,
        SemanticRelationshipKind::ActiveDescendant
    );
    assert!(semantics.actions.contains(SemanticAction::Focus));

    let disabled = harness
        .runtime
        .ui()
        .semantics
        .get(toolbar.items()[1].node())
        .unwrap();
    assert!(disabled.state.disabled);
    assert!(disabled.effective_actions().is_empty());
    let checked = harness
        .runtime
        .ui()
        .semantics
        .get(toolbar.items()[2].node())
        .unwrap();
    assert_eq!(checked.state.checked, Some(SemanticCheckState::Mixed));
    assert_eq!(
        harness
            .runtime
            .ui()
            .box_styles
            .get(toolbar.items()[0].node())
            .unwrap()
            .min_size,
        SizeRule2D {
            width: SizeRule::Logical(44.0),
            height: SizeRule::Logical(44.0),
        }
    );
}

#[test]
fn mounted_navigation_and_routes_preserve_sources_without_clone_actions() {
    let mut harness = mounted();
    let (root, disabled, third) = {
        let toolbar = harness.toolbar.borrow();
        let toolbar = toolbar.as_ref().unwrap();
        (
            toolbar.node(),
            toolbar.items()[1].node(),
            toolbar.items()[2].node(),
        )
    };
    {
        let toolbar = harness.toolbar.borrow();
        let toolbar = toolbar.as_ref().unwrap();
        toolbar
            .navigate(
                CompositeNavigationCommand::Right,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(toolbar.active_command(), Some(2));
        assert!(matches!(
            toolbar.invoke_active(ChangeSource::Accessibility),
            Err(ToolbarInvocationError::Composite(
                CompositeError::ActiveDescendantDisabled(2)
            ))
        ));
        toolbar
            .navigate(
                CompositeNavigationCommand::Right,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        let direct = toolbar.invoke_active(ChangeSource::Accessibility).unwrap();
        assert_eq!(direct.command(), &3);
        assert_eq!(direct.source(), ChangeSource::Accessibility);
        assert_eq!(direct.checked(), Some(CheckState::Mixed));
        assert_eq!(
            direct.into_action(),
            NonCloneAction {
                command: 3,
                source: ChangeSource::Accessibility,
            }
        );
    }
    assert!(
        !harness
            .runtime
            .dispatch_activation(disabled, ChangeSource::Pointer)
    );
    assert!(
        harness
            .runtime
            .dispatch_activation(third, ChangeSource::Pointer)
    );
    assert!(
        harness
            .runtime
            .dispatch_activation(root, ChangeSource::Programmatic)
    );
    assert_eq!(
        &*harness.actions.borrow(),
        &[
            NonCloneAction {
                command: 3,
                source: ChangeSource::Pointer,
            },
            NonCloneAction {
                command: 3,
                source: ChangeSource::Programmatic,
            },
        ]
    );
}
