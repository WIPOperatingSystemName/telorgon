//! Typed edge-attached sheet policy over the application overlay and placement owners.

use std::fmt;

use crate::components::application::primitives::EnvironmentValues;
use crate::foundation::{RectF, SizeF};
use crate::input::WritingDirection;
use crate::ui::layout::{PopupOverflowPolicy, PopupPlacementAlignment, PopupPlacementCandidate};
use crate::ui::{
    MountedUi, OutsidePressPolicy, OverlayAnchor, OverlayDismissPolicy, OverlayFocusContainment,
    OverlayFocusLifecycle, OverlayFocusRequest, OverlayFocusRestoration, OverlayId,
    OverlayInitialFocus, OverlayModality, OverlayOpenRequest, OverlayOpened, SemanticRole,
    UiNodeId,
};

use super::placement::application_usable_bounds;
use super::{
    ApplicationOverlayCommand, ApplicationOverlayController, ApplicationOverlayControllerError,
    ApplicationOverlayEffect, ApplicationPopupPlacement, ApplicationPopupPlacementError,
    ApplicationPopupPlacementPolicy, ApplicationPopupPlacementRequest, place_application_popup,
};

/// Logical view edge to which a sheet is attached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SheetEdge {
    BlockStart,
    #[default]
    BlockEnd,
    InlineStart,
    InlineEnd,
}

impl SheetEdge {
    pub const fn resolve(self, direction: WritingDirection) -> ResolvedSheetEdge {
        match self {
            Self::BlockStart => ResolvedSheetEdge::Top,
            Self::BlockEnd => ResolvedSheetEdge::Bottom,
            Self::InlineStart => match direction {
                WritingDirection::LeftToRight => ResolvedSheetEdge::Left,
                WritingDirection::RightToLeft => ResolvedSheetEdge::Right,
            },
            Self::InlineEnd => match direction {
                WritingDirection::LeftToRight => ResolvedSheetEdge::Right,
                WritingDirection::RightToLeft => ResolvedSheetEdge::Left,
            },
        }
    }
}

/// Physical attachment edge reported after resolving the environment's writing direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResolvedSheetEdge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Non-optional initial focus for a modal sheet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SheetInitialFocus {
    #[default]
    FirstFocusable,
    Explicit(UiNodeId),
}

impl SheetInitialFocus {
    const fn overlay(self) -> OverlayInitialFocus {
        match self {
            Self::FirstFocusable => OverlayInitialFocus::FirstFocusable,
            Self::Explicit(target) => OverlayInitialFocus::Explicit(target),
        }
    }
}

/// Outside-press handling at a modal sheet barrier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SheetBarrierPolicy {
    #[default]
    BlockOutsidePress,
    DismissAndConsume,
}

impl SheetBarrierPolicy {
    const fn outside_press(self) -> OutsidePressPolicy {
        match self {
            Self::BlockOutsidePress => OutsidePressPolicy::Ignore,
            Self::DismissAndConsume => OutsidePressPolicy::DismissAndConsume,
        }
    }
}

/// Explicit modality, focus, and barrier policy for a sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SheetMode {
    NonModal,
    Modal {
        initial_focus: SheetInitialFocus,
        barrier: SheetBarrierPolicy,
    },
}

impl SheetMode {
    const fn modality(self) -> OverlayModality {
        match self {
            Self::NonModal => OverlayModality::NonModal,
            Self::Modal { .. } => OverlayModality::Modal,
        }
    }

    const fn initial_focus(self) -> OverlayInitialFocus {
        match self {
            Self::NonModal => OverlayInitialFocus::None,
            Self::Modal { initial_focus, .. } => initial_focus.overlay(),
        }
    }

    const fn containment(self) -> OverlayFocusContainment {
        match self {
            Self::NonModal => OverlayFocusContainment::None,
            Self::Modal { .. } => OverlayFocusContainment::Contain,
        }
    }

    const fn outside_press(self) -> OutsidePressPolicy {
        match self {
            Self::NonModal => OutsidePressPolicy::Ignore,
            Self::Modal { barrier, .. } => barrier.outside_press(),
        }
    }
}

/// Barrier/inert output for the separate input and semantics owners.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SheetBarrierIntent {
    pub background_inert: bool,
    pub policy: Option<SheetBarrierPolicy>,
}

/// Desired sheet content and the smallest permitted scroll viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetExtent {
    pub content_size: SizeF,
    pub minimum_viewport: SizeF,
}

impl SheetExtent {
    pub const fn new(content_size: SizeF, minimum_viewport: SizeF) -> Self {
        Self {
            content_size,
            minimum_viewport,
        }
    }
}

/// One reusable edge-attached sheet configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    accessible_name: String,
    opener: UiNodeId,
    edge: SheetEdge,
    extent: SheetExtent,
    mode: SheetMode,
    parent: Option<OverlayId>,
    escape_dismissal: bool,
}

impl Sheet {
    pub fn new(
        accessible_name: impl Into<String>,
        opener: UiNodeId,
        edge: SheetEdge,
        extent: SheetExtent,
        mode: SheetMode,
    ) -> Result<Self, SheetError> {
        let accessible_name = accessible_name.into();
        if accessible_name.trim().is_empty() {
            return Err(SheetError::MissingAccessibleName);
        }
        Ok(Self {
            accessible_name,
            opener,
            edge,
            extent,
            mode,
            parent: None,
            escape_dismissal: true,
        })
    }

    pub fn parent(mut self, parent: OverlayId) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn escape_dismissal(mut self, enabled: bool) -> Self {
        self.escape_dismissal = enabled;
        self
    }

    pub fn accessible_name(&self) -> &str {
        &self.accessible_name
    }

    pub const fn semantic_role(&self) -> SemanticRole {
        match self.mode {
            SheetMode::NonModal => SemanticRole::Generic,
            SheetMode::Modal { .. } => SemanticRole::Dialog,
        }
    }

    pub const fn opener(&self) -> UiNodeId {
        self.opener
    }

    pub const fn edge(&self) -> SheetEdge {
        self.edge
    }

    pub const fn extent(&self) -> SheetExtent {
        self.extent
    }

    pub const fn mode(&self) -> SheetMode {
        self.mode
    }

    pub const fn parent_overlay(&self) -> Option<OverlayId> {
        self.parent
    }

    pub const fn barrier(&self) -> SheetBarrierIntent {
        match self.mode {
            SheetMode::NonModal => SheetBarrierIntent {
                background_inert: false,
                policy: None,
            },
            SheetMode::Modal { barrier, .. } => SheetBarrierIntent {
                background_inert: true,
                policy: Some(barrier),
            },
        }
    }

    /// Places and opens the sheet without mounting content or applying returned effects.
    pub fn open(
        &self,
        controller: &mut ApplicationOverlayController,
        ui: &MountedUi,
        environment: &EnvironmentValues,
    ) -> Result<SheetOpened, SheetError> {
        let safe_bounds = application_usable_bounds(environment).map_err(SheetError::Placement)?;
        let (anchor, candidate) =
            edge_placement(self.edge, safe_bounds, environment.writing_direction);
        let placement_policy = ApplicationPopupPlacementPolicy::new(
            [candidate],
            PopupOverflowPolicy::Scroll {
                minimum_viewport: self.extent.minimum_viewport,
            },
        );
        let placement_request =
            ApplicationPopupPlacementRequest::new(anchor, self.extent.content_size, environment)
                .policy(placement_policy);
        let placement =
            place_application_popup(&placement_request).map_err(SheetError::Placement)?;

        let request = OverlayOpenRequest {
            anchor: OverlayAnchor::Node(self.opener),
            parent: self.parent,
            modality: self.mode.modality(),
            dismissal: OverlayDismissPolicy {
                escape: self.escape_dismissal,
                outside_press: self.mode.outside_press(),
                focus_lost: false,
                pointer_departure: false,
            },
            focus: OverlayFocusLifecycle {
                initial: self.mode.initial_focus(),
                containment: self.mode.containment(),
                restoration: OverlayFocusRestoration::TargetThenNearest(self.opener),
            },
        };
        let effect = controller
            .route(ApplicationOverlayCommand::Open { ui, request })
            .map_err(SheetError::Controller)?;
        let ApplicationOverlayEffect::Opened(overlay) = effect else {
            unreachable!("an overlay open command can only return an opened effect")
        };
        Ok(SheetOpened {
            overlay,
            placement,
            edge: self.edge.resolve(environment.writing_direction),
            mode: self.mode,
            barrier: self.barrier(),
        })
    }
}

fn edge_placement(
    edge: SheetEdge,
    safe: RectF,
    direction: WritingDirection,
) -> (RectF, PopupPlacementCandidate) {
    let (anchor, candidate) = match edge {
        SheetEdge::BlockStart => (
            RectF {
                x: safe.x,
                y: safe.y,
                width: safe.width,
                height: 0.0,
            },
            PopupPlacementCandidate::below(PopupPlacementAlignment::Start),
        ),
        SheetEdge::BlockEnd => (
            RectF {
                x: safe.x,
                y: safe.bottom(),
                width: safe.width,
                height: 0.0,
            },
            PopupPlacementCandidate::above(PopupPlacementAlignment::Start),
        ),
        SheetEdge::InlineStart => (
            RectF {
                x: match direction {
                    WritingDirection::LeftToRight => safe.x,
                    WritingDirection::RightToLeft => safe.right(),
                },
                y: safe.y,
                width: 0.0,
                height: safe.height,
            },
            PopupPlacementCandidate::inline_end(PopupPlacementAlignment::Start),
        ),
        SheetEdge::InlineEnd => (
            RectF {
                x: match direction {
                    WritingDirection::LeftToRight => safe.right(),
                    WritingDirection::RightToLeft => safe.x,
                },
                y: safe.y,
                width: 0.0,
                height: safe.height,
            },
            PopupPlacementCandidate::inline_start(PopupPlacementAlignment::Start),
        ),
    };
    (anchor, candidate)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetOpened {
    pub overlay: OverlayOpened,
    pub placement: ApplicationPopupPlacement,
    pub edge: ResolvedSheetEdge,
    pub mode: SheetMode,
    pub barrier: SheetBarrierIntent,
}

impl SheetOpened {
    pub const fn id(self) -> OverlayId {
        self.overlay.id
    }

    pub const fn focus_request(self) -> OverlayFocusRequest {
        self.overlay.focus
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetError {
    MissingAccessibleName,
    Placement(ApplicationPopupPlacementError),
    Controller(ApplicationOverlayControllerError),
}

impl fmt::Display for SheetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAccessibleName => formatter.write_str("sheet accessible name is empty"),
            Self::Placement(error) => error.fmt(formatter),
            Self::Controller(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SheetError {
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

    use crate::foundation::EdgeInsets;
    use crate::runtime::{
        Component, ComponentRuntimeDriver, CreateContext, Ui, UpdateContext, ViewRuntime,
    };
    use crate::ui::{BoxStyle, LayoutStyle, OverlayError, UiRoot};

    use crate::components::application::ApplicationOverlayHostError;

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
            safe_area: EdgeInsets {
                top: 10.0,
                right: 20.0,
                bottom: 30.0,
                left: 40.0,
            },
            ..EnvironmentValues::default()
        }
    }

    fn extent() -> SheetExtent {
        SheetExtent::new(
            SizeF {
                width: 180.0,
                height: 120.0,
            },
            SizeF {
                width: 100.0,
                height: 80.0,
            },
        )
    }

    fn nonmodal(harness: &Harness, edge: SheetEdge) -> Sheet {
        Sheet::new(
            "Inspector",
            harness.opener,
            edge,
            extent(),
            SheetMode::NonModal,
        )
        .unwrap()
    }

    #[test]
    fn construction_requires_a_name_and_resolves_logical_edges() {
        assert_eq!(
            Sheet::new(
                " ",
                UiNodeId::new(1, 1),
                SheetEdge::BlockEnd,
                extent(),
                SheetMode::NonModal,
            ),
            Err(SheetError::MissingAccessibleName)
        );
        assert_eq!(
            SheetEdge::InlineStart.resolve(WritingDirection::LeftToRight),
            ResolvedSheetEdge::Left
        );
        assert_eq!(
            SheetEdge::InlineStart.resolve(WritingDirection::RightToLeft),
            ResolvedSheetEdge::Right
        );
    }

    #[test]
    fn nonmodal_block_end_sheet_is_safe_attached_and_noninert() {
        let harness = harness();
        let opened = nonmodal(&harness, SheetEdge::BlockEnd)
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        assert_eq!(opened.edge, ResolvedSheetEdge::Bottom);
        assert_eq!(opened.placement.placement.rect.x, 40.0);
        assert_eq!(opened.placement.placement.rect.bottom(), 230.0);
        assert_eq!(opened.focus_request(), OverlayFocusRequest::None);
        assert_eq!(
            opened.barrier,
            SheetBarrierIntent {
                background_inert: false,
                policy: None,
            }
        );
        let controller = harness.controller.borrow();
        let entry = controller.entry(opened.id()).unwrap();
        assert_eq!(entry.modality, OverlayModality::NonModal);
        assert_eq!(entry.focus.containment, OverlayFocusContainment::None);
        assert_eq!(
            entry.focus.restoration,
            OverlayFocusRestoration::TargetThenNearest(harness.opener)
        );
        assert!(!controller.state().background_is_inert);
    }

    #[test]
    fn modal_rtl_inline_start_sheet_contains_focus_and_reports_barrier() {
        let harness = harness();
        let mut environment = environment();
        environment.writing_direction = WritingDirection::RightToLeft;
        let sheet = Sheet::new(
            "Account",
            harness.opener,
            SheetEdge::InlineStart,
            extent(),
            SheetMode::Modal {
                initial_focus: SheetInitialFocus::Explicit(harness.explicit_focus),
                barrier: SheetBarrierPolicy::DismissAndConsume,
            },
        )
        .unwrap();
        let opened = sheet
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment,
            )
            .unwrap();

        assert_eq!(opened.edge, ResolvedSheetEdge::Right);
        assert_eq!(opened.placement.placement.rect.right(), 380.0);
        assert_eq!(
            opened.focus_request(),
            OverlayFocusRequest::Initial(OverlayInitialFocus::Explicit(harness.explicit_focus))
        );
        assert_eq!(
            opened.barrier,
            SheetBarrierIntent {
                background_inert: true,
                policy: Some(SheetBarrierPolicy::DismissAndConsume),
            }
        );
        let controller = harness.controller.borrow();
        let entry = controller.entry(opened.id()).unwrap();
        assert_eq!(entry.modality, OverlayModality::Modal);
        assert_eq!(entry.focus.containment, OverlayFocusContainment::Contain);
        assert_eq!(
            entry.dismissal.outside_press,
            OutsidePressPolicy::DismissAndConsume
        );
        assert!(controller.state().background_is_inert);
    }

    #[test]
    fn oversized_sheet_returns_a_constrained_scroll_viewport_without_mounting() {
        let harness = harness();
        let before = harness.runtime.ui().nodes.alive().len();
        let oversized = Sheet::new(
            "Layers",
            harness.opener,
            SheetEdge::InlineEnd,
            SheetExtent::new(
                SizeF {
                    width: 600.0,
                    height: 500.0,
                },
                SizeF {
                    width: 120.0,
                    height: 90.0,
                },
            ),
            SheetMode::NonModal,
        )
        .unwrap();
        let opened = oversized
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        assert!(opened.placement.requires_scroll());
        assert_eq!(
            opened.placement.placement.rect,
            RectF {
                x: 40.0,
                y: 10.0,
                width: 340.0,
                height: 220.0,
            }
        );
        assert_eq!(harness.runtime.ui().nodes.alive().len(), before);
    }

    #[test]
    fn placement_and_focus_rejection_are_atomic() {
        let harness = harness();
        let invalid_extent = Sheet::new(
            "Invalid extent",
            harness.opener,
            SheetEdge::BlockStart,
            SheetExtent::new(
                SizeF {
                    width: 80.0,
                    height: 60.0,
                },
                SizeF {
                    width: 100.0,
                    height: 80.0,
                },
            ),
            SheetMode::NonModal,
        )
        .unwrap();
        assert!(matches!(
            invalid_extent.open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            ),
            Err(SheetError::Placement(_))
        ));
        assert_eq!(harness.controller.borrow().state().entry_count, 0);

        let unknown = UiNodeId::new(u32::MAX, u32::MAX);
        let invalid_focus = Sheet::new(
            "Invalid focus",
            harness.opener,
            SheetEdge::BlockEnd,
            extent(),
            SheetMode::Modal {
                initial_focus: SheetInitialFocus::Explicit(unknown),
                barrier: SheetBarrierPolicy::BlockOutsidePress,
            },
        )
        .unwrap();
        assert_eq!(
            invalid_focus.open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            ),
            Err(SheetError::Controller(
                ApplicationOverlayControllerError::Host(ApplicationOverlayHostError::Lifecycle(
                    OverlayError::UnknownFocusTarget(unknown)
                ))
            ))
        );
        assert_eq!(harness.controller.borrow().state().entry_count, 0);
    }

    #[test]
    fn escape_parentage_and_default_modal_barrier_are_explicit() {
        let harness = harness();
        let parent = nonmodal(&harness, SheetEdge::InlineEnd)
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();
        let child = Sheet::new(
            "Child",
            harness.opener,
            SheetEdge::BlockStart,
            extent(),
            SheetMode::Modal {
                initial_focus: SheetInitialFocus::FirstFocusable,
                barrier: SheetBarrierPolicy::BlockOutsidePress,
            },
        )
        .unwrap()
        .parent(parent.id())
        .escape_dismissal(false);
        let opened = child
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        assert_eq!(
            opened.barrier.policy,
            Some(SheetBarrierPolicy::BlockOutsidePress)
        );
        let controller = harness.controller.borrow();
        let entry = controller.entry(opened.id()).unwrap();
        assert_eq!(entry.parent, Some(parent.id()));
        assert!(!entry.dismissal.escape);
        assert_eq!(entry.dismissal.outside_press, OutsidePressPolicy::Ignore);
    }
}
