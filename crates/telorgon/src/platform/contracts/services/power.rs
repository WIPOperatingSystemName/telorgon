//! Platform-neutral scoped power-inhibition admission.
//!
//! Portable code may request semantic idle or system-sleep inhibition for an explicit application
//! or view scope. A successful completion owns one adapter-provided RAII lease whose status can
//! report revocation and whose destructor releases the native effect. This module accepts no
//! duration, deadline, native inhibitor, or platform policy object and owns no power API, callback,
//! queue, executor, thread, timer, or event loop.

use std::error::Error;
use std::fmt;
use std::num::NonZeroU16;
use std::rc::Rc;

use super::ServiceKey;
use crate::platform::contracts::{
    CapabilityDescriptor, ExecutionRequirement, PermissionState, PowerInhibitionLeaseId,
    RequestAdmission, Support, UserGestureGrantHandle, UserGestureRequirement, ViewId,
};

/// Neutral hard bound on concurrently retained power-inhibition leases per service scope.
pub const MAX_POWER_INHIBITION_LEASES: u16 = 64;

/// Semantic power transition a lease asks the host to inhibit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerInhibitionKind {
    /// Inhibit automatic idle response such as dimming, blanking, or an idle transition.
    Idle,
    /// Inhibit automatic system sleep where platform policy permits it.
    SystemSleep,
}

/// Owner scope of one power-inhibition intention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerInhibitionScope {
    Application,
    View(ViewId),
}

impl PowerInhibitionScope {
    pub const fn view(self) -> Option<ViewId> {
        match self {
            Self::Application => None,
            Self::View(view) => Some(view),
        }
    }
}

/// Portable policy category explaining why inhibition is requested.
///
/// These categories permit adapter policy without carrying arbitrary text or choosing native
/// behavior in portable code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerInhibitionReason {
    InteractiveActivity,
    MediaPlayback,
    Presentation,
    UserInitiatedWork,
}

/// Host-observed policy state for the queried inhibition scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PowerPolicyState {
    Allowed,
    Denied,
    #[default]
    Unknown,
}

impl PowerPolicyState {
    pub const fn allows_inhibition(self) -> bool {
        matches!(self, Self::Allowed)
    }
}

/// Independently discoverable power-inhibition operations and scopes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PowerOperations {
    idle_inhibition: bool,
    system_sleep_inhibition: bool,
    application_scope: bool,
    view_scope: bool,
}

impl PowerOperations {
    pub const fn new(
        idle_inhibition: bool,
        system_sleep_inhibition: bool,
        application_scope: bool,
        view_scope: bool,
    ) -> Self {
        Self {
            idle_inhibition,
            system_sleep_inhibition,
            application_scope,
            view_scope,
        }
    }

    pub const fn supports_idle_inhibition(self) -> bool {
        self.idle_inhibition
    }

    pub const fn supports_system_sleep_inhibition(self) -> bool {
        self.system_sleep_inhibition
    }

    pub const fn supports_application_scope(self) -> bool {
        self.application_scope
    }

    pub const fn supports_view_scope(self) -> bool {
        self.view_scope
    }

    pub const fn supports_kind(self, kind: PowerInhibitionKind) -> bool {
        match kind {
            PowerInhibitionKind::Idle => self.idle_inhibition,
            PowerInhibitionKind::SystemSleep => self.system_sleep_inhibition,
        }
    }

    pub const fn supports_scope(self, scope: PowerInhibitionScope) -> bool {
        match scope {
            PowerInhibitionScope::Application => self.application_scope,
            PowerInhibitionScope::View(_) => self.view_scope,
        }
    }
}

/// Adapter-narrowed concurrent-lease capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PowerLimits {
    maximum_concurrent_leases: NonZeroU16,
}

impl PowerLimits {
    pub const fn new(maximum_concurrent_leases: NonZeroU16) -> Result<Self, PowerLimitError> {
        if maximum_concurrent_leases.get() > MAX_POWER_INHIBITION_LEASES {
            return Err(PowerLimitError::LeaseLimitTooLarge);
        }
        Ok(Self {
            maximum_concurrent_leases,
        })
    }

    pub const fn maximum_concurrent_leases(self) -> NonZeroU16 {
        self.maximum_concurrent_leases
    }
}

impl Default for PowerLimits {
    fn default() -> Self {
        Self {
            maximum_concurrent_leases: NonZeroU16::new(MAX_POWER_INHIBITION_LEASES)
                .expect("power-inhibition lease hard bound is nonzero"),
        }
    }
}

/// Invalid adapter-advertised power-service limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerLimitError {
    LeaseLimitTooLarge,
}

impl fmt::Display for PowerLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("power-inhibition lease limit exceeds the neutral hard bound")
    }
}

impl Error for PowerLimitError {}

/// Complete power capability for one exact query scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PowerCapability {
    descriptor: CapabilityDescriptor<PowerOperations, PowerLimits>,
    policy: PowerPolicyState,
}

impl PowerCapability {
    pub const fn new(
        descriptor: CapabilityDescriptor<PowerOperations, PowerLimits>,
        policy: PowerPolicyState,
    ) -> Self {
        Self { descriptor, policy }
    }

    pub const fn operations(&self) -> &PowerOperations {
        self.descriptor.operations()
    }

    pub const fn limits(&self) -> &PowerLimits {
        self.descriptor.limits()
    }

    pub const fn permission(self) -> PermissionState {
        self.descriptor.permission()
    }

    pub const fn execution(self) -> ExecutionRequirement {
        self.descriptor.execution()
    }

    pub const fn user_gesture(self) -> UserGestureRequirement {
        self.descriptor.user_gesture()
    }

    pub const fn policy(self) -> PowerPolicyState {
        self.policy
    }

    pub fn into_descriptor(self) -> CapabilityDescriptor<PowerOperations, PowerLimits> {
        self.descriptor
    }
}

/// Exact application or view scope used for a capability query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PowerCapabilityQuery {
    scope: PowerInhibitionScope,
}

impl PowerCapabilityQuery {
    pub const fn new(scope: PowerInhibitionScope) -> Self {
        Self { scope }
    }

    pub const fn scope(self) -> PowerInhibitionScope {
        self.scope
    }
}

/// One semantic scoped inhibition intention with optional opaque recent-gesture evidence.
pub struct PowerInhibitionRequest {
    scope: PowerInhibitionScope,
    kind: PowerInhibitionKind,
    reason: PowerInhibitionReason,
    user_gesture: Option<UserGestureGrantHandle>,
}

impl PowerInhibitionRequest {
    pub const fn new(
        scope: PowerInhibitionScope,
        kind: PowerInhibitionKind,
        reason: PowerInhibitionReason,
    ) -> Self {
        Self {
            scope,
            kind,
            reason,
            user_gesture: None,
        }
    }

    pub fn with_user_gesture(
        scope: PowerInhibitionScope,
        kind: PowerInhibitionKind,
        reason: PowerInhibitionReason,
        user_gesture: UserGestureGrantHandle,
    ) -> Self {
        Self {
            scope,
            kind,
            reason,
            user_gesture: Some(user_gesture),
        }
    }

    pub const fn scope(&self) -> PowerInhibitionScope {
        self.scope
    }

    pub const fn kind(&self) -> PowerInhibitionKind {
        self.kind
    }

    pub const fn reason(&self) -> PowerInhibitionReason {
        self.reason
    }

    pub const fn has_user_gesture(&self) -> bool {
        self.user_gesture.is_some()
    }

    pub fn into_parts(
        self,
    ) -> (
        PowerInhibitionScope,
        PowerInhibitionKind,
        PowerInhibitionReason,
        Option<UserGestureGrantHandle>,
    ) {
        (self.scope, self.kind, self.reason, self.user_gesture)
    }
}

impl fmt::Debug for PowerInhibitionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PowerInhibitionRequest")
            .field("scope", &self.scope)
            .field("kind", &self.kind)
            .field("reason", &self.reason)
            .field("has_user_gesture", &self.user_gesture.is_some())
            .finish_non_exhaustive()
    }
}

/// Why a formerly active power-inhibition lease no longer applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerInhibitionRevocation {
    ScopeClosed,
    ScopeSuspended,
    PermissionChanged,
    PolicyChanged,
    HostRevoked,
}

/// Current adapter-reported state of one power-inhibition lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerInhibitionLeaseStatus {
    Active,
    Revoked(PowerInhibitionRevocation),
}

/// Adapter-owned RAII lease for one active idle or system-sleep inhibition.
///
/// Concrete implementations must release the native effect from `Drop`. The lease is intentionally
/// not cloneable. Scope closure/suspension, permission or policy changes, and host revocation may
/// change [`Self::status`] before the lease is dropped.
pub trait PowerInhibitionLease: fmt::Debug {
    fn id(&self) -> PowerInhibitionLeaseId;
    fn scope(&self) -> PowerInhibitionScope;
    fn kind(&self) -> PowerInhibitionKind;
    fn reason(&self) -> PowerInhibitionReason;
    fn status(&self) -> PowerInhibitionLeaseStatus;
}

/// Non-cloneable owner of one adapter-provided power-inhibition lease.
pub type PowerInhibitionLeaseHandle = Box<dyn PowerInhibitionLease>;

/// Immediate rejection before a power-inhibition request is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerAdmissionError {
    UnsupportedOperation { kind: PowerInhibitionKind },
    UnsupportedScope { scope: PowerInhibitionScope },
    ViewUnavailable { view: ViewId },
    PolicyDenied,
    PolicyUnknown,
    PermissionDenied,
    AuthorizationRequired,
    UserGestureRequired,
    InvalidUserGesture,
    LeaseLimitReached { maximum: NonZeroU16 },
    CapabilityChanged,
    CapacityExceeded,
}

impl fmt::Display for PowerAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedOperation { .. } => "power-inhibition operation is unsupported",
            Self::UnsupportedScope { .. } => "power-inhibition scope is unsupported",
            Self::ViewUnavailable { .. } => "power-inhibition view is unavailable",
            Self::PolicyDenied => "power inhibition is denied by current host policy",
            Self::PolicyUnknown => "power-inhibition policy state is unknown",
            Self::PermissionDenied => "power-inhibition permission is denied",
            Self::AuthorizationRequired => "power inhibition requires authorization",
            Self::UserGestureRequired => "power inhibition requires a user gesture",
            Self::InvalidUserGesture => "power-inhibition gesture evidence is invalid",
            Self::LeaseLimitReached { .. } => "power-inhibition lease limit was reached",
            Self::CapabilityChanged => "power capability changed before admission",
            Self::CapacityExceeded => "power-inhibition admission capacity was exceeded",
        })
    }
}

impl Error for PowerAdmissionError {}

pub type PowerInhibitionAdmission =
    RequestAdmission<PowerInhibitionLeaseHandle, PowerAdmissionError>;

/// Object-safe power capability and scoped inhibition-admission boundary.
pub trait PowerService {
    fn capability(&self, query: PowerCapabilityQuery) -> Support<PowerCapability>;

    fn acquire_inhibition(&self, request: PowerInhibitionRequest) -> PowerInhibitionAdmission;
}

pub enum PowerServiceKey {}

impl ServiceKey for PowerServiceKey {
    type Handle = Rc<dyn PowerService>;
}
