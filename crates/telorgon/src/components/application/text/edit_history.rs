//! Bounded deterministic application edit-history policy.

use std::collections::VecDeque;
use std::fmt;

use crate::runtime::MonotonicInstant;
use crate::ui::text::{
    TextEdit, TextEditBatch, TextOffset, TextRange, TextRevision, TextSelection, TextSnapshot,
};

use super::{EditRejected, TextController, TextControllerUpdate};

/// Validated unit and retained-byte budgets plus the compatible-edit merge deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditHistoryPolicy {
    max_units: usize,
    max_retained_bytes: usize,
    merge_window_nanos: u64,
}

impl EditHistoryPolicy {
    pub fn new(
        max_units: usize,
        max_retained_bytes: usize,
        merge_window_nanos: u64,
    ) -> Result<Self, EditHistoryPolicyError> {
        if max_units == 0 {
            return Err(EditHistoryPolicyError::ZeroUnitBudget);
        }
        if max_retained_bytes == 0 {
            return Err(EditHistoryPolicyError::ZeroByteBudget);
        }
        Ok(Self {
            max_units,
            max_retained_bytes,
            merge_window_nanos,
        })
    }

    pub const fn max_units(self) -> usize {
        self.max_units
    }

    pub const fn max_retained_bytes(self) -> usize {
        self.max_retained_bytes
    }

    pub const fn merge_window_nanos(self) -> u64 {
        self.merge_window_nanos
    }
}

impl Default for EditHistoryPolicy {
    fn default() -> Self {
        Self {
            max_units: 100,
            max_retained_bytes: 4 * 1024 * 1024,
            merge_window_nanos: 750_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditHistoryPolicyError {
    ZeroUnitBudget,
    ZeroByteBudget,
}

impl fmt::Display for EditHistoryPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid edit-history policy: {self:?}")
    }
}

impl std::error::Error for EditHistoryPolicyError {}

/// Application meaning of one accepted text edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditHistoryKind {
    Typing,
    DeleteBackward,
    DeleteForward,
    Paste,
    Cut,
    Drop,
    Replacement,
    CompositionCommit,
    ProgrammaticReplacement,
    HistoryReset,
}

impl EditHistoryKind {
    const fn can_merge_with(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Typing, Self::Typing)
                | (Self::DeleteBackward, Self::DeleteBackward)
                | (Self::DeleteForward, Self::DeleteForward)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditHistoryRecordOutcome {
    Recorded,
    Merged,
    IgnoredSelectionOnly,
    Reset,
    ResetAndRecorded,
    DroppedOversized,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditHistoryDiagnostics {
    pub recorded_units: u64,
    pub merged_edits: u64,
    pub ignored_selection_only: u64,
    pub explicit_resets: u64,
    pub continuity_resets: u64,
    pub oversized_drops: u64,
    pub pruned_units: u64,
    pub undo_commits: u64,
    pub redo_commits: u64,
}

#[derive(Clone)]
struct HistoryState {
    revision: TextRevision,
    text: String,
    selection: TextSelection,
}

impl HistoryState {
    fn from_snapshot(snapshot: &TextSnapshot) -> Self {
        Self {
            revision: snapshot.revision(),
            text: snapshot.chunks().map(|chunk| chunk.text).collect(),
            selection: snapshot.selection(),
        }
    }

    fn retained_bytes(&self) -> usize {
        self.text.len()
    }

    fn same_text(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl fmt::Debug for HistoryState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HistoryState")
            .field("revision", &self.revision)
            .field("len_bytes", &self.text.len())
            .field("selection", &self.selection)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
struct EditHistoryUnit {
    before: HistoryState,
    after: HistoryState,
    kind: EditHistoryKind,
    recorded_at: MonotonicInstant,
}

impl EditHistoryUnit {
    fn retained_bytes(&self) -> usize {
        self.before
            .retained_bytes()
            .saturating_add(self.after.retained_bytes())
    }
}

/// Plaintext history store for ordinary fields. Secure fields must disable it until protected
/// retention exists.
pub struct EditHistory {
    policy: EditHistoryPolicy,
    undo: VecDeque<EditHistoryUnit>,
    redo: Vec<EditHistoryUnit>,
    retained_bytes: usize,
    merge_open: bool,
    last_recorded_at: Option<MonotonicInstant>,
    diagnostics: EditHistoryDiagnostics,
}

impl fmt::Debug for EditHistory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EditHistory")
            .field("policy", &self.policy)
            .field("undo_units", &self.undo.len())
            .field("redo_units", &self.redo.len())
            .field("retained_bytes", &self.retained_bytes)
            .field("merge_open", &self.merge_open)
            .field("last_recorded_at", &self.last_recorded_at)
            .field("diagnostics", &self.diagnostics)
            .finish_non_exhaustive()
    }
}

impl EditHistory {
    pub fn new(policy: EditHistoryPolicy) -> Self {
        Self {
            policy,
            undo: VecDeque::new(),
            redo: Vec::new(),
            retained_bytes: 0,
            merge_open: false,
            last_recorded_at: None,
            diagnostics: EditHistoryDiagnostics::default(),
        }
    }

    pub const fn policy(&self) -> EditHistoryPolicy {
        self.policy
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub const fn diagnostics(&self) -> EditHistoryDiagnostics {
        self.diagnostics
    }

    pub(crate) fn validate_recorded_at(
        &self,
        recorded_at: MonotonicInstant,
    ) -> Result<(), EditHistoryError> {
        if let Some(previous) = self.last_recorded_at
            && recorded_at < previous
        {
            return Err(EditHistoryError::NonMonotonicTimestamp {
                previous,
                received: recorded_at,
            });
        }
        Ok(())
    }

    /// Ends compatible typing/deletion grouping at an explicit focus, session, or command boundary.
    pub fn break_merge(&mut self) {
        self.merge_open = false;
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.retained_bytes = 0;
        self.merge_open = false;
        self.last_recorded_at = None;
    }

    pub fn record(
        &mut self,
        before: &TextSnapshot,
        after: &TextSnapshot,
        kind: EditHistoryKind,
        recorded_at: MonotonicInstant,
    ) -> Result<EditHistoryRecordOutcome, EditHistoryError> {
        if after.revision() <= before.revision() {
            return Err(EditHistoryError::NonAdvancingRevision {
                before: before.revision(),
                after: after.revision(),
            });
        }
        if before.composition().is_some() || after.composition().is_some() {
            return Err(EditHistoryError::ActiveComposition);
        }
        self.validate_recorded_at(recorded_at)?;

        if kind == EditHistoryKind::HistoryReset {
            self.clear();
            self.last_recorded_at = Some(recorded_at);
            self.diagnostics.explicit_resets += 1;
            return Ok(EditHistoryRecordOutcome::Reset);
        }

        let before = HistoryState::from_snapshot(before);
        let after = HistoryState::from_snapshot(after);
        if before.same_text(&after) {
            self.merge_open = false;
            self.last_recorded_at = Some(recorded_at);
            self.diagnostics.ignored_selection_only += 1;
            return Ok(EditHistoryRecordOutcome::IgnoredSelectionOnly);
        }

        let continuity_reset = self
            .undo
            .back()
            .is_some_and(|unit| !unit.after.same_text(&before));
        if continuity_reset {
            self.clear();
            self.diagnostics.continuity_resets += 1;
        } else {
            self.clear_redo();
        }

        let unit = EditHistoryUnit {
            before,
            after,
            kind,
            recorded_at,
        };
        if unit.retained_bytes() > self.policy.max_retained_bytes {
            self.clear();
            self.last_recorded_at = Some(recorded_at);
            self.diagnostics.oversized_drops += 1;
            return Ok(EditHistoryRecordOutcome::DroppedOversized);
        }

        let can_merge = !continuity_reset
            && self.merge_open
            && self.undo.back().is_some_and(|previous| {
                previous.kind.can_merge_with(kind)
                    && recorded_at
                        .as_nanos()
                        .saturating_sub(previous.recorded_at.as_nanos())
                        <= self.policy.merge_window_nanos
            });
        let outcome = if can_merge {
            let previous = self.undo.back_mut().expect("merge candidate exists");
            self.retained_bytes = self
                .retained_bytes
                .saturating_sub(previous.retained_bytes());
            previous.after = unit.after;
            previous.recorded_at = recorded_at;
            self.retained_bytes = self
                .retained_bytes
                .saturating_add(previous.retained_bytes());
            self.diagnostics.merged_edits += 1;
            EditHistoryRecordOutcome::Merged
        } else {
            self.retained_bytes = self.retained_bytes.saturating_add(unit.retained_bytes());
            self.undo.push_back(unit);
            self.diagnostics.recorded_units += 1;
            if continuity_reset {
                EditHistoryRecordOutcome::ResetAndRecorded
            } else {
                EditHistoryRecordOutcome::Recorded
            }
        };
        if self.retained_bytes > self.policy.max_retained_bytes {
            self.clear();
            self.last_recorded_at = Some(recorded_at);
            self.diagnostics.oversized_drops += 1;
            return Ok(EditHistoryRecordOutcome::DroppedOversized);
        }
        self.merge_open = matches!(
            kind,
            EditHistoryKind::Typing
                | EditHistoryKind::DeleteBackward
                | EditHistoryKind::DeleteForward
        );
        self.last_recorded_at = Some(recorded_at);
        self.prune_to_policy();
        Ok(outcome)
    }

    pub fn undo(
        &mut self,
        controller: &mut TextController,
    ) -> Result<TextControllerUpdate, EditHistoryError> {
        let unit = self.undo.back().ok_or(EditHistoryError::NothingToUndo)?;
        ensure_controller_matches(controller, &unit.after)?;
        let update = apply_state(controller, &unit.before).map_err(EditHistoryError::Controller)?;
        let unit = self.undo.pop_back().expect("undo unit checked as present");
        self.redo.push(unit);
        self.merge_open = false;
        self.diagnostics.undo_commits += 1;
        Ok(update)
    }

    pub fn redo(
        &mut self,
        controller: &mut TextController,
    ) -> Result<TextControllerUpdate, EditHistoryError> {
        let unit = self.redo.last().ok_or(EditHistoryError::NothingToRedo)?;
        ensure_controller_matches(controller, &unit.before)?;
        let update = apply_state(controller, &unit.after).map_err(EditHistoryError::Controller)?;
        let unit = self.redo.pop().expect("redo unit checked as present");
        self.undo.push_back(unit);
        self.merge_open = false;
        self.diagnostics.redo_commits += 1;
        Ok(update)
    }

    fn clear_redo(&mut self) {
        for unit in self.redo.drain(..) {
            self.retained_bytes = self.retained_bytes.saturating_sub(unit.retained_bytes());
        }
    }

    fn prune_to_policy(&mut self) {
        while self.undo.len() + self.redo.len() > self.policy.max_units
            || self.retained_bytes > self.policy.max_retained_bytes
        {
            let Some(unit) = self.undo.pop_front() else {
                break;
            };
            self.retained_bytes = self.retained_bytes.saturating_sub(unit.retained_bytes());
            self.diagnostics.pruned_units += 1;
        }
    }
}

impl Default for EditHistory {
    fn default() -> Self {
        Self::new(EditHistoryPolicy::default())
    }
}

fn ensure_controller_matches(
    controller: &TextController,
    expected: &HistoryState,
) -> Result<(), EditHistoryError> {
    if controller.composition().is_some() {
        return Err(EditHistoryError::ActiveComposition);
    }
    let current: String = controller
        .snapshot()
        .chunks()
        .map(|chunk| chunk.text)
        .collect();
    if current != expected.text {
        return Err(EditHistoryError::Diverged);
    }
    Ok(())
}

fn apply_state(
    controller: &mut TextController,
    target: &HistoryState,
) -> Result<TextControllerUpdate, EditRejected> {
    let current = controller.snapshot();
    controller.apply_edits(TextEditBatch {
        base_revision: current.revision(),
        edits: vec![TextEdit {
            range: TextRange {
                start: TextOffset::ZERO,
                end: current.end(),
            },
            replacement: target.text.clone(),
        }],
        selection: target.selection,
        composition: None,
    })
}

#[derive(Debug)]
pub enum EditHistoryError {
    NonAdvancingRevision {
        before: TextRevision,
        after: TextRevision,
    },
    NonMonotonicTimestamp {
        previous: MonotonicInstant,
        received: MonotonicInstant,
    },
    ActiveComposition,
    NothingToUndo,
    NothingToRedo,
    Diverged,
    Controller(EditRejected),
}

impl fmt::Display for EditHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonAdvancingRevision { before, after } => write!(
                formatter,
                "edit-history revision did not advance: {} -> {}",
                before.0, after.0
            ),
            Self::NonMonotonicTimestamp { previous, received } => write!(
                formatter,
                "edit-history timestamp moved backward: {} -> {}",
                previous.as_nanos(),
                received.as_nanos()
            ),
            Self::ActiveComposition => {
                formatter.write_str("edit history cannot record or traverse active composition")
            }
            Self::NothingToUndo => formatter.write_str("edit history has nothing to undo"),
            Self::NothingToRedo => formatter.write_str("edit history has nothing to redo"),
            Self::Diverged => formatter.write_str("edit history does not match controller text"),
            Self::Controller(error) => write!(formatter, "history edit was rejected: {error}"),
        }
    }
}

impl std::error::Error for EditHistoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::text::TextAffinity;

    fn text(snapshot: &TextSnapshot) -> String {
        snapshot.chunks().map(|chunk| chunk.text).collect()
    }

    fn replace_and_record(
        controller: &mut TextController,
        history: &mut EditHistory,
        replacement: &str,
        kind: EditHistoryKind,
        at: u64,
    ) -> EditHistoryRecordOutcome {
        let before = controller.snapshot();
        let update = controller
            .replace_text(before.revision(), replacement)
            .unwrap();
        history
            .record(
                &before,
                &update.snapshot,
                kind,
                MonotonicInstant::from_nanos(at),
            )
            .unwrap()
    }

    #[test]
    fn policy_rejects_unbounded_zero_capacity() {
        assert_eq!(
            EditHistoryPolicy::new(0, 1024, 10),
            Err(EditHistoryPolicyError::ZeroUnitBudget)
        );
        assert_eq!(
            EditHistoryPolicy::new(10, 0, 10),
            Err(EditHistoryPolicyError::ZeroByteBudget)
        );
    }

    #[test]
    fn compatible_typing_merges_until_deadline_or_explicit_boundary() {
        let mut controller = TextController::new();
        let mut history = EditHistory::new(EditHistoryPolicy::new(10, 1024, 10).unwrap());
        assert_eq!(
            replace_and_record(
                &mut controller,
                &mut history,
                "a",
                EditHistoryKind::Typing,
                1
            ),
            EditHistoryRecordOutcome::Recorded
        );
        assert_eq!(
            replace_and_record(
                &mut controller,
                &mut history,
                "ab",
                EditHistoryKind::Typing,
                5
            ),
            EditHistoryRecordOutcome::Merged
        );
        assert_eq!(history.undo_len(), 1);
        assert_eq!(
            replace_and_record(
                &mut controller,
                &mut history,
                "abc",
                EditHistoryKind::Typing,
                20
            ),
            EditHistoryRecordOutcome::Recorded
        );
        history.break_merge();
        assert_eq!(
            replace_and_record(
                &mut controller,
                &mut history,
                "abcd",
                EditHistoryKind::Typing,
                21
            ),
            EditHistoryRecordOutcome::Recorded
        );
        assert_eq!(history.undo_len(), 3);

        assert_eq!(
            text(&history.undo(&mut controller).unwrap().snapshot),
            "abc"
        );
        assert_eq!(text(&history.undo(&mut controller).unwrap().snapshot), "ab");
        assert_eq!(text(&history.undo(&mut controller).unwrap().snapshot), "");
        assert!(!history.can_undo());
        assert!(history.can_redo());
        assert_eq!(text(&history.redo(&mut controller).unwrap().snapshot), "ab");
    }

    #[test]
    fn paste_replacement_and_programmatic_edits_are_separate_units() {
        let mut controller = TextController::from_text("a").unwrap();
        let mut history = EditHistory::default();
        for (value, kind, at) in [
            ("ab", EditHistoryKind::Paste, 1),
            ("c", EditHistoryKind::Replacement, 2),
            ("program", EditHistoryKind::ProgrammaticReplacement, 3),
        ] {
            assert_eq!(
                replace_and_record(&mut controller, &mut history, value, kind, at),
                EditHistoryRecordOutcome::Recorded
            );
        }
        assert_eq!(history.undo_len(), 3);
        assert_eq!(text(&history.undo(&mut controller).unwrap().snapshot), "c");
        assert_eq!(text(&history.undo(&mut controller).unwrap().snapshot), "ab");
        assert_eq!(text(&history.undo(&mut controller).unwrap().snapshot), "a");
    }

    #[test]
    fn selection_only_updates_do_not_create_units_and_break_merging() {
        let mut controller = TextController::from_text("abc").unwrap();
        let mut history = EditHistory::default();
        replace_and_record(
            &mut controller,
            &mut history,
            "abcd",
            EditHistoryKind::Typing,
            1,
        );
        let before = controller.snapshot();
        let update = controller
            .set_selection(
                before.revision(),
                TextSelection::collapsed(TextOffset::ZERO, TextAffinity::Upstream),
            )
            .unwrap();
        assert_eq!(
            history
                .record(
                    &before,
                    &update.snapshot,
                    EditHistoryKind::Typing,
                    MonotonicInstant::from_nanos(2),
                )
                .unwrap(),
            EditHistoryRecordOutcome::IgnoredSelectionOnly
        );
        assert_eq!(history.undo_len(), 1);
        assert_eq!(
            replace_and_record(
                &mut controller,
                &mut history,
                "abcde",
                EditHistoryKind::Typing,
                3
            ),
            EditHistoryRecordOutcome::Recorded
        );
    }

    #[test]
    fn budgets_prune_oldest_units_and_oversized_edits_reset_history() {
        let mut controller = TextController::new();
        let mut history = EditHistory::new(EditHistoryPolicy::new(2, 64, 0).unwrap());
        for (index, value) in ["a", "ab", "abc"].into_iter().enumerate() {
            history.break_merge();
            replace_and_record(
                &mut controller,
                &mut history,
                value,
                EditHistoryKind::Typing,
                index as u64,
            );
        }
        assert_eq!(history.undo_len(), 2);
        assert_eq!(history.diagnostics().pruned_units, 1);

        let oversized = "x".repeat(80);
        assert_eq!(
            replace_and_record(
                &mut controller,
                &mut history,
                &oversized,
                EditHistoryKind::Paste,
                4,
            ),
            EditHistoryRecordOutcome::DroppedOversized
        );
        assert!(!history.can_undo());
        assert_eq!(history.retained_bytes(), 0);
    }

    #[test]
    fn merged_unit_that_crosses_byte_budget_is_dropped() {
        let mut controller = TextController::from_text("123456").unwrap();
        let mut history = EditHistory::new(EditHistoryPolicy::new(10, 10, 100).unwrap());
        assert_eq!(
            replace_and_record(
                &mut controller,
                &mut history,
                "1",
                EditHistoryKind::Typing,
                1,
            ),
            EditHistoryRecordOutcome::Recorded
        );
        assert_eq!(
            replace_and_record(
                &mut controller,
                &mut history,
                "123456",
                EditHistoryKind::Typing,
                2,
            ),
            EditHistoryRecordOutcome::DroppedOversized
        );
        assert!(!history.can_undo());
        assert_eq!(history.retained_bytes(), 0);
        assert_eq!(history.diagnostics().oversized_drops, 1);
    }

    #[test]
    fn new_edit_after_undo_clears_redo_and_explicit_reset_records_nothing() {
        let mut controller = TextController::new();
        let mut history = EditHistory::default();
        replace_and_record(
            &mut controller,
            &mut history,
            "a",
            EditHistoryKind::Typing,
            1,
        );
        history.break_merge();
        replace_and_record(
            &mut controller,
            &mut history,
            "ab",
            EditHistoryKind::Typing,
            2,
        );
        history.undo(&mut controller).unwrap();
        assert!(history.can_redo());
        replace_and_record(
            &mut controller,
            &mut history,
            "alternate",
            EditHistoryKind::Replacement,
            3,
        );
        assert!(!history.can_redo());
        let before = controller.snapshot();
        let update = controller
            .set_selection(before.revision(), before.selection())
            .unwrap();
        assert_eq!(
            history
                .record(
                    &before,
                    &update.snapshot,
                    EditHistoryKind::HistoryReset,
                    MonotonicInstant::from_nanos(4),
                )
                .unwrap(),
            EditHistoryRecordOutcome::Reset
        );
        assert!(!history.can_undo());
    }

    #[test]
    fn divergence_and_active_composition_reject_without_moving_stacks() {
        let mut controller = TextController::new();
        let mut history = EditHistory::default();
        replace_and_record(
            &mut controller,
            &mut history,
            "tracked",
            EditHistoryKind::Paste,
            1,
        );
        controller
            .replace_text(controller.revision(), "untracked")
            .unwrap();
        assert!(matches!(
            history.undo(&mut controller),
            Err(EditHistoryError::Diverged)
        ));
        assert_eq!(history.undo_len(), 1);

        let before = controller.snapshot();
        let composition = controller
            .apply_composition(crate::ui::text::TextCompositionCommand::Start {
                base_revision: before.revision(),
                edits: Vec::new(),
                selection: before.selection(),
                composition: TextRange::collapsed(before.end()),
            })
            .unwrap();
        assert!(matches!(
            history.record(
                &before,
                &composition.snapshot,
                EditHistoryKind::CompositionCommit,
                MonotonicInstant::from_nanos(2),
            ),
            Err(EditHistoryError::ActiveComposition)
        ));
    }
}
