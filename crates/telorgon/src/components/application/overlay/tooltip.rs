//! Typed nonfocusable tooltip policy over the application overlay and placement owners.

use std::fmt;
use std::time::Duration;

use crate::components::application::primitives::EnvironmentValues;
use crate::foundation::{RectF, SizeF};
use crate::ui::layout::PopupOverflowPolicy;
use crate::runtime::MonotonicInstant;
use crate::ui::{
    MountedUi, OutsidePressPolicy, OverlayAnchor, OverlayDismissPolicy, OverlayFocusContainment,
    OverlayFocusLifecycle, OverlayFocusRequest, OverlayFocusRestoration, OverlayId,
    OverlayInitialFocus, OverlayModality, OverlayOpenRequest, OverlayOpened,
    SemanticRelationshipKind, SemanticRole, UiNodeId,
};

use super::{
    ApplicationOverlayCommand, ApplicationOverlayController, ApplicationOverlayControllerError,
    ApplicationOverlayEffect, ApplicationPopupPlacement, ApplicationPopupPlacementError,
    ApplicationPopupPlacementPolicy, ApplicationPopupPlacementRequest,
    STANDARD_APPLICATION_POPUP_CANDIDATES, place_application_popup,
};

/// Source that requests a tooltip after its configured caller-owned deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TooltipTrigger {
    Hover,
    SustainedFocus,
}

/// Validated hover and sustained-focus delays. This value never schedules work itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TooltipTriggerPolicy {
    hover_delay_nanos: Option<u64>,
    sustained_focus_delay_nanos: Option<u64>,
}

impl TooltipTriggerPolicy {
    pub fn new(
        hover_delay: Option<Duration>,
        sustained_focus_delay: Option<Duration>,
    ) -> Result<Self, TooltipTriggerPolicyError> {
        if hover_delay.is_none() && sustained_focus_delay.is_none() {
            return Err(TooltipTriggerPolicyError::NoTriggers);
        }
        Ok(Self {
            hover_delay_nanos: hover_delay
                .map(|delay| validate_delay(TooltipTrigger::Hover, delay))
                .transpose()?,
            sustained_focus_delay_nanos: sustained_focus_delay
                .map(|delay| validate_delay(TooltipTrigger::SustainedFocus, delay))
                .transpose()?,
        })
    }

    pub fn hover(delay: Duration) -> Result<Self, TooltipTriggerPolicyError> {
        Self::new(Some(delay), None)
    }

    pub fn sustained_focus(delay: Duration) -> Result<Self, TooltipTriggerPolicyError> {
        Self::new(None, Some(delay))
    }

    pub fn hover_and_sustained_focus(
        hover_delay: Duration,
        sustained_focus_delay: Duration,
    ) -> Result<Self, TooltipTriggerPolicyError> {
        Self::new(Some(hover_delay), Some(sustained_focus_delay))
    }

    pub const fn is_enabled(self, trigger: TooltipTrigger) -> bool {
        self.delay_nanos(trigger).is_some()
    }

    pub const fn delay_nanos(self, trigger: TooltipTrigger) -> Option<u64> {
        match trigger {
            TooltipTrigger::Hover => self.hover_delay_nanos,
            TooltipTrigger::SustainedFocus => self.sustained_focus_delay_nanos,
        }
    }

    /// Resolves a host-clock deadline without starting a timer or retaining timer state.
    pub fn deadline(
        self,
        trigger: TooltipTrigger,
        began_at: MonotonicInstant,
    ) -> Result<Option<TooltipDeadlineIntent>, TooltipDeadlineError> {
        let Some(delay_nanos) = self.delay_nanos(trigger) else {
            return Ok(None);
        };
        let at = began_at
            .checked_add(Duration::from_nanos(delay_nanos))
            .ok_or(TooltipDeadlineError::Overflow)?;
        Ok(Some(TooltipDeadlineIntent { trigger, at }))
    }
}

fn validate_delay(
    trigger: TooltipTrigger,
    delay: Duration,
) -> Result<u64, TooltipTriggerPolicyError> {
    let nanos = u64::try_from(delay.as_nanos())
        .map_err(|_| TooltipTriggerPolicyError::DelayOutOfRange { trigger })?;
    if nanos == 0 {
        return Err(TooltipTriggerPolicyError::ZeroDelay { trigger });
    }
    Ok(nanos)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TooltipDeadlineIntent {
    pub trigger: TooltipTrigger,
    pub at: MonotonicInstant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooltipTriggerPolicyError {
    NoTriggers,
    ZeroDelay { trigger: TooltipTrigger },
    DelayOutOfRange { trigger: TooltipTrigger },
}

impl fmt::Display for TooltipTriggerPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid tooltip trigger policy: {self:?}")
    }
}

impl std::error::Error for TooltipTriggerPolicyError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooltipDeadlineError {
    Overflow,
}

impl fmt::Display for TooltipDeadlineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("tooltip deadline exceeds the host monotonic clock range")
    }
}

impl std::error::Error for TooltipDeadlineError {}

/// Lifecycle node and resolved logical geometry for a tooltip anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipAnchor {
    pub node: UiNodeId,
    pub bounds: RectF,
}

impl TooltipAnchor {
    pub const fn new(node: UiNodeId, bounds: RectF) -> Self {
        Self { node, bounds }
    }
}

/// Scale-one desired and minimum reflow sizes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipExtent {
    pub content_size_at_scale_one: SizeF,
    pub minimum_size_at_scale_one: SizeF,
}

impl TooltipExtent {
    pub const fn new(content_size_at_scale_one: SizeF, minimum_size_at_scale_one: SizeF) -> Self {
        Self {
            content_size_at_scale_one,
            minimum_size_at_scale_one,
        }
    }

    fn resolve(self, text_scale: f32) -> ResolvedTooltipExtent {
        ResolvedTooltipExtent {
            content_size: scaled_size(self.content_size_at_scale_one, text_scale),
            minimum_size: scaled_size(self.minimum_size_at_scale_one, text_scale),
            text_scale,
        }
    }
}

fn scaled_size(size: SizeF, scale: f32) -> SizeF {
    SizeF {
        width: size.width * scale,
        height: size.height * scale,
    }
}

/// Text-scale-resolved geometry returned to the later content/reflow owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedTooltipExtent {
    pub content_size: SizeF,
    pub minimum_size: SizeF,
    pub text_scale: f32,
}

/// Dismissal causes understood by the shared overlay lifecycle owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TooltipDismissalPolicy {
    pub escape: bool,
    pub pointer_departure: bool,
    pub focus_lost: bool,
}

impl Default for TooltipDismissalPolicy {
    fn default() -> Self {
        Self {
            escape: true,
            pointer_departure: true,
            focus_lost: true,
        }
    }
}

/// Supplemental-description semantics; a tooltip never contributes an accessible name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TooltipSemanticsIntent {
    pub role: SemanticRole,
    pub anchor: UiNodeId,
    pub anchor_relationship: SemanticRelationshipKind,
    pub contribution: TooltipAccessibleContribution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TooltipAccessibleContribution {
    DescriptionOnly,
}

/// One reusable nonfocusable tooltip configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct Tooltip {
    description: String,
    anchor: TooltipAnchor,
    extent: TooltipExtent,
    triggers: TooltipTriggerPolicy,
    dismissal: TooltipDismissalPolicy,
    parent: Option<OverlayId>,
    gap_at_scale_one: f32,
}

impl Tooltip {
    pub fn new(
        description: impl Into<String>,
        anchor: TooltipAnchor,
        extent: TooltipExtent,
        triggers: TooltipTriggerPolicy,
    ) -> Result<Self, TooltipError> {
        let description = description.into();
        if description.trim().is_empty() {
            return Err(TooltipError::MissingDescription);
        }
        Ok(Self {
            description,
            anchor,
            extent,
            triggers,
            dismissal: TooltipDismissalPolicy::default(),
            parent: None,
            gap_at_scale_one: 0.0,
        })
    }

    pub fn dismissal_policy(mut self, dismissal: TooltipDismissalPolicy) -> Self {
        self.dismissal = dismissal;
        self
    }

    pub fn parent(mut self, parent: OverlayId) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Logical spacing from the anchor before environment text scaling is applied.
    pub fn gap_at_scale_one(mut self, gap: f32) -> Self {
        self.gap_at_scale_one = gap;
        self
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub const fn anchor(&self) -> TooltipAnchor {
        self.anchor
    }

    pub const fn extent(&self) -> TooltipExtent {
        self.extent
    }

    pub const fn triggers(&self) -> TooltipTriggerPolicy {
        self.triggers
    }

    pub const fn dismissal(&self) -> TooltipDismissalPolicy {
        self.dismissal
    }

    pub const fn parent_overlay(&self) -> Option<OverlayId> {
        self.parent
    }

    pub const fn semantics_intent(&self) -> TooltipSemanticsIntent {
        TooltipSemanticsIntent {
            role: SemanticRole::Tooltip,
            anchor: self.anchor.node,
            anchor_relationship: SemanticRelationshipKind::DescribedBy,
            contribution: TooltipAccessibleContribution::DescriptionOnly,
        }
    }

    /// Places and opens the tooltip after the caller observes its deadline intent.
    ///
    /// This method does not start a timer, mount content, move focus, or apply returned effects.
    pub fn open(
        &self,
        trigger: TooltipTrigger,
        controller: &mut ApplicationOverlayController,
        ui: &MountedUi,
        environment: &EnvironmentValues,
    ) -> Result<TooltipOpened, TooltipError> {
        if !self.triggers.is_enabled(trigger) {
            return Err(TooltipError::TriggerDisabled(trigger));
        }
        if !environment.text_scale.is_finite() || environment.text_scale <= 0.0 {
            return Err(TooltipError::InvalidTextScale);
        }
        let extent = self.extent.resolve(environment.text_scale);
        let placement_policy = ApplicationPopupPlacementPolicy::new(
            STANDARD_APPLICATION_POPUP_CANDIDATES,
            PopupOverflowPolicy::Resize {
                minimum_size: extent.minimum_size,
            },
        )
        .gap(self.gap_at_scale_one * environment.text_scale);
        let placement_request = ApplicationPopupPlacementRequest::new(
            self.anchor.bounds,
            extent.content_size,
            environment,
        )
        .policy(placement_policy);
        let placement =
            place_application_popup(&placement_request).map_err(TooltipError::Placement)?;

        let request = OverlayOpenRequest {
            anchor: OverlayAnchor::Node(self.anchor.node),
            parent: self.parent,
            modality: OverlayModality::NonModal,
            dismissal: OverlayDismissPolicy {
                escape: self.dismissal.escape,
                outside_press: OutsidePressPolicy::Ignore,
                focus_lost: self.dismissal.focus_lost,
                pointer_departure: self.dismissal.pointer_departure,
            },
            focus: OverlayFocusLifecycle {
                initial: OverlayInitialFocus::None,
                containment: OverlayFocusContainment::None,
                restoration: OverlayFocusRestoration::None,
            },
        };
        let effect = controller
            .route(ApplicationOverlayCommand::Open { ui, request })
            .map_err(TooltipError::Controller)?;
        let ApplicationOverlayEffect::Opened(overlay) = effect else {
            unreachable!("an overlay open command can only return an opened effect")
        };
        Ok(TooltipOpened {
            overlay,
            placement,
            trigger,
            extent,
            semantics: self.semantics_intent(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipOpened {
    pub overlay: OverlayOpened,
    pub placement: ApplicationPopupPlacement,
    pub trigger: TooltipTrigger,
    pub extent: ResolvedTooltipExtent,
    pub semantics: TooltipSemanticsIntent,
}

impl TooltipOpened {
    pub const fn id(self) -> OverlayId {
        self.overlay.id
    }

    pub const fn focus_request(self) -> OverlayFocusRequest {
        self.overlay.focus
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooltipError {
    MissingDescription,
    TriggerDisabled(TooltipTrigger),
    InvalidTextScale,
    Placement(ApplicationPopupPlacementError),
    Controller(ApplicationOverlayControllerError),
}

impl fmt::Display for TooltipError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDescription => formatter.write_str("tooltip description is empty"),
            Self::TriggerDisabled(trigger) => {
                write!(formatter, "tooltip trigger is disabled: {trigger:?}")
            }
            Self::InvalidTextScale => formatter.write_str("tooltip text scale is invalid"),
            Self::Placement(error) => error.fmt(formatter),
            Self::Controller(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TooltipError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Placement(error) => Some(error),
            Self::Controller(error) => Some(error),
            Self::MissingDescription | Self::TriggerDisabled(_) | Self::InvalidTextScale => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::runtime::{
        Component, ComponentRuntimeDriver, CreateContext, Ui, UpdateContext, ViewRuntime,
    };
    use crate::ui::{BoxStyle, LayoutStyle, OverlayError, UiRoot};

    use crate::components::application::ApplicationOverlayHostError;

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

    fn anchor(node: UiNodeId) -> TooltipAnchor {
        TooltipAnchor::new(
            node,
            RectF {
                x: 120.0,
                y: 60.0,
                width: 40.0,
                height: 20.0,
            },
        )
    }

    fn extent() -> TooltipExtent {
        TooltipExtent::new(
            SizeF {
                width: 100.0,
                height: 50.0,
            },
            SizeF {
                width: 60.0,
                height: 30.0,
            },
        )
    }

    fn triggers() -> TooltipTriggerPolicy {
        TooltipTriggerPolicy::hover_and_sustained_focus(
            Duration::from_millis(500),
            Duration::from_millis(700),
        )
        .unwrap()
    }

    fn tooltip(harness: &Harness) -> Tooltip {
        Tooltip::new(
            "Shows account settings",
            anchor(harness.anchor),
            extent(),
            triggers(),
        )
        .unwrap()
    }

    #[test]
    fn trigger_policy_is_validated_and_returns_only_caller_owned_deadlines() {
        assert_eq!(
            TooltipTriggerPolicy::new(None, None),
            Err(TooltipTriggerPolicyError::NoTriggers)
        );
        assert_eq!(
            TooltipTriggerPolicy::hover(Duration::ZERO),
            Err(TooltipTriggerPolicyError::ZeroDelay {
                trigger: TooltipTrigger::Hover,
            })
        );
        let hover = TooltipTriggerPolicy::hover(Duration::from_millis(250)).unwrap();
        assert_eq!(
            hover
                .deadline(TooltipTrigger::Hover, MonotonicInstant::from_nanos(10))
                .unwrap(),
            Some(TooltipDeadlineIntent {
                trigger: TooltipTrigger::Hover,
                at: MonotonicInstant::from_nanos(250_000_010),
            })
        );
        assert_eq!(
            hover
                .deadline(TooltipTrigger::SustainedFocus, MonotonicInstant::ZERO,)
                .unwrap(),
            None
        );
        assert_eq!(
            hover.deadline(
                TooltipTrigger::Hover,
                MonotonicInstant::from_nanos(u64::MAX - 1),
            ),
            Err(TooltipDeadlineError::Overflow)
        );
    }

    #[test]
    fn construction_is_description_only_and_never_names_the_anchor() {
        let harness = harness();
        assert_eq!(
            Tooltip::new(" ", anchor(harness.anchor), extent(), triggers()),
            Err(TooltipError::MissingDescription)
        );
        let tooltip = tooltip(&harness);
        assert_eq!(tooltip.description(), "Shows account settings");
        assert_eq!(
            tooltip.semantics_intent(),
            TooltipSemanticsIntent {
                role: SemanticRole::Tooltip,
                anchor: harness.anchor,
                anchor_relationship: SemanticRelationshipKind::DescribedBy,
                contribution: TooltipAccessibleContribution::DescriptionOnly,
            }
        );
    }

    #[test]
    fn hover_tooltip_is_nonmodal_nonfocusable_and_dismissible() {
        let harness = harness();
        let opened = tooltip(&harness)
            .open(
                TooltipTrigger::Hover,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        assert_eq!(opened.trigger, TooltipTrigger::Hover);
        assert_eq!(opened.focus_request(), OverlayFocusRequest::None);
        let controller = harness.controller.borrow();
        let entry = controller.entry(opened.id()).unwrap();
        assert_eq!(entry.modality, OverlayModality::NonModal);
        assert_eq!(entry.focus.initial, OverlayInitialFocus::None);
        assert_eq!(entry.focus.containment, OverlayFocusContainment::None);
        assert_eq!(entry.focus.restoration, OverlayFocusRestoration::None);
        assert!(entry.dismissal.escape);
        assert!(entry.dismissal.pointer_departure);
        assert!(entry.dismissal.focus_lost);
        assert_eq!(entry.dismissal.outside_press, OutsidePressPolicy::Ignore);
        assert!(!controller.state().background_is_inert);
    }

    #[test]
    fn sustained_focus_trigger_preserves_custom_dismissal_and_parentage() {
        let harness = harness();
        let parent = tooltip(&harness)
            .open(
                TooltipTrigger::Hover,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();
        let dismissal = TooltipDismissalPolicy {
            escape: true,
            pointer_departure: false,
            focus_lost: true,
        };
        let opened = tooltip(&harness)
            .parent(parent.id())
            .dismissal_policy(dismissal)
            .open(
                TooltipTrigger::SustainedFocus,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        let controller = harness.controller.borrow();
        let entry = controller.entry(opened.id()).unwrap();
        assert_eq!(entry.parent, Some(parent.id()));
        assert!(!entry.dismissal.pointer_departure);
        assert!(entry.dismissal.focus_lost);
    }

    #[test]
    fn text_scale_resolves_reflow_geometry_without_mounting_content() {
        let harness = harness();
        let before = harness.runtime.ui().nodes.alive().len();
        let mut environment = environment();
        environment.available_size = SizeF {
            width: 180.0,
            height: 120.0,
        };
        environment.text_scale = 2.0;
        let opened = tooltip(&harness)
            .open(
                TooltipTrigger::Hover,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment,
            )
            .unwrap();

        assert_eq!(opened.extent.text_scale, 2.0);
        assert_eq!(
            opened.extent.content_size,
            SizeF {
                width: 200.0,
                height: 100.0,
            }
        );
        assert!(opened.placement.was_resized());
        assert_eq!(opened.placement.placement.rect.width, 180.0);
        assert_eq!(harness.runtime.ui().nodes.alive().len(), before);
    }

    #[test]
    fn trigger_placement_and_lifecycle_rejections_leave_the_host_unchanged() {
        let harness = harness();
        let focus_only = Tooltip::new(
            "Focus help",
            anchor(harness.anchor),
            extent(),
            TooltipTriggerPolicy::sustained_focus(Duration::from_millis(400)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            focus_only.open(
                TooltipTrigger::Hover,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            ),
            Err(TooltipError::TriggerDisabled(TooltipTrigger::Hover))
        );
        assert_eq!(harness.controller.borrow().state().entry_count, 0);

        let invalid_extent = Tooltip::new(
            "Invalid extent",
            anchor(harness.anchor),
            TooltipExtent::new(
                SizeF {
                    width: 40.0,
                    height: 30.0,
                },
                SizeF {
                    width: 60.0,
                    height: 50.0,
                },
            ),
            triggers(),
        )
        .unwrap();
        assert!(matches!(
            invalid_extent.open(
                TooltipTrigger::Hover,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            ),
            Err(TooltipError::Placement(_))
        ));
        assert_eq!(harness.controller.borrow().state().entry_count, 0);

        let unknown = UiNodeId::new(u32::MAX, u32::MAX);
        let invalid_anchor =
            Tooltip::new("Unknown anchor", anchor(unknown), extent(), triggers()).unwrap();
        assert_eq!(
            invalid_anchor.open(
                TooltipTrigger::Hover,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            ),
            Err(TooltipError::Controller(
                ApplicationOverlayControllerError::Host(ApplicationOverlayHostError::Lifecycle(
                    OverlayError::UnknownAnchor(unknown)
                ))
            ))
        );
        assert_eq!(harness.controller.borrow().state().entry_count, 0);
    }
}
