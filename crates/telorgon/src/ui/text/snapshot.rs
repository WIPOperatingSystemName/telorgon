use std::fmt;
use std::sync::Arc;

use crate::ui::text::{TextChunks, TextOffset, TextRange, TextRangeError, TextRevision, TextSelection};

#[derive(Clone)]
pub struct TextSnapshot {
    text: Arc<str>,
    revision: TextRevision,
    selection: TextSelection,
    composition: Option<TextRange>,
}

impl fmt::Debug for TextSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextSnapshot")
            .field("revision", &self.revision)
            .field("len_bytes", &self.text.len())
            .field("selection", &self.selection)
            .field("composition", &self.composition)
            .finish_non_exhaustive()
    }
}

impl TextSnapshot {
    pub(crate) fn from_parts(
        text: Arc<str>,
        revision: TextRevision,
        selection: TextSelection,
        composition: Option<TextRange>,
    ) -> Self {
        Self {
            text,
            revision,
            selection,
            composition,
        }
    }

    pub const fn revision(&self) -> TextRevision {
        self.revision
    }

    pub fn len_bytes(&self) -> u32 {
        self.text.len() as u32
    }

    pub fn end(&self) -> TextOffset {
        TextOffset(self.len_bytes())
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub const fn selection(&self) -> TextSelection {
        self.selection
    }

    pub const fn composition(&self) -> Option<TextRange> {
        self.composition
    }

    pub fn chunks(&self) -> TextChunks<'_> {
        TextChunks::single(TextOffset::ZERO, &self.text)
    }

    pub fn chunks_in(&self, range: TextRange) -> Result<TextChunks<'_>, TextRangeError> {
        let bytes = range.validate(&self.text)?;
        Ok(TextChunks::single(range.start, &self.text[bytes]))
    }

    pub fn validate_offset(&self, offset: TextOffset) -> Result<TextOffset, TextRangeError> {
        offset.validate(&self.text)?;
        Ok(offset)
    }

    pub fn validate_range(&self, range: TextRange) -> Result<TextRange, TextRangeError> {
        range.validate(&self.text)?;
        Ok(range)
    }

    pub fn validate_selection(
        &self,
        selection: TextSelection,
    ) -> Result<TextSelection, TextRangeError> {
        selection.validate(&self.text)
    }

    pub(crate) fn text_for_navigation(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::ui::text::{
        TextAffinity, TextBuffer, TextOffset, TextRange, TextRangeError, TextRevision,
        TextSelection,
    };

    #[test]
    fn snapshot_is_revisioned_immutable_chunk_access() {
        let buffer = TextBuffer::from_text("alpha βeta").unwrap();
        let snapshot = buffer.snapshot();
        let second_snapshot = buffer.snapshot();
        let clone = snapshot.clone();

        assert!(Arc::ptr_eq(&snapshot.text, &second_snapshot.text));
        assert_eq!(snapshot.revision(), TextRevision::INITIAL);
        assert_eq!(snapshot.len_bytes(), 11);
        assert_eq!(snapshot.end(), TextOffset(11));
        assert_eq!(snapshot.selection().active, TextOffset(11));
        assert_eq!(snapshot.composition(), None);
        assert_eq!(
            clone.chunks().map(|chunk| chunk.text).collect::<String>(),
            "alpha βeta"
        );
    }

    #[test]
    fn snapshot_validates_ranges_and_directional_selection() {
        let snapshot = TextBuffer::from_text("אבג text").unwrap().snapshot();
        let selection = TextSelection {
            anchor: snapshot.end(),
            active: TextOffset(7),
            affinity: TextAffinity::Upstream,
        };
        let range = selection.range();

        assert_eq!(snapshot.validate_selection(selection), Ok(selection));
        assert_eq!(snapshot.validate_range(range), Ok(range));
        assert_eq!(
            snapshot
                .chunks_in(TextRange::new(TextOffset(7), snapshot.end()).unwrap())
                .unwrap()
                .map(|chunk| chunk.text)
                .collect::<String>(),
            "text"
        );
        assert_eq!(
            snapshot.validate_offset(TextOffset(1)),
            Err(TextRangeError::NotCharBoundary {
                offset: TextOffset(1)
            })
        );
    }

    #[test]
    fn debug_output_does_not_expose_text_content() {
        let snapshot = TextBuffer::from_text("private value").unwrap().snapshot();
        let snapshot_debug = format!("{snapshot:?}");
        let chunk_debug = format!("{:?}", snapshot.chunks().next().unwrap());

        assert!(!snapshot_debug.contains("private value"));
        assert!(!chunk_debug.contains("private value"));
        assert!(snapshot_debug.contains("len_bytes"));
        assert!(chunk_debug.contains("len_bytes"));
    }
}
