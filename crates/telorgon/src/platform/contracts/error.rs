//! Structured, redaction-safe failures at the neutral platform boundary.
//!
//! Native adapters classify failures before they cross this boundary. Neither an arbitrary owned
//! message nor a native error code belongs in these records: either could retain secure text,
//! clipboard or transfer contents, paths, protocol identifiers, pointers, or file descriptors.

use std::fmt;

/// Stable categories on which portable request handling may branch.
///
/// Denied, unsupported, cancelled, and stale requests are terminal request outcomes rather than
/// failure kinds. Capability discovery has its own unavailable reasons; [`Unavailable`] describes
/// a capability or host that disappeared after a request was admitted.
///
/// [`Unavailable`]: PlatformErrorKind::Unavailable
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlatformErrorKind {
    /// A previously available host, adapter, service, or execution context became unavailable.
    Unavailable,
    /// Communication with the platform service or embedding host failed.
    TransportFailure,
    /// A platform or host operation did not complete within its declared bound.
    TimedOut,
    /// A bounded queue, transfer, or service-specific capacity was exceeded.
    CapacityExceeded,
    /// The host could not obtain memory or another finite platform resource.
    ResourceExhausted,
    /// A platform or host response was malformed, inconsistent, or outside its declared bounds.
    InvalidData,
    /// A platform protocol or host integration contract was violated.
    ProtocolViolation,
    /// An internal invariant failed after boundary validation.
    InvariantViolation,
}

impl fmt::Display for PlatformErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "unavailable",
            Self::TransportFailure => "transport failure",
            Self::TimedOut => "timed out",
            Self::CapacityExceeded => "capacity exceeded",
            Self::ResourceExhausted => "resource exhausted",
            Self::InvalidData => "invalid data",
            Self::ProtocolViolation => "protocol violation",
            Self::InvariantViolation => "invariant violation",
        })
    }
}

/// A sanitized lower-level cause retained by a [`PlatformError`].
///
/// An adapter maps its native error into this record instead of retaining the native error itself.
/// Context must be an author-written static description of the failed operation, never content or
/// an error string received from the platform. This preserves a useful standard-error source chain
/// without allowing sensitive, platform-specific payloads into portable state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlatformErrorSource {
    kind: PlatformErrorKind,
    context: &'static str,
}

impl PlatformErrorSource {
    pub const fn new(kind: PlatformErrorKind, context: &'static str) -> Self {
        Self { kind, context }
    }

    pub const fn kind(self) -> PlatformErrorKind {
        self.kind
    }

    pub const fn context(self) -> &'static str {
        self.context
    }
}

impl fmt::Display for PlatformErrorSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.context, self.kind)
    }
}

impl std::error::Error for PlatformErrorSource {}

/// Redaction-safe failure produced after a platform request was admitted.
///
/// `context` is deliberately `&'static str`. Adapters must classify native failures and supply a
/// static operation description; they cannot attach native messages, codes, pointers, handles,
/// paths, transferred bytes, or user content. Portable code branches on [`Self::kind`] and treats
/// [`Display`](fmt::Display) as diagnostic text only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlatformError {
    kind: PlatformErrorKind,
    context: &'static str,
    source: Option<PlatformErrorSource>,
}

impl PlatformError {
    /// Creates a classified failure with static diagnostic context.
    pub const fn new(kind: PlatformErrorKind, context: &'static str) -> Self {
        Self {
            kind,
            context,
            source: None,
        }
    }

    /// Creates a classified failure retaining one sanitized lower-level source.
    pub const fn with_source(
        kind: PlatformErrorKind,
        context: &'static str,
        source: PlatformErrorSource,
    ) -> Self {
        Self {
            kind,
            context,
            source: Some(source),
        }
    }

    pub const fn kind(self) -> PlatformErrorKind {
        self.kind
    }

    pub const fn context(self) -> &'static str {
        self.context
    }

    pub const fn source_record(self) -> Option<PlatformErrorSource> {
        self.source
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.context, self.kind)
    }
}

impl std::error::Error for PlatformError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

pub type PlatformResult<T> = Result<T, PlatformError>;

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::hash::Hash;

    use super::*;

    fn assert_wire_value<T: Copy + Eq + Hash + Send + Sync + 'static>() {}

    #[test]
    fn every_failure_category_remains_structurally_distinct() {
        let kinds = [
            PlatformErrorKind::Unavailable,
            PlatformErrorKind::TransportFailure,
            PlatformErrorKind::TimedOut,
            PlatformErrorKind::CapacityExceeded,
            PlatformErrorKind::ResourceExhausted,
            PlatformErrorKind::InvalidData,
            PlatformErrorKind::ProtocolViolation,
            PlatformErrorKind::InvariantViolation,
        ];

        for (index, kind) in kinds.into_iter().enumerate() {
            let error = PlatformError::new(kind, "platform request failed");
            assert_eq!(error.kind(), kind);
            assert_eq!(
                kinds.iter().filter(|candidate| **candidate == kind).count(),
                1
            );
            assert_eq!(error.source_record(), None);
            assert_eq!(error.context(), "platform request failed");
            assert_eq!(
                index,
                kinds
                    .iter()
                    .position(|candidate| *candidate == kind)
                    .unwrap()
            );
        }
    }

    #[test]
    fn sanitized_source_is_available_through_both_structured_and_standard_apis() {
        let source = PlatformErrorSource::new(
            PlatformErrorKind::TransportFailure,
            "host completion channel",
        );
        let error = PlatformError::with_source(
            PlatformErrorKind::Unavailable,
            "clipboard request failed",
            source,
        );

        assert_eq!(error.source_record(), Some(source));
        assert_eq!(source.kind(), PlatformErrorKind::TransportFailure);
        assert_eq!(source.context(), "host completion channel");
        assert_eq!(
            error.source().unwrap().to_string(),
            "host completion channel: transport failure"
        );
    }

    #[test]
    fn display_is_diagnostic_while_branching_uses_the_closed_kind() {
        let error = PlatformError::new(PlatformErrorKind::CapacityExceeded, "platform event queue");

        assert!(matches!(error.kind(), PlatformErrorKind::CapacityExceeded));
        assert_eq!(error.to_string(), "platform event queue: capacity exceeded");
    }

    #[test]
    fn records_are_compact_immutable_thread_transferable_values() {
        assert_wire_value::<PlatformErrorKind>();
        assert_wire_value::<PlatformErrorSource>();
        assert_wire_value::<PlatformError>();

        assert!(std::mem::size_of::<PlatformErrorSource>() <= 24);
        assert!(std::mem::size_of::<PlatformError>() <= 48);
    }
}
