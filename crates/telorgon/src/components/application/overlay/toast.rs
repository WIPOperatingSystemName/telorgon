//! Typed nonfocusable toast policy over the application overlay and placement owners.

use std::fmt;
use std::num::NonZeroU64;
use std::time::Duration;

use crate::components::application::primitives::EnvironmentValues;
use crate::foundation::{RectF, SizeF};
use crate::input::WritingDirection;
use crate::ui::layout::{PopupOverflowPolicy, PopupPlacementAlignment, PopupPlacementCandidate};
use crate::runtime::MonotonicInstant;
use crate::ui::{
    MountedUi, OutsidePressPolicy, OverlayAnchor, OverlayDismissPolicy, OverlayFocusContainment,
    OverlayFocusLifecycle, OverlayFocusRequest, OverlayFocusRestoration, OverlayId,
    OverlayInitialFocus, OverlayModality, OverlayOpenRequest, OverlayOpened, SemanticRole,
};

use super::placement::application_usable_bounds;
use super::{
    ApplicationOverlayCommand, ApplicationOverlayController, ApplicationOverlayControllerError,
    ApplicationOverlayEffect, ApplicationPopupPlacement, ApplicationPopupPlacementError,
    ApplicationPopupPlacementPolicy, ApplicationPopupPlacementRequest, place_application_popup,
};

/// Logical safe-area corner used by a toast stack owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToastCorner {
    BlockStartInlineStart,
    BlockStartInlineEnd,
    BlockEndInlineStart,
    #[default]
    BlockEndInlineEnd,
}

impl ToastCorner {
    pub const fn resolve(self, direction: WritingDirection) -> ResolvedToastCorner {
        match (self, direction) {
            (Self::BlockStartInlineStart, WritingDirection::LeftToRight)
            | (Self::BlockStartInlineEnd, WritingDirection::RightToLeft) => {
                ResolvedToastCorner::TopLeft
            }
            (Self::BlockStartInlineEnd, WritingDirection::LeftToRight)
            | (Self::BlockStartInlineStart, WritingDirection::RightToLeft) => {
                ResolvedToastCorner::TopRight
            }
            (Self::BlockEndInlineStart, WritingDirection::LeftToRight)
            | (Self::BlockEndInlineEnd, WritingDirection::RightToLeft) => {
                ResolvedToastCorner::BottomLeft
            }
            (Self::BlockEndInlineEnd, WritingDirection::LeftToRight)
            | (Self::BlockEndInlineStart, WritingDirection::RightToLeft) => {
                ResolvedToastCorner::BottomRight
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResolvedToastCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Priority supplied to the later live-region owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToastAnnouncementPriority {
    #[default]
    Polite,
    Assertive,
}

impl ToastAnnouncementPriority {
    const fn semantic_role(self) -> SemanticRole {
        match self {
            Self::Polite => SemanticRole::Status,
            Self::Assertive => SemanticRole::Alert,
        }
    }
}

/// Opaque nonzero identity used by a separate announcement/stack owner for replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastCoalescingKey(NonZeroU64);

impl ToastCoalescingKey {
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub const fn from_raw(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToastCoalescingIntent {
    #[default]
    Independent,
    ReplaceMatching(ToastCoalescingKey),
}

/// Where later announcement/diagnostic owners must omit the message text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToastRedactionIntent {
    #[default]
    None,
    Diagnostics,
    AnnouncementAndDiagnostics,
}

impl ToastRedactionIntent {
    const fn redacts_diagnostics(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Typed input for a separate live-region/coalescing owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastAnnouncementPolicy {
    pub priority: ToastAnnouncementPriority,
    pub coalescing: ToastCoalescingIntent,
    pub redaction: ToastRedactionIntent,
}

impl ToastAnnouncementPolicy {
    pub const fn new(priority: ToastAnnouncementPriority) -> Self {
        Self {
            priority,
            coalescing: ToastCoalescingIntent::Independent,
            redaction: ToastRedactionIntent::None,
        }
    }

    pub const fn coalescing(mut self, coalescing: ToastCoalescingIntent) -> Self {
        self.coalescing = coalescing;
        self
    }

    pub const fn redaction(mut self, redaction: ToastRedactionIntent) -> Self {
        self.redaction = redaction;
        self
    }

    pub const fn intent(self) -> ToastAnnouncementIntent {
        ToastAnnouncementIntent {
            role: self.priority.semantic_role(),
            priority: self.priority,
            coalescing: self.coalescing,
            redaction: self.redaction,
        }
    }
}

impl Default for ToastAnnouncementPolicy {
    fn default() -> Self {
        Self::new(ToastAnnouncementPriority::Polite)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastAnnouncementIntent {
    pub role: SemanticRole,
    pub priority: ToastAnnouncementPriority,
    pub coalescing: ToastCoalescingIntent,
    pub redaction: ToastRedactionIntent,
}

/// Validated persistent or caller-timed toast lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastLifetime {
    expiry_delay_nanos: Option<u64>,
}

impl ToastLifetime {
    pub const fn persistent() -> Self {
        Self {
            expiry_delay_nanos: None,
        }
    }

    pub fn expiring(after: Duration) -> Result<Self, ToastLifetimeError> {
        let nanos =
            u64::try_from(after.as_nanos()).map_err(|_| ToastLifetimeError::DurationOutOfRange)?;
        if nanos == 0 {
            return Err(ToastLifetimeError::ZeroDuration);
        }
        Ok(Self {
            expiry_delay_nanos: Some(nanos),
        })
    }

    pub const fn is_persistent(self) -> bool {
        self.expiry_delay_nanos.is_none()
    }

    /// Resolves an expiry request without starting or retaining a timer.
    pub fn expiry(
        self,
        opened_at: MonotonicInstant,
    ) -> Result<Option<ToastExpiryIntent>, ToastDeadlineError> {
        let Some(delay_nanos) = self.expiry_delay_nanos else {
            return Ok(None);
        };
        let at = opened_at
            .checked_add(Duration::from_nanos(delay_nanos))
            .ok_or(ToastDeadlineError::Overflow)?;
        Ok(Some(ToastExpiryIntent { at }))
    }
}

impl Default for ToastLifetime {
    fn default() -> Self {
        Self::persistent()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastLifetimeError {
    ZeroDuration,
    DurationOutOfRange,
}

impl fmt::Display for ToastLifetimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid toast lifetime: {self:?}")
    }
}

impl std::error::Error for ToastLifetimeError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastDeadlineError {
    Overflow,
}

impl fmt::Display for ToastDeadlineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("toast expiry exceeds the host monotonic clock range")
    }
}

impl std::error::Error for ToastDeadlineError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastExpiryIntent {
    pub at: MonotonicInstant,
}

/// Scale-one desired and minimum reflow sizes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToastExtent {
    pub content_size_at_scale_one: SizeF,
    pub minimum_size_at_scale_one: SizeF,
}

impl ToastExtent {
    pub const fn new(content_size_at_scale_one: SizeF, minimum_size_at_scale_one: SizeF) -> Self {
        Self {
            content_size_at_scale_one,
            minimum_size_at_scale_one,
        }
    }

    fn resolve(self, text_scale: f32) -> ResolvedToastExtent {
        ResolvedToastExtent {
            content_size: scale_size(self.content_size_at_scale_one, text_scale),
            minimum_size: scale_size(self.minimum_size_at_scale_one, text_scale),
            text_scale,
        }
    }
}

fn scale_size(size: SizeF, scale: f32) -> SizeF {
    SizeF {
        width: size.width * scale,
        height: size.height * scale,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedToastExtent {
    pub content_size: SizeF,
    pub minimum_size: SizeF,
    pub text_scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastDismissalPolicy {
    pub escape: bool,
    pub manual: bool,
}

impl Default for ToastDismissalPolicy {
    fn default() -> Self {
        Self {
            escape: true,
            manual: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastDismissalIntent {
    pub escape: bool,
    pub manual: bool,
    pub expiry: Option<ToastExpiryIntent>,
}

/// One reusable nonfocusable application toast configuration.
#[derive(Clone, PartialEq)]
pub struct Toast {
    message: String,
    corner: ToastCorner,
    extent: ToastExtent,
    announcement: ToastAnnouncementPolicy,
    lifetime: ToastLifetime,
    dismissal: ToastDismissalPolicy,
    parent: Option<OverlayId>,
    gap_at_scale_one: f32,
}

impl fmt::Debug for Toast {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message: &dyn fmt::Debug = if self.announcement.redaction.redacts_diagnostics() {
            &"<redacted>"
        } else {
            &self.message
        };
        formatter
            .debug_struct("Toast")
            .field("message", message)
            .field("corner", &self.corner)
            .field("extent", &self.extent)
            .field("announcement", &self.announcement)
            .field("lifetime", &self.lifetime)
            .field("dismissal", &self.dismissal)
            .field("parent", &self.parent)
            .field("gap_at_scale_one", &self.gap_at_scale_one)
            .finish()
    }
}

impl Toast {
    pub fn new(
        message: impl Into<String>,
        corner: ToastCorner,
        extent: ToastExtent,
        announcement: ToastAnnouncementPolicy,
        lifetime: ToastLifetime,
    ) -> Result<Self, ToastError> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(ToastError::MissingMessage);
        }
        Ok(Self {
            message,
            corner,
            extent,
            announcement,
            lifetime,
            dismissal: ToastDismissalPolicy::default(),
            parent: None,
            gap_at_scale_one: 0.0,
        })
    }

    pub fn dismissal_policy(mut self, dismissal: ToastDismissalPolicy) -> Self {
        self.dismissal = dismissal;
        self
    }

    pub fn parent(mut self, parent: OverlayId) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn gap_at_scale_one(mut self, gap: f32) -> Self {
        self.gap_at_scale_one = gap;
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub const fn corner(&self) -> ToastCorner {
        self.corner
    }

    pub const fn extent(&self) -> ToastExtent {
        self.extent
    }

    pub const fn announcement(&self) -> ToastAnnouncementPolicy {
        self.announcement
    }

    pub const fn lifetime(&self) -> ToastLifetime {
        self.lifetime
    }

    pub const fn dismissal(&self) -> ToastDismissalPolicy {
        self.dismissal
    }

    pub const fn parent_overlay(&self) -> Option<OverlayId> {
        self.parent
    }

    /// Places and opens a toast without scheduling expiry, mounting content, or applying effects.
    pub fn open(
        &self,
        opened_at: MonotonicInstant,
        controller: &mut ApplicationOverlayController,
        ui: &MountedUi,
        environment: &EnvironmentValues,
    ) -> Result<ToastOpened, ToastError> {
        let expiry = self
            .lifetime
            .expiry(opened_at)
            .map_err(ToastError::Deadline)?;
        if !environment.text_scale.is_finite() || environment.text_scale <= 0.0 {
            return Err(ToastError::InvalidTextScale);
        }
        let safe_bounds = application_usable_bounds(environment).map_err(ToastError::Placement)?;
        let extent = self.extent.resolve(environment.text_scale);
        let (anchor, candidate) = corner_placement(self.corner, safe_bounds);
        let placement_policy = ApplicationPopupPlacementPolicy::new(
            [candidate],
            PopupOverflowPolicy::Resize {
                minimum_size: extent.minimum_size,
            },
        )
        .gap(self.gap_at_scale_one * environment.text_scale);
        let placement_request =
            ApplicationPopupPlacementRequest::new(anchor, extent.content_size, environment)
                .policy(placement_policy);
        let placement =
            place_application_popup(&placement_request).map_err(ToastError::Placement)?;

        let request = OverlayOpenRequest {
            anchor: OverlayAnchor::Rect(anchor),
            parent: self.parent,
            modality: OverlayModality::NonModal,
            dismissal: OverlayDismissPolicy {
                escape: self.dismissal.escape,
                outside_press: OutsidePressPolicy::Ignore,
                focus_lost: false,
                pointer_departure: false,
            },
            focus: OverlayFocusLifecycle {
                initial: OverlayInitialFocus::None,
                containment: OverlayFocusContainment::None,
                restoration: OverlayFocusRestoration::None,
            },
        };
        let effect = controller
            .route(ApplicationOverlayCommand::Open { ui, request })
            .map_err(ToastError::Controller)?;
        let ApplicationOverlayEffect::Opened(overlay) = effect else {
            unreachable!("an overlay open command can only return an opened effect")
        };
        Ok(ToastOpened {
            overlay,
            placement,
            corner: self.corner.resolve(environment.writing_direction),
            extent,
            announcement: self.announcement.intent(),
            dismissal: ToastDismissalIntent {
                escape: self.dismissal.escape,
                manual: self.dismissal.manual,
                expiry,
            },
        })
    }
}

fn corner_placement(corner: ToastCorner, safe: RectF) -> (RectF, PopupPlacementCandidate) {
    match corner {
        ToastCorner::BlockStartInlineStart => (
            RectF {
                x: safe.x,
                y: safe.y,
                width: safe.width,
                height: 0.0,
            },
            PopupPlacementCandidate::below(PopupPlacementAlignment::Start),
        ),
        ToastCorner::BlockStartInlineEnd => (
            RectF {
                x: safe.x,
                y: safe.y,
                width: safe.width,
                height: 0.0,
            },
            PopupPlacementCandidate::below(PopupPlacementAlignment::End),
        ),
        ToastCorner::BlockEndInlineStart => (
            RectF {
                x: safe.x,
                y: safe.bottom(),
                width: safe.width,
                height: 0.0,
            },
            PopupPlacementCandidate::above(PopupPlacementAlignment::Start),
        ),
        ToastCorner::BlockEndInlineEnd => (
            RectF {
                x: safe.x,
                y: safe.bottom(),
                width: safe.width,
                height: 0.0,
            },
            PopupPlacementCandidate::above(PopupPlacementAlignment::End),
        ),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToastOpened {
    pub overlay: OverlayOpened,
    pub placement: ApplicationPopupPlacement,
    pub corner: ResolvedToastCorner,
    pub extent: ResolvedToastExtent,
    pub announcement: ToastAnnouncementIntent,
    pub dismissal: ToastDismissalIntent,
}

impl ToastOpened {
    pub const fn id(self) -> OverlayId {
        self.overlay.id
    }

    pub const fn focus_request(self) -> OverlayFocusRequest {
        self.overlay.focus
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastError {
    MissingMessage,
    InvalidTextScale,
    Deadline(ToastDeadlineError),
    Placement(ApplicationPopupPlacementError),
    Controller(ApplicationOverlayControllerError),
}

impl fmt::Display for ToastError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMessage => formatter.write_str("toast message is empty"),
            Self::InvalidTextScale => formatter.write_str("toast text scale is invalid"),
            Self::Deadline(error) => error.fmt(formatter),
            Self::Placement(error) => error.fmt(formatter),
            Self::Controller(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ToastError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Deadline(error) => Some(error),
            Self::Placement(error) => Some(error),
            Self::Controller(error) => Some(error),
            Self::MissingMessage | Self::InvalidTextScale => None,
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
    }

    fn harness() -> Harness {
        let controller = Rc::new(RefCell::new(ApplicationOverlayController::new()));
        let runtime = ViewRuntime::from_component(MountedController {
            controller: controller.clone(),
        })
        .unwrap();
        Harness {
            runtime,
            controller,
        }
    }

    fn environment() -> EnvironmentValues {
        EnvironmentValues {
            available_size: SizeF {
                width: 360.0,
                height: 240.0,
            },
            safe_area: EdgeInsets {
                top: 8.0,
                right: 16.0,
                bottom: 24.0,
                left: 32.0,
            },
            ..EnvironmentValues::default()
        }
    }

    fn extent() -> ToastExtent {
        ToastExtent::new(
            SizeF {
                width: 160.0,
                height: 64.0,
            },
            SizeF {
                width: 100.0,
                height: 40.0,
            },
        )
    }

    fn toast() -> Toast {
        Toast::new(
            "Settings saved",
            ToastCorner::BlockEndInlineEnd,
            extent(),
            ToastAnnouncementPolicy::default(),
            ToastLifetime::expiring(Duration::from_secs(5)).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn lifetime_is_validated_and_returns_only_caller_owned_expiry() {
        assert_eq!(
            ToastLifetime::expiring(Duration::ZERO),
            Err(ToastLifetimeError::ZeroDuration)
        );
        let lifetime = ToastLifetime::expiring(Duration::from_millis(500)).unwrap();
        assert_eq!(
            lifetime.expiry(MonotonicInstant::from_nanos(10)).unwrap(),
            Some(ToastExpiryIntent {
                at: MonotonicInstant::from_nanos(500_000_010),
            })
        );
        assert_eq!(
            ToastLifetime::persistent().expiry(MonotonicInstant::ZERO),
            Ok(None)
        );
        assert_eq!(
            lifetime.expiry(MonotonicInstant::from_nanos(u64::MAX - 1)),
            Err(ToastDeadlineError::Overflow)
        );
    }

    #[test]
    fn announcement_policy_types_priority_coalescing_and_redaction() {
        let key = ToastCoalescingKey::from_raw(7).unwrap();
        let policy = ToastAnnouncementPolicy::new(ToastAnnouncementPriority::Assertive)
            .coalescing(ToastCoalescingIntent::ReplaceMatching(key))
            .redaction(ToastRedactionIntent::AnnouncementAndDiagnostics);
        assert_eq!(ToastCoalescingKey::from_raw(0), None);
        assert_eq!(
            policy.intent(),
            ToastAnnouncementIntent {
                role: SemanticRole::Alert,
                priority: ToastAnnouncementPriority::Assertive,
                coalescing: ToastCoalescingIntent::ReplaceMatching(key),
                redaction: ToastRedactionIntent::AnnouncementAndDiagnostics,
            }
        );
        let toast = Toast::new(
            "Sensitive account event",
            ToastCorner::BlockEndInlineEnd,
            extent(),
            policy,
            ToastLifetime::persistent(),
        )
        .unwrap();
        let debug = format!("{toast:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("Sensitive account event"));
    }

    #[test]
    fn polite_toast_opens_safe_nonmodal_nonfocusable_and_expiring() {
        let harness = harness();
        let opened = toast()
            .open(
                MonotonicInstant::from_nanos(20),
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();

        assert_eq!(opened.corner, ResolvedToastCorner::BottomRight);
        assert_eq!(opened.placement.placement.rect.right(), 344.0);
        assert_eq!(opened.placement.placement.rect.bottom(), 216.0);
        assert_eq!(opened.announcement.role, SemanticRole::Status);
        assert_eq!(
            opened.dismissal.expiry,
            Some(ToastExpiryIntent {
                at: MonotonicInstant::from_nanos(5_000_000_020),
            })
        );
        assert_eq!(opened.focus_request(), OverlayFocusRequest::None);
        let controller = harness.controller.borrow();
        let entry = controller.entry(opened.id()).unwrap();
        assert_eq!(entry.modality, OverlayModality::NonModal);
        assert_eq!(entry.focus.initial, OverlayInitialFocus::None);
        assert_eq!(entry.focus.restoration, OverlayFocusRestoration::None);
        assert_eq!(entry.dismissal.outside_press, OutsidePressPolicy::Ignore);
        assert!(!entry.dismissal.focus_lost);
        assert!(!entry.dismissal.pointer_departure);
        assert!(!controller.state().background_is_inert);
    }

    #[test]
    fn logical_corner_resolves_in_rtl_without_leaving_safe_bounds() {
        let harness = harness();
        let mut environment = environment();
        environment.writing_direction = WritingDirection::RightToLeft;
        let opened = toast()
            .open(
                MonotonicInstant::ZERO,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment,
            )
            .unwrap();

        assert_eq!(opened.corner, ResolvedToastCorner::BottomLeft);
        assert_eq!(opened.placement.placement.rect.x, 32.0);
        assert_eq!(opened.placement.placement.rect.bottom(), 216.0);
    }

    #[test]
    fn assertive_persistent_toast_preserves_parent_and_manual_dismissal_intent() {
        let harness = harness();
        let parent = toast()
            .open(
                MonotonicInstant::ZERO,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            )
            .unwrap();
        let opened = Toast::new(
            "Connection lost",
            ToastCorner::BlockStartInlineStart,
            extent(),
            ToastAnnouncementPolicy::new(ToastAnnouncementPriority::Assertive),
            ToastLifetime::persistent(),
        )
        .unwrap()
        .parent(parent.id())
        .dismissal_policy(ToastDismissalPolicy {
            escape: false,
            manual: true,
        })
        .open(
            MonotonicInstant::ZERO,
            &mut harness.controller.borrow_mut(),
            harness.runtime.ui(),
            &environment(),
        )
        .unwrap();

        assert_eq!(opened.announcement.role, SemanticRole::Alert);
        assert_eq!(opened.dismissal.expiry, None);
        assert!(!opened.dismissal.escape);
        assert!(opened.dismissal.manual);
        let controller = harness.controller.borrow();
        let entry = controller.entry(opened.id()).unwrap();
        assert_eq!(entry.parent, Some(parent.id()));
        assert!(!entry.dismissal.escape);
    }

    #[test]
    fn scaling_placement_and_lifecycle_rejections_are_atomic_without_mounting() {
        let harness = harness();
        let before = harness.runtime.ui().nodes.alive().len();
        let mut scaled = environment();
        scaled.available_size = SizeF {
            width: 300.0,
            height: 180.0,
        };
        scaled.text_scale = 2.0;
        let opened = toast()
            .open(
                MonotonicInstant::ZERO,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &scaled,
            )
            .unwrap();
        assert_eq!(opened.extent.text_scale, 2.0);
        assert!(opened.placement.was_resized());
        assert_eq!(harness.runtime.ui().nodes.alive().len(), before);

        harness
            .controller
            .borrow_mut()
            .route(
                crate::components::application::ApplicationOverlayCommand::Dismiss {
                    id: opened.id(),
                    reason: crate::ui::DismissReason::Cancelled,
                },
            )
            .unwrap();
        let invalid_extent = Toast::new(
            "Invalid extent",
            ToastCorner::BlockEndInlineEnd,
            ToastExtent::new(
                SizeF {
                    width: 40.0,
                    height: 30.0,
                },
                SizeF {
                    width: 60.0,
                    height: 50.0,
                },
            ),
            ToastAnnouncementPolicy::default(),
            ToastLifetime::persistent(),
        )
        .unwrap();
        assert!(matches!(
            invalid_extent.open(
                MonotonicInstant::ZERO,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            ),
            Err(ToastError::Placement(_))
        ));
        assert_eq!(harness.controller.borrow().state().entry_count, 0);

        let unknown_parent = OverlayId::from_raw(u32::MAX, u32::MAX).unwrap();
        let invalid_parent = toast().parent(unknown_parent);
        assert_eq!(
            invalid_parent.open(
                MonotonicInstant::ZERO,
                &mut harness.controller.borrow_mut(),
                harness.runtime.ui(),
                &environment(),
            ),
            Err(ToastError::Controller(
                ApplicationOverlayControllerError::Host(ApplicationOverlayHostError::Lifecycle(
                    OverlayError::UnknownParent(unknown_parent)
                ))
            ))
        );
        assert_eq!(harness.controller.borrow().state().entry_count, 0);
    }
}
