//! Typed modal dialog policy over the application overlay and placement owners.

use std::fmt;

use crate::components::application::primitives::EnvironmentValues;
use crate::foundation::{RectF, SizeF};
use crate::ui::{
    MountedUi, OutsidePressPolicy, OverlayAnchor, OverlayDismissPolicy, OverlayFocusContainment,
    OverlayFocusLifecycle, OverlayFocusRequest, OverlayFocusRestoration, OverlayId,
    OverlayInitialFocus, OverlayModality, OverlayOpenRequest, OverlayOpened, SemanticRole,
    UiNodeId,
};

use super::{
    ApplicationOverlayCommand, ApplicationOverlayController, ApplicationOverlayControllerError,
    ApplicationOverlayEffect, ApplicationPopupPlacement, ApplicationPopupPlacementError,
    ApplicationPopupPlacementPolicy, ApplicationPopupPlacementRequest, place_application_popup,
};

/// Application meaning that can affect destructive dismissal policy and later presentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DialogKind {
    #[default]
    Standard,
    Destructive,
    Critical,
}

/// Non-optional initial focus for a modal dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DialogInitialFocus {
    #[default]
    FirstFocusable,
    Explicit(UiNodeId),
}

impl DialogInitialFocus {
    const fn overlay(self) -> OverlayInitialFocus {
        match self {
            Self::FirstFocusable => OverlayInitialFocus::FirstFocusable,
            Self::Explicit(target) => OverlayInitialFocus::Explicit(target),
        }
    }
}

/// Barrier handling returned for the separate input and semantics owners.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DialogBarrierPolicy {
    /// Keep the modal open. The modal inert boundary still prevents lower-content activation.
    #[default]
    BlockOutsidePress,
    /// Close and consume before ordinary routing can act on the same press.
    DismissAndConsume,
}

impl DialogBarrierPolicy {
    const fn outside_press(self) -> OutsidePressPolicy {
        match self {
            Self::BlockOutsidePress => OutsidePressPolicy::Ignore,
            Self::DismissAndConsume => OutsidePressPolicy::DismissAndConsume,
        }
    }
}

/// Explicit modal barrier/inert output; this package does not apply it inline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DialogBarrierIntent {
    pub background_inert: bool,
    pub policy: DialogBarrierPolicy,
}

/// One reusable modal dialog configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    accessible_name: String,
    opener: UiNodeId,
    placement_anchor: RectF,
    content_size: SizeF,
    initial_focus: DialogInitialFocus,
    kind: DialogKind,
    parent: Option<OverlayId>,
    escape_dismissal: bool,
    barrier: DialogBarrierPolicy,
    placement: ApplicationPopupPlacementPolicy,
}

impl Dialog {
    pub fn new(
        accessible_name: impl Into<String>,
        opener: UiNodeId,
        placement_anchor: RectF,
        content_size: SizeF,
        initial_focus: DialogInitialFocus,
    ) -> Result<Self, DialogError> {
        let accessible_name = accessible_name.into();
        if accessible_name.trim().is_empty() {
            return Err(DialogError::MissingAccessibleName);
        }
        Ok(Self {
            accessible_name,
            opener,
            placement_anchor,
            content_size,
            initial_focus,
            kind: DialogKind::Standard,
            parent: None,
            escape_dismissal: true,
            barrier: DialogBarrierPolicy::BlockOutsidePress,
            placement: ApplicationPopupPlacementPolicy::default(),
        })
    }

    pub fn kind(mut self, kind: DialogKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn parent(mut self, parent: OverlayId) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn escape_dismissal(mut self, enabled: bool) -> Self {
        self.escape_dismissal = enabled;
        self
    }

    /// Explicit opt-in; destructive and critical kinds never enable this implicitly.
    pub fn barrier_policy(mut self, policy: DialogBarrierPolicy) -> Self {
        self.barrier = policy;
        self
    }

    pub fn placement_policy(mut self, placement: ApplicationPopupPlacementPolicy) -> Self {
        self.placement = placement;
        self
    }

    pub fn accessible_name(&self) -> &str {
        &self.accessible_name
    }

    pub const fn semantic_role(&self) -> SemanticRole {
        SemanticRole::Dialog
    }

    pub const fn kind_value(&self) -> DialogKind {
        self.kind
    }

    pub const fn opener(&self) -> UiNodeId {
        self.opener
    }

    pub const fn placement_anchor(&self) -> RectF {
        self.placement_anchor
    }

    pub const fn content_size(&self) -> SizeF {
        self.content_size
    }

    pub const fn initial_focus(&self) -> DialogInitialFocus {
        self.initial_focus
    }

    pub const fn parent_overlay(&self) -> Option<OverlayId> {
        self.parent
    }

    pub const fn barrier(&self) -> DialogBarrierIntent {
        DialogBarrierIntent {
            background_inert: true,
            policy: self.barrier,
        }
    }

    pub fn placement(&self) -> &ApplicationPopupPlacementPolicy {
        &self.placement
    }

    /// Places and opens the dialog without mounting content or applying returned effects.
    pub fn open(
        &self,
        controller: &mut ApplicationOverlayController,
        ui: &MountedUi,
        environment: &EnvironmentValues,
    ) -> Result<DialogOpened, DialogError> {
        let placement_request = ApplicationPopupPlacementRequest::new(
            self.placement_anchor,
            self.content_size,
            environment,
        )
        .policy(self.placement.clone());
        let placement =
            place_application_popup(&placement_request).map_err(DialogError::Placement)?;

        let request = OverlayOpenRequest {
            anchor: OverlayAnchor::Node(self.opener),
            parent: self.parent,
            modality: OverlayModality::Modal,
            dismissal: OverlayDismissPolicy {
                escape: self.escape_dismissal,
                outside_press: self.barrier.outside_press(),
                focus_lost: false,
                pointer_departure: false,
            },
            focus: OverlayFocusLifecycle {
                initial: self.initial_focus.overlay(),
                containment: OverlayFocusContainment::Contain,
                restoration: OverlayFocusRestoration::TargetThenNearest(self.opener),
            },
        };
        let effect = controller
            .route(ApplicationOverlayCommand::Open { ui, request })
            .map_err(DialogError::Controller)?;
        let ApplicationOverlayEffect::Opened(overlay) = effect else {
            unreachable!("an overlay open command can only return an opened effect")
        };
        Ok(DialogOpened {
            overlay,
            placement,
            kind: self.kind,
            barrier: self.barrier(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogOpened {
    pub overlay: OverlayOpened,
    pub placement: ApplicationPopupPlacement,
    pub kind: DialogKind,
    pub barrier: DialogBarrierIntent,
}

impl DialogOpened {
    pub const fn id(self) -> OverlayId {
        self.overlay.id
    }

    pub const fn focus_request(self) -> OverlayFocusRequest {
        self.overlay.focus
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogError {
    MissingAccessibleName,
    Placement(ApplicationPopupPlacementError),
    Controller(ApplicationOverlayControllerError),
}

impl fmt::Display for DialogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAccessibleName => formatter.write_str("dialog accessible name is empty"),
            Self::Placement(error) => error.fmt(formatter),
            Self::Controller(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DialogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Placement(error) => Some(error),
            Self::Controller(error) => Some(error),
            Self::MissingAccessibleName => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::ui::layout::{PopupOverflowPolicy, PopupPlacementAlignment, PopupPlacementCandidate};
    use crate::runtime::{
        Component, ComponentRuntimeDriver, CreateContext, Ui, UpdateContext, ViewRuntime,
    };
    use crate::ui::{BoxStyle, LayoutStyle, OverlayError, UiRoot};

    use crate::components::application::{ApplicationOverlayHostError, Popup, PopupAnchor};

    use super::*;

    struct MountedController {
        controller: Rc<RefCell<ApplicationOverlayController>>,
        nodes: Rc<RefCell<Vec<UiNodeId>>>,
    }

    impl Component for MountedController {
        type State = ();
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let nodes = self.nodes.clone();
            let root =
                ui.foundation()
                    .root(BoxStyle::default(), LayoutStyle::default(), move |writer| {
                        nodes.borrow_mut().push(writer.container(
                            BoxStyle::default(),
                            LayoutStyle::default(),
                            |_| {},
                        ));
                    });
            self.controller.borrow_mut().mount(ui, root.0).unwrap();
            self.nodes.borrow_mut().insert(0, root.0);
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
        runtime: ViewRuntime<ComponentRuntimeDriver<MountedController>>,
        controller: Rc<RefCell<ApplicationOverlayController>>,
        opener: UiNodeId,
        explicit_focus: UiNodeId,
    }

    fn harness() -> Harness {
        let controller = Rc::new(RefCell::new(ApplicationOverlayController::new()));
        let nodes = Rc::new(RefCell::new(Vec::new()));
        let runtime = ViewRuntime::from_component(MountedController {
            controller: controller.clone(),
            nodes: nodes.clone(),
        })
        .unwrap();
        let nodes = nodes.borrow();
        Harness {
            runtime,
            controller,
            opener: nodes[0],
            explicit_focus: nodes[1],
        }
    }

    fn environment() -> EnvironmentValues {
        EnvironmentValues {
            available_size: SizeF {
                width: 400.0,
                height: 260.0,
            },
            ..EnvironmentValues::default()
        }
    }

    fn anchor() -> RectF {
        RectF {
            x: 150.0,
            y: 60.0,
            width: 100.0,
            height: 20.0,
        }
    }

    fn size() -> SizeF {
        SizeF {
            width: 180.0,
            height: 120.0,
        }
    }

    fn dialog(harness: &Harness) -> Dialog {
        Dialog::new(
            "Confirm settings",
            harness.opener,
            anchor(),
            size(),
            DialogInitialFocus::FirstFocusable,
        )
        .unwrap()
    }

    #[test]
    fn accessible_name_and_nonoptional_focus_are_fixed_at_construction() {
        assert_eq!(
            Dialog::new(
                "  ",
                UiNodeId::new(1, 1),
                anchor(),
                size(),
                DialogInitialFocus::FirstFocusable,
            ),
            Err(DialogError::MissingAccessibleName)
        );
        let harness = harness();
        let dialog = dialog(&harness);
        assert_eq!(dialog.accessible_name(), "Confirm settings");
        assert_eq!(dialog.semantic_role(), SemanticRole::Dialog);
        assert_eq!(dialog.initial_focus(), DialogInitialFocus::FirstFocusable);
    }

    #[test]
    fn standard_dialog_opens_modal_contained_and_restoring() {
        let harness = harness();
        let opened = dialog(&harness)
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        assert_eq!(
            opened.focus_request(),
            OverlayFocusRequest::Initial(OverlayInitialFocus::FirstFocusable)
        );
        assert_eq!(
            opened.barrier,
            DialogBarrierIntent {
                background_inert: true,
                policy: DialogBarrierPolicy::BlockOutsidePress,
            }
        );
        let controller = harness.controller.borrow();
        let entry = controller.entry(opened.id()).unwrap();
        assert_eq!(entry.modality, OverlayModality::Modal);
        assert_eq!(entry.focus.containment, OverlayFocusContainment::Contain);
        assert_eq!(
            entry.focus.restoration,
            OverlayFocusRestoration::TargetThenNearest(harness.opener)
        );
        assert_eq!(entry.dismissal.outside_press, OutsidePressPolicy::Ignore);
        assert!(controller.state().background_is_inert);
        assert_eq!(controller.state().active_modal, Some(opened.id()));
    }

    #[test]
    fn destructive_outside_dismissal_requires_explicit_opt_in() {
        let harness = harness();
        let destructive = dialog(&harness).kind(DialogKind::Destructive);
        assert_eq!(
            destructive.barrier().policy,
            DialogBarrierPolicy::BlockOutsidePress
        );

        let opened = destructive
            .barrier_policy(DialogBarrierPolicy::DismissAndConsume)
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();
        assert_eq!(opened.kind, DialogKind::Destructive);
        assert_eq!(
            opened.barrier.policy,
            DialogBarrierPolicy::DismissAndConsume
        );
        assert_eq!(
            harness
                .controller
                .borrow()
                .entry(opened.id())
                .unwrap()
                .dismissal
                .outside_press,
            OutsidePressPolicy::DismissAndConsume
        );
    }

    #[test]
    fn explicit_focus_and_parentage_are_preserved_without_second_state() {
        let harness = harness();
        let popup = Popup::new(
            PopupAnchor::node(harness.opener, anchor()),
            SizeF {
                width: 80.0,
                height: 40.0,
            },
        )
        .open(
            &mut harness.controller.borrow_mut(),
            harness.runtime.ui(),
            &environment(),
        )
        .unwrap();
        let dialog = Dialog::new(
            "Nested dialog",
            harness.opener,
            anchor(),
            size(),
            DialogInitialFocus::Explicit(harness.explicit_focus),
        )
        .unwrap()
        .parent(popup.id());
        let opened = dialog
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        assert_eq!(
            opened.focus_request(),
            OverlayFocusRequest::Initial(OverlayInitialFocus::Explicit(harness.explicit_focus))
        );
        assert_eq!(
            harness
                .controller
                .borrow()
                .entry(opened.id())
                .unwrap()
                .parent,
            Some(popup.id())
        );
    }

    #[test]
    fn placement_and_focus_rejection_are_atomic() {
        let harness = harness();
        let no_fit = dialog(&harness).placement_policy(ApplicationPopupPlacementPolicy::new(
            [PopupPlacementCandidate::below(
                PopupPlacementAlignment::Start,
            )],
            PopupOverflowPolicy::Reject,
        ));
        let mut too_large = no_fit;
        too_large.content_size = SizeF {
            width: 600.0,
            height: 500.0,
        };
        assert!(matches!(
            too_large.open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            ),
            Err(DialogError::Placement(_))
        ));
        assert_eq!(harness.controller.borrow().state().entry_count, 0);

        let unknown = UiNodeId::new(u32::MAX, u32::MAX);
        let invalid_focus = Dialog::new(
            "Invalid focus",
            harness.opener,
            anchor(),
            size(),
            DialogInitialFocus::Explicit(unknown),
        )
        .unwrap();
        assert_eq!(
            invalid_focus.open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            ),
            Err(DialogError::Controller(
                ApplicationOverlayControllerError::Host(ApplicationOverlayHostError::Lifecycle(
                    OverlayError::UnknownFocusTarget(unknown)
                ))
            ))
        );
        assert_eq!(harness.controller.borrow().state().entry_count, 0);
    }

    #[test]
    fn scroll_constrained_dialog_reports_safe_geometry_without_mounting_content() {
        let harness = harness();
        let before = harness.runtime.ui().nodes.alive().len();
        let dialog = dialog(&harness).placement_policy(ApplicationPopupPlacementPolicy::new(
            [PopupPlacementCandidate::below(
                PopupPlacementAlignment::Center,
            )],
            PopupOverflowPolicy::Scroll {
                minimum_viewport: SizeF {
                    width: 120.0,
                    height: 80.0,
                },
            },
        ));
        let mut oversized = dialog;
        oversized.content_size = SizeF {
            width: 500.0,
            height: 400.0,
        };
        let opened = oversized
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        assert!(opened.placement.requires_scroll());
        assert_eq!(opened.placement.placement.rect.width, 400.0);
        assert_eq!(opened.placement.placement.rect.height, 260.0);
        assert_eq!(harness.runtime.ui().nodes.alive().len(), before);
    }
}
