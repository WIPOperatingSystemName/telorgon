use std::fmt;

use crate::ui::text::{
    TextBuffer, TextEdit, TextEditBatch, TextEditError, TextEditOutcome, TextRange, TextRevision,
    TextSelection,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextCompositionKind {
    Start,
    Update,
    Commit,
    Cancel,
}

#[derive(Clone, PartialEq, Eq)]
pub enum TextCompositionCommand {
    Start {
        base_revision: TextRevision,
        edits: Vec<TextEdit>,
        selection: TextSelection,
        composition: TextRange,
    },
    Update {
        base_revision: TextRevision,
        edits: Vec<TextEdit>,
        selection: TextSelection,
        composition: TextRange,
    },
    Commit {
        base_revision: TextRevision,
        edits: Vec<TextEdit>,
        selection: TextSelection,
    },
    Cancel {
        base_revision: TextRevision,
        edits: Vec<TextEdit>,
        selection: TextSelection,
    },
}

impl fmt::Debug for TextCompositionCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, base_revision, edits, selection, composition) = self.parts();
        f.debug_struct("TextCompositionCommand")
            .field("kind", &kind)
            .field("base_revision", &base_revision)
            .field("edits", &edits)
            .field("selection", &selection)
            .field("composition", &composition)
            .finish()
    }
}

impl TextCompositionCommand {
    pub const fn kind(&self) -> TextCompositionKind {
        match self {
            Self::Start { .. } => TextCompositionKind::Start,
            Self::Update { .. } => TextCompositionKind::Update,
            Self::Commit { .. } => TextCompositionKind::Commit,
            Self::Cancel { .. } => TextCompositionKind::Cancel,
        }
    }

    pub const fn base_revision(&self) -> TextRevision {
        match self {
            Self::Start { base_revision, .. }
            | Self::Update { base_revision, .. }
            | Self::Commit { base_revision, .. }
            | Self::Cancel { base_revision, .. } => *base_revision,
        }
    }

    fn parts(
        &self,
    ) -> (
        TextCompositionKind,
        TextRevision,
        &[TextEdit],
        TextSelection,
        Option<TextRange>,
    ) {
        match self {
            Self::Start {
                base_revision,
                edits,
                selection,
                composition,
            } => (
                TextCompositionKind::Start,
                *base_revision,
                edits,
                *selection,
                Some(*composition),
            ),
            Self::Update {
                base_revision,
                edits,
                selection,
                composition,
            } => (
                TextCompositionKind::Update,
                *base_revision,
                edits,
                *selection,
                Some(*composition),
            ),
            Self::Commit {
                base_revision,
                edits,
                selection,
            } => (
                TextCompositionKind::Commit,
                *base_revision,
                edits,
                *selection,
                None,
            ),
            Self::Cancel {
                base_revision,
                edits,
                selection,
            } => (
                TextCompositionKind::Cancel,
                *base_revision,
                edits,
                *selection,
                None,
            ),
        }
    }

    fn into_batch(self) -> TextEditBatch {
        match self {
            Self::Start {
                base_revision,
                edits,
                selection,
                composition,
            }
            | Self::Update {
                base_revision,
                edits,
                selection,
                composition,
            } => TextEditBatch {
                base_revision,
                edits,
                selection,
                composition: Some(composition),
            },
            Self::Commit {
                base_revision,
                edits,
                selection,
            }
            | Self::Cancel {
                base_revision,
                edits,
                selection,
            } => TextEditBatch {
                base_revision,
                edits,
                selection,
                composition: None,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextCompositionError {
    AlreadyActive { composition: TextRange },
    NotActive { command: TextCompositionKind },
    Edit(TextEditError),
}

impl fmt::Display for TextCompositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyActive { composition } => write!(
                f,
                "cannot start text composition while range {}..{} is already active",
                composition.start.0, composition.end.0
            ),
            Self::NotActive { command } => {
                write!(
                    f,
                    "cannot {command:?} text composition because none is active"
                )
            }
            Self::Edit(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TextCompositionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Edit(error) => Some(error),
            Self::AlreadyActive { .. } | Self::NotActive { .. } => None,
        }
    }
}

impl From<TextEditError> for TextCompositionError {
    fn from(error: TextEditError) -> Self {
        Self::Edit(error)
    }
}

impl TextBuffer {
    pub fn apply_composition(
        &mut self,
        command: TextCompositionCommand,
    ) -> Result<TextEditOutcome, TextCompositionError> {
        let current_revision = self.revision();
        let base_revision = command.base_revision();
        if base_revision != current_revision {
            return Err(TextEditError::StaleRevision {
                base: base_revision,
                current: current_revision,
            }
            .into());
        }

        let kind = command.kind();
        match (kind, self.composition()) {
            (TextCompositionKind::Start, Some(composition)) => {
                return Err(TextCompositionError::AlreadyActive { composition });
            }
            (
                TextCompositionKind::Update
                | TextCompositionKind::Commit
                | TextCompositionKind::Cancel,
                None,
            ) => return Err(TextCompositionError::NotActive { command: kind }),
            _ => {}
        }

        self.apply_edits(command.into_batch()).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::text::{
        TextAffinity, TextBuffer, TextCompositionCommand, TextCompositionError,
        TextCompositionKind, TextEdit, TextEditError, TextOffset, TextRange, TextRevision,
        TextSelection,
    };

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

    fn text(buffer: &TextBuffer) -> String {
        buffer.chunks().map(|chunk| chunk.text).collect()
    }

    #[test]
    fn start_update_and_commit_publish_ordered_editing_values() {
        let mut buffer = TextBuffer::from_text("hello").unwrap();
        let start = buffer
            .apply_composition(TextCompositionCommand::Start {
                base_revision: TextRevision::INITIAL,
                edits: vec![TextEdit {
                    range: range(5, 5),
                    replacement: " 世".to_string(),
                }],
                selection: selection(9, 9),
                composition: range(5, 9),
            })
            .unwrap();

        assert_eq!(text(&buffer), "hello 世");
        assert_eq!(start.snapshot.revision(), TextRevision(1));
        assert_eq!(start.selection, selection(9, 9));
        assert_eq!(start.composition, Some(range(5, 9)));

        let update = buffer
            .apply_composition(TextCompositionCommand::Update {
                base_revision: TextRevision(1),
                edits: vec![TextEdit {
                    range: range(5, 9),
                    replacement: " world".to_string(),
                }],
                selection: selection(11, 11),
                composition: range(5, 11),
            })
            .unwrap();

        assert_eq!(text(&buffer), "hello world");
        assert_eq!(update.snapshot.revision(), TextRevision(2));
        assert_eq!(update.composition, Some(range(5, 11)));

        let commit = buffer
            .apply_composition(TextCompositionCommand::Commit {
                base_revision: TextRevision(2),
                edits: Vec::new(),
                selection: selection(11, 11),
            })
            .unwrap();

        assert_eq!(text(&buffer), "hello world");
        assert_eq!(commit.snapshot.revision(), TextRevision(3));
        assert_eq!(commit.composition, None);
        assert_eq!(buffer.composition(), None);
    }

    #[test]
    fn cancel_applies_explicit_rollback_edits_and_clears_composition() {
        let mut buffer = TextBuffer::from_text("abc").unwrap();
        buffer
            .apply_composition(TextCompositionCommand::Start {
                base_revision: TextRevision::INITIAL,
                edits: vec![TextEdit {
                    range: range(1, 1),
                    replacement: "temporary".to_string(),
                }],
                selection: selection(10, 10),
                composition: range(1, 10),
            })
            .unwrap();

        let outcome = buffer
            .apply_composition(TextCompositionCommand::Cancel {
                base_revision: TextRevision(1),
                edits: vec![TextEdit {
                    range: range(1, 10),
                    replacement: String::new(),
                }],
                selection: selection(1, 1),
            })
            .unwrap();

        assert_eq!(text(&buffer), "abc");
        assert_eq!(outcome.snapshot.revision(), TextRevision(2));
        assert_eq!(outcome.selection, selection(1, 1));
        assert_eq!(outcome.composition, None);
    }

    #[test]
    fn transition_preconditions_reject_without_mutation() {
        let mut buffer = TextBuffer::from_text("abc").unwrap();
        let before = buffer.snapshot();

        let update = buffer.apply_composition(TextCompositionCommand::Update {
            base_revision: TextRevision::INITIAL,
            edits: Vec::new(),
            selection: selection(3, 3),
            composition: range(3, 3),
        });
        assert_eq!(
            update.unwrap_err(),
            TextCompositionError::NotActive {
                command: TextCompositionKind::Update
            }
        );
        assert_eq!(buffer.revision(), before.revision());
        assert_eq!(text(&buffer), "abc");

        buffer
            .apply_composition(TextCompositionCommand::Start {
                base_revision: TextRevision::INITIAL,
                edits: Vec::new(),
                selection: selection(3, 3),
                composition: range(3, 3),
            })
            .unwrap();
        let active = buffer.snapshot();
        let start = buffer.apply_composition(TextCompositionCommand::Start {
            base_revision: TextRevision(1),
            edits: vec![TextEdit {
                range: range(0, 3),
                replacement: "changed".to_string(),
            }],
            selection: selection(7, 7),
            composition: range(0, 7),
        });
        assert_eq!(
            start.unwrap_err(),
            TextCompositionError::AlreadyActive {
                composition: range(3, 3)
            }
        );
        assert_eq!(buffer.revision(), active.revision());
        assert_eq!(text(&buffer), "abc");
    }

    #[test]
    fn stale_revision_wins_over_transition_state_and_requests_resync() {
        let mut buffer = TextBuffer::from_text("abc").unwrap();
        buffer
            .apply_composition(TextCompositionCommand::Start {
                base_revision: TextRevision::INITIAL,
                edits: Vec::new(),
                selection: selection(3, 3),
                composition: range(3, 3),
            })
            .unwrap();

        let result = buffer.apply_composition(TextCompositionCommand::Start {
            base_revision: TextRevision::INITIAL,
            edits: Vec::new(),
            selection: selection(3, 3),
            composition: range(3, 3),
        });

        assert_eq!(
            result.unwrap_err(),
            TextCompositionError::Edit(TextEditError::StaleRevision {
                base: TextRevision::INITIAL,
                current: TextRevision(1),
            })
        );
        assert_eq!(buffer.revision(), TextRevision(1));
        assert_eq!(buffer.composition(), Some(range(3, 3)));
    }

    #[test]
    fn failed_edit_validation_keeps_the_active_composition() {
        let mut buffer = TextBuffer::from_text("é").unwrap();
        buffer
            .apply_composition(TextCompositionCommand::Start {
                base_revision: TextRevision::INITIAL,
                edits: Vec::new(),
                selection: selection(2, 2),
                composition: range(0, 2),
            })
            .unwrap();
        let before = buffer.snapshot();

        let result = buffer.apply_composition(TextCompositionCommand::Update {
            base_revision: TextRevision(1),
            edits: vec![TextEdit {
                range: range(1, 1),
                replacement: "private".to_string(),
            }],
            selection: selection(2, 2),
            composition: range(0, 2),
        });

        assert!(matches!(
            result,
            Err(TextCompositionError::Edit(
                TextEditError::InvalidEditRange { .. }
            ))
        ));
        assert_eq!(buffer.revision(), before.revision());
        assert_eq!(buffer.composition(), before.composition());
        assert_eq!(text(&buffer), "é");
    }

    #[test]
    fn command_debug_output_redacts_replacement_content() {
        let command = TextCompositionCommand::Start {
            base_revision: TextRevision::INITIAL,
            edits: vec![TextEdit {
                range: range(0, 0),
                replacement: "private preedit".to_string(),
            }],
            selection: selection(0, 0),
            composition: range(0, 0),
        };
        let debug = format!("{command:?}");

        assert!(!debug.contains("private preedit"));
        assert!(debug.contains("replacement_len_bytes"));
        assert!(debug.contains("Start"));
    }
}
