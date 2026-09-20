//! Typed menu-chain lifecycle and highlight coordination.
//!
//! The application overlay remains the sole owner of overlay entries, the neutral composite
//! remains the sole owner of each level's highlight, and `CommandSpec` remains the command owner.
//! This controller only coordinates their ordered effects. It mounts no menu rows, applies no
//! focus/input effect, and starts no timer.

use std::fmt;
use std::hash::Hash;
use std::time::Duration;

use crate::input::{
    ChangeSource, CompositeChange, CompositeEdgeBehavior, CompositeError, CompositeItem,
    CompositeNavigationCommand, CompositeNavigationPolicy, CompositeOrientation,
    CompositeSelectionBehavior, CompositeStateMachine, DisabledItemPolicy, WritingDirection,
};
use crate::runtime::MonotonicInstant;
use crate::ui::{
    DismissReason, MountedUi, OutsidePressPolicy, OverlayAnchor, OverlayCloseOutcome,
    OverlayDismissPolicy, OverlayDismissResult, OverlayFocusContainment, OverlayFocusLifecycle,
    OverlayFocusRequest, OverlayFocusRestoration, OverlayId, OverlayInitialFocus, OverlayModality,
    OverlayOpenRequest,
};

use super::{CommandSpec, ResolvedCommandState};
use crate::components::application::{
    ApplicationOverlayCommand, ApplicationOverlayController, ApplicationOverlayControllerError,
    ApplicationOverlayEffect, CheckState,
};

/// Whether opening a menu level asks the separate focus owner to enter its composite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuOpeningFocus {
    None,
    #[default]
    SelectedOrFirst,
}

impl MenuOpeningFocus {
    const fn overlay(self) -> OverlayInitialFocus {
        match self {
            Self::None => OverlayInitialFocus::None,
            Self::SelectedOrFirst => OverlayInitialFocus::SelectedOrFirst,
        }
    }
}

/// Controlled inputs for opening one menu level.
#[derive(Clone, Debug)]
pub struct MenuOpenRequest<K> {
    pub anchor: OverlayAnchor,
    pub parent: Option<OverlayId>,
    pub items: Vec<CompositeItem<K>>,
    pub selected: Option<K>,
    pub opening_focus: MenuOpeningFocus,
}

impl<K> MenuOpenRequest<K> {
    pub fn root(anchor: OverlayAnchor, items: impl IntoIterator<Item = CompositeItem<K>>) -> Self {
        Self {
            anchor,
            parent: None,
            items: items.into_iter().collect(),
            selected: None,
            opening_focus: MenuOpeningFocus::SelectedOrFirst,
        }
    }

    pub fn submenu(
        parent: OverlayId,
        anchor: OverlayAnchor,
        items: impl IntoIterator<Item = CompositeItem<K>>,
    ) -> Self {
        Self {
            anchor,
            parent: Some(parent),
            items: items.into_iter().collect(),
            selected: None,
            opening_focus: MenuOpeningFocus::SelectedOrFirst,
        }
    }

    pub fn selected(mut self, selected: K) -> Self {
        self.selected = Some(selected);
        self
    }

    pub fn opening_focus(mut self, opening_focus: MenuOpeningFocus) -> Self {
        self.opening_focus = opening_focus;
        self
    }
}

/// Observable open result; the caller applies the focus request after mounting the level's rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuOpened<K> {
    pub overlay: OverlayId,
    pub parent: Option<OverlayId>,
    pub focus: OverlayFocusRequest,
    pub highlight: CompositeChange<K>,
}

/// Snapshot of controller-owned highlight facts for one live overlay level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuLevelState<K> {
    pub overlay: OverlayId,
    pub parent: Option<OverlayId>,
    pub active_command: Option<K>,
}

/// Which overlay range activation closes before producing the command action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuActivationDismissal {
    Level,
    #[default]
    Chain,
}

/// A fresh typed action plus the already-completed close effect that must precede its enqueue.
#[derive(Debug)]
pub struct MenuCommandIntent<K, A> {
    command: K,
    action: A,
    source: ChangeSource,
    checked: Option<CheckState>,
    close: OverlayCloseOutcome,
}

impl<K, A> MenuCommandIntent<K, A> {
    pub const fn command(&self) -> &K {
        &self.command
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub const fn checked(&self) -> Option<CheckState> {
        self.checked
    }

    pub fn close_effect(&self) -> &OverlayCloseOutcome {
        &self.close
    }

    pub fn into_action(self) -> A {
        self.action
    }

    pub fn into_parts(self) -> (OverlayCloseOutcome, A) {
        (self.close, self.action)
    }
}

/// Caller-owned request to schedule a delayed submenu open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuSubmenuDeadline<K> {
    pub parent: OverlayId,
    pub command: K,
    pub at: MonotonicInstant,
}

/// Caller-owned request to cancel any matching pending submenu open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuSubmenuCancellation<K> {
    pub parent: OverlayId,
    pub command: K,
}

#[derive(Clone, Debug)]
struct MenuLevel<K> {
    overlay: OverlayId,
    parent: Option<OverlayId>,
    composite: CompositeStateMachine<K>,
}

/// One linear root-to-leaf menu chain over existing overlay and composite owners.
#[derive(Clone, Debug, Default)]
pub struct MenuController<K> {
    levels: Vec<MenuLevel<K>>,
}

impl<K> MenuController<K>
where
    K: Copy + Eq + Hash,
{
    pub fn new() -> Self {
        Self { levels: Vec::new() }
    }

    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    pub fn root_overlay(&self) -> Option<OverlayId> {
        self.levels.first().map(|level| level.overlay)
    }

    pub fn active_overlay(&self) -> Option<OverlayId> {
        self.levels.last().map(|level| level.overlay)
    }

    pub fn level(&self, overlay: OverlayId) -> Option<MenuLevelState<K>> {
        self.levels
            .iter()
            .find(|level| level.overlay == overlay)
            .map(level_state)
    }

    pub fn levels(&self) -> impl ExactSizeIterator<Item = MenuLevelState<K>> + '_ {
        self.levels.iter().map(level_state)
    }

    /// Opens one root or one child of the current leaf without mounting its rows or applying focus.
    pub fn open(
        &mut self,
        overlays: &mut ApplicationOverlayController,
        ui: &MountedUi,
        request: MenuOpenRequest<K>,
    ) -> Result<MenuOpened<K>, MenuControllerError<K>> {
        if request.items.is_empty() {
            return Err(MenuControllerError::EmptyLevel);
        }
        match (self.levels.last(), request.parent) {
            (None, None) => {}
            (Some(_), None) => return Err(MenuControllerError::RootAlreadyOpen),
            (Some(level), Some(parent)) if level.overlay == parent => {}
            (_, Some(parent)) => return Err(MenuControllerError::InvalidParent(parent)),
        }

        let mut composite = CompositeStateMachine::new(menu_navigation_policy());
        composite
            .update_items(request.items)
            .map_err(MenuControllerError::Composite)?;
        let highlight = composite
            .enter(request.selected)
            .map_err(MenuControllerError::Composite)?;

        let restoration = match request.anchor {
            OverlayAnchor::Node(node) => OverlayFocusRestoration::TargetThenNearest(node),
            OverlayAnchor::Point(_) | OverlayAnchor::Rect(_) => OverlayFocusRestoration::None,
        };
        let overlay_request = OverlayOpenRequest {
            anchor: request.anchor,
            parent: request.parent,
            modality: OverlayModality::NonModal,
            dismissal: OverlayDismissPolicy {
                escape: true,
                outside_press: OutsidePressPolicy::DismissAndConsume,
                focus_lost: true,
                pointer_departure: false,
            },
            focus: OverlayFocusLifecycle {
                initial: request.opening_focus.overlay(),
                containment: OverlayFocusContainment::None,
                restoration,
            },
        };
        let ApplicationOverlayEffect::Opened(opened) = overlays
            .route(ApplicationOverlayCommand::Open {
                ui,
                request: overlay_request,
            })
            .map_err(MenuControllerError::Overlay)?
        else {
            unreachable!("open command has one effect variant")
        };
        self.levels.push(MenuLevel {
            overlay: opened.id,
            parent: request.parent,
            composite,
        });
        Ok(MenuOpened {
            overlay: opened.id,
            parent: request.parent,
            focus: opened.focus,
            highlight,
        })
    }

    pub fn navigate(
        &mut self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<CompositeChange<K>, MenuControllerError<K>> {
        self.active_level_mut()?
            .composite
            .navigate(command, direction)
            .map_err(MenuControllerError::Composite)
    }

    pub fn set_highlight(
        &mut self,
        command: K,
    ) -> Result<CompositeChange<K>, MenuControllerError<K>> {
        self.active_level_mut()?
            .composite
            .set_active_descendant(command)
            .map_err(MenuControllerError::Composite)
    }

    /// Closes only the current leaf. Overlay close order remains topmost to bottommost.
    pub fn dismiss_level(
        &mut self,
        overlays: &mut ApplicationOverlayController,
        reason: DismissReason,
    ) -> Result<OverlayCloseOutcome, MenuControllerError<K>> {
        let overlay = self
            .active_overlay()
            .ok_or(MenuControllerError::NoOpenMenu)?;
        self.dismiss(overlays, overlay, reason)
    }

    /// Closes the root and every descendant in one topmost-to-root ordered overlay effect.
    pub fn dismiss_chain(
        &mut self,
        overlays: &mut ApplicationOverlayController,
        reason: DismissReason,
    ) -> Result<OverlayCloseOutcome, MenuControllerError<K>> {
        let overlay = self.root_overlay().ok_or(MenuControllerError::NoOpenMenu)?;
        self.dismiss(overlays, overlay, reason)
    }

    /// Reconciles externally produced overlay closes without copying overlay lifecycle state.
    pub fn observe_close(&mut self, close: &OverlayCloseOutcome) {
        self.levels.retain(|level| {
            !close
                .dismissed
                .iter()
                .any(|dismissed| dismissed.id == level.overlay)
        });
    }

    /// Returns a deadline for the caller's scheduler; no timer is retained or started here.
    pub fn submenu_deadline(
        &self,
        parent: OverlayId,
        command: K,
        began_at: MonotonicInstant,
        delay: Duration,
    ) -> Result<MenuSubmenuDeadline<K>, MenuControllerError<K>> {
        if delay.is_zero() {
            return Err(MenuControllerError::ZeroSubmenuDelay);
        }
        let Some(level) = self.levels.last().filter(|level| level.overlay == parent) else {
            return Err(MenuControllerError::InvalidParent(parent));
        };
        if level.composite.active_descendant() != Some(command) {
            return Err(MenuControllerError::CommandNotHighlighted(command));
        }
        let at = began_at
            .checked_add(delay)
            .ok_or(MenuControllerError::SubmenuDeadlineOverflow)?;
        Ok(MenuSubmenuDeadline {
            parent,
            command,
            at,
        })
    }

    /// Returns a cancellation key to the caller; no scheduler state is mutated here.
    pub fn cancel_submenu(
        &self,
        parent: OverlayId,
        command: K,
    ) -> Result<MenuSubmenuCancellation<K>, MenuControllerError<K>> {
        if self
            .levels
            .last()
            .is_none_or(|level| level.overlay != parent)
        {
            return Err(MenuControllerError::InvalidParent(parent));
        }
        Ok(MenuSubmenuCancellation { parent, command })
    }

    /// Closes first, then constructs one action. The caller remains the action-queue owner.
    pub fn activate<A: 'static>(
        &mut self,
        overlays: &mut ApplicationOverlayController,
        command: &CommandSpec<K, A>,
        state: ResolvedCommandState,
        source: ChangeSource,
        dismissal: MenuActivationDismissal,
    ) -> Result<MenuCommandIntent<K, A>, MenuControllerError<K>> {
        let request = self
            .active_level_mut()?
            .composite
            .request_active_selection(source)
            .map_err(MenuControllerError::Composite)?;
        if request.key != *command.id() {
            return Err(MenuControllerError::CommandMismatch {
                highlighted: request.key,
                supplied: *command.id(),
            });
        }
        if !state.enabled() {
            return Err(MenuControllerError::DisabledCommand(request.key));
        }

        let close = match dismissal {
            MenuActivationDismissal::Level => {
                self.dismiss_level(overlays, DismissReason::Accepted)?
            }
            MenuActivationDismissal::Chain => {
                self.dismiss_chain(overlays, DismissReason::Accepted)?
            }
        };
        let checked = state.checked();
        let Some(action) = command.invoke(state, source).into_action() else {
            unreachable!("enabled snapshot was checked before command invocation")
        };
        Ok(MenuCommandIntent {
            command: request.key,
            action,
            source,
            checked,
            close,
        })
    }

    fn active_level_mut(&mut self) -> Result<&mut MenuLevel<K>, MenuControllerError<K>> {
        self.levels
            .last_mut()
            .ok_or(MenuControllerError::NoOpenMenu)
    }

    fn dismiss(
        &mut self,
        overlays: &mut ApplicationOverlayController,
        overlay: OverlayId,
        reason: DismissReason,
    ) -> Result<OverlayCloseOutcome, MenuControllerError<K>> {
        let ApplicationOverlayEffect::Dismissal(result) = overlays
            .route(ApplicationOverlayCommand::Dismiss {
                id: overlay,
                reason,
            })
            .map_err(MenuControllerError::Overlay)?
        else {
            unreachable!("dismiss command has one effect variant")
        };
        match result {
            OverlayDismissResult::Dismissed(close) => {
                self.observe_close(&close);
                Ok(close)
            }
            OverlayDismissResult::Blocked { .. } => {
                Err(MenuControllerError::DismissalBlocked { overlay, reason })
            }
        }
    }
}

fn level_state<K: Copy + Eq + Hash>(level: &MenuLevel<K>) -> MenuLevelState<K> {
    MenuLevelState {
        overlay: level.overlay,
        parent: level.parent,
        active_command: level.composite.active_descendant(),
    }
}

const fn menu_navigation_policy() -> CompositeNavigationPolicy {
    CompositeNavigationPolicy {
        orientation: CompositeOrientation::Vertical,
        edge_behavior: CompositeEdgeBehavior::Wrap,
        disabled_items: DisabledItemPolicy::Include,
        selection: CompositeSelectionBehavior::Independent,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuControllerError<K> {
    EmptyLevel,
    RootAlreadyOpen,
    NoOpenMenu,
    InvalidParent(OverlayId),
    CommandNotHighlighted(K),
    CommandMismatch {
        highlighted: K,
        supplied: K,
    },
    DisabledCommand(K),
    ZeroSubmenuDelay,
    SubmenuDeadlineOverflow,
    DismissalBlocked {
        overlay: OverlayId,
        reason: DismissReason,
    },
    Composite(CompositeError<K>),
    Overlay(ApplicationOverlayControllerError),
}

impl<K: fmt::Debug> fmt::Display for MenuControllerError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "menu controller transition failed: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for MenuControllerError<K> {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::runtime::{
        Component, ComponentRuntimeDriver, CreateContext, State, Ui, UpdateContext, ViewRuntime,
    };
    use crate::ui::{BoxStyle, LayoutStyle, UiNodeId, UiRoot};

    use crate::components::application::ActionFactory;

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneAction {
        command: u32,
        source: ChangeSource,
    }

    struct MountedOwner {
        overlays: Rc<RefCell<ApplicationOverlayController>>,
        nodes: Rc<RefCell<Vec<UiNodeId>>>,
        commands: Rc<RefCell<Vec<CommandSpec<u32, NonCloneAction>>>>,
    }

    struct MountedOwnerState {
        _enabled: State<bool>,
    }

    impl Component for MountedOwner {
        type State = MountedOwnerState;
        type Action = ();

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            let enabled = context.state(true);
            let owner = context.component();
            let make = move |command| {
                CommandSpec::new(
                    command,
                    format!("Command {command}"),
                    enabled.read(),
                    ActionFactory::new(owner, move |source| NonCloneAction { command, source }),
                )
                .unwrap()
            };
            self.commands.replace(vec![make(1), make(2), make(3)]);
            MountedOwnerState { _enabled: enabled }
        }

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let nodes = self.nodes.clone();
            let root =
                ui.foundation()
                    .root(BoxStyle::default(), LayoutStyle::default(), move |writer| {
                        for _ in 0..3 {
                            nodes.borrow_mut().push(writer.container(
                                BoxStyle::default(),
                                LayoutStyle::default(),
                                |_| {},
                            ));
                        }
                    });
            self.overlays.borrow_mut().mount(ui, root.0).unwrap();
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
        runtime: ViewRuntime<ComponentRuntimeDriver<MountedOwner>>,
        overlays: Rc<RefCell<ApplicationOverlayController>>,
        nodes: Rc<RefCell<Vec<UiNodeId>>>,
        commands: Rc<RefCell<Vec<CommandSpec<u32, NonCloneAction>>>>,
    }

    fn harness() -> Harness {
        let overlays = Rc::new(RefCell::new(ApplicationOverlayController::new()));
        let nodes = Rc::new(RefCell::new(Vec::new()));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let runtime = ViewRuntime::from_component(MountedOwner {
            overlays: overlays.clone(),
            nodes: nodes.clone(),
            commands: commands.clone(),
        })
        .unwrap();
        Harness {
            runtime,
            overlays,
            nodes,
            commands,
        }
    }

    fn items() -> [CompositeItem<u32>; 3] {
        [
            CompositeItem {
                key: 1,
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
        ]
    }

    fn open_root(controller: &mut MenuController<u32>, harness: &Harness) -> MenuOpened<u32> {
        controller
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                MenuOpenRequest::root(OverlayAnchor::Node(harness.nodes.borrow()[0]), items()),
            )
            .unwrap()
    }

    #[test]
    fn opening_uses_explicit_parentage_and_selected_or_first_focus_intent() {
        let harness = harness();
        let mut menus = MenuController::new();
        let root = open_root(&mut menus, &harness);
        assert_eq!(
            root.focus,
            OverlayFocusRequest::Initial(OverlayInitialFocus::SelectedOrFirst)
        );
        assert!(matches!(
            root.highlight,
            CompositeChange::Entered {
                target: crate::input::CompositeFocusTarget::Item(1),
                ..
            }
        ));

        let child = menus
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                MenuOpenRequest::submenu(
                    root.overlay,
                    OverlayAnchor::Node(harness.nodes.borrow()[1]),
                    [CompositeItem {
                        key: 3,
                        enabled: true,
                    }],
                ),
            )
            .unwrap();
        assert_eq!(child.parent, Some(root.overlay));
        assert_eq!(
            harness
                .overlays
                .borrow()
                .entry(child.overlay)
                .unwrap()
                .parent,
            Some(root.overlay)
        );
        assert_eq!(menus.level_count(), 2);
        assert_eq!(
            menus.open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                MenuOpenRequest::root(OverlayAnchor::Node(harness.nodes.borrow()[2]), items()),
            ),
            Err(MenuControllerError::RootAlreadyOpen)
        );
    }

    #[test]
    fn navigation_discovers_disabled_items_and_submenu_timing_stays_caller_owned() {
        let harness = harness();
        let mut menus = MenuController::new();
        let root = open_root(&mut menus, &harness);
        menus
            .navigate(
                CompositeNavigationCommand::Down,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(menus.level(root.overlay).unwrap().active_command, Some(2));
        assert_eq!(
            menus
                .submenu_deadline(
                    root.overlay,
                    2,
                    MonotonicInstant::from_nanos(10),
                    Duration::from_millis(250),
                )
                .unwrap(),
            MenuSubmenuDeadline {
                parent: root.overlay,
                command: 2,
                at: MonotonicInstant::from_nanos(250_000_010),
            }
        );
        assert_eq!(
            menus.cancel_submenu(root.overlay, 2).unwrap(),
            MenuSubmenuCancellation {
                parent: root.overlay,
                command: 2,
            }
        );
        assert_eq!(
            menus.submenu_deadline(root.overlay, 2, MonotonicInstant::ZERO, Duration::ZERO,),
            Err(MenuControllerError::ZeroSubmenuDelay)
        );
    }

    #[test]
    fn escape_closes_one_level_while_chain_close_is_top_to_root() {
        let harness = harness();
        let mut menus = MenuController::new();
        let root = open_root(&mut menus, &harness);
        let child = menus
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                MenuOpenRequest::submenu(
                    root.overlay,
                    OverlayAnchor::Node(harness.nodes.borrow()[1]),
                    [CompositeItem {
                        key: 3,
                        enabled: true,
                    }],
                ),
            )
            .unwrap();
        let leaf_close = menus
            .dismiss_level(&mut harness.overlays.borrow_mut(), DismissReason::Escape)
            .unwrap();
        assert_eq!(leaf_close.dismissed[0].id, child.overlay);
        assert_eq!(menus.active_overlay(), Some(root.overlay));

        let child = menus
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                MenuOpenRequest::submenu(
                    root.overlay,
                    OverlayAnchor::Node(harness.nodes.borrow()[2]),
                    [CompositeItem {
                        key: 3,
                        enabled: true,
                    }],
                ),
            )
            .unwrap();
        let chain_close = menus
            .dismiss_chain(&mut harness.overlays.borrow_mut(), DismissReason::Cancelled)
            .unwrap();
        assert_eq!(
            chain_close
                .dismissed
                .iter()
                .map(|dismissed| dismissed.id)
                .collect::<Vec<_>>(),
            vec![child.overlay, root.overlay]
        );
        assert_eq!(menus.level_count(), 0);
    }

    #[test]
    fn activation_rejects_disabled_and_creates_nonclone_action_after_close() {
        let harness = harness();
        let mut menus = MenuController::new();
        let root = open_root(&mut menus, &harness);
        menus
            .navigate(
                CompositeNavigationCommand::Down,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert!(matches!(
            menus.activate(
                &mut harness.overlays.borrow_mut(),
                &harness.commands.borrow()[1],
                ResolvedCommandState::new(false, None),
                ChangeSource::Keyboard,
                MenuActivationDismissal::Chain,
            ),
            Err(MenuControllerError::Composite(
                CompositeError::ActiveDescendantDisabled(2)
            ))
        ));
        assert_eq!(menus.active_overlay(), Some(root.overlay));

        menus
            .navigate(
                CompositeNavigationCommand::Down,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        let intent = menus
            .activate(
                &mut harness.overlays.borrow_mut(),
                &harness.commands.borrow()[2],
                ResolvedCommandState::new(true, Some(CheckState::Mixed)),
                ChangeSource::Accessibility,
                MenuActivationDismissal::Chain,
            )
            .unwrap();
        assert_eq!(intent.command(), &3);
        assert_eq!(intent.source(), ChangeSource::Accessibility);
        assert_eq!(intent.checked(), Some(CheckState::Mixed));
        assert_eq!(intent.close_effect().dismissed[0].id, root.overlay);
        assert_eq!(harness.overlays.borrow().state().entry_count, 0);
        assert_eq!(
            intent.into_action(),
            NonCloneAction {
                command: 3,
                source: ChangeSource::Accessibility,
            }
        );
    }
}
