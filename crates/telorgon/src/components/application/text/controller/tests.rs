use crate::ui::text::{
    TextAffinity, TextInputPurpose, TextMultiline, TextRangeError, TextReturnKeyAction,
    TextSessionCommand, TextVirtualKeyboardPreference,
};

use super::*;

fn id(slot: u32, generation: u32) -> TextSessionId {
    TextSessionId::from_raw(slot, generation).unwrap()
}

fn range(start: u32, end: u32) -> TextRange {
    TextRange::new(TextOffset(start), TextOffset(end)).unwrap()
}

fn selection(anchor: u32, active: u32) -> TextSelection {
    TextSelection {
        anchor: TextOffset(anchor),
        active: TextOffset(active),
        affinity: TextAffinity::Downstream,
    }
}

fn text(snapshot: &TextSnapshot) -> String {
    snapshot.chunks().map(|chunk| chunk.text).collect()
}

fn configuration() -> TextInputConfiguration {
    TextInputConfiguration {
        purpose: TextInputPurpose::Search,
        multiline: TextMultiline::SingleLine,
        return_key: TextReturnKeyAction::Search,
        virtual_keyboard: TextVirtualKeyboardPreference::Show,
        ..TextInputConfiguration::default()
    }
}

#[test]
fn programmatic_replacement_is_revision_checked_and_publishes_typed_changes() {
    let mut controller = TextController::from_text("draft").unwrap();
    let old = controller.snapshot();
    let update = controller
        .replace_text(TextRevision::INITIAL, "published")
        .unwrap();

    assert_eq!(text(&old), "draft");
    assert_eq!(text(&update.snapshot), "published");
    assert_eq!(controller.revision(), TextRevision(1));
    assert!(update.changed_text());
    assert!(update.changed_selection());
    assert!(!update.changed_composition());
    assert_eq!(update.text_changed.as_ref().unwrap().changes.len(), 1);
    assert_eq!(update.selection_changed.unwrap().current, selection(9, 9));
}

#[test]
fn stale_and_invalid_edits_return_redacted_current_snapshot_without_mutation() {
    let mut controller = TextController::from_text("éx").unwrap();
    controller
        .replace_text(TextRevision::INITIAL, "stable")
        .unwrap();
    let stale = controller
        .replace_text(TextRevision::INITIAL, "private stale value")
        .unwrap_err();
    assert!(matches!(
        stale.reason,
        EditRejectedReason::Edit(TextEditError::StaleRevision { .. })
    ));
    assert_eq!(text(&stale.current), "stable");
    assert_eq!(text(&controller.snapshot()), "stable");
    assert!(!format!("{stale:?}").contains("private stale value"));

    let invalid = controller
        .apply_edits(TextEditBatch {
            base_revision: TextRevision(1),
            edits: vec![TextEdit {
                range: range(1, 2),
                replacement: String::new(),
            }],
            selection: selection(0, 0),
            composition: None,
        })
        .unwrap();
    assert_eq!(text(&invalid.snapshot), "sable");

    let mut unicode = TextController::from_text("éx").unwrap();
    let invalid = unicode
        .apply_edits(TextEditBatch {
            base_revision: TextRevision::INITIAL,
            edits: vec![TextEdit {
                range: range(1, 2),
                replacement: String::new(),
            }],
            selection: selection(0, 0),
            composition: None,
        })
        .unwrap_err();
    assert!(matches!(
        invalid.reason,
        EditRejectedReason::Edit(TextEditError::InvalidEditRange {
            error: TextRangeError::NotCharBoundary { .. },
            ..
        })
    ));
    assert_eq!(text(&unicode.snapshot()), "éx");
}

#[test]
fn selection_and_composition_outputs_preserve_direction_and_transition_state() {
    let mut controller = TextController::from_text("hello").unwrap();
    let selected = controller
        .set_selection(TextRevision::INITIAL, selection(5, 1))
        .unwrap();
    assert!(!selected.changed_text());
    assert_eq!(selected.selection_changed.unwrap().current, selection(5, 1));

    let started = controller
        .apply_composition(TextCompositionCommand::Start {
            base_revision: TextRevision(1),
            edits: Vec::new(),
            selection: selection(5, 5),
            composition: range(1, 5),
        })
        .unwrap();
    assert_eq!(
        started.composition_changed.unwrap().current,
        Some(range(1, 5))
    );
    assert_eq!(controller.composition(), Some(range(1, 5)));

    let rejected = controller
        .apply_composition(TextCompositionCommand::Start {
            base_revision: TextRevision(2),
            edits: Vec::new(),
            selection: selection(5, 5),
            composition: range(1, 5),
        })
        .unwrap_err();
    assert_eq!(
        rejected.reason,
        EditRejectedReason::Composition(TextCompositionError::AlreadyActive {
            composition: range(1, 5)
        })
    );
    assert_eq!(controller.revision(), TextRevision(2));
}

#[test]
fn session_lifecycle_routes_applied_edits_and_submission_without_platform_types() {
    let mut controller = TextController::from_text("find").unwrap();
    let session = id(7, 3);
    let open = controller
        .open_session(session, configuration(), 32)
        .unwrap();
    assert!(matches!(open, TextInputRequest::Open(_)));
    assert_eq!(controller.session_phase(), Some(TextSessionPhase::Open));
    assert_eq!(
        controller.open_session(id(8, 1), configuration(), 32),
        Err(TextControllerError::SessionAlreadyOwned { session })
    );

    let outcome = controller
        .apply_session_delta(TextSessionDelta {
            session,
            command: TextSessionCommand::Edit(TextEditBatch {
                base_revision: TextRevision::INITIAL,
                edits: vec![TextEdit {
                    range: range(4, 4),
                    replacement: " me".to_owned(),
                }],
                selection: selection(7, 7),
                composition: None,
            }),
        })
        .unwrap();
    let TextControllerSessionOutcome::Updated { update, request } = outcome else {
        panic!("session edit must update the controller");
    };
    assert_eq!(text(&update.snapshot), "find me");
    assert!(matches!(request, TextInputRequest::Update(_)));

    let outcome = controller
        .apply_session_delta(TextSessionDelta {
            session,
            command: TextSessionCommand::PerformAction {
                base_revision: TextRevision(1),
            },
        })
        .unwrap();
    let TextControllerSessionOutcome::Submitted(submitted) = outcome else {
        panic!("return action must remain distinct from an edit");
    };
    assert_eq!(submitted.revision, TextRevision(1));
    assert_eq!(submitted.action, TextReturnKeyAction::Search);
    assert!(matches!(
        controller.close_session().unwrap(),
        TextInputRequest::Close { session: closed } if closed == session
    ));
    assert_eq!(controller.session_id(), None);
}

#[test]
fn stale_and_wrong_session_deltas_reject_with_explicit_resynchronization() {
    let mut controller = TextController::from_text("value").unwrap();
    let session = id(1, 1);
    controller
        .open_session(session, configuration(), 16)
        .unwrap();

    let wrong = controller
        .apply_session_delta(TextSessionDelta {
            session: id(2, 1),
            command: TextSessionCommand::PerformAction {
                base_revision: TextRevision::INITIAL,
            },
        })
        .unwrap();
    assert!(matches!(
        wrong,
        TextControllerSessionOutcome::Rejected {
            rejection: EditRejected {
                reason: EditRejectedReason::WrongSession { .. },
                ..
            },
            request: None
        }
    ));

    controller
        .replace_text(TextRevision::INITIAL, "new value")
        .unwrap();
    let stale = controller
        .apply_session_delta(TextSessionDelta {
            session,
            command: TextSessionCommand::Edit(TextEditBatch {
                base_revision: TextRevision::INITIAL,
                edits: Vec::new(),
                selection: selection(5, 5),
                composition: None,
            }),
        })
        .unwrap();
    assert!(matches!(
        stale,
        TextControllerSessionOutcome::Rejected {
            rejection: EditRejected {
                reason: EditRejectedReason::Resynchronize(
                    TextInputResyncReason::StaleRevision { .. }
                ),
                ..
            },
            request: Some(TextInputRequest::Update(_))
        }
    ));
    assert_eq!(text(&controller.snapshot()), "new value");
}

#[test]
fn owned_history_records_merges_and_executes_typed_commands() {
    let mut controller = TextController::new();
    controller
        .enable_edit_history(EditHistoryPolicy::new(10, 1024, 10).unwrap())
        .unwrap();
    controller
        .replace_text_recorded(
            TextRevision::INITIAL,
            "a",
            EditHistoryKind::Typing,
            MonotonicInstant::from_nanos(1),
        )
        .unwrap();
    controller
        .replace_text_recorded(
            TextRevision(1),
            "ab",
            EditHistoryKind::Typing,
            MonotonicInstant::from_nanos(5),
        )
        .unwrap();

    assert_eq!(controller.edit_history().unwrap().undo_len(), 1);
    assert_eq!(
        controller.edit_history_availability(),
        EditHistoryAvailability {
            enabled: true,
            can_undo: true,
            can_redo: false,
        }
    );
    let undone = controller
        .apply_edit_history_command(EditHistoryCommand::Undo)
        .unwrap();
    assert_eq!(text(&undone.snapshot), "");
    assert_eq!(
        controller.edit_history_availability(),
        EditHistoryAvailability {
            enabled: true,
            can_undo: false,
            can_redo: true,
        }
    );
    let redone = controller
        .apply_edit_history_command(EditHistoryCommand::Redo)
        .unwrap();
    assert_eq!(text(&redone.snapshot), "ab");
}

#[test]
fn invalid_timestamp_rejects_before_mutation_and_untracked_edits_reset_history() {
    let mut controller = TextController::new();
    controller
        .enable_edit_history(EditHistoryPolicy::default())
        .unwrap();
    controller
        .replace_text_recorded(
            TextRevision::INITIAL,
            "tracked",
            EditHistoryKind::Paste,
            MonotonicInstant::from_nanos(10),
        )
        .unwrap();
    let revision = controller.revision();
    assert!(matches!(
        controller.replace_text_recorded(
            revision,
            "must not apply",
            EditHistoryKind::Typing,
            MonotonicInstant::from_nanos(9),
        ),
        Err(TextControllerHistoryError::History(
            EditHistoryError::NonMonotonicTimestamp { .. }
        ))
    ));
    assert_eq!(controller.revision(), revision);
    assert_eq!(text(&controller.snapshot()), "tracked");
    assert!(controller.edit_history_availability().can_undo);

    controller
        .replace_text(revision, "untracked replacement")
        .unwrap();
    assert!(!controller.edit_history_availability().can_undo);
}

#[test]
fn selection_boundary_prevents_later_typing_merge_without_erasing_history() {
    let mut controller = TextController::new();
    controller
        .enable_edit_history(EditHistoryPolicy::default())
        .unwrap();
    controller
        .replace_text_recorded(
            TextRevision::INITIAL,
            "a",
            EditHistoryKind::Typing,
            MonotonicInstant::from_nanos(1),
        )
        .unwrap();
    controller
        .set_selection(TextRevision(1), selection(0, 0))
        .unwrap();
    controller
        .replace_text_recorded(
            TextRevision(2),
            "ab",
            EditHistoryKind::Typing,
            MonotonicInstant::from_nanos(2),
        )
        .unwrap();
    assert_eq!(controller.edit_history().unwrap().undo_len(), 2);
}

#[test]
fn session_edit_records_with_explicit_kind_and_rejected_delta_preserves_history() {
    let mut controller = TextController::from_text("find").unwrap();
    controller
        .enable_edit_history(EditHistoryPolicy::default())
        .unwrap();
    let session = id(9, 1);
    controller
        .open_session(session, configuration(), 32)
        .unwrap();
    let outcome = controller
        .apply_session_delta_recorded(
            TextSessionDelta {
                session,
                command: TextSessionCommand::Edit(TextEditBatch {
                    base_revision: TextRevision::INITIAL,
                    edits: vec![TextEdit {
                        range: range(4, 4),
                        replacement: " me".to_owned(),
                    }],
                    selection: selection(7, 7),
                    composition: None,
                }),
            },
            EditHistoryKind::Typing,
            MonotonicInstant::from_nanos(1),
        )
        .unwrap();
    assert!(matches!(
        outcome,
        TextControllerSessionOutcome::Updated { .. }
    ));
    assert!(controller.edit_history_availability().can_undo);

    let rejected = controller
        .apply_session_delta_recorded(
            TextSessionDelta {
                session,
                command: TextSessionCommand::Edit(TextEditBatch {
                    base_revision: TextRevision::INITIAL,
                    edits: Vec::new(),
                    selection: selection(0, 0),
                    composition: None,
                }),
            },
            EditHistoryKind::Typing,
            MonotonicInstant::from_nanos(2),
        )
        .unwrap();
    assert!(matches!(
        rejected,
        TextControllerSessionOutcome::Rejected { .. }
    ));
    assert_eq!(controller.edit_history().unwrap().undo_len(), 1);
}

#[test]
fn committed_composition_is_one_unit_from_precomposition_text() {
    let mut controller = TextController::from_text("a").unwrap();
    controller
        .enable_edit_history(EditHistoryPolicy::default())
        .unwrap();
    controller
        .apply_composition_recorded(
            TextCompositionCommand::Start {
                base_revision: TextRevision::INITIAL,
                edits: vec![TextEdit {
                    range: range(1, 1),
                    replacement: "x".to_owned(),
                }],
                selection: selection(2, 2),
                composition: range(1, 2),
            },
            MonotonicInstant::from_nanos(1),
        )
        .unwrap();
    controller
        .apply_composition_recorded(
            TextCompositionCommand::Update {
                base_revision: TextRevision(1),
                edits: vec![TextEdit {
                    range: range(1, 2),
                    replacement: "xy".to_owned(),
                }],
                selection: selection(3, 3),
                composition: range(1, 3),
            },
            MonotonicInstant::from_nanos(2),
        )
        .unwrap();
    controller
        .apply_composition_recorded(
            TextCompositionCommand::Commit {
                base_revision: TextRevision(2),
                edits: vec![TextEdit {
                    range: range(1, 3),
                    replacement: "字".to_owned(),
                }],
                selection: selection(4, 4),
            },
            MonotonicInstant::from_nanos(3),
        )
        .unwrap();
    assert_eq!(text(&controller.snapshot()), "a字");
    assert_eq!(controller.edit_history().unwrap().undo_len(), 1);
    let undone = controller
        .apply_edit_history_command(EditHistoryCommand::Undo)
        .unwrap();
    assert_eq!(text(&undone.snapshot), "a");
}

#[test]
fn secure_session_drops_plaintext_history_and_prevents_reenable() {
    let mut controller = TextController::from_text("ordinary").unwrap();
    controller
        .enable_edit_history(EditHistoryPolicy::default())
        .unwrap();
    controller
        .replace_text_recorded(
            TextRevision::INITIAL,
            "retained before secure entry",
            EditHistoryKind::Typing,
            MonotonicInstant::from_nanos(1),
        )
        .unwrap();
    let mut secure = configuration();
    secure.secure_entry = true;
    controller.open_session(id(10, 1), secure, 0).unwrap();
    assert_eq!(
        controller.edit_history_availability(),
        EditHistoryAvailability::default()
    );
    assert!(matches!(
        controller.enable_edit_history(EditHistoryPolicy::default()),
        Err(TextControllerHistoryError::SecureEntry)
    ));
}
