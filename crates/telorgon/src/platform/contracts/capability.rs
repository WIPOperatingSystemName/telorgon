//! Shared capability availability, permission, limit, and execution values.

use std::any::Any;
use std::fmt;

/// Runtime support for a typed capability descriptor at its queried scope.
///
/// The payload normally is a [`CapabilityDescriptor`] whose operation and limit records are owned
/// by one narrow service. Permission denial does not make a compiled, supported capability
/// unavailable; it remains an available descriptor with the corresponding [`PermissionState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Support<T> {
    Unavailable(UnavailableReason),
    Available(T),
}

impl<T> Support<T> {
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    pub const fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }

    pub const fn as_ref(&self) -> Support<&T> {
        match self {
            Self::Unavailable(reason) => Support::Unavailable(*reason),
            Self::Available(value) => Support::Available(value),
        }
    }

    pub const fn unavailable_reason(&self) -> Option<UnavailableReason> {
        match self {
            Self::Unavailable(reason) => Some(*reason),
            Self::Available(_) => None,
        }
    }

    pub fn into_available(self) -> Option<T> {
        match self {
            Self::Unavailable(_) => None,
            Self::Available(value) => Some(value),
        }
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Support<U> {
        match self {
            Self::Unavailable(reason) => Support::Unavailable(reason),
            Self::Available(value) => Support::Available(map(value)),
        }
    }
}

/// Why a capability descriptor cannot be supplied at the queried scope.
///
/// These reasons describe support discovery, not request failure and not permission state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnavailableReason {
    /// The adapter was excluded by the selected compile-time feature profile.
    AdapterNotCompiled,
    /// The adapter was compiled but could not be initialized or discovered.
    AdapterUnavailable,
    /// The current operating platform cannot provide the capability.
    UnsupportedByPlatform,
    /// The capability is not meaningful or present for this host, view, or data-offer scope.
    UnavailableInScope,
    /// Host/application policy disables the capability in this environment.
    DisabledByPolicy,
    /// The host did not supply the execution context required by the adapter.
    ExecutionContextUnavailable,
    /// The capability may become available after an external platform state change.
    TemporarilyUnavailable,
}

impl fmt::Display for UnavailableReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AdapterNotCompiled => "adapter not compiled",
            Self::AdapterUnavailable => "adapter unavailable",
            Self::UnsupportedByPlatform => "unsupported by platform",
            Self::UnavailableInScope => "unavailable in this scope",
            Self::DisabledByPolicy => "disabled by policy",
            Self::ExecutionContextUnavailable => "required execution context unavailable",
            Self::TemporarilyUnavailable => "temporarily unavailable",
        })
    }
}

/// Observed permission state for one supported capability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PermissionState {
    NotRequired,
    #[default]
    Unknown,
    PromptRequired,
    Granted,
    Denied,
    Restricted,
}

impl PermissionState {
    /// Whether the permission state currently permits use without a permission transition.
    pub const fn allows_use(self) -> bool {
        matches!(self, Self::NotRequired | Self::Granted)
    }

    /// Whether a request must be associated with an explicit permission prompt flow.
    pub const fn requires_prompt(self) -> bool {
        matches!(self, Self::PromptRequired)
    }

    /// Whether current policy denies use without claiming the capability is unsupported.
    pub const fn blocks_use(self) -> bool {
        matches!(self, Self::Denied | Self::Restricted)
    }
}

/// Host execution context required by an available capability.
///
/// This is declarative. It never creates a thread, executor, event loop, or callback channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExecutionRequirement {
    /// The adapter accepts requests from any thread allowed by its service handle.
    AnyThread,
    /// Work must execute on the portable runtime's single-writer owner.
    RuntimeOwner,
    /// Work must be marshalled to the host event-loop owner.
    HostEventLoop,
    /// Work must execute on the operating platform's designated main thread.
    PlatformMainThread,
    /// Work requires an executor supplied or explicitly enabled by the host.
    HostExecutor,
}

/// Whether an operation requires a recent host-validated user gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UserGestureRequirement {
    NotRequired,
    RecentRequired,
}

impl UserGestureRequirement {
    pub const fn is_required(self) -> bool {
        matches!(self, Self::RecentRequired)
    }
}

/// Opaque, scoped, single-use evidence of a host-observed user gesture.
///
/// Adapter crates implement this trait with their private serial/token representation. Portable
/// code cannot inspect its source serial, seat, token, time, or scope. The boxed handle is
/// intentionally neither `Clone` nor `Copy`; request APIs consume it and the receiving adapter
/// validates concrete type, view, age, generation, focus, scope, and single use before making a
/// native call. Debug formatting is deliberately not required so native token data cannot leak
/// through a generic request diagnostic.
pub trait UserGestureGrant: 'static {
    /// Returns the adapter-private value for validation and consumption.
    ///
    /// Implementations normally downcast this only inside the same adapter that issued the grant.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

/// Non-cloneable owner of one adapter-issued recent-user-gesture grant.
pub type UserGestureGrantHandle = Box<dyn UserGestureGrant>;

/// A typed maximum reported by a capability query.
///
/// `Unspecified` makes no promise and is not synonymous with an unlimited resource. Services use
/// `Bounded` with their own validated numeric or geometric type when a limit is known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CapabilityLimit<T> {
    #[default]
    Unspecified,
    Bounded(T),
}

impl<T> CapabilityLimit<T> {
    pub const fn as_ref(&self) -> CapabilityLimit<&T> {
        match self {
            Self::Unspecified => CapabilityLimit::Unspecified,
            Self::Bounded(value) => CapabilityLimit::Bounded(value),
        }
    }

    pub const fn is_bounded(&self) -> bool {
        matches!(self, Self::Bounded(_))
    }

    pub fn into_bound(self) -> Option<T> {
        match self {
            Self::Unspecified => None,
            Self::Bounded(value) => Some(value),
        }
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> CapabilityLimit<U> {
        match self {
            Self::Unspecified => CapabilityLimit::Unspecified,
            Self::Bounded(value) => CapabilityLimit::Bounded(map(value)),
        }
    }
}

/// Explicit marker for a capability that has no service-specific limit record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NoCapabilityLimits;

/// Common metadata for one available typed capability.
///
/// Service packages define `Operations` and `Limits`; this common owner keeps permission,
/// execution, and gesture semantics consistent without knowing or invoking service methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CapabilityDescriptor<Operations, Limits = NoCapabilityLimits> {
    operations: Operations,
    limits: Limits,
    permission: PermissionState,
    execution: ExecutionRequirement,
    user_gesture: UserGestureRequirement,
}

impl<Operations, Limits> CapabilityDescriptor<Operations, Limits> {
    pub const fn new(
        operations: Operations,
        limits: Limits,
        permission: PermissionState,
        execution: ExecutionRequirement,
        user_gesture: UserGestureRequirement,
    ) -> Self {
        Self {
            operations,
            limits,
            permission,
            execution,
            user_gesture,
        }
    }

    pub const fn operations(&self) -> &Operations {
        &self.operations
    }

    pub const fn limits(&self) -> &Limits {
        &self.limits
    }

    pub const fn permission(&self) -> PermissionState {
        self.permission
    }

    pub const fn execution(&self) -> ExecutionRequirement {
        self.execution
    }

    pub const fn user_gesture(&self) -> UserGestureRequirement {
        self.user_gesture
    }

    pub fn into_parts(
        self,
    ) -> (
        Operations,
        Limits,
        PermissionState,
        ExecutionRequirement,
        UserGestureRequirement,
    ) {
        (
            self.operations,
            self.limits,
            self.permission,
            self.execution,
            self.user_gesture,
        )
    }

    pub fn map_operations<Mapped>(
        self,
        map: impl FnOnce(Operations) -> Mapped,
    ) -> CapabilityDescriptor<Mapped, Limits> {
        CapabilityDescriptor::new(
            map(self.operations),
            self.limits,
            self.permission,
            self.execution,
            self.user_gesture,
        )
    }
}

impl<Operations> CapabilityDescriptor<Operations, NoCapabilityLimits> {
    pub const fn without_limits(
        operations: Operations,
        permission: PermissionState,
        execution: ExecutionRequirement,
        user_gesture: UserGestureRequirement,
    ) -> Self {
        Self::new(
            operations,
            NoCapabilityLimits,
            permission,
            execution,
            user_gesture,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;
    use std::num::NonZeroU32;

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    struct Operations(u8);

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    struct Limits {
        maximum_items: CapabilityLimit<NonZeroU32>,
    }

    fn assert_wire_value<T: Copy + Eq + Hash + Send + Sync + 'static>() {}

    #[test]
    fn available_descriptors_preserve_typed_operations_limits_and_requirements() {
        let descriptor = CapabilityDescriptor::new(
            Operations(0b101),
            Limits {
                maximum_items: CapabilityLimit::Bounded(NonZeroU32::new(32).unwrap()),
            },
            PermissionState::PromptRequired,
            ExecutionRequirement::HostExecutor,
            UserGestureRequirement::RecentRequired,
        );
        let support = Support::Available(descriptor);

        assert!(support.is_available());
        assert_eq!(support.unavailable_reason(), None);
        assert_eq!(
            support.as_ref().into_available().unwrap().operations(),
            &Operations(0b101)
        );
        assert_eq!(
            descriptor
                .limits()
                .maximum_items
                .into_bound()
                .unwrap()
                .get(),
            32
        );
        assert!(descriptor.permission().requires_prompt());
        assert_eq!(descriptor.execution(), ExecutionRequirement::HostExecutor);
        assert!(descriptor.user_gesture().is_required());
        assert_wire_value::<CapabilityDescriptor<Operations, Limits>>();
    }

    #[test]
    fn permission_and_support_remain_distinct_facts() {
        let denied = CapabilityDescriptor::without_limits(
            Operations(1),
            PermissionState::Denied,
            ExecutionRequirement::PlatformMainThread,
            UserGestureRequirement::NotRequired,
        );

        assert!(PermissionState::NotRequired.allows_use());
        assert!(PermissionState::Granted.allows_use());
        assert!(PermissionState::Denied.blocks_use());
        assert!(PermissionState::Restricted.blocks_use());
        assert!(!PermissionState::Unknown.allows_use());
        assert!(Support::Available(denied).is_available());
    }

    #[test]
    fn unavailable_support_retains_reason_without_constructing_a_descriptor() {
        let unavailable: Support<Operations> =
            Support::Unavailable(UnavailableReason::ExecutionContextUnavailable);
        let mapped = unavailable.map(|_| panic!("unavailable support must not map a payload"));

        assert!(mapped.is_unavailable());
        assert_eq!(
            mapped.unavailable_reason(),
            Some(UnavailableReason::ExecutionContextUnavailable)
        );
        assert_eq!(mapped.into_available(), None);
        assert_eq!(
            UnavailableReason::AdapterNotCompiled.to_string(),
            "adapter not compiled"
        );
    }

    #[test]
    fn unspecified_limits_never_claim_to_be_unbounded() {
        let unspecified = CapabilityLimit::<NonZeroU32>::Unspecified;
        assert!(!unspecified.is_bounded());
        assert_eq!(unspecified.into_bound(), None);
        assert_eq!(
            CapabilityLimit::Bounded(NonZeroU32::new(4).unwrap())
                .map(NonZeroU32::get)
                .into_bound(),
            Some(4)
        );
    }
}
