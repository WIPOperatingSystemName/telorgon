//! Native text-input and virtual-keyboard service boundary.
//!
//! [`crate::ui::text`] remains the sole owner of revisioned UTF-8 storage, session lifecycle,
//! configuration, surrounding-text snapshots, edits, selection, and composition. This module only
//! binds those canonical values to a platform [`ViewId`], validates the metadata that is safe to
//! admit to a native adapter, and describes asynchronous synchronization admission.
//!
//! Native adapters convert native index/range conventions into [`crate::ui::text`] byte offsets before
//! constructing [`TextInputDeltaEvent`]. The portable runtime then applies the retained
//! [`TextSessionDelta`] through its owning [`crate::ui::text::TextInputSession`], which performs the
//! authoritative session and revision checks. No native input-method object, callback, queue,
//! executor, event loop, text buffer, or virtual keyboard is owned here.

use std::fmt;
use std::num::NonZeroU32;
use std::rc::Rc;

use crate::foundation::RectF;
use crate::ui::text::{
    TextCompositionKind, TextInputRequest, TextInputSnapshot, TextRevision, TextSessionCommand,
    TextSessionDelta, TextSessionId,
};

use crate::platform::contracts::services::{ServiceKey, ServiceUnavailable};
use crate::platform::contracts::{CapabilityDescriptor, RequestAdmission, Support, ViewId};

/// Hard neutral bound for surrounding UTF-8 text admitted in one synchronization request.
pub const MAX_TEXT_INPUT_SURROUNDING_BYTES: u32 = 64 * 1024;

/// Independently discoverable text-input operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextInputOperations {
    input_method: bool,
    virtual_keyboard: bool,
    surrounding_text: bool,
    selection: bool,
    composition: bool,
    editor_actions: bool,
}

impl TextInputOperations {
    /// Builds one exact operation set advertised by a host adapter.
    pub const fn new(
        input_method: bool,
        virtual_keyboard: bool,
        surrounding_text: bool,
        selection: bool,
        composition: bool,
        editor_actions: bool,
    ) -> Self {
        Self {
            input_method,
            virtual_keyboard,
            surrounding_text,
            selection,
            composition,
            editor_actions,
        }
    }

    pub const fn supports_input_method(self) -> bool {
        self.input_method
    }

    pub const fn supports_virtual_keyboard(self) -> bool {
        self.virtual_keyboard
    }

    pub const fn supports_surrounding_text(self) -> bool {
        self.surrounding_text
    }

    pub const fn supports_selection(self) -> bool {
        self.selection
    }

    pub const fn supports_composition(self) -> bool {
        self.composition
    }

    pub const fn supports_editor_actions(self) -> bool {
        self.editor_actions
    }
}

/// Host-advertised bounds for one text-input service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextInputLimits {
    maximum_surrounding_bytes: NonZeroU32,
}

impl TextInputLimits {
    /// Creates limits that cannot exceed the neutral hard bound.
    pub const fn new(maximum_surrounding_bytes: NonZeroU32) -> Result<Self, TextInputLimitError> {
        if maximum_surrounding_bytes.get() > MAX_TEXT_INPUT_SURROUNDING_BYTES {
            return Err(TextInputLimitError::SurroundingTextLimitTooLarge);
        }
        Ok(Self {
            maximum_surrounding_bytes,
        })
    }

    pub const fn maximum_surrounding_bytes(self) -> NonZeroU32 {
        self.maximum_surrounding_bytes
    }
}

impl Default for TextInputLimits {
    fn default() -> Self {
        Self {
            maximum_surrounding_bytes: NonZeroU32::new(MAX_TEXT_INPUT_SURROUNDING_BYTES)
                .expect("text-input hard bound is nonzero"),
        }
    }
}

/// Invalid host-advertised text-input limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextInputLimitError {
    SurroundingTextLimitTooLarge,
}

impl fmt::Display for TextInputLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("text-input surrounding-text limit exceeds the neutral hard bound")
    }
}

impl std::error::Error for TextInputLimitError {}

/// Complete capability returned for one live view.
pub type TextInputCapability = CapabilityDescriptor<TextInputOperations, TextInputLimits>;

/// Scope for a text-input capability query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextInputCapabilityQuery {
    view: ViewId,
}

impl TextInputCapabilityQuery {
    pub const fn new(view: ViewId) -> Self {
        Self { view }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }
}

/// Lifecycle meaning of one canonical [`TextInputRequest`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextInputSyncKind {
    Open,
    Update,
    Close,
}

/// Invalid canonical text-input metadata at the platform admission boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextInputSyncError {
    /// Surrounding UTF-8 text exceeds the neutral hard bound.
    SurroundingTextTooLarge,
    /// Secure-entry configuration carried surrounding plaintext.
    SecureSurroundingTextExposed,
    /// The surrounding-text base plus byte length cannot be represented by `TextOffset`.
    SurroundingTextRangeOverflow,
    /// The active canonical cursor is outside the supplied surrounding-text range.
    ActiveOffsetOutsideSurroundingText,
    /// The active canonical cursor is not a UTF-8 boundary in supplied surrounding text.
    ActiveOffsetNotCharacterBoundary,
    /// Caret or selection geometry contains a non-finite component.
    NonFiniteGeometry,
    /// Caret or selection geometry has a negative extent.
    NegativeGeometryExtent,
}

impl fmt::Display for TextInputSyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SurroundingTextTooLarge => {
                "text-input surrounding text exceeds the neutral hard bound"
            }
            Self::SecureSurroundingTextExposed => {
                "secure text-input snapshot exposes surrounding plaintext"
            }
            Self::SurroundingTextRangeOverflow => {
                "text-input surrounding-text range cannot be represented"
            }
            Self::ActiveOffsetOutsideSurroundingText => {
                "text-input active offset is outside surrounding text"
            }
            Self::ActiveOffsetNotCharacterBoundary => {
                "text-input active offset is not a surrounding-text UTF-8 boundary"
            }
            Self::NonFiniteGeometry => "text-input geometry must be finite",
            Self::NegativeGeometryExtent => "text-input geometry extents must be nonnegative",
        })
    }
}

impl std::error::Error for TextInputSyncError {}

/// One view-scoped canonical session synchronization request.
///
/// Construction validates bounded surrounding text, mandatory secure-entry redaction, the active
/// UTF-8 cursor, and finite view-logical geometry before a service can admit the request. Debug
/// output reports metadata only and never formats surrounding text.
#[derive(Clone, PartialEq)]
pub struct TextInputSyncRequest {
    view: ViewId,
    request: TextInputRequest,
}

impl TextInputSyncRequest {
    pub fn new(view: ViewId, request: TextInputRequest) -> Result<Self, TextInputSyncError> {
        if let Some(snapshot) = request.snapshot() {
            validate_snapshot(snapshot)?;
        }
        Ok(Self { view, request })
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn session(&self) -> TextSessionId {
        self.request.session()
    }

    pub const fn kind(&self) -> TextInputSyncKind {
        match &self.request {
            TextInputRequest::Open(_) => TextInputSyncKind::Open,
            TextInputRequest::Update(_) => TextInputSyncKind::Update,
            TextInputRequest::Close { .. } => TextInputSyncKind::Close,
        }
    }

    pub const fn revision(&self) -> Option<TextRevision> {
        match &self.request {
            TextInputRequest::Open(snapshot) | TextInputRequest::Update(snapshot) => {
                Some(snapshot.revision)
            }
            TextInputRequest::Close { .. } => None,
        }
    }

    pub const fn canonical_request(&self) -> &TextInputRequest {
        &self.request
    }

    pub fn into_canonical_request(self) -> TextInputRequest {
        self.request
    }

    fn secure_entry(&self) -> Option<bool> {
        self.request
            .snapshot()
            .map(|snapshot| snapshot.configuration.secure_entry)
    }

    fn surrounding_byte_len(&self) -> Option<usize> {
        self.request
            .snapshot()
            .map(|snapshot| snapshot.surrounding.text.len())
    }
}

impl fmt::Debug for TextInputSyncRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextInputSyncRequest")
            .field("view", &self.view)
            .field("session", &self.session())
            .field("kind", &self.kind())
            .field("revision", &self.revision())
            .field("secure_entry", &self.secure_entry())
            .field("surrounding_byte_len", &self.surrounding_byte_len())
            .finish_non_exhaustive()
    }
}

/// Metadata returned when a synchronization request applies.
///
/// This receipt says that the adapter completed the request; it is not a native observation and
/// does not mutate either the text session or a view snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextInputApplied {
    view: ViewId,
    session: TextSessionId,
    kind: TextInputSyncKind,
    revision: Option<TextRevision>,
}

impl TextInputApplied {
    pub const fn from_request(request: &TextInputSyncRequest) -> Self {
        Self {
            view: request.view(),
            session: request.session(),
            kind: request.kind(),
            revision: request.revision(),
        }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn session(self) -> TextSessionId {
        self.session
    }

    pub const fn kind(self) -> TextInputSyncKind {
        self.kind
    }

    pub const fn revision(self) -> Option<TextRevision> {
        self.revision
    }
}

/// Payload category carried by a canonical platform-to-runtime session delta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextInputDeltaKind {
    Edit,
    Composition(TextCompositionKind),
    PerformAction,
}

/// View-scoped canonical delta emitted after a native adapter converts its index conventions.
///
/// The envelope performs no edit. The owning [`crate::ui::text::TextInputSession`] validates its
/// session generation and cited revision before applying the inner delta. Debug output deliberately
/// omits edits and replacement/preedit content.
#[derive(Clone, PartialEq, Eq)]
pub struct TextInputDeltaEvent {
    view: ViewId,
    delta: TextSessionDelta,
}

impl TextInputDeltaEvent {
    pub const fn new(view: ViewId, delta: TextSessionDelta) -> Self {
        Self { view, delta }
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn session(&self) -> TextSessionId {
        self.delta.session
    }

    pub const fn observed_revision(&self) -> TextRevision {
        self.delta.command.base_revision()
    }

    pub const fn kind(&self) -> TextInputDeltaKind {
        match &self.delta.command {
            TextSessionCommand::Edit(_) => TextInputDeltaKind::Edit,
            TextSessionCommand::Composition(command) => {
                TextInputDeltaKind::Composition(command.kind())
            }
            TextSessionCommand::PerformAction { .. } => TextInputDeltaKind::PerformAction,
        }
    }

    pub const fn canonical_delta(&self) -> &TextSessionDelta {
        &self.delta
    }

    pub fn into_canonical_delta(self) -> TextSessionDelta {
        self.delta
    }
}

impl fmt::Debug for TextInputDeltaEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextInputDeltaEvent")
            .field("view", &self.view)
            .field("session", &self.session())
            .field("observed_revision", &self.observed_revision())
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}

/// Immediate rejection before a text-input synchronization request is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextInputAdmissionError {
    ServiceUnavailable(ServiceUnavailable),
    ViewUnavailable {
        view: ViewId,
    },
    SessionUnavailable {
        view: ViewId,
        session: TextSessionId,
    },
    Unsupported,
    Denied,
    CapacityExceeded,
}

impl fmt::Display for TextInputAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServiceUnavailable(reason) => {
                write!(formatter, "text-input service is unavailable: {reason:?}")
            }
            Self::ViewUnavailable { view } => {
                write!(formatter, "text-input view {view} is unavailable")
            }
            Self::SessionUnavailable { view, session } => write!(
                formatter,
                "text-input session {}:{} is unavailable for view {view}",
                session.slot(),
                session.generation()
            ),
            Self::Unsupported => formatter.write_str("text-input operation is unsupported"),
            Self::Denied => formatter.write_str("text-input operation was denied"),
            Self::CapacityExceeded => {
                formatter.write_str("text-input admission capacity was exceeded")
            }
        }
    }
}

impl std::error::Error for TextInputAdmissionError {}

/// Linear asynchronous admission for one canonical text-input synchronization request.
pub type TextInputAdmission = RequestAdmission<TextInputApplied, TextInputAdmissionError>;

/// Narrow service surface for capability discovery and canonical session synchronization.
pub trait TextInputService {
    fn capability(&self, query: TextInputCapabilityQuery) -> Support<TextInputCapability>;

    fn synchronize(&self, request: TextInputSyncRequest) -> TextInputAdmission;
}

/// Type-level registry key for an owner-local text-input service handle.
pub enum TextInputServiceKey {}

impl ServiceKey for TextInputServiceKey {
    type Handle = Rc<dyn TextInputService>;
}

fn validate_snapshot(snapshot: &TextInputSnapshot) -> Result<(), TextInputSyncError> {
    let surrounding = &snapshot.surrounding;
    let surrounding_len = u32::try_from(surrounding.text.len())
        .map_err(|_| TextInputSyncError::SurroundingTextTooLarge)?;
    if surrounding_len > MAX_TEXT_INPUT_SURROUNDING_BYTES {
        return Err(TextInputSyncError::SurroundingTextTooLarge);
    }
    if snapshot.configuration.secure_entry && !surrounding.text.is_empty() {
        return Err(TextInputSyncError::SecureSurroundingTextExposed);
    }

    let start = surrounding.base.bytes();
    let end = start
        .checked_add(surrounding_len)
        .ok_or(TextInputSyncError::SurroundingTextRangeOverflow)?;
    let active = snapshot.selection.active.bytes();
    if active < start || active > end {
        return Err(TextInputSyncError::ActiveOffsetOutsideSurroundingText);
    }
    let local_active = usize::try_from(active - start)
        .expect("u32 text offset difference is representable as usize");
    if !surrounding.text.is_char_boundary(local_active) {
        return Err(TextInputSyncError::ActiveOffsetNotCharacterBoundary);
    }

    validate_rect(snapshot.geometry.caret)?;
    if let Some(selection) = snapshot.geometry.selection_bounds {
        validate_rect(selection)?;
    }
    Ok(())
}

fn validate_rect(rect: RectF) -> Result<(), TextInputSyncError> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
    {
        return Err(TextInputSyncError::NonFiniteGeometry);
    }
    if rect.width < 0.0 || rect.height < 0.0 {
        return Err(TextInputSyncError::NegativeGeometryExtent);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::ui::text::{
        TextAffinity, TextBuffer, TextEdit, TextEditBatch, TextInputConfiguration,
        TextInputGeometry, TextInputSession, TextOffset, TextRange, TextSelection,
        TextSurroundingText,
    };

    use crate::platform::contracts::{
        AdmittedRequest, ExecutionRequirement, PermissionState, RequestId, RequestOutcome,
        ServiceLookup, ServiceRegistry, UnavailableReason, UserGestureRequirement,
    };

    use super::*;

    fn view() -> ViewId {
        ViewId::from_raw(7, 2).unwrap()
    }

    fn session_id() -> TextSessionId {
        TextSessionId::from_raw(3, 4).unwrap()
    }

    fn open_request(text: &str, secure_entry: bool) -> TextInputRequest {
        let configuration = TextInputConfiguration {
            secure_entry,
            ..TextInputConfiguration::default()
        };
        let buffer = TextBuffer::from_text(text).unwrap();
        let mut session = TextInputSession::new(session_id(), configuration, 256);
        session.open(&buffer).unwrap()
    }

    #[test]
    fn capability_preserves_independent_operations_limits_and_host_requirements() {
        let operations = TextInputOperations::new(true, true, true, true, true, false);
        let capability = TextInputCapability::new(
            operations,
            TextInputLimits::new(NonZeroU32::new(4096).unwrap()).unwrap(),
            PermissionState::NotRequired,
            ExecutionRequirement::PlatformMainThread,
            UserGestureRequirement::NotRequired,
        );

        assert!(capability.operations().supports_input_method());
        assert!(capability.operations().supports_virtual_keyboard());
        assert!(capability.operations().supports_composition());
        assert!(!capability.operations().supports_editor_actions());
        assert_eq!(capability.limits().maximum_surrounding_bytes().get(), 4096);
        assert_eq!(
            capability.execution(),
            ExecutionRequirement::PlatformMainThread
        );
        assert_eq!(
            TextInputLimits::new(NonZeroU32::new(MAX_TEXT_INPUT_SURROUNDING_BYTES + 1).unwrap()),
            Err(TextInputLimitError::SurroundingTextLimitTooLarge)
        );
    }

    #[test]
    fn synchronization_reuses_canonical_session_values_and_redacts_surrounding_text() {
        let request = TextInputSyncRequest::new(view(), open_request("private draft", false))
            .expect("canonical session snapshot is admissible");
        assert_eq!(request.session(), session_id());
        assert_eq!(request.kind(), TextInputSyncKind::Open);
        assert_eq!(request.revision(), Some(TextRevision::INITIAL));
        let debug = format!("{request:?}");
        assert!(debug.contains("surrounding_byte_len: Some(13)"));
        assert!(!debug.contains("private draft"));

        let applied = TextInputApplied::from_request(&request);
        assert_eq!(applied.view(), view());
        assert_eq!(applied.session(), session_id());
        assert_eq!(applied.revision(), Some(TextRevision::INITIAL));
    }

    #[test]
    fn secure_plaintext_oversized_surrounding_and_invalid_geometry_are_rejected() {
        let mut exposed = open_request("secret", true);
        let TextInputRequest::Open(snapshot) = &mut exposed else {
            panic!("fixture opens a session")
        };
        snapshot.surrounding.text = "secret".to_owned();
        assert_eq!(
            TextInputSyncRequest::new(view(), exposed),
            Err(TextInputSyncError::SecureSurroundingTextExposed)
        );

        let oversized = TextInputRequest::Open(TextInputSnapshot {
            session: session_id(),
            revision: TextRevision::INITIAL,
            selection: TextSelection::collapsed(TextOffset::ZERO, TextAffinity::Downstream),
            composition: None,
            surrounding: TextSurroundingText {
                base: TextOffset::ZERO,
                text: "x".repeat(MAX_TEXT_INPUT_SURROUNDING_BYTES as usize + 1),
            },
            geometry: TextInputGeometry::default(),
            configuration: TextInputConfiguration::default(),
        });
        assert_eq!(
            TextInputSyncRequest::new(view(), oversized),
            Err(TextInputSyncError::SurroundingTextTooLarge)
        );

        let mut invalid_geometry = open_request("geometry", false);
        let TextInputRequest::Open(snapshot) = &mut invalid_geometry else {
            panic!("fixture opens a session")
        };
        snapshot.geometry.caret.width = f32::NAN;
        assert_eq!(
            TextInputSyncRequest::new(view(), invalid_geometry),
            Err(TextInputSyncError::NonFiniteGeometry)
        );
    }

    #[test]
    fn active_cursor_must_cite_a_canonical_boundary_inside_surrounding_text() {
        let mut outside = open_request("hello", false);
        let TextInputRequest::Open(snapshot) = &mut outside else {
            panic!("fixture opens a session")
        };
        snapshot.selection.active = TextOffset::from_bytes(6);
        assert_eq!(
            TextInputSyncRequest::new(view(), outside),
            Err(TextInputSyncError::ActiveOffsetOutsideSurroundingText)
        );

        let mut split_scalar = open_request("é", false);
        let TextInputRequest::Open(snapshot) = &mut split_scalar else {
            panic!("fixture opens a session")
        };
        snapshot.selection.active = TextOffset::from_bytes(1);
        assert_eq!(
            TextInputSyncRequest::new(view(), split_scalar),
            Err(TextInputSyncError::ActiveOffsetNotCharacterBoundary)
        );
    }

    #[test]
    fn delta_envelope_preserves_identity_and_revision_without_debugging_inserted_text() {
        let delta = TextSessionDelta {
            session: session_id(),
            command: TextSessionCommand::Edit(TextEditBatch {
                base_revision: TextRevision::INITIAL,
                edits: vec![TextEdit {
                    range: TextRange::collapsed(TextOffset::ZERO),
                    replacement: "sensitive insertion".to_owned(),
                }],
                selection: TextSelection::collapsed(TextOffset::ZERO, TextAffinity::Downstream),
                composition: None,
            }),
        };
        let event = TextInputDeltaEvent::new(view(), delta);
        assert_eq!(event.session(), session_id());
        assert_eq!(event.observed_revision(), TextRevision::INITIAL);
        assert_eq!(event.kind(), TextInputDeltaKind::Edit);
        assert!(!format!("{event:?}").contains("sensitive insertion"));
    }

    struct FixtureTextInputService {
        view: ViewId,
        next_request: Cell<u64>,
    }

    impl TextInputService for FixtureTextInputService {
        fn capability(&self, query: TextInputCapabilityQuery) -> Support<TextInputCapability> {
            if query.view() != self.view {
                return Support::Unavailable(UnavailableReason::UnavailableInScope);
            }
            Support::Available(TextInputCapability::new(
                TextInputOperations::new(true, true, true, true, true, true),
                TextInputLimits::default(),
                PermissionState::NotRequired,
                ExecutionRequirement::PlatformMainThread,
                UserGestureRequirement::NotRequired,
            ))
        }

        fn synchronize(&self, request: TextInputSyncRequest) -> TextInputAdmission {
            if request.view() != self.view {
                return Err(TextInputAdmissionError::ViewUnavailable {
                    view: request.view(),
                });
            }
            let next = self.next_request.get() + 1;
            self.next_request.set(next);
            Ok(AdmittedRequest::new(RequestId::from_raw(next).unwrap()))
        }
    }

    #[test]
    fn owner_local_service_only_admits_and_completion_remains_explicit() {
        let concrete = Rc::new(FixtureTextInputService {
            view: view(),
            next_request: Cell::new(40),
        });
        let service: Rc<dyn TextInputService> = concrete;
        let mut registry = ServiceRegistry::new();
        assert!(
            registry
                .register::<TextInputServiceKey>(service)
                .is_registered()
        );
        let ServiceLookup::Available(service) = registry.lookup::<TextInputServiceKey>() else {
            panic!("registered text-input service must remain available")
        };
        assert!(
            service
                .capability(TextInputCapabilityQuery::new(view()))
                .is_available()
        );

        let request = TextInputSyncRequest::new(view(), open_request("hello", false)).unwrap();
        let applied = TextInputApplied::from_request(&request);
        let completion = service
            .synchronize(request)
            .unwrap()
            .complete(RequestOutcome::Applied(applied));
        assert_eq!(completion.request_id().get(), 41);
        assert_eq!(completion.outcome().applied().unwrap().view(), view());
    }
}
