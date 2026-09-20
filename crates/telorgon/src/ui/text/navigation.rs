use unicode_segmentation::UnicodeSegmentation;

use crate::ui::text::{
    TextAffinity, TextOffset, TextRange, TextRangeError, TextSelection, TextSnapshot,
};

pub const TEXT_SEGMENTATION_CRATE_VERSION: &str = "1.13.2";
pub const TEXT_SEGMENTATION_UNICODE_VERSION: &str = "17.0.0";
pub const TEXT_SEGMENTATION_PROFILE: &str =
    "UAX29-C1-1 extended grapheme clusters and UAX29-C2-1 default words; no tailoring";

/// Logical movement through UTF-8 text order, independent of visual bidi direction.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextNavigationDirection {
    Backward,
    Forward,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextNavigationUnit {
    Grapheme,
    Word,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextSelectionAdjustment {
    Move,
    Extend,
}

impl TextSnapshot {
    /// Returns the preceding extended-grapheme boundary, or zero at the start.
    pub fn previous_grapheme_boundary(
        &self,
        offset: TextOffset,
    ) -> Result<TextOffset, TextRangeError> {
        let text = self.text_for_navigation();
        let offset = offset.validate(text)?;
        Ok(TextOffset(previous_grapheme_boundary(text, offset) as u32))
    }

    /// Returns the following extended-grapheme boundary, or the text end at the end.
    pub fn next_grapheme_boundary(&self, offset: TextOffset) -> Result<TextOffset, TextRangeError> {
        let text = self.text_for_navigation();
        let offset = offset.validate(text)?;
        Ok(TextOffset(next_grapheme_boundary(text, offset) as u32))
    }

    /// Returns the start of the current or preceding default Unicode word.
    ///
    /// Non-word segments such as punctuation and whitespace are skipped.
    pub fn previous_word_boundary(&self, offset: TextOffset) -> Result<TextOffset, TextRangeError> {
        let text = self.text_for_navigation();
        let offset = offset.validate(text)?;
        Ok(TextOffset(previous_word_boundary(text, offset) as u32))
    }

    /// Returns the end of the current or following default Unicode word.
    ///
    /// Non-word segments such as punctuation and whitespace are skipped.
    pub fn next_word_boundary(&self, offset: TextOffset) -> Result<TextOffset, TextRangeError> {
        let text = self.text_for_navigation();
        let offset = offset.validate(text)?;
        Ok(TextOffset(next_word_boundary(text, offset) as u32))
    }

    /// Returns the default Unicode word containing `offset`.
    ///
    /// An offset on punctuation, whitespace, or the end boundary returns `None`.
    pub fn word_range_at(&self, offset: TextOffset) -> Result<Option<TextRange>, TextRangeError> {
        let text = self.text_for_navigation();
        let offset = offset.validate(text)?;
        Ok(word_range_at(text, offset).map(|(start, end)| TextRange {
            start: TextOffset(start as u32),
            end: TextOffset(end as u32),
        }))
    }

    /// Moves or extends a selection by one logical unit.
    ///
    /// Moving a non-collapsed selection first collapses it toward `direction` and normalizes the
    /// result to an extended-grapheme boundary. `affinity` is supplied explicitly because visual
    /// line wrapping and bidi caret placement belong to the layout/controller layer.
    pub fn navigate_selection(
        &self,
        selection: TextSelection,
        unit: TextNavigationUnit,
        direction: TextNavigationDirection,
        adjustment: TextSelectionAdjustment,
        affinity: TextAffinity,
    ) -> Result<TextSelection, TextRangeError> {
        let text = self.text_for_navigation();
        selection.validate(text)?;

        let active = if adjustment == TextSelectionAdjustment::Move && !selection.is_collapsed() {
            let range = selection.range();
            match direction {
                TextNavigationDirection::Backward => {
                    TextOffset(grapheme_boundary_at_or_before(text, range.start.as_usize()) as u32)
                }
                TextNavigationDirection::Forward => {
                    TextOffset(grapheme_boundary_at_or_after(text, range.end.as_usize()) as u32)
                }
            }
        } else {
            let active = selection.active.as_usize();
            TextOffset(match (unit, direction) {
                (TextNavigationUnit::Grapheme, TextNavigationDirection::Backward) => {
                    previous_grapheme_boundary(text, active)
                }
                (TextNavigationUnit::Grapheme, TextNavigationDirection::Forward) => {
                    next_grapheme_boundary(text, active)
                }
                (TextNavigationUnit::Word, TextNavigationDirection::Backward) => {
                    previous_word_boundary(text, active)
                }
                (TextNavigationUnit::Word, TextNavigationDirection::Forward) => {
                    next_word_boundary(text, active)
                }
            } as u32)
        };

        Ok(match adjustment {
            TextSelectionAdjustment::Move => TextSelection::collapsed(active, affinity),
            TextSelectionAdjustment::Extend => TextSelection {
                anchor: selection.anchor,
                active,
                affinity,
            },
        })
    }
}

fn previous_grapheme_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(start, _)| start)
        .take_while(|boundary| *boundary < offset)
        .last()
        .unwrap_or(0)
}

fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(start, _)| start)
        .find(|boundary| *boundary > offset)
        .unwrap_or(text.len())
}

fn grapheme_boundary_at_or_before(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(start, _)| start)
        .take_while(|boundary| *boundary <= offset)
        .last()
        .unwrap_or(0)
}

fn grapheme_boundary_at_or_after(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(start, _)| start)
        .find(|boundary| *boundary >= offset)
        .unwrap_or(text.len())
}

fn previous_word_boundary(text: &str, offset: usize) -> usize {
    let mut previous_start = 0;
    for (start, word) in text.unicode_word_indices() {
        let end = start + word.len();
        if offset <= start {
            break;
        }
        if offset <= end {
            return start;
        }
        previous_start = start;
    }
    previous_start
}

fn next_word_boundary(text: &str, offset: usize) -> usize {
    text.unicode_word_indices()
        .find_map(|(start, word)| {
            let end = start + word.len();
            (offset < end).then_some(end)
        })
        .unwrap_or(text.len())
}

fn word_range_at(text: &str, offset: usize) -> Option<(usize, usize)> {
    text.unicode_word_indices().find_map(|(start, word)| {
        let end = start + word.len();
        (start <= offset && offset < end).then_some((start, end))
    })
}

#[cfg(test)]
mod tests {
    use crate::ui::text::{
        TEXT_SEGMENTATION_CRATE_VERSION, TEXT_SEGMENTATION_PROFILE,
        TEXT_SEGMENTATION_UNICODE_VERSION, TextAffinity, TextBuffer, TextNavigationDirection,
        TextNavigationUnit, TextOffset, TextRange, TextRangeError, TextSelection,
        TextSelectionAdjustment,
    };

    fn collapsed(offset: u32) -> TextSelection {
        TextSelection::collapsed(TextOffset(offset), TextAffinity::Downstream)
    }

    #[test]
    fn segmentation_versions_are_declared() {
        assert_eq!(TEXT_SEGMENTATION_CRATE_VERSION, "1.13.2");
        assert_eq!(TEXT_SEGMENTATION_UNICODE_VERSION, "17.0.0");
        assert!(TEXT_SEGMENTATION_PROFILE.contains("UAX29-C1-1"));
        assert!(TEXT_SEGMENTATION_PROFILE.contains("UAX29-C2-1"));
    }

    #[test]
    fn grapheme_navigation_keeps_combining_and_zwj_sequences_indivisible() {
        let combining = TextBuffer::from_text("e\u{301}x").unwrap().snapshot();
        assert_eq!(
            combining.next_grapheme_boundary(TextOffset::ZERO),
            Ok(TextOffset(3))
        );
        assert_eq!(
            combining.previous_grapheme_boundary(TextOffset(3)),
            Ok(TextOffset::ZERO)
        );
        assert_eq!(
            combining.next_grapheme_boundary(TextOffset(1)),
            Ok(TextOffset(3))
        );
        assert_eq!(
            combining.previous_grapheme_boundary(TextOffset(1)),
            Ok(TextOffset::ZERO)
        );

        let emoji = TextBuffer::from_text("👩‍💻x").unwrap().snapshot();
        assert_eq!(
            emoji.next_grapheme_boundary(TextOffset::ZERO),
            Ok(TextOffset(11))
        );
        assert_eq!(
            emoji.previous_grapheme_boundary(TextOffset(11)),
            Ok(TextOffset::ZERO)
        );
    }

    #[test]
    fn boundaries_clamp_at_empty_start_and_end() {
        let empty = TextBuffer::new().snapshot();
        assert_eq!(
            empty.previous_grapheme_boundary(TextOffset::ZERO),
            Ok(TextOffset::ZERO)
        );
        assert_eq!(
            empty.next_grapheme_boundary(TextOffset::ZERO),
            Ok(TextOffset::ZERO)
        );

        let text = TextBuffer::from_text("ab").unwrap().snapshot();
        assert_eq!(
            text.previous_grapheme_boundary(TextOffset::ZERO),
            Ok(TextOffset::ZERO)
        );
        assert_eq!(text.next_grapheme_boundary(text.end()), Ok(text.end()));
    }

    #[test]
    fn word_navigation_uses_unicode_words_and_skips_separators() {
        let snapshot = TextBuffer::from_text("can't 32.3 東京").unwrap().snapshot();

        assert_eq!(
            snapshot.next_word_boundary(TextOffset::ZERO),
            Ok(TextOffset(5))
        );
        assert_eq!(
            snapshot.next_word_boundary(TextOffset(5)),
            Ok(TextOffset(10))
        );
        assert_eq!(
            snapshot.previous_word_boundary(TextOffset(11)),
            Ok(TextOffset(6))
        );
        assert_eq!(
            snapshot.previous_word_boundary(snapshot.end()),
            Ok(TextOffset(14))
        );
        assert_eq!(
            snapshot.word_range_at(TextOffset(2)),
            Ok(Some(TextRange {
                start: TextOffset::ZERO,
                end: TextOffset(5),
            }))
        );
        assert_eq!(snapshot.word_range_at(TextOffset(5)), Ok(None));
        assert_eq!(
            snapshot.word_range_at(TextOffset(11)),
            Ok(Some(TextRange {
                start: TextOffset(11),
                end: TextOffset(14),
            }))
        );
    }

    #[test]
    fn selection_move_collapses_and_normalizes_platform_scalar_ranges() {
        let snapshot = TextBuffer::from_text("e\u{301}x").unwrap().snapshot();
        let scalar_selection = TextSelection {
            anchor: TextOffset(1),
            active: TextOffset(3),
            affinity: TextAffinity::Downstream,
        };

        assert_eq!(
            snapshot.navigate_selection(
                scalar_selection,
                TextNavigationUnit::Word,
                TextNavigationDirection::Backward,
                TextSelectionAdjustment::Move,
                TextAffinity::Upstream,
            ),
            Ok(TextSelection::collapsed(
                TextOffset::ZERO,
                TextAffinity::Upstream
            ))
        );
        assert_eq!(
            snapshot.navigate_selection(
                scalar_selection,
                TextNavigationUnit::Word,
                TextNavigationDirection::Forward,
                TextSelectionAdjustment::Move,
                TextAffinity::Downstream,
            ),
            Ok(TextSelection::collapsed(
                TextOffset(3),
                TextAffinity::Downstream
            ))
        );
    }

    #[test]
    fn selection_extend_preserves_anchor_direction_and_explicit_affinity() {
        let snapshot = TextBuffer::from_text("אב x").unwrap().snapshot();
        let selection = TextSelection {
            anchor: TextOffset(4),
            active: TextOffset::ZERO,
            affinity: TextAffinity::Downstream,
        };

        let extended = snapshot
            .navigate_selection(
                selection,
                TextNavigationUnit::Grapheme,
                TextNavigationDirection::Forward,
                TextSelectionAdjustment::Extend,
                TextAffinity::Upstream,
            )
            .unwrap();

        assert_eq!(extended.anchor, TextOffset(4));
        assert_eq!(extended.active, TextOffset(2));
        assert!(!extended.is_forward());
        assert_eq!(extended.affinity, TextAffinity::Upstream);
    }

    #[test]
    fn collapsed_selection_moves_by_requested_logical_unit() {
        let snapshot = TextBuffer::from_text("one two").unwrap().snapshot();
        assert_eq!(
            snapshot.navigate_selection(
                collapsed(0),
                TextNavigationUnit::Grapheme,
                TextNavigationDirection::Forward,
                TextSelectionAdjustment::Move,
                TextAffinity::Downstream,
            ),
            Ok(collapsed(1))
        );
        assert_eq!(
            snapshot.navigate_selection(
                collapsed(0),
                TextNavigationUnit::Word,
                TextNavigationDirection::Forward,
                TextSelectionAdjustment::Move,
                TextAffinity::Downstream,
            ),
            Ok(collapsed(3))
        );
    }

    #[test]
    fn invalid_utf8_offsets_are_rejected_before_navigation() {
        let snapshot = TextBuffer::from_text("é").unwrap().snapshot();

        assert_eq!(
            snapshot.next_grapheme_boundary(TextOffset(1)),
            Err(TextRangeError::NotCharBoundary {
                offset: TextOffset(1)
            })
        );
        assert_eq!(
            snapshot.word_range_at(TextOffset(3)),
            Err(TextRangeError::OutOfBounds {
                offset: TextOffset(3),
                len_bytes: 2,
            })
        );
    }
}
