use std::fmt;

use crate::ui::text::buffer::validate_supported_len;
use crate::ui::text::{
    TextBuffer, TextBufferError, TextOffset, TextRange, TextRangeError, TextRevision,
    TextSelection, TextSnapshot,
};

#[derive(Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub range: TextRange,
    pub replacement: String,
}

impl fmt::Debug for TextEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextEdit")
            .field("range", &self.range)
            .field("replacement_len_bytes", &self.replacement.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TextEditBatch {
    pub base_revision: TextRevision,
    pub edits: Vec<TextEdit>,
    pub selection: TextSelection,
    pub composition: Option<TextRange>,
}

impl fmt::Debug for TextEditBatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextEditBatch")
            .field("base_revision", &self.base_revision)
            .field("edits", &self.edits)
            .field("selection", &self.selection)
            .field("composition", &self.composition)
            .finish()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TextChange {
    pub old: TextRange,
    pub new: TextRange,
}

#[derive(Clone, Debug)]
pub struct TextEditOutcome {
    pub snapshot: TextSnapshot,
    pub changes: Vec<TextChange>,
    pub selection: TextSelection,
    pub composition: Option<TextRange>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextEditError {
    StaleRevision {
        base: TextRevision,
        current: TextRevision,
    },
    RevisionExhausted {
        revision: TextRevision,
    },
    InvalidEditRange {
        edit_index: usize,
        error: TextRangeError,
    },
    Unsorted {
        previous_index: usize,
        edit_index: usize,
        previous: TextRange,
        current: TextRange,
    },
    Overlapping {
        previous_index: usize,
        edit_index: usize,
        previous: TextRange,
        current: TextRange,
    },
    ResultTooLong {
        len_bytes: usize,
        max_bytes: u32,
    },
    SizeOverflow,
    InvalidSelection {
        error: TextRangeError,
    },
    InvalidComposition {
        error: TextRangeError,
    },
}

impl fmt::Display for TextEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { base, current } => write!(
                f,
                "text edit is based on revision {} but the buffer is at revision {}",
                base.0, current.0
            ),
            Self::RevisionExhausted { revision } => {
                write!(f, "text revision {} cannot be incremented", revision.0)
            }
            Self::InvalidEditRange { edit_index, error } => {
                write!(f, "text edit {edit_index} has an invalid range: {error}")
            }
            Self::Unsorted {
                previous_index,
                edit_index,
                ..
            } => write!(
                f,
                "text edits are not sorted: edit {edit_index} starts before edit {previous_index}"
            ),
            Self::Overlapping {
                previous_index,
                edit_index,
                ..
            } => write!(
                f,
                "text edits overlap: edit {edit_index} begins inside edit {previous_index}"
            ),
            Self::ResultTooLong {
                len_bytes,
                max_bytes,
            } => write!(
                f,
                "edited text has {len_bytes} UTF-8 bytes but supports at most {max_bytes}"
            ),
            Self::SizeOverflow => f.write_str("edited text byte length overflowed usize"),
            Self::InvalidSelection { error } => {
                write!(f, "edited text has an invalid selection: {error}")
            }
            Self::InvalidComposition { error } => {
                write!(f, "edited text has an invalid composition range: {error}")
            }
        }
    }
}

impl std::error::Error for TextEditError {}

impl TextBuffer {
    pub fn apply_edits(&mut self, batch: TextEditBatch) -> Result<TextEditOutcome, TextEditError> {
        let current_revision = self.revision();
        if batch.base_revision != current_revision {
            return Err(TextEditError::StaleRevision {
                base: batch.base_revision,
                current: current_revision,
            });
        }
        let next_revision = TextRevision(current_revision.0.checked_add(1).ok_or(
            TextEditError::RevisionExhausted {
                revision: current_revision,
            },
        )?);

        let old_text = self.text_for_edit();
        validate_edits(old_text, &batch.edits)?;
        let new_len = edited_len(old_text.len(), &batch.edits)?;
        let mut new_text = String::with_capacity(new_len);
        let mut changes = Vec::with_capacity(batch.edits.len());
        let mut old_cursor = 0usize;

        for edit in &batch.edits {
            let old_start = edit.range.start.as_usize();
            let old_end = edit.range.end.as_usize();
            new_text.push_str(&old_text[old_cursor..old_start]);
            let new_start = TextOffset(new_text.len() as u32);
            new_text.push_str(&edit.replacement);
            let new_end = TextOffset(new_text.len() as u32);
            changes.push(TextChange {
                old: edit.range,
                new: TextRange {
                    start: new_start,
                    end: new_end,
                },
            });
            old_cursor = old_end;
        }
        new_text.push_str(&old_text[old_cursor..]);
        debug_assert_eq!(new_text.len(), new_len);

        batch
            .selection
            .validate(&new_text)
            .map_err(|error| TextEditError::InvalidSelection { error })?;
        if let Some(composition) = batch.composition {
            composition
                .validate(&new_text)
                .map_err(|error| TextEditError::InvalidComposition { error })?;
        }

        self.commit_edit(new_text, next_revision, batch.selection, batch.composition);
        Ok(TextEditOutcome {
            snapshot: self.snapshot(),
            changes,
            selection: batch.selection,
            composition: batch.composition,
        })
    }
}

fn validate_edits(text: &str, edits: &[TextEdit]) -> Result<(), TextEditError> {
    let mut previous: Option<(usize, TextRange)> = None;
    for (edit_index, edit) in edits.iter().enumerate() {
        edit.range
            .validate(text)
            .map_err(|error| TextEditError::InvalidEditRange { edit_index, error })?;
        if let Some((previous_index, previous_range)) = previous {
            if edit.range.start < previous_range.start {
                return Err(TextEditError::Unsorted {
                    previous_index,
                    edit_index,
                    previous: previous_range,
                    current: edit.range,
                });
            }
            if edit.range.start < previous_range.end {
                return Err(TextEditError::Overlapping {
                    previous_index,
                    edit_index,
                    previous: previous_range,
                    current: edit.range,
                });
            }
        }
        previous = Some((edit_index, edit.range));
    }
    Ok(())
}

fn edited_len(old_len: usize, edits: &[TextEdit]) -> Result<usize, TextEditError> {
    let removed = edits.iter().try_fold(0usize, |total, edit| {
        total.checked_add(edit.range.len_bytes().expect("validated text edit range") as usize)
    });
    let inserted = edits.iter().try_fold(0usize, |total, edit| {
        total.checked_add(edit.replacement.len())
    });
    let new_len = removed
        .and_then(|removed| old_len.checked_sub(removed))
        .and_then(|retained| inserted.and_then(|inserted| retained.checked_add(inserted)))
        .ok_or(TextEditError::SizeOverflow)?;
    validate_supported_len(new_len).map_err(|error| match error {
        TextBufferError::TooLong {
            len_bytes,
            max_bytes,
        } => TextEditError::ResultTooLong {
            len_bytes,
            max_bytes,
        },
    })?;
    Ok(new_len)
}

#[cfg(test)]
mod tests {
    use crate::ui::text::{
        TextAffinity, TextBuffer, TextChange, TextEdit, TextEditBatch, TextEditError, TextOffset,
        TextRange, TextRangeError, TextRevision, TextSelection,
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

    fn text(snapshot: &crate::ui::text::TextSnapshot) -> String {
        snapshot.chunks().map(|chunk| chunk.text).collect()
    }

    #[test]
    fn applies_sorted_multi_edit_atomically_and_reports_old_new_ranges() {
        let mut buffer = TextBuffer::from_text("aé中z").unwrap();
        let before = buffer.snapshot();
        let outcome = buffer
            .apply_edits(TextEditBatch {
                base_revision: TextRevision::INITIAL,
                edits: vec![
                    TextEdit {
                        range: range(1, 3),
                        replacement: "E".to_string(),
                    },
                    TextEdit {
                        range: range(6, 7),
                        replacement: "!".to_string(),
                    },
                ],
                selection: selection(6, 2),
                composition: Some(range(1, 2)),
            })
            .unwrap();

        assert_eq!(buffer.revision(), TextRevision(1));
        assert_eq!(outcome.snapshot.revision(), TextRevision(1));
        assert_eq!(text(&before), "aé中z");
        assert_eq!(text(&outcome.snapshot), "aE中!");
        assert_eq!(outcome.selection, selection(6, 2));
        assert_eq!(outcome.composition, Some(range(1, 2)));
        assert_eq!(buffer.selection(), selection(6, 2));
        assert_eq!(buffer.composition(), Some(range(1, 2)));
        assert_eq!(buffer.snapshot().composition(), Some(range(1, 2)));
        assert_eq!(
            outcome.changes,
            vec![
                TextChange {
                    old: range(1, 3),
                    new: range(1, 2),
                },
                TextChange {
                    old: range(6, 7),
                    new: range(5, 6),
                }
            ]
        );
    }

    #[test]
    fn stale_revision_rejects_without_mutation() {
        let mut buffer = TextBuffer::from_text("stable").unwrap();
        let error = buffer
            .apply_edits(TextEditBatch {
                base_revision: TextRevision(9),
                edits: Vec::new(),
                selection: selection(0, 0),
                composition: None,
            })
            .unwrap_err();

        assert_eq!(
            error,
            TextEditError::StaleRevision {
                base: TextRevision(9),
                current: TextRevision::INITIAL
            }
        );
        assert_eq!(buffer.revision(), TextRevision::INITIAL);
        assert_eq!(text(&buffer.snapshot()), "stable");
    }

    #[test]
    fn rejects_unsorted_overlap_and_scalar_split_without_mutation() {
        let cases = [
            (
                "abcd",
                vec![
                    TextEdit {
                        range: range(2, 3),
                        replacement: String::new(),
                    },
                    TextEdit {
                        range: range(0, 1),
                        replacement: String::new(),
                    },
                ],
            ),
            (
                "abcd",
                vec![
                    TextEdit {
                        range: range(0, 2),
                        replacement: String::new(),
                    },
                    TextEdit {
                        range: range(1, 3),
                        replacement: String::new(),
                    },
                ],
            ),
            (
                "éx",
                vec![TextEdit {
                    range: range(1, 2),
                    replacement: String::new(),
                }],
            ),
        ];

        for (case, (original, edits)) in cases.into_iter().enumerate() {
            let mut buffer = TextBuffer::from_text(original).unwrap();
            let result = buffer.apply_edits(TextEditBatch {
                base_revision: TextRevision::INITIAL,
                edits,
                selection: selection(0, 0),
                composition: None,
            });

            match (case, result) {
                (0, Err(TextEditError::Unsorted { .. }))
                | (1, Err(TextEditError::Overlapping { .. }))
                | (
                    2,
                    Err(TextEditError::InvalidEditRange {
                        error: TextRangeError::NotCharBoundary { .. },
                        ..
                    }),
                ) => {}
                (_, other) => panic!("unexpected edit result: {other:?}"),
            }
            assert_eq!(buffer.revision(), TextRevision::INITIAL);
            assert_eq!(text(&buffer.snapshot()), original);
            assert_eq!(
                buffer.selection(),
                selection(original.len() as u32, original.len() as u32)
            );
            assert_eq!(buffer.composition(), None);
        }
    }

    #[test]
    fn permits_adjacent_ranges_and_stable_insertions_at_one_offset() {
        let mut buffer = TextBuffer::from_text("ab").unwrap();
        let outcome = buffer
            .apply_edits(TextEditBatch {
                base_revision: TextRevision::INITIAL,
                edits: vec![
                    TextEdit {
                        range: range(0, 1),
                        replacement: "A".to_string(),
                    },
                    TextEdit {
                        range: range(1, 1),
                        replacement: "1".to_string(),
                    },
                    TextEdit {
                        range: range(1, 1),
                        replacement: "2".to_string(),
                    },
                    TextEdit {
                        range: range(1, 2),
                        replacement: "B".to_string(),
                    },
                ],
                selection: selection(4, 4),
                composition: None,
            })
            .unwrap();

        assert_eq!(text(&outcome.snapshot), "A12B");
        assert_eq!(
            outcome.changes,
            vec![
                TextChange {
                    old: range(0, 1),
                    new: range(0, 1),
                },
                TextChange {
                    old: range(1, 1),
                    new: range(1, 2),
                },
                TextChange {
                    old: range(1, 1),
                    new: range(2, 3),
                },
                TextChange {
                    old: range(1, 2),
                    new: range(3, 4),
                }
            ]
        );
    }

    #[test]
    fn invalid_result_selection_and_composition_are_atomic() {
        for invalid_selection in [true, false] {
            let mut buffer = TextBuffer::from_text("abc").unwrap();
            let result = buffer.apply_edits(TextEditBatch {
                base_revision: TextRevision::INITIAL,
                edits: vec![TextEdit {
                    range: range(0, 3),
                    replacement: "é".to_string(),
                }],
                selection: if invalid_selection {
                    selection(1, 1)
                } else {
                    selection(0, 0)
                },
                composition: (!invalid_selection).then_some(range(1, 1)),
            });

            if invalid_selection {
                assert!(matches!(
                    result,
                    Err(TextEditError::InvalidSelection { .. })
                ));
            } else {
                assert!(matches!(
                    result,
                    Err(TextEditError::InvalidComposition { .. })
                ));
            }
            assert_eq!(buffer.revision(), TextRevision::INITIAL);
            assert_eq!(text(&buffer.snapshot()), "abc");
            assert_eq!(buffer.selection(), selection(3, 3));
            assert_eq!(buffer.composition(), None);
        }
    }

    #[test]
    fn empty_batch_still_publishes_one_revision() {
        let mut buffer = TextBuffer::from_text("unchanged").unwrap();
        let outcome = buffer
            .apply_edits(TextEditBatch {
                base_revision: TextRevision::INITIAL,
                edits: Vec::new(),
                selection: selection(9, 9),
                composition: None,
            })
            .unwrap();

        assert_eq!(buffer.revision(), TextRevision(1));
        assert_eq!(buffer.selection(), selection(9, 9));
        assert!(outcome.changes.is_empty());
        assert_eq!(text(&outcome.snapshot), "unchanged");
    }

    #[test]
    fn edit_debug_output_redacts_replacement_content() {
        let edit = TextEdit {
            range: range(0, 0),
            replacement: "private replacement".to_string(),
        };
        let debug = format!("{edit:?}");

        assert!(!debug.contains("private replacement"));
        assert!(debug.contains("replacement_len_bytes"));
    }
}
