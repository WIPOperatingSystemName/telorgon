use std::cell::{Cell, RefCell};

use crate::input::CompositeItem;
use crate::runtime::{
    Component, ComponentRuntimeDriver, CreateContext, NoAction, Read, State, UpdateContext,
    ViewRuntime,
};
use crate::ui::{BoxStyle, LayoutStyle, OverlayAnchor, SemanticAction, UiRoot};

use crate::components::application::{ActionFactory, ApplicationOverlayController};

use super::*;

#[derive(Debug, PartialEq, Eq)]
struct NonCloneAction {
    command: u32,
    source: ChangeSource,
}

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

fn captured_read() -> (Read<bool>, ViewRuntime<ComponentRuntimeDriver<ReadCapture>>) {
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
fn construction_requires_name_items_unique_commands_and_one_owner() {
    let (local, _local_runtime) = captured_read();
    let (foreign, _foreign_runtime) = captured_read();
    assert!(matches!(
        Menu::<u32, ()>::new(" ", [MenuItem::command(command(1, local))]),
        Err(MenuError::MissingAccessibleName)
    ));
    assert!(matches!(
        Menu::<u32, ()>::new("File", []),
        Err(MenuError::Empty)
    ));
    assert!(matches!(
        Menu::new(
            "File",
            [
                MenuItem::command(command(1, local)),
                MenuItem::submenu(command(1, local)),
            ],
        ),
        Err(MenuError::DuplicateCommand(1))
    ));
    assert!(matches!(
        Menu::new(
            "File",
            [
                MenuItem::command(command(1, local)),
                MenuItem::command(command(2, foreign)),
            ],
        ),
        Err(MenuError::OwnerMismatch { .. })
    ));
}

struct MountedMenu {
    overlays: Rc<RefCell<ApplicationOverlayController>>,
    menu: Rc<RefCell<Option<MenuRef<u32, NonCloneAction>>>>,
    anchor: Rc<Cell<Option<UiNodeId>>>,
}

struct MountedMenuState {
    menu: Menu<u32, NonCloneAction>,
    _enabled: State<bool>,
    _disabled: State<bool>,
}

impl Component for MountedMenu {
    type State = MountedMenuState;
    type Action = MenuRouteRequest<u32>;

    fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
        let enabled = context.state(true);
        let disabled = context.state(false);
        let owner = context.component();
        let make = |command, label, available| {
            CommandSpec::new(
                command,
                label,
                available,
                ActionFactory::new(owner, move |source| NonCloneAction { command, source }),
            )
            .unwrap()
        };
        MountedMenuState {
            menu: Menu::new(
                "Build menu",
                [
                    MenuItem::command(make(1, "Archive", disabled.read())),
                    MenuItem::command(make(2, "Build", enabled.read())),
                    MenuItem::submenu(make(3, "Branches", enabled.read())),
                ],
            )
            .unwrap(),
            _enabled: enabled,
            _disabled: disabled,
        }
    }

    fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let root = ui
            .foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
        self.overlays.borrow_mut().mount(ui, root.0).unwrap();
        self.anchor.set(Some(root.0));
        let menu = state
            .menu
            .mount(
                ui,
                root.0,
                MenuLevelState {
                    overlay: OverlayId::from_raw(1, 1).unwrap(),
                    parent: None,
                    active_command: Some(2),
                },
                |request| request,
            )
            .unwrap();
        self.menu.replace(Some(menu));
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

struct Harness {
    runtime: ViewRuntime<ComponentRuntimeDriver<MountedMenu>>,
    overlays: Rc<RefCell<ApplicationOverlayController>>,
    controller: MenuController<u32>,
    menu: MenuRef<u32, NonCloneAction>,
}

fn harness() -> Harness {
    let overlays = Rc::new(RefCell::new(ApplicationOverlayController::new()));
    let menu = Rc::new(RefCell::new(None));
    let anchor = Rc::new(Cell::new(None));
    let runtime = ViewRuntime::from_component(MountedMenu {
        overlays: overlays.clone(),
        menu: menu.clone(),
        anchor: anchor.clone(),
    })
    .unwrap();
    let mut controller = MenuController::new();
    let opened = controller
        .open(
            &mut overlays.borrow_mut(),
            runtime.ui(),
            super::super::MenuOpenRequest::root(
                OverlayAnchor::Node(anchor.get().unwrap()),
                [
                    CompositeItem {
                        key: 1,
                        enabled: false,
                    },
                    CompositeItem {
                        key: 2,
                        enabled: true,
                    },
                    CompositeItem {
                        key: 3,
                        enabled: true,
                    },
                ],
            ),
        )
        .unwrap();
    let mounted = menu.borrow().as_ref().unwrap().clone();
    assert_eq!(opened.overlay, mounted.overlay());
    Harness {
        runtime,
        overlays,
        controller,
        menu: mounted,
    }
}

#[test]
fn mounted_level_has_one_focus_entry_and_typeahead_cycles_without_hiding_disabled_items() {
    let mut harness = harness();
    assert!(
        harness
            .runtime
            .ui()
            .interactions
            .get(harness.menu.node())
            .unwrap()
            .focusable
    );
    assert!(harness.menu.items().iter().all(|item| {
        !harness
            .runtime
            .ui()
            .interactions
            .get(item.node())
            .is_some_and(|interaction| interaction.focusable)
    }));
    let semantics = harness
        .runtime
        .ui()
        .semantics
        .get(harness.menu.node())
        .unwrap();
    assert_eq!(semantics.role, SemanticRole::Menu);
    assert!(semantics.actions.contains(SemanticAction::Focus));

    let first = harness.menu.typeahead(&harness.controller, "b").unwrap();
    assert_eq!(first.command, 3);
    harness
        .menu
        .apply_typeahead(&mut harness.controller, &first)
        .unwrap();
    let wrapped = harness.menu.typeahead(&harness.controller, "B").unwrap();
    assert_eq!(wrapped.command, 2);
    let disabled = harness.menu.typeahead(&harness.controller, "arc").unwrap();
    assert_eq!(disabled.command, 1);
    harness
        .menu
        .apply_typeahead(&mut harness.controller, &disabled)
        .unwrap();
    assert_eq!(harness.menu.active_command(&harness.controller), Some(1));
    assert_eq!(
        harness.menu.typeahead(&harness.controller, " "),
        Err(MenuInteractionError::EmptyTypeahead)
    );
}

#[test]
fn directional_submenu_intent_mirrors_and_disabled_commands_never_dispatch() {
    let mut harness = harness();
    harness
        .menu
        .navigate(
            &mut harness.controller,
            &mut harness.overlays.borrow_mut(),
            CompositeNavigationCommand::Down,
            WritingDirection::LeftToRight,
        )
        .unwrap();
    assert_eq!(harness.menu.active_command(&harness.controller), Some(3));
    assert_eq!(
        harness
            .menu
            .navigate(
                &mut harness.controller,
                &mut harness.overlays.borrow_mut(),
                CompositeNavigationCommand::Right,
                WritingDirection::LeftToRight,
            )
            .unwrap(),
        MenuNavigation::Submenu(MenuSubmenuIntent {
            parent: harness.menu.overlay(),
            command: 3,
            source: ChangeSource::Directional,
        })
    );
    assert!(matches!(
        harness
            .menu
            .navigate(
                &mut harness.controller,
                &mut harness.overlays.borrow_mut(),
                CompositeNavigationCommand::Left,
                WritingDirection::RightToLeft,
            )
            .unwrap(),
        MenuNavigation::Submenu(MenuSubmenuIntent { command: 3, .. })
    ));

    let disabled = MenuRouteRequest::Activate {
        command: 1,
        source: ChangeSource::Accessibility,
        dismissal: MenuActivationDismissal::Chain,
    };
    assert!(matches!(
        harness.menu.dispatch(
            &mut harness.controller,
            &mut harness.overlays.borrow_mut(),
            disabled,
        ),
        Err(MenuInteractionError::DisabledCommand(1))
    ));
    assert_eq!(harness.overlays.borrow().state().entry_count, 1);
}

#[test]
fn command_dispatch_closes_first_and_preserves_a_nonclone_action_source() {
    let mut harness = harness();
    let dispatch = harness
        .menu
        .dispatch(
            &mut harness.controller,
            &mut harness.overlays.borrow_mut(),
            MenuRouteRequest::Activate {
                command: 2,
                source: ChangeSource::Accessibility,
                dismissal: MenuActivationDismissal::Chain,
            },
        )
        .unwrap();
    let MenuDispatch::Command(intent) = dispatch else {
        panic!("command row must dispatch a command intent")
    };
    assert_eq!(intent.source(), ChangeSource::Accessibility);
    assert_eq!(
        intent.close_effect().dismissed[0].id,
        harness.menu.overlay()
    );
    assert_eq!(harness.overlays.borrow().state().entry_count, 0);
    assert_eq!(
        intent.into_action(),
        NonCloneAction {
            command: 2,
            source: ChangeSource::Accessibility,
        }
    );
}
