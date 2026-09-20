//! Platform-neutral, per-view window service requests and completion values.
//!
//! This module defines capability discovery, validated request payloads, and typed applied
//! receipts. It does not own a native window, mutate a view snapshot, execute a request, create a
//! queue, or decide application exit policy. A service implementation admits requests and later
//! completes their [`crate::platform::contracts::AdmittedRequest`] token; the host's next revisioned view publication
//! remains
//! the source of observed truth.

use std::error::Error;
use std::fmt;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;

use crate::foundation::SizeF;

use super::ServiceKey;
use crate::platform::contracts::{
    CapabilityDescriptor, CapabilityLimit, RequestAdmission, Support, ViewId, ViewRevision,
};

/// Hard allocation bound for one portable window title, measured in UTF-8 bytes.
///
/// A capability may advertise a smaller limit. `Unspecified` capability limits do not relax this
/// bound and do not claim that a native platform accepts every value below it.
pub const MAX_WINDOW_TITLE_BYTES: usize = 4_096;

/// One independently discoverable operation in the per-view window service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowOperation {
    SetTitle,
    SetState,
    SetSizeConstraints,
    RequestAttention,
    RequestClose,
}

/// Service-specific limits attached to one window capability descriptor.
///
/// `maximum_title_bytes` is meaningful for [`WindowOperation::SetTitle`]. Other operations leave
/// it unspecified. This type does not treat an unspecified native limit as unlimited.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WindowCapabilityLimits {
    maximum_title_bytes: CapabilityLimit<NonZeroU32>,
}

impl WindowCapabilityLimits {
    pub const fn new(maximum_title_bytes: CapabilityLimit<NonZeroU32>) -> Self {
        Self {
            maximum_title_bytes,
        }
    }

    pub const fn unspecified() -> Self {
        Self::new(CapabilityLimit::Unspecified)
    }

    pub const fn maximum_title_bytes(self) -> CapabilityLimit<NonZeroU32> {
        self.maximum_title_bytes
    }
}

/// Capability metadata for one [`WindowOperation`] at one queried view.
pub type WindowCapability = CapabilityDescriptor<WindowOperation, WindowCapabilityLimits>;

/// Per-view query for one independently supported window operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowCapabilityQuery {
    view: ViewId,
    operation: WindowOperation,
}

impl WindowCapabilityQuery {
    pub const fn new(view: ViewId, operation: WindowOperation) -> Self {
        Self { view, operation }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn operation(self) -> WindowOperation {
        self.operation
    }
}

/// Bounded UTF-8 title payload.
///
/// Empty titles are valid. Debug output intentionally omits the text because window titles can
/// contain document names or other sensitive application data.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct WindowTitle(Arc<str>);

impl WindowTitle {
    pub fn new(title: impl AsRef<str>) -> Result<Self, WindowTitleError> {
        let title = title.as_ref();
        if title.len() > MAX_WINDOW_TITLE_BYTES {
            return Err(WindowTitleError::TooLong {
                byte_len: title.len(),
                maximum_bytes: MAX_WINDOW_TITLE_BYTES,
            });
        }
        Ok(Self(Arc::from(title)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn byte_len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for WindowTitle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowTitle")
            .field("byte_len", &self.byte_len())
            .field("redacted", &true)
            .finish()
    }
}

/// Failure to construct a bounded [`WindowTitle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowTitleError {
    TooLong {
        byte_len: usize,
        maximum_bytes: usize,
    },
}

impl fmt::Display for WindowTitleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong {
                byte_len,
                maximum_bytes,
            } => write!(
                formatter,
                "window title contains {byte_len} UTF-8 bytes; maximum is {maximum_bytes}"
            ),
        }
    }
}

impl Error for WindowTitleError {}

/// Desired native window state.
///
/// This is an intention, not observed visibility or lifecycle state. In particular, minimizing a
/// window does not let portable code invent a [`crate::platform::contracts::VisibilityState`] publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowStateIntent {
    Normal,
    Minimized,
    Maximized,
    Fullscreen,
}

/// Whether a logical size is the minimum or maximum constraint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowConstraintBound {
    Minimum,
    Maximum,
}

/// Logical size axis involved in invalid constraint input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowConstraintAxis {
    Width,
    Height,
}

/// Validated optional minimum and maximum sizes in view-logical units.
///
/// `None` means the service request does not impose that bound. Present sizes have finite,
/// strictly positive dimensions. When both are present, each minimum dimension is no greater than
/// its maximum. Equal bounds express a fixed size without a separate `resizable` flag.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowSizeConstraints {
    minimum: Option<SizeF>,
    maximum: Option<SizeF>,
}

impl WindowSizeConstraints {
    pub fn new(
        minimum: Option<SizeF>,
        maximum: Option<SizeF>,
    ) -> Result<Self, WindowSizeConstraintsError> {
        if let Some(minimum) = minimum {
            validate_constraint_size(WindowConstraintBound::Minimum, minimum)?;
        }
        if let Some(maximum) = maximum {
            validate_constraint_size(WindowConstraintBound::Maximum, maximum)?;
        }
        if let (Some(minimum), Some(maximum)) = (minimum, maximum) {
            if minimum.width > maximum.width {
                return Err(WindowSizeConstraintsError::MinimumExceedsMaximum {
                    axis: WindowConstraintAxis::Width,
                    minimum: minimum.width,
                    maximum: maximum.width,
                });
            }
            if minimum.height > maximum.height {
                return Err(WindowSizeConstraintsError::MinimumExceedsMaximum {
                    axis: WindowConstraintAxis::Height,
                    minimum: minimum.height,
                    maximum: maximum.height,
                });
            }
        }
        Ok(Self { minimum, maximum })
    }

    pub const fn unconstrained() -> Self {
        Self {
            minimum: None,
            maximum: None,
        }
    }

    pub const fn minimum(self) -> Option<SizeF> {
        self.minimum
    }

    pub const fn maximum(self) -> Option<SizeF> {
        self.maximum
    }

    pub const fn is_unconstrained(self) -> bool {
        self.minimum.is_none() && self.maximum.is_none()
    }

    pub fn is_fixed(self) -> bool {
        self.minimum == self.maximum && self.minimum.is_some()
    }
}

fn validate_constraint_size(
    bound: WindowConstraintBound,
    size: SizeF,
) -> Result<(), WindowSizeConstraintsError> {
    for (axis, value) in [
        (WindowConstraintAxis::Width, size.width),
        (WindowConstraintAxis::Height, size.height),
    ] {
        if !value.is_finite() {
            return Err(WindowSizeConstraintsError::NonFinite { bound, axis });
        }
        if value <= 0.0 {
            return Err(WindowSizeConstraintsError::NonPositive { bound, axis });
        }
    }
    Ok(())
}

/// Failure to construct coherent view-logical window size constraints.
///
/// Invalid floating-point payloads are intentionally omitted from Debug and Display output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowSizeConstraintsError {
    NonFinite {
        bound: WindowConstraintBound,
        axis: WindowConstraintAxis,
    },
    NonPositive {
        bound: WindowConstraintBound,
        axis: WindowConstraintAxis,
    },
    MinimumExceedsMaximum {
        axis: WindowConstraintAxis,
        minimum: f32,
        maximum: f32,
    },
}

impl fmt::Display for WindowSizeConstraintsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite { bound, axis } => {
                write!(
                    formatter,
                    "{bound:?} window constraint {axis:?} is not finite"
                )
            }
            Self::NonPositive { bound, axis } => write!(
                formatter,
                "{bound:?} window constraint {axis:?} is not strictly positive"
            ),
            Self::MinimumExceedsMaximum {
                axis,
                minimum,
                maximum,
            } => write!(
                formatter,
                "minimum window constraint {axis:?} {minimum} exceeds maximum {maximum}"
            ),
        }
    }
}

impl Error for WindowSizeConstraintsError {}

/// Desired user-attention behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowAttentionIntent {
    Clear,
    Informational,
    Critical,
}

/// Why portable code asks the service to close a view.
///
/// Accepting a routed close request cites the exact view revision the application observed.
/// Programmatic requests have no fabricated close event or revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowCloseIntent {
    ApplicationRequested,
    AcceptedRequest { observed_revision: ViewRevision },
}

/// Request to update the title of one exact view generation.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct WindowTitleRequest {
    view: ViewId,
    title: WindowTitle,
}

impl WindowTitleRequest {
    pub const fn new(view: ViewId, title: WindowTitle) -> Self {
        Self { view, title }
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn title(&self) -> &WindowTitle {
        &self.title
    }
}

impl fmt::Debug for WindowTitleRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowTitleRequest")
            .field("view", &self.view)
            .field("title", &self.title)
            .finish()
    }
}

/// Request to change the native state of one exact view generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowStateRequest {
    view: ViewId,
    intent: WindowStateIntent,
}

impl WindowStateRequest {
    pub const fn new(view: ViewId, intent: WindowStateIntent) -> Self {
        Self { view, intent }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn intent(self) -> WindowStateIntent {
        self.intent
    }
}

/// Request to replace the logical size constraints of one exact view generation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowSizeConstraintsRequest {
    view: ViewId,
    constraints: WindowSizeConstraints,
}

impl WindowSizeConstraintsRequest {
    pub const fn new(view: ViewId, constraints: WindowSizeConstraints) -> Self {
        Self { view, constraints }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn constraints(self) -> WindowSizeConstraints {
        self.constraints
    }
}

/// Request to change user-attention signaling for one exact view generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowAttentionRequest {
    view: ViewId,
    intent: WindowAttentionIntent,
}

impl WindowAttentionRequest {
    pub const fn new(view: ViewId, intent: WindowAttentionIntent) -> Self {
        Self { view, intent }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn intent(self) -> WindowAttentionIntent {
        self.intent
    }
}

/// Request to close one exact view generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowCloseRequest {
    view: ViewId,
    intent: WindowCloseIntent,
}

impl WindowCloseRequest {
    pub const fn new(view: ViewId, intent: WindowCloseIntent) -> Self {
        Self { view, intent }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn intent(self) -> WindowCloseIntent {
        self.intent
    }
}

/// Immediate failure to admit an otherwise well-formed window request.
///
/// Permission denial, unsupported execution, cancellation, staleness after admission, and native
/// failure remain distinct terminal [`crate::platform::contracts::RequestOutcome`] variants rather than this error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAdmissionError {
    ViewUnavailable {
        view: ViewId,
    },
    CapabilityChanged {
        view: ViewId,
        operation: WindowOperation,
    },
    TitleExceedsCapability {
        view: ViewId,
        byte_len: usize,
        maximum_bytes: NonZeroU32,
    },
}

impl fmt::Display for WindowAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ViewUnavailable { view } => {
                write!(formatter, "window service view {view} is unavailable")
            }
            Self::CapabilityChanged { view, operation } => write!(
                formatter,
                "window service capability {operation:?} changed for view {view} before admission"
            ),
            Self::TitleExceedsCapability {
                view,
                byte_len,
                maximum_bytes,
            } => write!(
                formatter,
                "window title for view {view} contains {byte_len} UTF-8 bytes; capability maximum is {maximum_bytes}"
            ),
        }
    }
}

impl Error for WindowAdmissionError {}

/// Immediate admission result for a typed window operation.
pub type WindowRequestAdmission<T> = RequestAdmission<T, WindowAdmissionError>;

/// Applied receipt for a title request.
///
/// The title is omitted to keep completion diagnostics payload-redacted. This receipt does not
/// publish or assert observed native title state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowTitleApplied {
    view: ViewId,
    title_byte_len: usize,
}

impl WindowTitleApplied {
    pub fn from_request(request: &WindowTitleRequest) -> Self {
        Self {
            view: request.view,
            title_byte_len: request.title.byte_len(),
        }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn title_byte_len(self) -> usize {
        self.title_byte_len
    }
}

/// Applied receipt for a native state request; observed state still comes from a view publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowStateApplied {
    view: ViewId,
    intent: WindowStateIntent,
}

impl WindowStateApplied {
    pub const fn from_request(request: WindowStateRequest) -> Self {
        Self {
            view: request.view,
            intent: request.intent,
        }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn intent(self) -> WindowStateIntent {
        self.intent
    }
}

/// Applied receipt for a logical size-constraint request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowSizeConstraintsApplied {
    view: ViewId,
    constraints: WindowSizeConstraints,
}

impl WindowSizeConstraintsApplied {
    pub const fn from_request(request: WindowSizeConstraintsRequest) -> Self {
        Self {
            view: request.view,
            constraints: request.constraints,
        }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn constraints(self) -> WindowSizeConstraints {
        self.constraints
    }
}

/// Applied receipt for an attention request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowAttentionApplied {
    view: ViewId,
    intent: WindowAttentionIntent,
}

impl WindowAttentionApplied {
    pub const fn from_request(request: WindowAttentionRequest) -> Self {
        Self {
            view: request.view,
            intent: request.intent,
        }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn intent(self) -> WindowAttentionIntent {
        self.intent
    }
}

/// Applied receipt for a close request.
///
/// This means the close intention was accepted and completed by the service. It does not assert
/// that the view is closed; forced destruction and the next [`crate::platform::contracts::ViewSnapshot`] remain distinct
/// host observations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowCloseApplied {
    view: ViewId,
    intent: WindowCloseIntent,
}

impl WindowCloseApplied {
    pub const fn from_request(request: WindowCloseRequest) -> Self {
        Self {
            view: request.view,
            intent: request.intent,
        }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn intent(self) -> WindowCloseIntent {
        self.intent
    }
}

/// Narrow service surface for capability discovery and typed per-view request admission.
///
/// Methods only admit requests. They do not synchronously claim observed platform state; each
/// admitted token receives one later [`crate::platform::contracts::RequestOutcome`]. Implementations obey the execution
/// requirement reported by `capability` and belong to a host or platform adapter.
pub trait WindowService {
    fn capability(&self, query: WindowCapabilityQuery) -> Support<WindowCapability>;

    fn set_title(&self, request: WindowTitleRequest) -> WindowRequestAdmission<WindowTitleApplied>;

    fn set_state(&self, request: WindowStateRequest) -> WindowRequestAdmission<WindowStateApplied>;

    fn set_size_constraints(
        &self,
        request: WindowSizeConstraintsRequest,
    ) -> WindowRequestAdmission<WindowSizeConstraintsApplied>;

    fn request_attention(
        &self,
        request: WindowAttentionRequest,
    ) -> WindowRequestAdmission<WindowAttentionApplied>;

    fn request_close(
        &self,
        request: WindowCloseRequest,
    ) -> WindowRequestAdmission<WindowCloseApplied>;
}

/// Type-level registry key for an owner-local window service handle.
pub enum WindowServiceKey {}

impl ServiceKey for WindowServiceKey {
    type Handle = Rc<dyn WindowService>;
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::hash::Hash;

    use crate::platform::contracts::{
        AdmittedRequest, ExecutionRequirement, PermissionState, RequestId, RequestOutcome,
        UserGestureRequirement,
    };

    use super::*;

    fn view() -> ViewId {
        ViewId::from_raw(9, 3).unwrap()
    }

    fn size(width: f32, height: f32) -> SizeF {
        SizeF { width, height }
    }

    fn assert_value<T: Copy + Eq + Hash + Send + Sync + 'static>() {}

    #[test]
    fn titles_are_utf8_bounded_and_debug_redacted() {
        let sensitive = "project-orchid/customer-record";
        let title = WindowTitle::new(sensitive).unwrap();
        assert_eq!(title.as_str(), sensitive);
        assert_eq!(title.byte_len(), sensitive.len());
        assert!(!title.is_empty());
        assert!(WindowTitle::new("").unwrap().is_empty());

        let request = WindowTitleRequest::new(view(), title);
        let debug = format!("{request:?}");
        assert!(debug.contains("byte_len"));
        assert!(debug.contains("redacted"));
        assert!(!debug.contains("project-orchid"));

        let oversized = "x".repeat(MAX_WINDOW_TITLE_BYTES + 1);
        assert_eq!(
            WindowTitle::new(oversized),
            Err(WindowTitleError::TooLong {
                byte_len: MAX_WINDOW_TITLE_BYTES + 1,
                maximum_bytes: MAX_WINDOW_TITLE_BYTES,
            })
        );
    }

    #[test]
    fn logical_constraints_reject_malformed_or_inverted_bounds() {
        let fixed =
            WindowSizeConstraints::new(Some(size(640.0, 480.0)), Some(size(640.0, 480.0))).unwrap();
        assert!(fixed.is_fixed());
        assert_eq!(fixed.minimum(), Some(size(640.0, 480.0)));
        assert!(!fixed.is_unconstrained());
        assert!(WindowSizeConstraints::unconstrained().is_unconstrained());

        assert_eq!(
            WindowSizeConstraints::new(Some(size(f32::NAN, 10.0)), None),
            Err(WindowSizeConstraintsError::NonFinite {
                bound: WindowConstraintBound::Minimum,
                axis: WindowConstraintAxis::Width,
            })
        );
        assert_eq!(
            WindowSizeConstraints::new(None, Some(size(100.0, 0.0))),
            Err(WindowSizeConstraintsError::NonPositive {
                bound: WindowConstraintBound::Maximum,
                axis: WindowConstraintAxis::Height,
            })
        );
        assert_eq!(
            WindowSizeConstraints::new(Some(size(800.0, 400.0)), Some(size(700.0, 500.0))),
            Err(WindowSizeConstraintsError::MinimumExceedsMaximum {
                axis: WindowConstraintAxis::Width,
                minimum: 800.0,
                maximum: 700.0,
            })
        );
    }

    #[test]
    fn capability_query_keeps_view_operation_limits_and_policy_distinct() {
        let query = WindowCapabilityQuery::new(view(), WindowOperation::RequestAttention);
        let maximum = NonZeroU32::new(256).unwrap();
        let capability = WindowCapability::new(
            WindowOperation::SetTitle,
            WindowCapabilityLimits::new(CapabilityLimit::Bounded(maximum)),
            PermissionState::NotRequired,
            ExecutionRequirement::HostEventLoop,
            UserGestureRequirement::RecentRequired,
        );

        assert_eq!(query.view(), view());
        assert_eq!(query.operation(), WindowOperation::RequestAttention);
        assert_eq!(capability.operations(), &WindowOperation::SetTitle);
        assert_eq!(
            capability.limits().maximum_title_bytes().into_bound(),
            Some(maximum)
        );
        assert_eq!(capability.execution(), ExecutionRequirement::HostEventLoop);
        assert!(capability.user_gesture().is_required());
        assert_eq!(
            WindowCapabilityLimits::unspecified()
                .maximum_title_bytes()
                .into_bound(),
            None
        );
        assert_value::<WindowCapabilityQuery>();
    }

    #[test]
    fn typed_receipts_preserve_intentions_without_claiming_observed_state() {
        let state_request = WindowStateRequest::new(view(), WindowStateIntent::Fullscreen);
        let state = WindowStateApplied::from_request(state_request);
        assert_eq!(state.view(), view());
        assert_eq!(state.intent(), WindowStateIntent::Fullscreen);

        let constraints =
            WindowSizeConstraints::new(Some(size(320.0, 240.0)), Some(size(1920.0, 1080.0)))
                .unwrap();
        let constraints = WindowSizeConstraintsApplied::from_request(
            WindowSizeConstraintsRequest::new(view(), constraints),
        );
        assert_eq!(
            constraints.constraints().maximum(),
            Some(size(1920.0, 1080.0))
        );

        let attention_request =
            WindowAttentionRequest::new(view(), WindowAttentionIntent::Critical);
        let attention = WindowAttentionApplied::from_request(attention_request);
        assert_eq!(attention.intent(), WindowAttentionIntent::Critical);

        let close_intent = WindowCloseIntent::AcceptedRequest {
            observed_revision: ViewRevision::from_raw(12).unwrap(),
        };
        let close_request = WindowCloseRequest::new(view(), close_intent);
        let close = WindowCloseApplied::from_request(close_request);
        assert_eq!(close.intent(), close_intent);

        let title_request =
            WindowTitleRequest::new(view(), WindowTitle::new("private title").unwrap());
        let title = WindowTitleApplied::from_request(&title_request);
        assert_eq!(title.title_byte_len(), 13);
        assert!(!format!("{title:?}").contains("private title"));
        assert_value::<WindowStateApplied>();
        assert_value::<WindowAttentionApplied>();
        assert_value::<WindowCloseApplied>();
    }

    struct RecordingService {
        next_request: Cell<u64>,
    }

    impl RecordingService {
        fn admit<T>(&self) -> AdmittedRequest<T> {
            let next = self.next_request.get() + 1;
            self.next_request.set(next);
            AdmittedRequest::new(RequestId::from_raw(next).unwrap())
        }
    }

    impl WindowService for RecordingService {
        fn capability(&self, query: WindowCapabilityQuery) -> Support<WindowCapability> {
            Support::Available(WindowCapability::new(
                query.operation(),
                WindowCapabilityLimits::unspecified(),
                PermissionState::NotRequired,
                ExecutionRequirement::RuntimeOwner,
                UserGestureRequirement::NotRequired,
            ))
        }

        fn set_title(
            &self,
            _request: WindowTitleRequest,
        ) -> WindowRequestAdmission<WindowTitleApplied> {
            Ok(self.admit())
        }

        fn set_state(
            &self,
            _request: WindowStateRequest,
        ) -> WindowRequestAdmission<WindowStateApplied> {
            Ok(self.admit())
        }

        fn set_size_constraints(
            &self,
            _request: WindowSizeConstraintsRequest,
        ) -> WindowRequestAdmission<WindowSizeConstraintsApplied> {
            Ok(self.admit())
        }

        fn request_attention(
            &self,
            _request: WindowAttentionRequest,
        ) -> WindowRequestAdmission<WindowAttentionApplied> {
            Ok(self.admit())
        }

        fn request_close(
            &self,
            _request: WindowCloseRequest,
        ) -> WindowRequestAdmission<WindowCloseApplied> {
            Ok(self.admit())
        }
    }

    #[test]
    fn object_safe_service_only_admits_typed_requests_and_completion_stays_explicit() {
        let service: Rc<dyn WindowService> = Rc::new(RecordingService {
            next_request: Cell::new(40),
        });
        let capability = service
            .capability(WindowCapabilityQuery::new(
                view(),
                WindowOperation::SetTitle,
            ))
            .into_available()
            .unwrap();
        assert_eq!(capability.operations(), &WindowOperation::SetTitle);

        let request = WindowTitleRequest::new(view(), WindowTitle::new("admitted title").unwrap());
        let applied = WindowTitleApplied::from_request(&request);
        let admitted = service.set_title(request).unwrap();
        assert_eq!(admitted.request_id().get(), 41);
        let completion = admitted.complete(RequestOutcome::Applied(applied));
        assert_eq!(completion.request_id().get(), 41);
        assert_eq!(completion.outcome().applied().unwrap().view(), view());
    }

    #[test]
    fn immediate_admission_errors_do_not_collapse_terminal_outcomes() {
        let maximum_bytes = NonZeroU32::new(8).unwrap();
        let error = WindowAdmissionError::TitleExceedsCapability {
            view: view(),
            byte_len: 9,
            maximum_bytes,
        };
        assert!(error.to_string().contains("capability maximum is 8"));

        let terminal = [
            RequestOutcome::<WindowTitleApplied>::Denied,
            RequestOutcome::Unsupported,
            RequestOutcome::Cancelled,
            RequestOutcome::Stale,
        ];
        assert!(terminal[0].is_denied());
        assert!(terminal[1].is_unsupported());
        assert!(terminal[2].is_cancelled());
        assert!(terminal[3].is_stale());
    }
}
