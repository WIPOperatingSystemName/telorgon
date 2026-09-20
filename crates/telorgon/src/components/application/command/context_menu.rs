//! Typed context-menu opening and dismissal policy over the shared menu controller.
//!
//! `MenuController` remains the sole menu-chain/highlight owner, `CommandSpec` remains the command
//! owner, and the application overlay remains the lifecycle owner. This wrapper records no copied
//! overlay or command state, mounts no rows, applies no focus/input effect, and invokes no platform
//! menu service.

use std::fmt;
use std::hash::Hash;

use crate::foundation::PointF;
use crate::input::{ChangeSource, CompositeItem};
use crate::ui::{
    DismissReason, MountedUi, OverlayAnchor, OverlayCloseOutcome, OverlayFocusRequest,
    OverlayFocusRestoration, UiNodeId,
};

use super::{
    CommandSpec, MenuActivationDismissal, MenuCommandIntent, MenuController, MenuControllerError,
    MenuOpenRequest, MenuOpened, MenuOpeningFocus, ResolvedCommandState,
};
use crate::components::application::ApplicationOverlayController;

/// Source and lifecycle anchor of one context-menu opening request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContextMenuOpening {
    /// A secondary-pointer or recognized context gesture at a logical view position.
    Pointer(PointF),
    /// A keyboard context-menu request associated with the currently focused node.
    Keyboard(UiNodeId),
    /// A caller-requested opening at an explicit platform-neutral anchor.
    Programmatic(OverlayAnchor),
}

impl ContextMenuOpening {
    pub const fn pointer(position: PointF) -> Self {
        Self::Pointer(position)
    }

    pub const fn keyboard(anchor: UiNodeId) -> Self {
        Self::Keyboard(anchor)
    }

    pub const fn programmatic(anchor: OverlayAnchor) -> Self {
        Self::Programmatic(anchor)
    }

    pub const fn source(self) -> ChangeSource {
        match self {
            Self::Pointer(_) => ChangeSource::Pointer,
            Self::Keyboard(_) => ChangeSource::Keyboard,
            Self::Programmatic(_) => ChangeSource::Programmatic,
        }
    }

    pub const fn anchor(self) -> OverlayAnchor {
        match self {
            Self::Pointer(position) => OverlayAnchor::Point(position),
            Self::Keyboard(node) => OverlayAnchor::Node(node),
            Self::Programmatic(anchor) => anchor,
        }
    }

    /// Keyboard openings enter the composite immediately. Pointer/programmatic callers may opt in.
    pub const fn default_focus(self) -> MenuOpeningFocus {
        match self {
            Self::Keyboard(_) => MenuOpeningFocus::SelectedOrFirst,
            Self::Pointer(_) | Self::Programmatic(_) => MenuOpeningFocus::None,
        }
    }

    /// Focus restoration recorded by the shared overlay owner for this anchor kind.
    pub const fn restoration(self) -> OverlayFocusRestoration {
        match self.anchor() {
            OverlayAnchor::Node(node) => OverlayFocusRestoration::TargetThenNearest(node),
            OverlayAnchor::Point(_) | OverlayAnchor::Rect(_) => OverlayFocusRestoration::None,
        }
    }
}

/// Controlled root-level inputs. Item availability is a snapshot supplied by the command owner.
#[derive(Clone, Debug)]
pub struct ContextMenuOpenRequest<K> {
    pub opening: ContextMenuOpening,
    pub items: Vec<CompositeItem<K>>,
    pub selected: Option<K>,
    pub opening_focus: MenuOpeningFocus,
}

impl<K> ContextMenuOpenRequest<K> {
    pub fn pointer(position: PointF, items: impl IntoIterator<Item = CompositeItem<K>>) -> Self {
        Self::new(ContextMenuOpening::pointer(position), items)
    }

    pub fn keyboard(anchor: UiNodeId, items: impl IntoIterator<Item = CompositeItem<K>>) -> Self {
        Self::new(ContextMenuOpening::keyboard(anchor), items)
    }

    pub fn programmatic(
        anchor: OverlayAnchor,
        items: impl IntoIterator<Item = CompositeItem<K>>,
    ) -> Self {
        Self::new(ContextMenuOpening::programmatic(anchor), items)
    }

    pub fn new(
        opening: ContextMenuOpening,
        items: impl IntoIterator<Item = CompositeItem<K>>,
    ) -> Self {
        Self {
            opening,
            items: items.into_iter().collect(),
            selected: None,
            opening_focus: opening.default_focus(),
        }
    }

    pub fn selected(mut self, selected: K) -> Self {
        self.selected = Some(selected);
        self
    }

    /// Overrides the source-specific default without applying the returned focus effect.
    pub fn opening_focus(mut self, opening_focus: MenuOpeningFocus) -> Self {
        self.opening_focus = opening_focus;
        self
    }
}

/// A root menu open plus the source that requested it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextMenuOpened<K> {
    pub menu: MenuOpened<K>,
    pub source: ChangeSource,
    pub restoration: OverlayFocusRestoration,
}

impl<K> ContextMenuOpened<K> {
    pub const fn focus_request(&self) -> OverlayFocusRequest {
        self.menu.focus
    }
}

/// User/lifecycle intent and its required menu-chain scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContextMenuDismissal {
    /// Escape closes only the current level, restoring its parent item or root anchor as applicable.
    Escape,
    /// An outside press closes the whole chain and is consumed by the overlay policy.
    OutsidePress,
    /// Explicit cancellation closes the whole chain.
    Cancelled,
    /// A replacement request closes the previous whole chain before a later open.
    Replaced,
}

impl ContextMenuDismissal {
    const fn reason(self) -> DismissReason {
        match self {
            Self::Escape => DismissReason::Escape,
            Self::OutsidePress => DismissReason::OutsidePress,
            Self::Cancelled => DismissReason::Cancelled,
            Self::Replaced => DismissReason::Replaced,
        }
    }

    const fn closes_level(self) -> bool {
        matches!(self, Self::Escape)
    }
}

/// Context-menu policy owning exactly one shared menu-chain controller.
#[derive(Clone, Debug, Default)]
pub struct ContextMenu<K> {
    controller: MenuController<K>,
}

impl<K> ContextMenu<K>
where
    K: Copy + Eq + Hash,
{
    pub fn new() -> Self {
        Self {
            controller: MenuController::new(),
        }
    }

    pub const fn controller(&self) -> &MenuController<K> {
        &self.controller
    }

    /// Exposes the one controller for submenu/highlight coordination; no second state is retained.
    pub fn controller_mut(&mut self) -> &mut MenuController<K> {
        &mut self.controller
    }

    /// Opens one root through the existing menu and overlay owners.
    pub fn open(
        &mut self,
        overlays: &mut ApplicationOverlayController,
        ui: &MountedUi,
        request: ContextMenuOpenRequest<K>,
    ) -> Result<ContextMenuOpened<K>, ContextMenuError<K>> {
        let source = request.opening.source();
        let restoration = request.opening.restoration();
        let mut menu = MenuOpenRequest::root(request.opening.anchor(), request.items)
            .opening_focus(request.opening_focus);
        if let Some(selected) = request.selected {
            menu = menu.selected(selected);
        }
        let menu = self
            .controller
            .open(overlays, ui, menu)
            .map_err(ContextMenuError::Menu)?;
        Ok(ContextMenuOpened {
            menu,
            source,
            restoration,
        })
    }

    /// Applies context-menu dismissal scope but leaves returned focus/input effects to the caller.
    pub fn dismiss(
        &mut self,
        overlays: &mut ApplicationOverlayController,
        dismissal: ContextMenuDismissal,
    ) -> Result<OverlayCloseOutcome, ContextMenuError<K>> {
        let result = if dismissal.closes_level() {
            self.controller.dismiss_level(overlays, dismissal.reason())
        } else {
            self.controller.dismiss_chain(overlays, dismissal.reason())
        };
        result.map_err(ContextMenuError::Menu)
    }

    /// Closes the complete context-menu chain before constructing one fresh typed action.
    pub fn activate<A: 'static>(
        &mut self,
        overlays: &mut ApplicationOverlayController,
        command: &CommandSpec<K, A>,
        state: ResolvedCommandState,
        source: ChangeSource,
    ) -> Result<MenuCommandIntent<K, A>, ContextMenuError<K>> {
        self.controller
            .activate(
                overlays,
                command,
                state,
                source,
                MenuActivationDismissal::Chain,
            )
            .map_err(ContextMenuError::Menu)
    }

    /// Reconciles forced overlay closure through the same menu owner.
    pub fn observe_close(&mut self, close: &OverlayCloseOutcome) {
        self.controller.observe_close(close);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuError<K> {
    Menu(MenuControllerError<K>),
}

impl<K: fmt::Debug> fmt::Display for ContextMenuError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "context-menu transition failed: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for ContextMenuError<K> {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::foundation::RectF;
    use crate::runtime::{
        Component, ComponentRuntimeDriver, CreateContext, State, Ui, UpdateContext, ViewRuntime,
    };
    use crate::ui::{BoxStyle, LayoutStyle, OverlayFocusRestoration, OverlayInitialFocus, UiRoot};

    use crate::components::application::{ActionFactory, CheckState};

    use super::*;

    struct Fixture {
        overlays: Rc<RefCell<ApplicationOverlayController>>,
        anchor: Rc<Cell<Option<UiNodeId>>>,
        command: Rc<RefCell<Option<CommandSpec<u32, NonCloneAction>>>>,
        calls: Rc<Cell<usize>>,
    }

    struct FixtureState {
        _enabled: State<bool>,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneAction {
        source: ChangeSource,
        sequence: usize,
    }

    impl Component for Fixture {
        type State = FixtureState;
        type Action = ();

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            let enabled = context.state(true);
            let calls = self.calls.clone();
            self.command.replace(Some(
                CommandSpec::new(
                    7,
                    "Inspect",
                    enabled.read(),
                    ActionFactory::new(context.component(), move |source| {
                        let sequence = calls.get() + 1;
                        calls.set(sequence);
                        NonCloneAction { source, sequence }
                    }),
                )
                .unwrap(),
            ));
            FixtureState { _enabled: enabled }
        }

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.overlays.borrow_mut().mount(ui, root.0).unwrap();
            self.anchor.set(Some(root.0));
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
        runtime: ViewRuntime<ComponentRuntimeDriver<Fixture>>,
        overlays: Rc<RefCell<ApplicationOverlayController>>,
        anchor: UiNodeId,
        command: Rc<RefCell<Option<CommandSpec<u32, NonCloneAction>>>>,
        calls: Rc<Cell<usize>>,
    }

    fn harness() -> Harness {
        let overlays = Rc::new(RefCell::new(ApplicationOverlayController::new()));
        let anchor = Rc::new(Cell::new(None));
        let command = Rc::new(RefCell::new(None));
        let calls = Rc::new(Cell::new(0));
        let runtime = ViewRuntime::from_component(Fixture {
            overlays: overlays.clone(),
            anchor: anchor.clone(),
            command: command.clone(),
            calls: calls.clone(),
        })
        .unwrap();
        Harness {
            runtime,
            overlays,
            anchor: anchor.get().unwrap(),
            command,
            calls,
        }
    }

    fn item(key: u32, enabled: bool) -> CompositeItem<u32> {
        CompositeItem { key, enabled }
    }

    #[test]
    fn opening_source_selects_anchor_and_explicit_focus_policy() {
        let harness = harness();
        let mut context = ContextMenu::new();
        let pointer = context
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                ContextMenuOpenRequest::pointer(PointF { x: 8.0, y: 12.0 }, [item(7, true)]),
            )
            .unwrap();
        assert_eq!(pointer.source, ChangeSource::Pointer);
        assert_eq!(pointer.focus_request(), OverlayFocusRequest::None);
        let overlays = harness.overlays.borrow();
        let entry = overlays.entry(pointer.menu.overlay).unwrap();
        assert_eq!(
            entry.anchor,
            OverlayAnchor::Point(PointF { x: 8.0, y: 12.0 })
        );
        assert_eq!(entry.focus.restoration, OverlayFocusRestoration::None);
        drop(overlays);
        context
            .dismiss(
                &mut harness.overlays.borrow_mut(),
                ContextMenuDismissal::Cancelled,
            )
            .unwrap();

        let keyboard = context
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                ContextMenuOpenRequest::keyboard(harness.anchor, [item(7, true)]),
            )
            .unwrap();
        assert_eq!(keyboard.source, ChangeSource::Keyboard);
        assert_eq!(
            keyboard.focus_request(),
            OverlayFocusRequest::Initial(OverlayInitialFocus::SelectedOrFirst)
        );
        assert_eq!(
            harness
                .overlays
                .borrow()
                .entry(keyboard.menu.overlay)
                .unwrap()
                .focus
                .restoration,
            OverlayFocusRestoration::TargetThenNearest(harness.anchor)
        );
    }

    #[test]
    fn programmatic_open_can_request_focus_without_platform_service() {
        let harness = harness();
        let mut context = ContextMenu::new();
        let opened = context
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                ContextMenuOpenRequest::programmatic(
                    OverlayAnchor::Rect(RectF {
                        x: 1.0,
                        y: 2.0,
                        width: 10.0,
                        height: 4.0,
                    }),
                    [item(7, true)],
                )
                .opening_focus(MenuOpeningFocus::SelectedOrFirst),
            )
            .unwrap();
        assert_eq!(opened.source, ChangeSource::Programmatic);
        assert_eq!(
            opened.focus_request(),
            OverlayFocusRequest::Initial(OverlayInitialFocus::SelectedOrFirst)
        );
        assert_eq!(harness.runtime.ui().nodes.alive().len(), 2);
    }

    #[test]
    fn escape_closes_one_level_while_outside_press_closes_and_consumes_chain() {
        let harness = harness();
        let mut context = ContextMenu::new();
        let root = context
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                ContextMenuOpenRequest::keyboard(harness.anchor, [item(7, true)]),
            )
            .unwrap();
        let child = context
            .controller_mut()
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                MenuOpenRequest::submenu(
                    root.menu.overlay,
                    OverlayAnchor::Node(harness.anchor),
                    [item(8, true)],
                ),
            )
            .unwrap();
        let escaped = context
            .dismiss(
                &mut harness.overlays.borrow_mut(),
                ContextMenuDismissal::Escape,
            )
            .unwrap();
        assert_eq!(escaped.dismissed.len(), 1);
        assert_eq!(escaped.dismissed[0].id, child.overlay);
        assert_eq!(
            context.controller().active_overlay(),
            Some(root.menu.overlay)
        );

        let outside = context
            .dismiss(
                &mut harness.overlays.borrow_mut(),
                ContextMenuDismissal::OutsidePress,
            )
            .unwrap();
        assert_eq!(outside.dismissed[0].id, root.menu.overlay);
        assert!(outside.consume_input);
        assert_eq!(
            outside.focus,
            OverlayFocusRequest::Restore {
                target: harness.anchor,
                nearest_fallback: true,
            }
        );
    }

    #[test]
    fn activation_rejects_disabled_then_closes_before_fresh_nonclone_action() {
        let harness = harness();
        let mut context = ContextMenu::new();
        context
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                ContextMenuOpenRequest::keyboard(harness.anchor, [item(7, true)]),
            )
            .unwrap();
        assert!(matches!(
            context.activate(
                &mut harness.overlays.borrow_mut(),
                harness.command.borrow().as_ref().unwrap(),
                ResolvedCommandState::new(false, Some(CheckState::Mixed)),
                ChangeSource::Accessibility,
            ),
            Err(ContextMenuError::Menu(
                MenuControllerError::DisabledCommand(7)
            ))
        ));
        assert_eq!(harness.calls.get(), 0);
        assert_eq!(context.controller().level_count(), 1);

        let intent = context
            .activate(
                &mut harness.overlays.borrow_mut(),
                harness.command.borrow().as_ref().unwrap(),
                ResolvedCommandState::new(true, Some(CheckState::Mixed)),
                ChangeSource::Accessibility,
            )
            .unwrap();
        assert_eq!(intent.checked(), Some(CheckState::Mixed));
        assert_eq!(intent.close_effect().dismissed.len(), 1);
        assert_eq!(context.controller().level_count(), 0);
        assert_eq!(harness.overlays.borrow().state().entry_count, 0);
        assert_eq!(
            intent.into_action(),
            NonCloneAction {
                source: ChangeSource::Accessibility,
                sequence: 1,
            }
        );
    }
}
