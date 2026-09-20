use std::fmt;
use std::num::NonZeroU32;

use crate::foundation::RectF;

use crate::ui::text::{
    TextBuffer, TextCompositionCommand, TextCompositionError, TextEditBatch, TextEditError,
    TextEditOutcome, TextOffset, TextRange, TextRevision, TextSelection,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextSessionId {
    slot: NonZeroU32,
    generation: NonZeroU32,
}

impl TextSessionId {
    pub const fn new(slot: NonZeroU32, generation: NonZeroU32) -> Self {
        Self { slot, generation }
    }

    pub const fn from_raw(slot: u32, generation: u32) -> Option<Self> {
        match (NonZeroU32::new(slot), NonZeroU32::new(generation)) {
            (Some(slot), Some(generation)) => Some(Self { slot, generation }),
            _ => None,
        }
    }

    pub const fn slot(self) -> u32 {
        self.slot.get()
    }

    pub const fn generation(self) -> u32 {
        self.generation.get()
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextInputPurpose {
    #[default]
    Text,
    Name,
    Email,
    Url,
    Telephone,
    Number,
    Decimal,
    Search,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextCapitalization {
    #[default]
    None,
    Characters,
    Words,
    Sentences,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextInputPolicy {
    #[default]
    Automatic,
    Enabled,
    Disabled,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextMultiline {
    #[default]
    SingleLine,
    MultiLine,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextReturnKeyAction {
    #[default]
    Default,
    Done,
    Go,
    Search,
    Send,
    Next,
    Previous,
    Newline,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextVirtualKeyboardPreference {
    #[default]
    Automatic,
    Show,
    Hide,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TextInputConfiguration {
    pub purpose: TextInputPurpose,
    pub capitalization: TextCapitalization,
    pub correction: TextInputPolicy,
    pub spelling: TextInputPolicy,
    pub secure_entry: bool,
    pub multiline: TextMultiline,
    pub return_key: TextReturnKeyAction,
    pub virtual_keyboard: TextVirtualKeyboardPreference,
}

impl Default for TextInputConfiguration {
    fn default() -> Self {
        Self {
            purpose: TextInputPurpose::Text,
            capitalization: TextCapitalization::None,
            correction: TextInputPolicy::Automatic,
            spelling: TextInputPolicy::Automatic,
            secure_entry: false,
            multiline: TextMultiline::SingleLine,
            return_key: TextReturnKeyAction::Default,
            virtual_keyboard: TextVirtualKeyboardPreference::Automatic,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct TextInputGeometry {
    pub caret: RectF,
    pub selection_bounds: Option<RectF>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct TextSurroundingText {
    pub base: TextOffset,
    pub text: String,
}

impl fmt::Debug for TextSurroundingText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextSurroundingText")
            .field("base", &self.base)
            .field("len_bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

impl TextSurroundingText {
    pub fn end(&self) -> TextOffset {
        let len = u32::try_from(self.text.len()).unwrap_or(u32::MAX);
        TextOffset(self.base.0.saturating_add(len))
    }
}

#[derive(Clone, PartialEq)]
pub struct TextInputSnapshot {
    pub session: TextSessionId,
    pub revision: TextRevision,
    pub selection: TextSelection,
    pub composition: Option<TextRange>,
    pub surrounding: TextSurroundingText,
    pub geometry: TextInputGeometry,
    pub configuration: TextInputConfiguration,
}

impl fmt::Debug for TextInputSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextInputSnapshot")
            .field("session", &self.session)
            .field("revision", &self.revision)
            .field("selection", &self.selection)
            .field("composition", &self.composition)
            .field("surrounding", &self.surrounding)
            .field("geometry", &self.geometry)
            .field("configuration", &self.configuration)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextInputRequest {
    Open(TextInputSnapshot),
    Update(TextInputSnapshot),
    Close { session: TextSessionId },
}

impl TextInputRequest {
    pub const fn session(&self) -> TextSessionId {
        match self {
            Self::Open(snapshot) | Self::Update(snapshot) => snapshot.session,
            Self::Close { session } => *session,
        }
    }

    pub const fn snapshot(&self) -> Option<&TextInputSnapshot> {
        match self {
            Self::Open(snapshot) | Self::Update(snapshot) => Some(snapshot),
            Self::Close { .. } => None,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextSessionPhase {
    #[default]
    Created,
    Open,
    Closed,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextSessionStateError {
    AlreadyOpen,
    NotOpen,
    Closed,
}

impl fmt::Display for TextSessionStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyOpen => f.write_str("text input session is already open"),
            Self::NotOpen => f.write_str("text input session is not open"),
            Self::Closed => f.write_str("text input session is closed"),
        }
    }
}

impl std::error::Error for TextSessionStateError {}

pub struct TextInputSession {
    id: TextSessionId,
    phase: TextSessionPhase,
    configuration: TextInputConfiguration,
    geometry: TextInputGeometry,
    max_surrounding_bytes: u32,
    last_issued_revision: Option<TextRevision>,
}

impl fmt::Debug for TextInputSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextInputSession")
            .field("id", &self.id)
            .field("phase", &self.phase)
            .field("configuration", &self.configuration)
            .field("geometry", &self.geometry)
            .field("max_surrounding_bytes", &self.max_surrounding_bytes)
            .field("last_issued_revision", &self.last_issued_revision)
            .finish()
    }
}

impl TextInputSession {
    pub fn new(
        id: TextSessionId,
        configuration: TextInputConfiguration,
        max_surrounding_bytes: u32,
    ) -> Self {
        Self {
            id,
            phase: TextSessionPhase::Created,
            configuration,
            geometry: TextInputGeometry::default(),
            max_surrounding_bytes,
            last_issued_revision: None,
        }
    }

    pub const fn id(&self) -> TextSessionId {
        self.id
    }

    pub const fn phase(&self) -> TextSessionPhase {
        self.phase
    }

    pub const fn configuration(&self) -> TextInputConfiguration {
        self.configuration
    }

    pub const fn geometry(&self) -> TextInputGeometry {
        self.geometry
    }

    pub const fn max_surrounding_bytes(&self) -> u32 {
        self.max_surrounding_bytes
    }

    pub const fn last_issued_revision(&self) -> Option<TextRevision> {
        self.last_issued_revision
    }

    pub fn set_configuration(
        &mut self,
        configuration: TextInputConfiguration,
    ) -> Result<(), TextSessionStateError> {
        self.ensure_not_closed()?;
        self.configuration = configuration;
        Ok(())
    }

    pub fn set_geometry(
        &mut self,
        geometry: TextInputGeometry,
    ) -> Result<(), TextSessionStateError> {
        self.ensure_not_closed()?;
        self.geometry = geometry;
        Ok(())
    }

    pub fn open(&mut self, buffer: &TextBuffer) -> Result<TextInputRequest, TextSessionStateError> {
        match self.phase {
            TextSessionPhase::Created => {
                self.phase = TextSessionPhase::Open;
                Ok(TextInputRequest::Open(self.issue_snapshot(buffer)))
            }
            TextSessionPhase::Open => Err(TextSessionStateError::AlreadyOpen),
            TextSessionPhase::Closed => Err(TextSessionStateError::Closed),
        }
    }

    pub fn update(
        &mut self,
        buffer: &TextBuffer,
    ) -> Result<TextInputRequest, TextSessionStateError> {
        self.ensure_open()?;
        Ok(TextInputRequest::Update(self.issue_snapshot(buffer)))
    }

    pub fn close(&mut self) -> Result<TextInputRequest, TextSessionStateError> {
        self.ensure_open()?;
        self.phase = TextSessionPhase::Closed;
        self.last_issued_revision = None;
        Ok(TextInputRequest::Close { session: self.id })
    }

    pub fn apply_delta(
        &mut self,
        buffer: &mut TextBuffer,
        delta: TextSessionDelta,
    ) -> TextSessionDeltaOutcome {
        if delta.session != self.id {
            return TextSessionDeltaOutcome::RejectedSession {
                expected: self.id,
                received: delta.session,
            };
        }
        if self.phase != TextSessionPhase::Open {
            return TextSessionDeltaOutcome::RejectedInactive { phase: self.phase };
        }

        let observed = delta.command.base_revision();
        let current = buffer.revision();
        let Some(issued) = self.last_issued_revision else {
            return self.resynchronize(
                buffer,
                TextInputResyncReason::Unpublished { observed, current },
            );
        };
        if observed != issued || observed != current {
            return self.resynchronize(
                buffer,
                TextInputResyncReason::StaleRevision {
                    observed,
                    issued,
                    current,
                },
            );
        }

        match delta.command {
            TextSessionCommand::Edit(batch) => match buffer.apply_edits(batch) {
                Ok(outcome) => self.applied(buffer, outcome),
                Err(error) => self.resynchronize(buffer, TextInputResyncReason::InvalidEdit(error)),
            },
            TextSessionCommand::Composition(command) => match buffer.apply_composition(command) {
                Ok(outcome) => self.applied(buffer, outcome),
                Err(error) => {
                    self.resynchronize(buffer, TextInputResyncReason::InvalidComposition(error))
                }
            },
            TextSessionCommand::PerformAction { .. } => TextSessionDeltaOutcome::Action {
                action: self.configuration.return_key,
            },
        }
    }

    fn ensure_not_closed(&self) -> Result<(), TextSessionStateError> {
        if self.phase == TextSessionPhase::Closed {
            Err(TextSessionStateError::Closed)
        } else {
            Ok(())
        }
    }

    fn ensure_open(&self) -> Result<(), TextSessionStateError> {
        match self.phase {
            TextSessionPhase::Open => Ok(()),
            TextSessionPhase::Created => Err(TextSessionStateError::NotOpen),
            TextSessionPhase::Closed => Err(TextSessionStateError::Closed),
        }
    }

    fn applied(
        &mut self,
        buffer: &TextBuffer,
        outcome: TextEditOutcome,
    ) -> TextSessionDeltaOutcome {
        let request = TextInputRequest::Update(self.issue_snapshot(buffer));
        TextSessionDeltaOutcome::Applied { outcome, request }
    }

    fn resynchronize(
        &mut self,
        buffer: &TextBuffer,
        reason: TextInputResyncReason,
    ) -> TextSessionDeltaOutcome {
        let request = TextInputRequest::Update(self.issue_snapshot(buffer));
        TextSessionDeltaOutcome::Resynchronize { reason, request }
    }

    fn issue_snapshot(&mut self, buffer: &TextBuffer) -> TextInputSnapshot {
        let revision = buffer.revision();
        self.last_issued_revision = Some(revision);
        TextInputSnapshot {
            session: self.id,
            revision,
            selection: buffer.selection(),
            composition: buffer.composition(),
            surrounding: surrounding_text(
                buffer.text_for_edit(),
                buffer.selection().active,
                self.max_surrounding_bytes,
                self.configuration.secure_entry,
            ),
            geometry: self.geometry,
            configuration: self.configuration,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSessionDelta {
    pub session: TextSessionId,
    pub command: TextSessionCommand,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextSessionCommand {
    Edit(TextEditBatch),
    Composition(TextCompositionCommand),
    PerformAction { base_revision: TextRevision },
}

impl TextSessionCommand {
    pub const fn base_revision(&self) -> TextRevision {
        match self {
            Self::Edit(batch) => batch.base_revision,
            Self::Composition(command) => command.base_revision(),
            Self::PerformAction { base_revision } => *base_revision,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextInputResyncReason {
    Unpublished {
        observed: TextRevision,
        current: TextRevision,
    },
    StaleRevision {
        observed: TextRevision,
        issued: TextRevision,
        current: TextRevision,
    },
    InvalidEdit(TextEditError),
    InvalidComposition(TextCompositionError),
}

#[derive(Clone, Debug)]
pub enum TextSessionDeltaOutcome {
    Applied {
        outcome: TextEditOutcome,
        request: TextInputRequest,
    },
    Action {
        action: TextReturnKeyAction,
    },
    Resynchronize {
        reason: TextInputResyncReason,
        request: TextInputRequest,
    },
    RejectedSession {
        expected: TextSessionId,
        received: TextSessionId,
    },
    RejectedInactive {
        phase: TextSessionPhase,
    },
}

fn surrounding_text(
    text: &str,
    active: TextOffset,
    max_bytes: u32,
    secure_entry: bool,
) -> TextSurroundingText {
    let active = active.as_usize();
    if secure_entry || max_bytes == 0 {
        return TextSurroundingText {
            base: TextOffset(active as u32),
            text: String::new(),
        };
    }

    let max_bytes = max_bytes as usize;
    if text.len() <= max_bytes {
        return TextSurroundingText {
            base: TextOffset::ZERO,
            text: text.to_string(),
        };
    }

    let mut start = active.saturating_sub(max_bytes / 2);
    let mut end = (start + max_bytes).min(text.len());
    if end == text.len() {
        start = end.saturating_sub(max_bytes);
    }
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    debug_assert!(start <= active && active <= end);

    TextSurroundingText {
        base: TextOffset(start as u32),
        text: text[start..end].to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::foundation::RectF;

    use crate::ui::text::{
        TextAffinity, TextBuffer, TextCompositionCommand, TextEdit, TextEditBatch,
        TextInputConfiguration, TextInputGeometry, TextInputPolicy, TextInputPurpose,
        TextInputRequest, TextInputResyncReason, TextInputSession, TextMultiline, TextOffset,
        TextRange, TextReturnKeyAction, TextRevision, TextSelection, TextSessionCommand,
        TextSessionDelta, TextSessionDeltaOutcome, TextSessionId, TextSessionPhase,
        TextSessionStateError, TextVirtualKeyboardPreference,
    };

    fn session_id(slot: u32, generation: u32) -> TextSessionId {
        TextSessionId::from_raw(slot, generation).unwrap()
    }

    fn selection(anchor: u32, active: u32) -> TextSelection {
        TextSelection {
            anchor: TextOffset(anchor),
            active: TextOffset(active),
            affinity: TextAffinity::Downstream,
        }
    }

    fn range(start: u32, end: u32) -> TextRange {
        TextRange::new(TextOffset(start), TextOffset(end)).unwrap()
    }

    fn snapshot(request: &TextInputRequest) -> &crate::ui::text::TextInputSnapshot {
        request.snapshot().unwrap()
    }

    fn text(buffer: &TextBuffer) -> String {
        buffer.chunks().map(|chunk| chunk.text).collect()
    }

    #[test]
    fn session_ids_are_nonzero_and_generation_aware() {
        assert_eq!(TextSessionId::from_raw(0, 1), None);
        assert_eq!(TextSessionId::from_raw(1, 0), None);
        let first = session_id(3, 1);
        let replacement = session_id(3, 2);
        assert_ne!(first, replacement);
        assert_eq!(replacement.slot(), 3);
        assert_eq!(replacement.generation(), 2);
    }

    #[test]
    fn lifecycle_emits_ordered_open_update_and_close_requests() {
        let buffer = TextBuffer::from_text("hello").unwrap();
        let mut session =
            TextInputSession::new(session_id(1, 1), TextInputConfiguration::default(), 64);

        assert_eq!(session.phase(), TextSessionPhase::Created);
        assert_eq!(session.update(&buffer), Err(TextSessionStateError::NotOpen));
        let open = session.open(&buffer).unwrap();
        assert!(matches!(open, TextInputRequest::Open(_)));
        assert_eq!(snapshot(&open).revision, TextRevision::INITIAL);
        assert_eq!(session.phase(), TextSessionPhase::Open);
        assert_eq!(
            session.open(&buffer),
            Err(TextSessionStateError::AlreadyOpen)
        );
        assert!(matches!(
            session.update(&buffer),
            Ok(TextInputRequest::Update(_))
        ));
        assert_eq!(
            session.close().unwrap(),
            TextInputRequest::Close {
                session: session_id(1, 1)
            }
        );
        assert_eq!(session.phase(), TextSessionPhase::Closed);
        assert_eq!(session.update(&buffer), Err(TextSessionStateError::Closed));
        assert_eq!(
            session.set_geometry(TextInputGeometry::default()),
            Err(TextSessionStateError::Closed)
        );
    }

    #[test]
    fn snapshot_contains_bounded_scalar_safe_surrounding_state_and_geometry() {
        let buffer = TextBuffer::from_text("prefix é suffix").unwrap();
        let configuration = TextInputConfiguration {
            purpose: TextInputPurpose::Email,
            correction: TextInputPolicy::Disabled,
            multiline: TextMultiline::MultiLine,
            virtual_keyboard: TextVirtualKeyboardPreference::Show,
            ..TextInputConfiguration::default()
        };
        let geometry = TextInputGeometry {
            caret: RectF {
                x: 10.0,
                y: 20.0,
                width: 1.0,
                height: 18.0,
            },
            selection_bounds: Some(RectF {
                x: 5.0,
                y: 20.0,
                width: 6.0,
                height: 18.0,
            }),
        };
        let mut session = TextInputSession::new(session_id(1, 1), configuration, 7);
        session.set_geometry(geometry).unwrap();
        let request = session.open(&buffer).unwrap();
        let snapshot = snapshot(&request);

        assert!(snapshot.surrounding.text.len() <= 7);
        assert!(
            buffer
                .text_for_edit()
                .is_char_boundary(snapshot.surrounding.base.as_usize())
        );
        assert!(
            buffer
                .text_for_edit()
                .is_char_boundary(snapshot.surrounding.end().as_usize())
        );
        assert_eq!(snapshot.geometry, geometry);
        assert_eq!(snapshot.configuration, configuration);
        assert_eq!(snapshot.selection, buffer.selection());
    }

    #[test]
    fn secure_snapshot_and_debug_output_do_not_expose_text() {
        let buffer = TextBuffer::from_text("private credential").unwrap();
        let configuration = TextInputConfiguration {
            secure_entry: true,
            ..TextInputConfiguration::default()
        };
        let mut session = TextInputSession::new(session_id(1, 1), configuration, u32::MAX);
        let request = session.open(&buffer).unwrap();
        let snapshot = snapshot(&request);
        let debug = format!("{request:?}");

        assert!(snapshot.surrounding.text.is_empty());
        assert_eq!(snapshot.surrounding.base, buffer.selection().active);
        assert!(!debug.contains("private credential"));
        assert!(debug.contains("len_bytes"));
    }

    #[test]
    fn valid_delta_applies_atomically_and_issues_the_new_snapshot() {
        let mut buffer = TextBuffer::from_text("abc").unwrap();
        let mut session =
            TextInputSession::new(session_id(1, 1), TextInputConfiguration::default(), 64);
        session.open(&buffer).unwrap();
        let delta = TextSessionDelta {
            session: session.id(),
            command: TextSessionCommand::Edit(TextEditBatch {
                base_revision: TextRevision::INITIAL,
                edits: vec![TextEdit {
                    range: range(1, 2),
                    replacement: "é".to_string(),
                }],
                selection: selection(3, 3),
                composition: None,
            }),
        };

        let result = session.apply_delta(&mut buffer, delta);
        let TextSessionDeltaOutcome::Applied { outcome, request } = result else {
            panic!("expected applied delta");
        };
        assert_eq!(text(&buffer), "aéc");
        assert_eq!(outcome.snapshot.revision(), TextRevision(1));
        assert_eq!(snapshot(&request).revision, TextRevision(1));
        assert_eq!(session.last_issued_revision(), Some(TextRevision(1)));
    }

    #[test]
    fn stale_revision_requests_resync_without_mutation() {
        let mut buffer = TextBuffer::from_text("abc").unwrap();
        let mut session =
            TextInputSession::new(session_id(1, 1), TextInputConfiguration::default(), 64);
        session.open(&buffer).unwrap();
        buffer
            .apply_edits(TextEditBatch {
                base_revision: TextRevision::INITIAL,
                edits: Vec::new(),
                selection: selection(3, 3),
                composition: None,
            })
            .unwrap();

        let result = session.apply_delta(
            &mut buffer,
            TextSessionDelta {
                session: session.id(),
                command: TextSessionCommand::Edit(TextEditBatch {
                    base_revision: TextRevision::INITIAL,
                    edits: vec![TextEdit {
                        range: range(0, 3),
                        replacement: "stale private value".to_string(),
                    }],
                    selection: selection(19, 19),
                    composition: None,
                }),
            },
        );

        let TextSessionDeltaOutcome::Resynchronize { reason, request } = result else {
            panic!("expected resynchronization");
        };
        assert_eq!(
            reason,
            TextInputResyncReason::StaleRevision {
                observed: TextRevision::INITIAL,
                issued: TextRevision::INITIAL,
                current: TextRevision(1),
            }
        );
        assert_eq!(snapshot(&request).revision, TextRevision(1));
        assert_eq!(text(&buffer), "abc");
    }

    #[test]
    fn invalid_delta_requests_resync_and_preserves_the_editing_value() {
        let mut buffer = TextBuffer::from_text("é").unwrap();
        let mut session =
            TextInputSession::new(session_id(1, 1), TextInputConfiguration::default(), 64);
        session.open(&buffer).unwrap();
        let before = buffer.snapshot();

        let result = session.apply_delta(
            &mut buffer,
            TextSessionDelta {
                session: session.id(),
                command: TextSessionCommand::Edit(TextEditBatch {
                    base_revision: TextRevision::INITIAL,
                    edits: vec![TextEdit {
                        range: range(1, 1),
                        replacement: "private".to_string(),
                    }],
                    selection: selection(2, 2),
                    composition: None,
                }),
            },
        );

        assert!(matches!(
            result,
            TextSessionDeltaOutcome::Resynchronize {
                reason: TextInputResyncReason::InvalidEdit(_),
                ..
            }
        ));
        assert_eq!(buffer.revision(), before.revision());
        assert_eq!(buffer.selection(), before.selection());
        assert_eq!(text(&buffer), "é");
    }

    #[test]
    fn composition_delta_uses_the_existing_ordered_composition_owner() {
        let mut buffer = TextBuffer::from_text("a").unwrap();
        let mut session =
            TextInputSession::new(session_id(1, 1), TextInputConfiguration::default(), 64);
        session.open(&buffer).unwrap();

        let result = session.apply_delta(
            &mut buffer,
            TextSessionDelta {
                session: session.id(),
                command: TextSessionCommand::Composition(TextCompositionCommand::Start {
                    base_revision: TextRevision::INITIAL,
                    edits: vec![TextEdit {
                        range: range(1, 1),
                        replacement: "中".to_string(),
                    }],
                    selection: selection(4, 4),
                    composition: range(1, 4),
                }),
            },
        );

        assert!(matches!(result, TextSessionDeltaOutcome::Applied { .. }));
        assert_eq!(text(&buffer), "a中");
        assert_eq!(buffer.composition(), Some(range(1, 4)));
        assert_eq!(session.last_issued_revision(), Some(TextRevision(1)));
    }

    #[test]
    fn stale_generation_and_closed_session_callbacks_are_rejected() {
        let mut buffer = TextBuffer::from_text("abc").unwrap();
        let mut session =
            TextInputSession::new(session_id(2, 4), TextInputConfiguration::default(), 64);
        session.open(&buffer).unwrap();
        let stale = session.apply_delta(
            &mut buffer,
            TextSessionDelta {
                session: session_id(2, 3),
                command: TextSessionCommand::PerformAction {
                    base_revision: TextRevision::INITIAL,
                },
            },
        );
        assert!(matches!(
            stale,
            TextSessionDeltaOutcome::RejectedSession { .. }
        ));

        session.close().unwrap();
        let closed = session.apply_delta(
            &mut buffer,
            TextSessionDelta {
                session: session.id(),
                command: TextSessionCommand::PerformAction {
                    base_revision: TextRevision::INITIAL,
                },
            },
        );
        assert!(matches!(
            closed,
            TextSessionDeltaOutcome::RejectedInactive {
                phase: TextSessionPhase::Closed
            }
        ));
    }

    #[test]
    fn action_delta_returns_the_configured_semantic_action() {
        let mut buffer = TextBuffer::from_text("query").unwrap();
        let configuration = TextInputConfiguration {
            return_key: TextReturnKeyAction::Search,
            ..TextInputConfiguration::default()
        };
        let mut session = TextInputSession::new(session_id(1, 1), configuration, 64);
        session.open(&buffer).unwrap();

        let result = session.apply_delta(
            &mut buffer,
            TextSessionDelta {
                session: session.id(),
                command: TextSessionCommand::PerformAction {
                    base_revision: TextRevision::INITIAL,
                },
            },
        );

        assert!(matches!(
            result,
            TextSessionDeltaOutcome::Action {
                action: TextReturnKeyAction::Search
            }
        ));
        assert_eq!(buffer.revision(), TextRevision::INITIAL);
    }

    #[test]
    fn delta_debug_output_redacts_replacement_content() {
        let delta = TextSessionDelta {
            session: session_id(1, 1),
            command: TextSessionCommand::Edit(TextEditBatch {
                base_revision: TextRevision::INITIAL,
                edits: vec![TextEdit {
                    range: range(0, 0),
                    replacement: "private callback text".to_string(),
                }],
                selection: selection(0, 0),
                composition: None,
            }),
        };
        let debug = format!("{delta:?}");

        assert!(!debug.contains("private callback text"));
        assert!(debug.contains("replacement_len_bytes"));
    }
}
