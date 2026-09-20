//! Platform-neutral state-restoration token admission.
//!
//! Restoration bytes are opaque, hard-bounded, and redacted from diagnostics. Application, view,
//! and session scopes carry independent exact revisions. This module transports tokens but does
//! not interpret or serialize runtime state, read or write storage, retain a native restoration
//! object, choose persistence policy, or own a callback, queue, executor, thread, timer, or event
//! loop.

use std::error::Error;
use std::fmt;
use std::num::{NonZeroU32, NonZeroU64};
use std::rc::Rc;

use super::ServiceKey;
use crate::platform::contracts::{
    CapabilityDescriptor, RequestAdmission, RestorationSessionId, Support, ViewId,
};

/// Neutral hard bound on one opaque restoration token.
pub const MAX_RESTORATION_TOKEN_BYTES: usize = 64 * 1_024;

/// Owner scope of one restoration history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RestorationScope {
    Application,
    View(ViewId),
    Session(RestorationSessionId),
}

impl RestorationScope {
    pub const fn view(self) -> Option<ViewId> {
        match self {
            Self::View(view) => Some(view),
            Self::Application | Self::Session(_) => None,
        }
    }

    pub const fn session(self) -> Option<RestorationSessionId> {
        match self {
            Self::Session(session) => Some(session),
            Self::Application | Self::View(_) => None,
        }
    }
}

/// Monotonic revision within one [`RestorationScope`] history.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RestorationRevision(NonZeroU64);

impl RestorationRevision {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    pub const fn new(revision: NonZeroU64) -> Self {
        Self(revision)
    }

    pub const fn from_raw(revision: u64) -> Option<Self> {
        match NonZeroU64::new(revision) {
            Some(revision) => Some(Self(revision)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub const fn checked_next(self) -> Option<Self> {
        match self.get().checked_add(1) {
            Some(revision) => Self::from_raw(revision),
            None => None,
        }
    }
}

impl fmt::Display for RestorationRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// Exact identity of one restoration publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RestorationSnapshotId {
    scope: RestorationScope,
    revision: RestorationRevision,
}

impl RestorationSnapshotId {
    pub const fn new(scope: RestorationScope, revision: RestorationRevision) -> Self {
        Self { scope, revision }
    }

    pub const fn scope(self) -> RestorationScope {
        self.scope
    }

    pub const fn revision(self) -> RestorationRevision {
        self.revision
    }
}

/// Opaque bounded restoration bytes omitted from generic diagnostics.
///
/// The owner is intentionally not `Clone`; publishing or consuming a token moves it into one
/// linear request path. Callers may explicitly copy bytes before construction if their own policy
/// requires a second independent owner.
#[derive(PartialEq, Eq)]
pub struct RestorationToken(Box<[u8]>);

impl RestorationToken {
    pub fn new(bytes: Vec<u8>) -> Result<Self, RestorationTokenError> {
        if bytes.is_empty() {
            return Err(RestorationTokenError::Empty);
        }
        if bytes.len() > MAX_RESTORATION_TOKEN_BYTES {
            return Err(RestorationTokenError::TooLarge {
                byte_len: bytes.len(),
                maximum_bytes: MAX_RESTORATION_TOKEN_BYTES,
            });
        }
        Ok(Self(bytes.into_boxed_slice()))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn byte_len(&self) -> usize {
        self.0.len()
    }

    pub fn into_bytes(self) -> Box<[u8]> {
        self.0
    }
}

impl fmt::Debug for RestorationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RestorationToken")
            .field("byte_len", &self.byte_len())
            .field("redacted", &true)
            .finish()
    }
}

/// Invalid opaque restoration token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RestorationTokenError {
    Empty,
    TooLarge {
        byte_len: usize,
        maximum_bytes: usize,
    },
}

impl fmt::Display for RestorationTokenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "restoration token is empty",
            Self::TooLarge { .. } => "restoration token exceeds the neutral hard bound",
        })
    }
}

impl Error for RestorationTokenError {}

/// One exact scoped restoration token publication or consumption candidate.
#[derive(PartialEq, Eq)]
pub struct RestorationRecord {
    snapshot: RestorationSnapshotId,
    token: RestorationToken,
}

impl RestorationRecord {
    pub const fn new(snapshot: RestorationSnapshotId, token: RestorationToken) -> Self {
        Self { snapshot, token }
    }

    pub const fn snapshot(&self) -> RestorationSnapshotId {
        self.snapshot
    }

    pub const fn scope(&self) -> RestorationScope {
        self.snapshot.scope
    }

    pub const fn revision(&self) -> RestorationRevision {
        self.snapshot.revision
    }

    pub const fn token(&self) -> &RestorationToken {
        &self.token
    }

    pub fn into_parts(self) -> (RestorationSnapshotId, RestorationToken) {
        (self.snapshot, self.token)
    }
}

impl fmt::Debug for RestorationRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RestorationRecord")
            .field("snapshot", &self.snapshot)
            .field("token_byte_len", &self.token.byte_len())
            .field("token_redacted", &true)
            .finish()
    }
}

/// Initial or successor publication of one complete opaque token.
#[derive(PartialEq, Eq)]
pub struct RestorationPublicationRequest {
    previous: Option<RestorationSnapshotId>,
    record: RestorationRecord,
}

impl RestorationPublicationRequest {
    pub fn initial(record: RestorationRecord) -> Result<Self, RestorationPublicationError> {
        if record.revision() != RestorationRevision::INITIAL {
            return Err(RestorationPublicationError::InitialRevisionRequired {
                observed: record.revision(),
            });
        }
        Ok(Self {
            previous: None,
            record,
        })
    }

    pub fn advance(
        previous: RestorationSnapshotId,
        record: RestorationRecord,
    ) -> Result<Self, RestorationPublicationError> {
        if previous.scope != record.scope() {
            return Err(RestorationPublicationError::ScopeMismatch {
                expected: previous.scope,
                observed: record.scope(),
            });
        }
        let expected = previous.revision.checked_next().ok_or(
            RestorationPublicationError::RevisionExhausted {
                scope: previous.scope,
            },
        )?;
        if record.revision() != expected {
            return Err(RestorationPublicationError::RevisionNotSuccessor {
                previous: previous.revision,
                expected,
                observed: record.revision(),
            });
        }
        Ok(Self {
            previous: Some(previous),
            record,
        })
    }

    pub const fn previous(&self) -> Option<RestorationSnapshotId> {
        self.previous
    }

    pub const fn record(&self) -> &RestorationRecord {
        &self.record
    }

    pub const fn snapshot(&self) -> RestorationSnapshotId {
        self.record.snapshot
    }

    pub fn into_record(self) -> RestorationRecord {
        self.record
    }
}

impl fmt::Debug for RestorationPublicationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RestorationPublicationRequest")
            .field("previous", &self.previous)
            .field("record", &self.record)
            .finish_non_exhaustive()
    }
}

/// Invalid initial or advancing restoration publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RestorationPublicationError {
    InitialRevisionRequired {
        observed: RestorationRevision,
    },
    ScopeMismatch {
        expected: RestorationScope,
        observed: RestorationScope,
    },
    RevisionExhausted {
        scope: RestorationScope,
    },
    RevisionNotSuccessor {
        previous: RestorationRevision,
        expected: RestorationRevision,
        observed: RestorationRevision,
    },
}

impl fmt::Display for RestorationPublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InitialRevisionRequired { .. } => {
                "initial restoration publication requires the initial revision"
            }
            Self::ScopeMismatch { .. } => "restoration update belongs to a different scope history",
            Self::RevisionExhausted { .. } => "restoration revision space is exhausted",
            Self::RevisionNotSuccessor { .. } => {
                "restoration update is not the immediate successor revision"
            }
        })
    }
}

impl Error for RestorationPublicationError {}

/// Metadata returned after one restoration publication applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RestorationPublicationApplied {
    snapshot: RestorationSnapshotId,
}

impl RestorationPublicationApplied {
    pub const fn from_request(request: &RestorationPublicationRequest) -> Self {
        Self {
            snapshot: request.snapshot(),
        }
    }

    pub const fn snapshot(self) -> RestorationSnapshotId {
        self.snapshot
    }
}

/// Exact-current removal of a restoration publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RestorationClearRequest {
    expected: RestorationSnapshotId,
}

impl RestorationClearRequest {
    pub const fn new(expected: RestorationSnapshotId) -> Self {
        Self { expected }
    }

    pub const fn expected(self) -> RestorationSnapshotId {
        self.expected
    }
}

/// Metadata returned when exact-current restoration state is cleared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RestorationClearApplied {
    cleared: RestorationSnapshotId,
}

impl RestorationClearApplied {
    pub const fn from_request(request: RestorationClearRequest) -> Self {
        Self {
            cleared: request.expected,
        }
    }

    pub const fn cleared(self) -> RestorationSnapshotId {
        self.cleared
    }
}

/// Single-owner request to validate and consume one exact restoration record.
#[derive(PartialEq, Eq)]
pub struct RestorationConsumptionRequest {
    record: RestorationRecord,
}

impl RestorationConsumptionRequest {
    pub const fn new(record: RestorationRecord) -> Self {
        Self { record }
    }

    pub const fn snapshot(&self) -> RestorationSnapshotId {
        self.record.snapshot
    }

    pub const fn record(&self) -> &RestorationRecord {
        &self.record
    }

    pub fn into_record(self) -> RestorationRecord {
        self.record
    }
}

impl fmt::Debug for RestorationConsumptionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RestorationConsumptionRequest")
            .field("record", &self.record)
            .finish_non_exhaustive()
    }
}

/// Applied consumption result returning the single owned opaque token to portable code.
#[derive(PartialEq, Eq)]
pub struct RestorationConsumptionApplied {
    record: RestorationRecord,
}

impl RestorationConsumptionApplied {
    pub const fn new(record: RestorationRecord) -> Self {
        Self { record }
    }

    pub const fn snapshot(&self) -> RestorationSnapshotId {
        self.record.snapshot
    }

    pub const fn token(&self) -> &RestorationToken {
        &self.record.token
    }

    pub fn into_record(self) -> RestorationRecord {
        self.record
    }
}

impl fmt::Debug for RestorationConsumptionApplied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RestorationConsumptionApplied")
            .field("record", &self.record)
            .finish_non_exhaustive()
    }
}

/// Independently discoverable restoration operations and scopes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RestorationOperations {
    publish: bool,
    update: bool,
    consume: bool,
    clear: bool,
    application_scope: bool,
    view_scope: bool,
    session_scope: bool,
}

impl RestorationOperations {
    pub const fn new(
        publish: bool,
        update: bool,
        consume: bool,
        clear: bool,
        application_scope: bool,
        view_scope: bool,
        session_scope: bool,
    ) -> Self {
        Self {
            publish,
            update,
            consume,
            clear,
            application_scope,
            view_scope,
            session_scope,
        }
    }

    pub const fn supports_publish(self) -> bool {
        self.publish
    }

    pub const fn supports_update(self) -> bool {
        self.update
    }

    pub const fn supports_consume(self) -> bool {
        self.consume
    }

    pub const fn supports_clear(self) -> bool {
        self.clear
    }

    pub const fn supports_application_scope(self) -> bool {
        self.application_scope
    }

    pub const fn supports_view_scope(self) -> bool {
        self.view_scope
    }

    pub const fn supports_session_scope(self) -> bool {
        self.session_scope
    }

    pub const fn supports_scope(self, scope: RestorationScope) -> bool {
        match scope {
            RestorationScope::Application => self.application_scope,
            RestorationScope::View(_) => self.view_scope,
            RestorationScope::Session(_) => self.session_scope,
        }
    }
}

/// Adapter-narrowed restoration-token size bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RestorationLimits {
    maximum_token_bytes: NonZeroU32,
}

impl RestorationLimits {
    pub const fn new(maximum_token_bytes: NonZeroU32) -> Result<Self, RestorationLimitError> {
        if maximum_token_bytes.get() as usize > MAX_RESTORATION_TOKEN_BYTES {
            return Err(RestorationLimitError::TokenBytesTooLarge);
        }
        Ok(Self {
            maximum_token_bytes,
        })
    }

    pub const fn maximum_token_bytes(self) -> NonZeroU32 {
        self.maximum_token_bytes
    }
}

impl Default for RestorationLimits {
    fn default() -> Self {
        Self {
            maximum_token_bytes: NonZeroU32::new(MAX_RESTORATION_TOKEN_BYTES as u32)
                .expect("restoration token hard bound is nonzero"),
        }
    }
}

/// Invalid adapter-advertised restoration limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RestorationLimitError {
    TokenBytesTooLarge,
}

impl fmt::Display for RestorationLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("restoration token limit exceeds the neutral hard bound")
    }
}

impl Error for RestorationLimitError {}

pub type RestorationCapability = CapabilityDescriptor<RestorationOperations, RestorationLimits>;

/// Exact scope used to query restoration capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RestorationCapabilityQuery {
    scope: RestorationScope,
}

impl RestorationCapabilityQuery {
    pub const fn new(scope: RestorationScope) -> Self {
        Self { scope }
    }

    pub const fn scope(self) -> RestorationScope {
        self.scope
    }
}

/// Immediate rejection before a restoration request is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RestorationAdmissionError {
    UnsupportedOperation,
    UnsupportedScope {
        scope: RestorationScope,
    },
    ViewUnavailable {
        view: ViewId,
    },
    SessionUnavailable {
        session: RestorationSessionId,
    },
    PermissionDenied,
    AuthorizationRequired,
    TokenExceedsCapability,
    SnapshotUnavailable {
        scope: RestorationScope,
    },
    RevisionMismatch {
        expected: RestorationSnapshotId,
        observed: Option<RestorationSnapshotId>,
    },
    CapabilityChanged,
    CapacityExceeded,
}

impl fmt::Display for RestorationAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedOperation => "restoration operation is unsupported",
            Self::UnsupportedScope { .. } => "restoration scope is unsupported",
            Self::ViewUnavailable { .. } => "restoration view is unavailable",
            Self::SessionUnavailable { .. } => "restoration session is unavailable",
            Self::PermissionDenied => "restoration permission is denied",
            Self::AuthorizationRequired => "restoration authorization is required",
            Self::TokenExceedsCapability => "restoration token exceeds current capability",
            Self::SnapshotUnavailable { .. } => "restoration snapshot is unavailable",
            Self::RevisionMismatch { .. } => "restoration request cites a stale revision",
            Self::CapabilityChanged => "restoration capability changed before admission",
            Self::CapacityExceeded => "restoration admission capacity was exceeded",
        })
    }
}

impl Error for RestorationAdmissionError {}

pub type RestorationPublicationAdmission =
    RequestAdmission<RestorationPublicationApplied, RestorationAdmissionError>;
pub type RestorationConsumptionAdmission =
    RequestAdmission<RestorationConsumptionApplied, RestorationAdmissionError>;
pub type RestorationClearAdmission =
    RequestAdmission<RestorationClearApplied, RestorationAdmissionError>;

/// Object-safe restoration capability and linear request-admission boundary.
pub trait RestorationService {
    fn capability(&self, query: RestorationCapabilityQuery) -> Support<RestorationCapability>;

    fn publish(&self, request: RestorationPublicationRequest) -> RestorationPublicationAdmission;

    fn consume(&self, request: RestorationConsumptionRequest) -> RestorationConsumptionAdmission;

    fn clear(&self, request: RestorationClearRequest) -> RestorationClearAdmission;
}

pub enum RestorationServiceKey {}

impl ServiceKey for RestorationServiceKey {
    type Handle = Rc<dyn RestorationService>;
}
