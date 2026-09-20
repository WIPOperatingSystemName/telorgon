//! Typed terminal outcomes for admitted platform requests.
//!
//! Admission and completion are deliberately separate. An adapter first validates a request and
//! either returns an immediate error or creates one [`AdmittedRequest`] for its issued
//! [`RequestId`]. Completing that non-cloneable token consumes it and produces one non-cloneable
//! [`RequestCompletion`]. This module records that transition; it owns no request executor, queue,
//! callback, service, native object, clock, or cancellation side effect.

use std::marker::PhantomData;

use crate::platform::contracts::{PlatformError, RequestId};

/// Immediate validation/admission result for a request that will eventually produce `T`.
///
/// `E` belongs to the request-specific validation boundary. The error branch has no request
/// identity and cannot be completed. A successful branch contains a typed admitted token rather
/// than claiming that the requested platform state has already been observed.
pub type RequestAdmission<T, E> = Result<AdmittedRequest<T>, E>;

/// The closed set of terminal outcomes for an admitted platform request.
///
/// [`Applied`](Self::Applied) says that the platform accepted and completed the operation. It does
/// not create or update a view snapshot: the next revisioned platform observation remains the
/// source of truth.
#[derive(Debug, PartialEq, Eq)]
pub enum RequestOutcome<T> {
    /// The platform accepted and completed the operation with the typed result.
    Applied(T),
    /// Policy or permission denied the admitted operation.
    Denied,
    /// The current host or service does not support the admitted operation.
    Unsupported,
    /// The admitted operation was cancelled before it applied.
    Cancelled,
    /// The admitted operation cited state that is no longer current.
    Stale,
    /// The admitted operation failed after admission.
    Failed(PlatformError),
}

impl<T> RequestOutcome<T> {
    /// Reports whether the platform accepted and completed the operation.
    pub const fn is_applied(&self) -> bool {
        matches!(self, Self::Applied(_))
    }

    /// Reports whether policy or permission denied the operation.
    pub const fn is_denied(&self) -> bool {
        matches!(self, Self::Denied)
    }

    /// Reports whether the operation was unsupported by the current host or service.
    pub const fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported)
    }

    /// Reports whether the operation was cancelled before it applied.
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Reports whether the operation cited stale state.
    pub const fn is_stale(&self) -> bool {
        matches!(self, Self::Stale)
    }

    /// Reports whether the operation failed after admission.
    pub const fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    /// Borrows the applied value, if this is an applied outcome.
    pub const fn applied(&self) -> Option<&T> {
        match self {
            Self::Applied(value) => Some(value),
            Self::Denied | Self::Unsupported | Self::Cancelled | Self::Stale | Self::Failed(_) => {
                None
            }
        }
    }

    /// Returns the structured post-admission failure, if this is a failed outcome.
    pub const fn failure(&self) -> Option<PlatformError> {
        match self {
            Self::Failed(error) => Some(*error),
            Self::Applied(_) | Self::Denied | Self::Unsupported | Self::Cancelled | Self::Stale => {
                None
            }
        }
    }

    /// Borrows an applied value while preserving every terminal outcome.
    pub const fn as_ref(&self) -> RequestOutcome<&T> {
        match self {
            Self::Applied(value) => RequestOutcome::Applied(value),
            Self::Denied => RequestOutcome::Denied,
            Self::Unsupported => RequestOutcome::Unsupported,
            Self::Cancelled => RequestOutcome::Cancelled,
            Self::Stale => RequestOutcome::Stale,
            Self::Failed(error) => RequestOutcome::Failed(*error),
        }
    }

    /// Maps only an applied value and preserves every other terminal outcome exactly.
    pub fn map_applied<U>(self, map: impl FnOnce(T) -> U) -> RequestOutcome<U> {
        match self {
            Self::Applied(value) => RequestOutcome::Applied(map(value)),
            Self::Denied => RequestOutcome::Denied,
            Self::Unsupported => RequestOutcome::Unsupported,
            Self::Cancelled => RequestOutcome::Cancelled,
            Self::Stale => RequestOutcome::Stale,
            Self::Failed(error) => RequestOutcome::Failed(error),
        }
    }
}

/// A typed, admitted request that has not yet been completed through this token.
///
/// An adapter constructs exactly one token after it has validated a request and issued the
/// identity. The token is intentionally neither `Clone` nor `Copy`; [`Self::complete`] consumes it.
/// Request-specific code may express immediate validation as [`RequestAdmission<T, E>`], whose
/// error branch never creates this value.
///
/// ```compile_fail
/// use crate::platform::contracts::{AdmittedRequest, RequestId, RequestOutcome};
///
/// let request = AdmittedRequest::<()>::new(RequestId::MIN);
/// let first = request.complete(RequestOutcome::Applied(()));
/// let second = request.complete(RequestOutcome::Cancelled); // request was already consumed
/// # let _ = (first, second);
/// ```
#[must_use = "an admitted request must eventually receive one terminal outcome"]
#[derive(Debug, PartialEq, Eq)]
pub struct AdmittedRequest<T> {
    request_id: RequestId,
    result_type: PhantomData<fn() -> T>,
}

impl<T> AdmittedRequest<T> {
    /// Creates the one typed completion token associated with an admitted request identity.
    ///
    /// The issuing adapter must call this exactly once for a newly admitted `RequestId`. This
    /// constructor performs no validation, admission, request execution, or platform mutation.
    pub const fn new(request_id: RequestId) -> Self {
        Self {
            request_id,
            result_type: PhantomData,
        }
    }

    /// Returns the admitted request identity without consuming its completion token.
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Consumes the admitted token and binds its identity to one terminal outcome.
    pub const fn complete(self, outcome: RequestOutcome<T>) -> RequestCompletion<T> {
        RequestCompletion {
            request_id: self.request_id,
            outcome,
        }
    }
}

/// One typed terminal completion bound to its admitted request identity.
///
/// This immutable value is intentionally neither `Clone` nor `Copy`, so ordinary completion
/// delivery can move it exactly once. Mapping consumes the old completion and preserves the same
/// identity and terminal classification.
#[must_use = "a terminal request completion must be delivered or explicitly handled"]
#[derive(Debug, PartialEq, Eq)]
pub struct RequestCompletion<T> {
    request_id: RequestId,
    outcome: RequestOutcome<T>,
}

impl<T> RequestCompletion<T> {
    /// Returns the identity issued when the request was admitted.
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Borrows the typed terminal outcome.
    pub const fn outcome(&self) -> &RequestOutcome<T> {
        &self.outcome
    }

    /// Consumes the completion and returns its terminal outcome.
    pub fn into_outcome(self) -> RequestOutcome<T> {
        self.outcome
    }

    /// Consumes the completion and returns its identity and terminal outcome.
    pub fn into_parts(self) -> (RequestId, RequestOutcome<T>) {
        (self.request_id, self.outcome)
    }

    /// Maps only an applied value while preserving request identity and every rejection or failure.
    pub fn map_applied<U>(self, map: impl FnOnce(T) -> U) -> RequestCompletion<U> {
        RequestCompletion {
            request_id: self.request_id,
            outcome: self.outcome.map_applied(map),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::platform::contracts::PlatformErrorKind;

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct AppliedRevision(u64);

    #[derive(Debug, PartialEq, Eq)]
    enum ValidationError {
        InvalidExtent,
    }

    #[test]
    fn admitted_token_is_consumed_into_one_identity_preserving_completion() {
        let request_id = RequestId::from_raw(41).unwrap();
        let admitted = AdmittedRequest::new(request_id);
        assert_eq!(admitted.request_id(), request_id);

        let completion = admitted.complete(RequestOutcome::Applied(AppliedRevision(7)));
        assert_eq!(completion.request_id(), request_id);
        assert!(completion.outcome().is_applied());
        assert_eq!(completion.outcome().applied(), Some(&AppliedRevision(7)));
        assert_eq!(
            completion.into_parts(),
            (request_id, RequestOutcome::Applied(AppliedRevision(7)))
        );
    }

    #[test]
    fn immediate_validation_rejection_has_no_admitted_identity_or_terminal_completion() {
        let rejected: RequestAdmission<AppliedRevision, ValidationError> =
            Err(ValidationError::InvalidExtent);
        assert_eq!(rejected, Err(ValidationError::InvalidExtent));

        let admitted: RequestAdmission<AppliedRevision, ValidationError> =
            Ok(AdmittedRequest::new(RequestId::MIN));
        let Ok(admitted) = admitted else {
            panic!("valid request must be admitted");
        };
        assert_eq!(admitted.request_id(), RequestId::MIN);
    }

    #[test]
    fn terminal_rejections_remain_distinct_from_structured_failure() {
        let denied = RequestOutcome::<()>::Denied;
        let unsupported = RequestOutcome::<()>::Unsupported;
        let cancelled = RequestOutcome::<()>::Cancelled;
        let stale = RequestOutcome::<()>::Stale;
        let error = PlatformError::new(
            PlatformErrorKind::TransportFailure,
            "request completion channel",
        );
        let failed = RequestOutcome::<()>::Failed(error);

        assert!(denied.is_denied());
        assert!(unsupported.is_unsupported());
        assert!(cancelled.is_cancelled());
        assert!(stale.is_stale());
        assert!(failed.is_failed());
        assert_eq!(failed.failure(), Some(error));
        assert_eq!(denied.failure(), None);
        assert!(!failed.is_denied());
    }

    #[test]
    fn mapping_changes_only_applied_data_and_preserves_identity_and_failure() {
        let request_id = RequestId::from_raw(90).unwrap();
        let applied = AdmittedRequest::new(request_id)
            .complete(RequestOutcome::Applied(6_u16))
            .map_applied(u32::from);
        assert_eq!(applied.request_id(), request_id);
        assert_eq!(applied.into_outcome(), RequestOutcome::Applied(6_u32));

        let error = PlatformError::new(PlatformErrorKind::TimedOut, "platform request");
        let failed = AdmittedRequest::<u16>::new(request_id)
            .complete(RequestOutcome::Failed(error))
            .map_applied(u32::from);
        assert_eq!(failed.request_id(), request_id);
        assert_eq!(failed.outcome().as_ref(), RequestOutcome::Failed(error));
    }
}
