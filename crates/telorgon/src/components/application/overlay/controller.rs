//! Typed application command/effect boundary over the mounted overlay host.

use std::fmt;

use crate::runtime::Ui;
use crate::ui::{
    DismissReason, MountedUi, OverlayCloseOutcome, OverlayDiagnostics, OverlayDismissResult,
    OverlayEntry, OverlayId, OverlayOpenRequest, OverlayOpened, UiNodeId,
};

use super::{ApplicationOverlayHost, ApplicationOverlayHostError, ApplicationOverlayHostRef};

/// One application overlay transition request.
///
/// Only opening carries a mounted-UI reference because only that transition validates anchors,
/// focus targets, and the host's exact mounted generation.
pub enum ApplicationOverlayCommand<'ui> {
    Open {
        ui: &'ui MountedUi,
        request: OverlayOpenRequest,
    },
    Dismiss {
        id: OverlayId,
        reason: DismissReason,
    },
    AnchorRemoved {
        anchor: UiNodeId,
    },
    ViewLost,
    OwnerUnmounted,
}

impl fmt::Debug for ApplicationOverlayCommand<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { request, .. } => formatter
                .debug_struct("Open")
                .field("ui", &"<mounted-ui>")
                .field("request", request)
                .finish(),
            Self::Dismiss { id, reason } => formatter
                .debug_struct("Dismiss")
                .field("id", id)
                .field("reason", reason)
                .finish(),
            Self::AnchorRemoved { anchor } => formatter
                .debug_struct("AnchorRemoved")
                .field("anchor", anchor)
                .finish(),
            Self::ViewLost => formatter.write_str("ViewLost"),
            Self::OwnerUnmounted => formatter.write_str("OwnerUnmounted"),
        }
    }
}

/// Typed effects returned for their separate portal, focus, semantics, and input owners.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationOverlayEffect {
    Opened(OverlayOpened),
    Dismissal(OverlayDismissResult),
    AnchorsRemoved {
        anchor: UiNodeId,
        outcomes: Vec<OverlayCloseOutcome>,
    },
    ViewLost(OverlayCloseOutcome),
    OwnerUnmounted(OverlayCloseOutcome),
}

/// Content-derived controller facts without a second overlay-state copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplicationOverlayControllerState {
    pub mounted: bool,
    pub entry_count: usize,
    pub top: Option<OverlayId>,
    pub active_modal: Option<OverlayId>,
    pub background_is_inert: bool,
}

/// One application command owner around one mounted host and its neutral lifecycle engine.
#[derive(Default)]
pub struct ApplicationOverlayController {
    host: ApplicationOverlayHost,
}

impl fmt::Debug for ApplicationOverlayController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApplicationOverlayController")
            .field("state", &self.state())
            .field("diagnostics", &self.host.diagnostics())
            .finish()
    }
}

impl ApplicationOverlayController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_host(host: ApplicationOverlayHost) -> Self {
        Self { host }
    }

    pub fn into_host(self) -> ApplicationOverlayHost {
        self.host
    }

    /// Delegates the one-per-instance visual mount association to the host owner.
    pub fn mount<Action: 'static>(
        &mut self,
        ui: &mut Ui<'_, '_, Action>,
        parent: UiNodeId,
    ) -> Result<ApplicationOverlayHostRef, ApplicationOverlayControllerError> {
        self.host
            .mount(ui, parent)
            .map_err(ApplicationOverlayControllerError::Host)
    }

    pub fn mounted(&self) -> Option<ApplicationOverlayHostRef> {
        self.host.mounted()
    }

    pub fn state(&self) -> ApplicationOverlayControllerState {
        ApplicationOverlayControllerState {
            mounted: self.host.mounted().is_some(),
            entry_count: self.host.entries().len(),
            top: self.host.top().map(|entry| entry.id),
            active_modal: self.host.active_modal(),
            background_is_inert: self.host.background_is_inert(),
        }
    }

    pub fn entries(&self) -> &[OverlayEntry] {
        self.host.entries()
    }

    pub fn entry(&self, id: OverlayId) -> Option<&OverlayEntry> {
        self.host.entry(id)
    }

    pub fn diagnostics(&self) -> OverlayDiagnostics {
        self.host.diagnostics()
    }

    /// Applies exactly one typed transition without executing any returned effect inline.
    pub fn route(
        &mut self,
        command: ApplicationOverlayCommand<'_>,
    ) -> Result<ApplicationOverlayEffect, ApplicationOverlayControllerError> {
        match command {
            ApplicationOverlayCommand::Open { ui, request } => self
                .host
                .open(ui, request)
                .map(ApplicationOverlayEffect::Opened)
                .map_err(ApplicationOverlayControllerError::Host),
            ApplicationOverlayCommand::Dismiss { id, reason } => self
                .host
                .dismiss(id, reason)
                .map(ApplicationOverlayEffect::Dismissal)
                .map_err(ApplicationOverlayControllerError::Host),
            ApplicationOverlayCommand::AnchorRemoved { anchor } => {
                Ok(ApplicationOverlayEffect::AnchorsRemoved {
                    anchor,
                    outcomes: self.host.anchor_removed(anchor),
                })
            }
            ApplicationOverlayCommand::ViewLost => {
                Ok(ApplicationOverlayEffect::ViewLost(self.host.view_lost()))
            }
            ApplicationOverlayCommand::OwnerUnmounted => Ok(
                ApplicationOverlayEffect::OwnerUnmounted(self.host.unmount()),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationOverlayControllerError {
    Host(ApplicationOverlayHostError),
}

impl fmt::Display for ApplicationOverlayControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ApplicationOverlayControllerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Host(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::runtime::{
        Component, ComponentRuntimeDriver, CreateContext, UpdateContext, ViewRuntime,
    };
    use crate::ui::{
        BoxStyle, LayoutStyle, OutsidePressPolicy, OverlayAnchor, OverlayDismissPolicy,
        OverlayFocusContainment, OverlayFocusLifecycle, OverlayFocusRequest,
        OverlayFocusRestoration, OverlayInitialFocus, OverlayModality, UiRoot,
    };

    use super::*;

    struct MountedController {
        controller: Rc<RefCell<ApplicationOverlayController>>,
        anchor: Rc<Cell<Option<UiNodeId>>>,
    }

    impl Component for MountedController {
        type State = ();
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.controller.borrow_mut().mount(ui, root.0).unwrap();
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

    struct ControllerHarness {
        runtime: ViewRuntime<ComponentRuntimeDriver<MountedController>>,
        controller: Rc<RefCell<ApplicationOverlayController>>,
        anchor: UiNodeId,
    }

    fn mounted_controller() -> ControllerHarness {
        let controller = Rc::new(RefCell::new(ApplicationOverlayController::new()));
        let anchor = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(MountedController {
            controller: controller.clone(),
            anchor: anchor.clone(),
        })
        .unwrap();
        ControllerHarness {
            runtime,
            controller,
            anchor: anchor.get().unwrap(),
        }
    }

    fn popup(anchor: UiNodeId) -> OverlayOpenRequest {
        OverlayOpenRequest {
            anchor: OverlayAnchor::Node(anchor),
            parent: None,
            modality: OverlayModality::NonModal,
            dismissal: OverlayDismissPolicy {
                escape: true,
                outside_press: OutsidePressPolicy::Ignore,
                focus_lost: false,
                pointer_departure: false,
            },
            focus: OverlayFocusLifecycle {
                initial: OverlayInitialFocus::FirstFocusable,
                containment: OverlayFocusContainment::None,
                restoration: OverlayFocusRestoration::TargetThenNearest(anchor),
            },
        }
    }

    fn open(harness: &ControllerHarness, request: OverlayOpenRequest) -> OverlayOpened {
        let ApplicationOverlayEffect::Opened(opened) = harness
            .controller
            .borrow_mut()
            .route(ApplicationOverlayCommand::Open {
                ui: harness.runtime.ui(),
                request,
            })
            .unwrap()
        else {
            panic!("open command must return an opened effect")
        };
        opened
    }

    #[test]
    fn open_effect_and_state_are_derived_from_the_single_host_owner() {
        let harness = mounted_controller();
        let opened = open(&harness, popup(harness.anchor));
        assert_eq!(
            opened.focus,
            OverlayFocusRequest::Initial(OverlayInitialFocus::FirstFocusable)
        );
        assert_eq!(
            harness.controller.borrow().state(),
            ApplicationOverlayControllerState {
                mounted: true,
                entry_count: 1,
                top: Some(opened.id),
                active_modal: None,
                background_is_inert: false,
            }
        );
        assert_eq!(harness.controller.borrow().entries()[0].id, opened.id);
    }

    #[test]
    fn blocked_dismissal_is_a_typed_nonmutating_effect() {
        let harness = mounted_controller();
        let opened = open(&harness, popup(harness.anchor));
        let effect = harness
            .controller
            .borrow_mut()
            .route(ApplicationOverlayCommand::Dismiss {
                id: opened.id,
                reason: DismissReason::OutsidePress,
            })
            .unwrap();
        assert_eq!(
            effect,
            ApplicationOverlayEffect::Dismissal(OverlayDismissResult::Blocked {
                id: opened.id,
                reason: DismissReason::OutsidePress,
            })
        );
        assert_eq!(harness.controller.borrow().state().entry_count, 1);
        assert_eq!(
            harness.controller.borrow().diagnostics().blocked_dismissals,
            1
        );
    }

    #[test]
    fn anchor_removal_preserves_top_first_subtree_outcomes() {
        let harness = mounted_controller();
        let parent = open(&harness, popup(harness.anchor)).id;
        let mut child = popup(harness.anchor);
        child.parent = Some(parent);
        let child = open(&harness, child).id;
        let effect = harness
            .controller
            .borrow_mut()
            .route(ApplicationOverlayCommand::AnchorRemoved {
                anchor: harness.anchor,
            })
            .unwrap();
        let ApplicationOverlayEffect::AnchorsRemoved { anchor, outcomes } = effect else {
            panic!("anchor removal must return close outcomes")
        };
        assert_eq!(anchor, harness.anchor);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0]
                .dismissed
                .iter()
                .map(|dismissed| dismissed.id)
                .collect::<Vec<_>>(),
            vec![child, parent]
        );
        assert_eq!(harness.controller.borrow().state().entry_count, 0);
    }

    #[test]
    fn view_loss_retains_mount_but_owner_unmount_releases_it() {
        let harness = mounted_controller();
        let first = open(&harness, popup(harness.anchor)).id;
        let view_lost = harness
            .controller
            .borrow_mut()
            .route(ApplicationOverlayCommand::ViewLost)
            .unwrap();
        let ApplicationOverlayEffect::ViewLost(outcome) = view_lost else {
            panic!("view loss must return its close outcome")
        };
        assert_eq!(outcome.dismissed[0].id, first);
        assert_eq!(outcome.dismissed[0].reason, DismissReason::ViewLost);
        assert!(harness.controller.borrow().state().mounted);

        let second = open(&harness, popup(harness.anchor)).id;
        let unmounted = harness
            .controller
            .borrow_mut()
            .route(ApplicationOverlayCommand::OwnerUnmounted)
            .unwrap();
        let ApplicationOverlayEffect::OwnerUnmounted(outcome) = unmounted else {
            panic!("owner unmount must return its close outcome")
        };
        assert_eq!(outcome.dismissed[0].id, second);
        assert_eq!(outcome.dismissed[0].reason, DismissReason::OwnerUnmounted);
        assert_eq!(
            harness.controller.borrow().state(),
            ApplicationOverlayControllerState::default()
        );
        assert!(matches!(
            harness
                .controller
                .borrow_mut()
                .route(ApplicationOverlayCommand::Open {
                    ui: harness.runtime.ui(),
                    request: popup(harness.anchor),
                }),
            Err(ApplicationOverlayControllerError::Host(
                ApplicationOverlayHostError::NotMounted
            ))
        ));
    }

    #[test]
    fn open_command_debug_omits_the_mounted_ui_contents() {
        let harness = mounted_controller();
        let debug = format!(
            "{:?}",
            ApplicationOverlayCommand::Open {
                ui: harness.runtime.ui(),
                request: popup(harness.anchor),
            }
        );
        assert!(debug.contains("<mounted-ui>"));
        assert!(!debug.contains("MountedUi"));
    }
}
