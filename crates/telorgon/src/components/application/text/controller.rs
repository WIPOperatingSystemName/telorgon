//! Application-domain controller over the revisioned `telorgon-text` editing owners.

use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use crate::runtime::MonotonicInstant;
use crate::ui::text::{
    TextBuffer, TextBufferError, TextChange, TextCompositionCommand, TextCompositionError,
    TextCompositionKind, TextEdit, TextEditBatch, TextEditError, TextEditOutcome,
    TextInputConfiguration, TextInputGeometry, TextInputRequest, TextInputResyncReason,
    TextInputSession, TextOffset, TextRange, TextReturnKeyAction, TextRevision, TextSelection,
    TextSessionCommand, TextSessionDelta, TextSessionDeltaOutcome, TextSessionId, TextSessionPhase,
    TextSessionStateError, TextSnapshot,
};

use super::{EditHistory, EditHistoryError, EditHistoryKind, EditHistoryPolicy};

/// Text ranges changed by one accepted controller edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChanged {
    pub revision: TextRevision,
    pub changes: Vec<TextChange>,
}

/// Directional selection change published with the accepted revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionChanged {
    pub revision: TextRevision,
    pub previous: TextSelection,
    pub current: TextSelection,
}

/// Composition range change published with the accepted revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompositionChanged {
    pub revision: TextRevision,
    pub previous: Option<TextRange>,
    pub current: Option<TextRange>,
}

/// Return/action submission requested by the active text-input session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Submitted {
    pub revision: TextRevision,
    pub action: TextReturnKeyAction,
}

/// Accepted atomic controller update and its immutable resulting snapshot.
#[derive(Clone, Debug)]
pub struct TextControllerUpdate {
    pub snapshot: TextSnapshot,
    pub text_changed: Option<TextChanged>,
    pub selection_changed: Option<SelectionChanged>,
    pub composition_changed: Option<CompositionChanged>,
}

impl TextControllerUpdate {
    pub const fn changed_text(&self) -> bool {
        self.text_changed.is_some()
    }

    pub const fn changed_selection(&self) -> bool {
        self.selection_changed.is_some()
    }

    pub const fn changed_composition(&self) -> bool {
        self.composition_changed.is_some()
    }
}

/// Typed reason an edit-like controller request was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditRejectedReason {
    Edit(TextEditError),
    Composition(TextCompositionError),
    Resynchronize(TextInputResyncReason),
    WrongSession {
        expected: TextSessionId,
        received: TextSessionId,
    },
    InactiveSession {
        phase: TextSessionPhase,
    },
}

/// Rejection plus the unchanged current snapshot needed for explicit resynchronization.
#[derive(Clone, Debug)]
pub struct EditRejected {
    pub reason: EditRejectedReason,
    pub current: TextSnapshot,
}

impl fmt::Display for EditRejectedReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Edit(error) => write!(formatter, "text edit rejected: {error}"),
            Self::Composition(error) => write!(formatter, "text composition rejected: {error}"),
            Self::Resynchronize(reason) => {
                write!(
                    formatter,
                    "text session requires resynchronization: {reason:?}"
                )
            }
            Self::WrongSession { expected, received } => write!(
                formatter,
                "text session mismatch: expected {expected:?}, received {received:?}"
            ),
            Self::InactiveSession { phase } => {
                write!(formatter, "text session is not active: {phase:?}")
            }
        }
    }
}

impl std::error::Error for EditRejectedReason {}

impl fmt::Display for EditRejected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(formatter)
    }
}

impl std::error::Error for EditRejected {}

/// Application-facing result of a delta from the active text-input session.
#[derive(Clone, Debug)]
pub enum TextControllerSessionOutcome {
    Updated {
        update: TextControllerUpdate,
        request: TextInputRequest,
    },
    Submitted(Submitted),
    Rejected {
        rejection: EditRejected,
        request: Option<TextInputRequest>,
    },
}

/// Current command availability for the controller-owned optional history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditHistoryAvailability {
    pub enabled: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Typed traversal command over the controller-owned edit history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditHistoryCommand {
    Undo,
    Redo,
}

/// Failure from a history-aware controller operation.
#[derive(Debug)]
pub enum TextControllerHistoryError {
    Disabled,
    SecureEntry,
    InvalidCompositionKind { received: EditHistoryKind },
    Edit(EditRejected),
    Controller(TextControllerError),
    History(EditHistoryError),
}

impl fmt::Display for TextControllerHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("text edit history is disabled"),
            Self::SecureEntry => {
                formatter.write_str("plaintext edit history is disabled for secure entry")
            }
            Self::InvalidCompositionKind { received } => write!(
                formatter,
                "composition history requires CompositionCommit, received {received:?}"
            ),
            Self::Edit(error) => error.fmt(formatter),
            Self::Controller(error) => error.fmt(formatter),
            Self::History(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TextControllerHistoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Edit(error) => Some(error),
            Self::Controller(error) => Some(error),
            Self::History(error) => Some(error),
            Self::Disabled | Self::SecureEntry | Self::InvalidCompositionKind { .. } => None,
        }
    }
}

/// Controller/session ownership or lifecycle failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextControllerError {
    SessionAlreadyOwned { session: TextSessionId },
    NoSession,
    SessionState(TextSessionStateError),
}

impl From<TextSessionStateError> for TextControllerError {
    fn from(error: TextSessionStateError) -> Self {
        Self::SessionState(error)
    }
}

impl fmt::Display for TextControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionAlreadyOwned { session } => {
                write!(
                    formatter,
                    "text controller already owns session {session:?}"
                )
            }
            Self::NoSession => formatter.write_str("text controller has no input session"),
            Self::SessionState(error) => write!(formatter, "invalid text session state: {error}"),
        }
    }
}

impl std::error::Error for TextControllerError {}

/// Local application editing owner over one neutral text buffer and at most one input session.
pub struct TextController {
    buffer: TextBuffer,
    session: Option<TextInputSession>,
    history: Option<EditHistory>,
    composition_history_origin: Option<TextSnapshot>,
    local: PhantomData<Rc<()>>,
}

impl fmt::Debug for TextController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextController")
            .field("revision", &self.buffer.revision())
            .field("len_bytes", &self.buffer.len_bytes())
            .field("selection", &self.buffer.selection())
            .field("composition", &self.buffer.composition())
            .field("session", &self.session)
            .field("history", &self.history)
            .field(
                "has_composition_history_origin",
                &self.composition_history_origin.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl TextController {
    pub fn new() -> Self {
        Self {
            buffer: TextBuffer::new(),
            session: None,
            history: None,
            composition_history_origin: None,
            local: PhantomData,
        }
    }

    pub fn from_text(text: impl Into<String>) -> Result<Self, TextBufferError> {
        Ok(Self {
            buffer: TextBuffer::from_text(text)?,
            session: None,
            history: None,
            composition_history_origin: None,
            local: PhantomData,
        })
    }

    pub const fn revision(&self) -> TextRevision {
        self.buffer.revision()
    }

    pub const fn selection(&self) -> TextSelection {
        self.buffer.selection()
    }

    pub const fn composition(&self) -> Option<TextRange> {
        self.buffer.composition()
    }

    pub fn snapshot(&self) -> TextSnapshot {
        self.buffer.snapshot()
    }

    /// Enables bounded plaintext history for an ordinary, non-secure controller.
    pub fn enable_edit_history(
        &mut self,
        policy: EditHistoryPolicy,
    ) -> Result<(), TextControllerHistoryError> {
        if self.secure_entry_active() {
            return Err(TextControllerHistoryError::SecureEntry);
        }
        self.history = Some(EditHistory::new(policy));
        self.composition_history_origin = None;
        Ok(())
    }

    /// Disables history and returns the removed store to the caller.
    pub fn disable_edit_history(&mut self) -> Option<EditHistory> {
        self.composition_history_origin = None;
        self.history.take()
    }

    pub const fn edit_history(&self) -> Option<&EditHistory> {
        self.history.as_ref()
    }

    pub fn edit_history_availability(&self) -> EditHistoryAvailability {
        match &self.history {
            Some(history) => EditHistoryAvailability {
                enabled: true,
                can_undo: history.can_undo(),
                can_redo: history.can_redo(),
            },
            None => EditHistoryAvailability::default(),
        }
    }

    /// Ends compatible edit grouping at a focus, session, or application command boundary.
    pub fn break_edit_history_merge(&mut self) {
        if let Some(history) = &mut self.history {
            history.break_merge();
        }
    }

    pub fn clear_edit_history(&mut self) {
        self.composition_history_origin = None;
        if let Some(history) = &mut self.history {
            history.clear();
        }
    }

    pub fn apply_edits(
        &mut self,
        batch: TextEditBatch,
    ) -> Result<TextControllerUpdate, EditRejected> {
        let update = self.apply_edits_raw(batch)?;
        self.reconcile_untracked_update(&update);
        Ok(update)
    }

    /// Applies and automatically records one explicitly classified direct edit.
    pub fn apply_edits_recorded(
        &mut self,
        batch: TextEditBatch,
        kind: EditHistoryKind,
        recorded_at: MonotonicInstant,
    ) -> Result<TextControllerUpdate, TextControllerHistoryError> {
        self.ensure_recordable_edit(batch.composition, recorded_at)?;
        let before = self.buffer.snapshot();
        let update = self
            .apply_edits_raw(batch)
            .map_err(TextControllerHistoryError::Edit)?;
        self.composition_history_origin = None;
        self.record_history(&before, &update.snapshot, kind, recorded_at)?;
        Ok(update)
    }

    fn apply_edits_raw(
        &mut self,
        batch: TextEditBatch,
    ) -> Result<TextControllerUpdate, EditRejected> {
        let previous_selection = self.buffer.selection();
        let previous_composition = self.buffer.composition();
        match self.buffer.apply_edits(batch) {
            Ok(outcome) => Ok(update_from_outcome(
                outcome,
                previous_selection,
                previous_composition,
            )),
            Err(error) => Err(self.rejected(EditRejectedReason::Edit(error))),
        }
    }

    pub fn apply_composition(
        &mut self,
        command: TextCompositionCommand,
    ) -> Result<TextControllerUpdate, EditRejected> {
        let update = self.apply_composition_raw(command)?;
        self.reconcile_untracked_update(&update);
        Ok(update)
    }

    /// Applies neutral composition transitions and records the completed composition as one unit.
    pub fn apply_composition_recorded(
        &mut self,
        command: TextCompositionCommand,
        recorded_at: MonotonicInstant,
    ) -> Result<TextControllerUpdate, TextControllerHistoryError> {
        self.ensure_history_enabled()?;
        let command_kind = command.kind();
        if command_kind == TextCompositionKind::Commit {
            self.validate_history_timestamp(recorded_at)?;
        }
        let before = self.buffer.snapshot();
        let update = self
            .apply_composition_raw(command)
            .map_err(TextControllerHistoryError::Edit)?;
        self.finish_recorded_composition(command_kind, before, &update, recorded_at)?;
        Ok(update)
    }

    fn apply_composition_raw(
        &mut self,
        command: TextCompositionCommand,
    ) -> Result<TextControllerUpdate, EditRejected> {
        let previous_selection = self.buffer.selection();
        let previous_composition = self.buffer.composition();
        match self.buffer.apply_composition(command) {
            Ok(outcome) => Ok(update_from_outcome(
                outcome,
                previous_selection,
                previous_composition,
            )),
            Err(error) => Err(self.rejected(EditRejectedReason::Composition(error))),
        }
    }

    /// Explicit revision-checked replacement used for programmatic synchronization.
    pub fn replace_text(
        &mut self,
        base_revision: TextRevision,
        replacement: impl Into<String>,
    ) -> Result<TextControllerUpdate, EditRejected> {
        let replacement = replacement.into();
        let end = TextOffset(u32::try_from(replacement.len()).unwrap_or(u32::MAX));
        self.apply_edits(TextEditBatch {
            base_revision,
            edits: vec![TextEdit {
                range: TextRange {
                    start: TextOffset::ZERO,
                    end: self.buffer.end(),
                },
                replacement,
            }],
            selection: TextSelection::collapsed(end, crate::ui::text::TextAffinity::Downstream),
            composition: None,
        })
    }

    /// Explicit revision-checked replacement that participates in owned history.
    pub fn replace_text_recorded(
        &mut self,
        base_revision: TextRevision,
        replacement: impl Into<String>,
        kind: EditHistoryKind,
        recorded_at: MonotonicInstant,
    ) -> Result<TextControllerUpdate, TextControllerHistoryError> {
        let replacement = replacement.into();
        let end = TextOffset(u32::try_from(replacement.len()).unwrap_or(u32::MAX));
        self.apply_edits_recorded(
            TextEditBatch {
                base_revision,
                edits: vec![TextEdit {
                    range: TextRange {
                        start: TextOffset::ZERO,
                        end: self.buffer.end(),
                    },
                    replacement,
                }],
                selection: TextSelection::collapsed(end, crate::ui::text::TextAffinity::Downstream),
                composition: None,
            },
            kind,
            recorded_at,
        )
    }

    pub fn set_selection(
        &mut self,
        base_revision: TextRevision,
        selection: TextSelection,
    ) -> Result<TextControllerUpdate, EditRejected> {
        self.apply_edits(TextEditBatch {
            base_revision,
            edits: Vec::new(),
            selection,
            composition: self.buffer.composition(),
        })
    }

    pub const fn session_id(&self) -> Option<TextSessionId> {
        match &self.session {
            Some(session) => Some(session.id()),
            None => None,
        }
    }

    pub const fn session_phase(&self) -> Option<TextSessionPhase> {
        match &self.session {
            Some(session) => Some(session.phase()),
            None => None,
        }
    }

    pub fn open_session(
        &mut self,
        id: TextSessionId,
        configuration: TextInputConfiguration,
        max_surrounding_bytes: u32,
    ) -> Result<TextInputRequest, TextControllerError> {
        if let Some(session) = &self.session {
            return Err(TextControllerError::SessionAlreadyOwned {
                session: session.id(),
            });
        }
        let mut session = TextInputSession::new(id, configuration, max_surrounding_bytes);
        let request = session.open(&self.buffer)?;
        self.session = Some(session);
        self.break_edit_history_merge();
        if configuration.secure_entry {
            self.disable_edit_history();
        }
        Ok(request)
    }

    pub fn update_session(&mut self) -> Result<TextInputRequest, TextControllerError> {
        let session = self
            .session
            .as_mut()
            .ok_or(TextControllerError::NoSession)?;
        Ok(session.update(&self.buffer)?)
    }

    pub fn set_session_configuration(
        &mut self,
        configuration: TextInputConfiguration,
    ) -> Result<TextInputRequest, TextControllerError> {
        let session = self
            .session
            .as_mut()
            .ok_or(TextControllerError::NoSession)?;
        session.set_configuration(configuration)?;
        let request = session.update(&self.buffer)?;
        if configuration.secure_entry {
            self.disable_edit_history();
        }
        Ok(request)
    }

    pub fn set_session_geometry(
        &mut self,
        geometry: TextInputGeometry,
    ) -> Result<TextInputRequest, TextControllerError> {
        let session = self
            .session
            .as_mut()
            .ok_or(TextControllerError::NoSession)?;
        session.set_geometry(geometry)?;
        Ok(session.update(&self.buffer)?)
    }

    pub fn close_session(&mut self) -> Result<TextInputRequest, TextControllerError> {
        let mut session = self.session.take().ok_or(TextControllerError::NoSession)?;
        match session.close() {
            Ok(request) => {
                self.break_edit_history_merge();
                self.composition_history_origin = None;
                Ok(request)
            }
            Err(error) => {
                self.session = Some(session);
                Err(error.into())
            }
        }
    }

    pub fn apply_session_delta(
        &mut self,
        delta: TextSessionDelta,
    ) -> Result<TextControllerSessionOutcome, TextControllerError> {
        let outcome = self.apply_session_delta_raw(delta)?;
        match &outcome {
            TextControllerSessionOutcome::Updated { update, .. } => {
                self.reconcile_untracked_update(update);
            }
            TextControllerSessionOutcome::Submitted(_) => self.break_edit_history_merge(),
            TextControllerSessionOutcome::Rejected { .. } => {}
        }
        Ok(outcome)
    }

    /// Applies a session delta and automatically records accepted edits with explicit metadata.
    pub fn apply_session_delta_recorded(
        &mut self,
        delta: TextSessionDelta,
        kind: EditHistoryKind,
        recorded_at: MonotonicInstant,
    ) -> Result<TextControllerSessionOutcome, TextControllerHistoryError> {
        self.ensure_history_enabled()?;
        let (is_edit, composition_kind) = match &delta.command {
            TextSessionCommand::Edit(batch) => {
                self.ensure_recordable_edit(batch.composition, recorded_at)?;
                (true, None)
            }
            TextSessionCommand::Composition(command) => {
                if kind != EditHistoryKind::CompositionCommit {
                    return Err(TextControllerHistoryError::InvalidCompositionKind {
                        received: kind,
                    });
                }
                if command.kind() == TextCompositionKind::Commit {
                    self.validate_history_timestamp(recorded_at)?;
                }
                (false, Some(command.kind()))
            }
            TextSessionCommand::PerformAction { .. } => (false, None),
        };
        let before = self.buffer.snapshot();
        let outcome = self
            .apply_session_delta_raw(delta)
            .map_err(TextControllerHistoryError::Controller)?;
        match &outcome {
            TextControllerSessionOutcome::Updated { update, .. } if is_edit => {
                self.composition_history_origin = None;
                self.record_history(&before, &update.snapshot, kind, recorded_at)?;
            }
            TextControllerSessionOutcome::Updated { update, .. } => {
                if let Some(command_kind) = composition_kind {
                    self.finish_recorded_composition(command_kind, before, update, recorded_at)?;
                }
            }
            TextControllerSessionOutcome::Submitted(_) => self.break_edit_history_merge(),
            TextControllerSessionOutcome::Rejected { .. } => {}
        }
        Ok(outcome)
    }

    fn apply_session_delta_raw(
        &mut self,
        delta: TextSessionDelta,
    ) -> Result<TextControllerSessionOutcome, TextControllerError> {
        let previous_selection = self.buffer.selection();
        let previous_composition = self.buffer.composition();
        let session = self
            .session
            .as_mut()
            .ok_or(TextControllerError::NoSession)?;
        let outcome = session.apply_delta(&mut self.buffer, delta);
        Ok(match outcome {
            TextSessionDeltaOutcome::Applied { outcome, request } => {
                TextControllerSessionOutcome::Updated {
                    update: update_from_outcome(outcome, previous_selection, previous_composition),
                    request,
                }
            }
            TextSessionDeltaOutcome::Action { action } => {
                TextControllerSessionOutcome::Submitted(Submitted {
                    revision: self.buffer.revision(),
                    action,
                })
            }
            TextSessionDeltaOutcome::Resynchronize { reason, request } => {
                TextControllerSessionOutcome::Rejected {
                    rejection: self.rejected(EditRejectedReason::Resynchronize(reason)),
                    request: Some(request),
                }
            }
            TextSessionDeltaOutcome::RejectedSession { expected, received } => {
                TextControllerSessionOutcome::Rejected {
                    rejection: self
                        .rejected(EditRejectedReason::WrongSession { expected, received }),
                    request: None,
                }
            }
            TextSessionDeltaOutcome::RejectedInactive { phase } => {
                TextControllerSessionOutcome::Rejected {
                    rejection: self.rejected(EditRejectedReason::InactiveSession { phase }),
                    request: None,
                }
            }
        })
    }

    pub fn apply_edit_history_command(
        &mut self,
        command: EditHistoryCommand,
    ) -> Result<TextControllerUpdate, TextControllerHistoryError> {
        let mut history = self
            .history
            .take()
            .ok_or(TextControllerHistoryError::Disabled)?;
        let result = match command {
            EditHistoryCommand::Undo => history.undo(self),
            EditHistoryCommand::Redo => history.redo(self),
        };
        self.history = Some(history);
        self.composition_history_origin = None;
        result.map_err(TextControllerHistoryError::History)
    }

    fn secure_entry_active(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.configuration().secure_entry)
    }

    fn ensure_history_enabled(&self) -> Result<(), TextControllerHistoryError> {
        if self.secure_entry_active() {
            return Err(TextControllerHistoryError::SecureEntry);
        }
        if self.history.is_none() {
            return Err(TextControllerHistoryError::Disabled);
        }
        Ok(())
    }

    fn validate_history_timestamp(
        &self,
        recorded_at: MonotonicInstant,
    ) -> Result<(), TextControllerHistoryError> {
        self.ensure_history_enabled()?;
        self.history
            .as_ref()
            .expect("history checked as enabled")
            .validate_recorded_at(recorded_at)
            .map_err(TextControllerHistoryError::History)
    }

    fn ensure_recordable_edit(
        &self,
        resulting_composition: Option<TextRange>,
        recorded_at: MonotonicInstant,
    ) -> Result<(), TextControllerHistoryError> {
        self.validate_history_timestamp(recorded_at)?;
        if self.buffer.composition().is_some() || resulting_composition.is_some() {
            return Err(TextControllerHistoryError::History(
                EditHistoryError::ActiveComposition,
            ));
        }
        Ok(())
    }

    fn record_history(
        &mut self,
        before: &TextSnapshot,
        after: &TextSnapshot,
        kind: EditHistoryKind,
        recorded_at: MonotonicInstant,
    ) -> Result<(), TextControllerHistoryError> {
        self.history
            .as_mut()
            .ok_or(TextControllerHistoryError::Disabled)?
            .record(before, after, kind, recorded_at)
            .map(|_| ())
            .map_err(TextControllerHistoryError::History)
    }

    fn finish_recorded_composition(
        &mut self,
        command_kind: TextCompositionKind,
        before: TextSnapshot,
        update: &TextControllerUpdate,
        recorded_at: MonotonicInstant,
    ) -> Result<(), TextControllerHistoryError> {
        match command_kind {
            TextCompositionKind::Start => {
                self.break_edit_history_merge();
                self.composition_history_origin = Some(before);
            }
            TextCompositionKind::Update => {}
            TextCompositionKind::Commit => {
                let Some(origin) = self.composition_history_origin.take() else {
                    self.clear_edit_history();
                    return Ok(());
                };
                self.record_history(
                    &origin,
                    &update.snapshot,
                    EditHistoryKind::CompositionCommit,
                    recorded_at,
                )?;
            }
            TextCompositionKind::Cancel => {
                self.break_edit_history_merge();
                if self
                    .composition_history_origin
                    .take()
                    .is_some_and(|origin| !snapshots_have_same_text(&origin, &update.snapshot))
                {
                    self.clear_edit_history();
                }
            }
        }
        Ok(())
    }

    fn reconcile_untracked_update(&mut self, update: &TextControllerUpdate) {
        self.composition_history_origin = None;
        if update.changed_text() {
            self.clear_edit_history();
        } else if update.changed_selection() || update.changed_composition() {
            self.break_edit_history_merge();
        }
    }

    fn rejected(&self, reason: EditRejectedReason) -> EditRejected {
        EditRejected {
            reason,
            current: self.buffer.snapshot(),
        }
    }
}

impl Default for TextController {
    fn default() -> Self {
        Self::new()
    }
}

fn snapshots_have_same_text(left: &TextSnapshot, right: &TextSnapshot) -> bool {
    let left: String = left.chunks().map(|chunk| chunk.text).collect();
    let right: String = right.chunks().map(|chunk| chunk.text).collect();
    left == right
}

fn update_from_outcome(
    outcome: TextEditOutcome,
    previous_selection: TextSelection,
    previous_composition: Option<TextRange>,
) -> TextControllerUpdate {
    let revision = outcome.snapshot.revision();
    let text_changed = (!outcome.changes.is_empty()).then_some(TextChanged {
        revision,
        changes: outcome.changes,
    });
    let selection_changed = (outcome.selection != previous_selection).then_some(SelectionChanged {
        revision,
        previous: previous_selection,
        current: outcome.selection,
    });
    let composition_changed =
        (outcome.composition != previous_composition).then_some(CompositionChanged {
            revision,
            previous: previous_composition,
            current: outcome.composition,
        });
    TextControllerUpdate {
        snapshot: outcome.snapshot,
        text_changed,
        selection_changed,
        composition_changed,
    }
}

#[cfg(test)]
mod tests;
