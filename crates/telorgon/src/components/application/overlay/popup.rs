//! Typed nonmodal popup configuration over the application overlay and placement owners.

use std::fmt;

use crate::foundation::{PointF, RectF, SizeF};
use crate::ui::{
    MountedUi, OutsidePressPolicy, OverlayAnchor, OverlayDismissPolicy, OverlayFocusContainment,
    OverlayFocusLifecycle, OverlayFocusRequest, OverlayFocusRestoration, OverlayId,
    OverlayInitialFocus, OverlayModality, OverlayOpenRequest, OverlayOpened, UiNodeId,
};

use crate::components::application::primitives::EnvironmentValues;

use super::{
    ApplicationOverlayCommand, ApplicationOverlayController, ApplicationOverlayControllerError,
    ApplicationOverlayEffect, ApplicationPopupPlacement, ApplicationPopupPlacementError,
    ApplicationPopupPlacementPolicy, ApplicationPopupPlacementRequest, place_application_popup,
};

/// Lifecycle anchor paired with the resolved logical geometry used by placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PopupAnchor {
    Node { node: UiNodeId, bounds: RectF },
    Point(PointF),
    Rect(RectF),
}

impl PopupAnchor {
    pub const fn node(node: UiNodeId, bounds: RectF) -> Self {
        Self::Node { node, bounds }
    }

    pub const fn point(point: PointF) -> Self {
        Self::Point(point)
    }

    pub const fn rect(rect: RectF) -> Self {
        Self::Rect(rect)
    }

    const fn lifecycle_anchor(self) -> OverlayAnchor {
        match self {
            Self::Node { node, .. } => OverlayAnchor::Node(node),
            Self::Point(point) => OverlayAnchor::Point(point),
            Self::Rect(rect) => OverlayAnchor::Rect(rect),
        }
    }

    const fn placement_bounds(self) -> RectF {
        match self {
            Self::Node { bounds, .. } | Self::Rect(bounds) => bounds,
            Self::Point(point) => RectF {
                x: point.x,
                y: point.y,
                width: 0.0,
                height: 0.0,
            },
        }
    }

    const fn restoration(self) -> OverlayFocusRestoration {
        match self {
            Self::Node { node, .. } => OverlayFocusRestoration::TargetThenNearest(node),
            Self::Point(_) | Self::Rect(_) => OverlayFocusRestoration::None,
        }
    }
}

/// One reusable nonmodal popup configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct Popup {
    anchor: PopupAnchor,
    content_size: SizeF,
    parent: Option<OverlayId>,
    dismissal: OverlayDismissPolicy,
    focus: OverlayFocusLifecycle,
    placement: ApplicationPopupPlacementPolicy,
}

impl Popup {
    /// Creates an ordinary popup that closes on Escape, outside press, or focus loss.
    ///
    /// Outside press is consumed so the same input cannot close and immediately reopen the popup.
    /// Node anchors restore focus to that node, with nearest-live fallback, when the popup closes.
    pub fn new(anchor: PopupAnchor, content_size: SizeF) -> Self {
        Self {
            anchor,
            content_size,
            parent: None,
            dismissal: OverlayDismissPolicy {
                escape: true,
                outside_press: OutsidePressPolicy::DismissAndConsume,
                focus_lost: true,
                pointer_departure: false,
            },
            focus: OverlayFocusLifecycle {
                initial: OverlayInitialFocus::None,
                containment: OverlayFocusContainment::None,
                restoration: anchor.restoration(),
            },
            placement: ApplicationPopupPlacementPolicy::default(),
        }
    }

    pub fn parent(mut self, parent: OverlayId) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn dismissal_policy(mut self, dismissal: OverlayDismissPolicy) -> Self {
        self.dismissal = dismissal;
        self
    }

    pub fn focus_lifecycle(mut self, focus: OverlayFocusLifecycle) -> Self {
        self.focus = focus;
        self
    }

    pub fn placement_policy(mut self, placement: ApplicationPopupPlacementPolicy) -> Self {
        self.placement = placement;
        self
    }

    pub const fn anchor(&self) -> PopupAnchor {
        self.anchor
    }

    pub const fn content_size(&self) -> SizeF {
        self.content_size
    }

    pub const fn parent_overlay(&self) -> Option<OverlayId> {
        self.parent
    }

    pub const fn dismissal(&self) -> OverlayDismissPolicy {
        self.dismissal
    }

    pub const fn focus(&self) -> OverlayFocusLifecycle {
        self.focus
    }

    pub fn placement(&self) -> &ApplicationPopupPlacementPolicy {
        &self.placement
    }

    /// Places and opens the popup without mounting content or applying returned focus/input effects.
    pub fn open(
        &self,
        controller: &mut ApplicationOverlayController,
        ui: &MountedUi,
        environment: &EnvironmentValues,
    ) -> Result<PopupOpened, PopupError> {
        let placement_request = ApplicationPopupPlacementRequest::new(
            self.anchor.placement_bounds(),
            self.content_size,
            environment,
        )
        .policy(self.placement.clone());
        let placement =
            place_application_popup(&placement_request).map_err(PopupError::Placement)?;

        let request = OverlayOpenRequest {
            anchor: self.anchor.lifecycle_anchor(),
            parent: self.parent,
            modality: OverlayModality::NonModal,
            dismissal: self.dismissal,
            focus: self.focus,
        };
        let effect = controller
            .route(ApplicationOverlayCommand::Open { ui, request })
            .map_err(PopupError::Controller)?;
        let ApplicationOverlayEffect::Opened(overlay) = effect else {
            unreachable!("an overlay open command can only return an opened effect")
        };
        Ok(PopupOpened { overlay, placement })
    }
}

/// Successful lifecycle and geometry outputs for one popup generation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopupOpened {
    pub overlay: OverlayOpened,
    pub placement: ApplicationPopupPlacement,
}

impl PopupOpened {
    pub const fn id(self) -> OverlayId {
        self.overlay.id
    }

    pub const fn focus_request(self) -> OverlayFocusRequest {
        self.overlay.focus
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupError {
    Placement(ApplicationPopupPlacementError),
    Controller(ApplicationOverlayControllerError),
}

impl fmt::Display for PopupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Placement(error) => error.fmt(formatter),
            Self::Controller(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PopupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Placement(error) => Some(error),
            Self::Controller(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::ui::layout::{
        PopupOverflowPolicy, PopupPlacementAdjustment, PopupPlacementAlignment,
        PopupPlacementCandidate,
    };
    use crate::runtime::{
        Component, ComponentRuntimeDriver, CreateContext, Ui, UpdateContext, ViewRuntime,
    };
    use crate::ui::{BoxStyle, LayoutStyle, OverlayError, OverlayFocusRequest, UiRoot};

    use crate::components::application::{
        ApplicationOverlayHostError, ApplicationPopupPlacementError,
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

    struct Harness {
        runtime: ViewRuntime<ComponentRuntimeDriver<MountedController>>,
        controller: Rc<RefCell<ApplicationOverlayController>>,
        anchor: UiNodeId,
    }

    fn harness() -> Harness {
        let controller = Rc::new(RefCell::new(ApplicationOverlayController::new()));
        let anchor = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(MountedController {
            controller: controller.clone(),
            anchor: anchor.clone(),
        })
        .unwrap();
        Harness {
            runtime,
            controller,
            anchor: anchor.get().unwrap(),
        }
    }

    fn environment() -> EnvironmentValues {
        EnvironmentValues {
            available_size: SizeF {
                width: 320.0,
                height: 200.0,
            },
            ..EnvironmentValues::default()
        }
    }

    fn bounds() -> RectF {
        RectF {
            x: 120.0,
            y: 60.0,
            width: 40.0,
            height: 20.0,
        }
    }

    fn size() -> SizeF {
        SizeF {
            width: 100.0,
            height: 80.0,
        }
    }

    #[test]
    fn standard_popup_opens_one_nonmodal_entry_with_placement() {
        let harness = harness();
        let popup = Popup::new(PopupAnchor::node(harness.anchor, bounds()), size());
        let opened = popup
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        assert_eq!(
            opened.placement.placement.rect,
            RectF {
                x: 120.0,
                y: 80.0,
                width: 100.0,
                height: 80.0,
            }
        );
        assert_eq!(opened.focus_request(), OverlayFocusRequest::None);
        let controller = harness.controller.borrow();
        let entry = controller.entry(opened.id()).unwrap();
        assert_eq!(entry.modality, OverlayModality::NonModal);
        assert_eq!(
            entry.dismissal.outside_press,
            OutsidePressPolicy::DismissAndConsume
        );
        assert!(entry.dismissal.escape);
        assert!(entry.dismissal.focus_lost);
        assert_eq!(
            entry.focus.restoration,
            OverlayFocusRestoration::TargetThenNearest(harness.anchor)
        );
        assert!(!controller.state().background_is_inert);
    }

    #[test]
    fn placement_failure_is_atomic_before_lifecycle_open() {
        let harness = harness();
        let popup = Popup::new(
            PopupAnchor::node(harness.anchor, bounds()),
            SizeF {
                width: 400.0,
                height: 300.0,
            },
        )
        .placement_policy(ApplicationPopupPlacementPolicy::new(
            [PopupPlacementCandidate::below(
                PopupPlacementAlignment::Start,
            )],
            PopupOverflowPolicy::Reject,
        ));
        let result = popup.open(
            &mut harness.controller.borrow_mut(),
            harness.runtime.ui(),
            &environment(),
        );

        assert_eq!(
            result,
            Err(PopupError::Placement(
                ApplicationPopupPlacementError::Layout(crate::ui::layout::PopupPlacementError::NoFit)
            ))
        );
        assert_eq!(harness.controller.borrow().state().entry_count, 0);
    }

    #[test]
    fn lifecycle_failure_after_placement_remains_atomic() {
        let harness = harness();
        let unknown = UiNodeId::new(u32::MAX, u32::MAX);
        let popup = Popup::new(PopupAnchor::node(unknown, bounds()), size());
        let result = popup.open(
            &mut harness.controller.borrow_mut(),
            harness.runtime.ui(),
            &environment(),
        );

        assert_eq!(
            result,
            Err(PopupError::Controller(
                ApplicationOverlayControllerError::Host(ApplicationOverlayHostError::Lifecycle(
                    OverlayError::UnknownAnchor(unknown)
                ))
            ))
        );
        assert_eq!(harness.controller.borrow().state().entry_count, 0);
    }

    #[test]
    fn nested_popup_preserves_parent_and_custom_focus_policy() {
        let harness = harness();
        let parent = Popup::new(PopupAnchor::node(harness.anchor, bounds()), size())
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();
        let focus = OverlayFocusLifecycle {
            initial: OverlayInitialFocus::FirstFocusable,
            containment: OverlayFocusContainment::None,
            restoration: OverlayFocusRestoration::TargetThenNearest(harness.anchor),
        };
        let child = Popup::new(PopupAnchor::rect(bounds()), size())
            .parent(parent.id())
            .focus_lifecycle(focus)
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        assert_eq!(
            child.focus_request(),
            OverlayFocusRequest::Initial(OverlayInitialFocus::FirstFocusable)
        );
        let controller = harness.controller.borrow();
        assert_eq!(
            controller.entry(child.id()).unwrap().parent,
            Some(parent.id())
        );
        assert_eq!(controller.state().entry_count, 2);
        assert!(!controller.state().background_is_inert);
    }

    #[test]
    fn custom_scroll_placement_is_returned_without_mounting_content() {
        let harness = harness();
        let popup = Popup::new(
            PopupAnchor::point(PointF { x: 150.0, y: 90.0 }),
            SizeF {
                width: 500.0,
                height: 400.0,
            },
        )
        .placement_policy(ApplicationPopupPlacementPolicy::new(
            [PopupPlacementCandidate::below(
                PopupPlacementAlignment::Center,
            )],
            PopupOverflowPolicy::Scroll {
                minimum_viewport: SizeF {
                    width: 100.0,
                    height: 80.0,
                },
            },
        ));
        let opened = popup
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        assert!(opened.placement.requires_scroll());
        assert!(matches!(
            opened.placement.placement.adjustment,
            PopupPlacementAdjustment::ScrollViewport { .. }
        ));
        assert_eq!(harness.runtime.ui().nodes.alive().len(), 2);
    }

    #[test]
    fn popup_is_always_nonmodal_even_with_custom_policies() {
        let harness = harness();
        let popup = Popup::new(PopupAnchor::rect(bounds()), size()).dismissal_policy(
            OverlayDismissPolicy {
                escape: false,
                outside_press: OutsidePressPolicy::DismissAndPropagate,
                focus_lost: false,
                pointer_departure: true,
            },
        );
        let opened = popup
            .open(
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();
        let controller = harness.controller.borrow();

        assert_eq!(
            controller.entry(opened.id()).unwrap().modality,
            OverlayModality::NonModal
        );
        assert!(!controller.state().background_is_inert);
    }
}
