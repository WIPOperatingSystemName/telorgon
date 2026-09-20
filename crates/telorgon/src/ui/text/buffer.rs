use std::fmt;
use std::iter::FusedIterator;
use std::sync::Arc;

use crate::ui::text::{
    TextAffinity, TextOffset, TextRange, TextRangeError, TextRevision, TextSelection, TextSnapshot,
};

pub struct TextBuffer {
    text: Arc<str>,
    revision: TextRevision,
    selection: TextSelection,
    composition: Option<TextRange>,
}

impl TextBuffer {
    pub fn new() -> Self {
        Self {
            text: Arc::from(""),
            revision: TextRevision::INITIAL,
            selection: TextSelection::collapsed(TextOffset::ZERO, TextAffinity::Downstream),
            composition: None,
        }
    }

    pub fn from_text(text: impl Into<String>) -> Result<Self, TextBufferError> {
        let text = text.into();
        validate_supported_len(text.len())?;
        let end = TextOffset(text.len() as u32);
        Ok(Self {
            text: Arc::from(text),
            revision: TextRevision::INITIAL,
            selection: TextSelection::collapsed(end, TextAffinity::Downstream),
            composition: None,
        })
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

    pub fn snapshot(&self) -> TextSnapshot {
        TextSnapshot::from_parts(
            Arc::clone(&self.text),
            self.revision,
            self.selection,
            self.composition,
        )
    }

    pub fn chunks(&self) -> TextChunks<'_> {
        TextChunks::single(TextOffset::ZERO, &self.text)
    }

    pub fn chunks_in(&self, range: TextRange) -> Result<TextChunks<'_>, TextRangeError> {
        let bytes = range.validate(&self.text)?;
        Ok(TextChunks::single(range.start, &self.text[bytes]))
    }

    pub(crate) fn text_for_edit(&self) -> &str {
        &self.text
    }

    pub(crate) fn commit_edit(
        &mut self,
        text: String,
        revision: TextRevision,
        selection: TextSelection,
        composition: Option<TextRange>,
    ) {
        self.text = Arc::from(text);
        self.revision = revision;
        self.selection = selection;
        self.composition = composition;
    }
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct TextChunk<'a> {
    pub start: TextOffset,
    pub text: &'a str,
}

impl fmt::Debug for TextChunk<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextChunk")
            .field("start", &self.start)
            .field("len_bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct TextChunks<'a> {
    next: Option<TextChunk<'a>>,
}

impl fmt::Debug for TextChunks<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextChunks")
            .field("remaining_chunks", &self.len())
            .finish_non_exhaustive()
    }
}

impl<'a> TextChunks<'a> {
    pub(crate) fn single(start: TextOffset, text: &'a str) -> Self {
        Self {
            next: (!text.is_empty()).then_some(TextChunk { start, text }),
        }
    }
}

impl<'a> Iterator for TextChunks<'a> {
    type Item = TextChunk<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next.take()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = usize::from(self.next.is_some());
        (len, Some(len))
    }
}

impl ExactSizeIterator for TextChunks<'_> {}
impl FusedIterator for TextChunks<'_> {}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextBufferError {
    TooLong { len_bytes: usize, max_bytes: u32 },
}

impl fmt::Display for TextBufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong {
                len_bytes,
                max_bytes,
            } => write!(
                f,
                "text buffer has {len_bytes} UTF-8 bytes but supports at most {max_bytes}"
            ),
        }
    }
}

impl std::error::Error for TextBufferError {}

pub(crate) fn validate_supported_len(len_bytes: usize) -> Result<u32, TextBufferError> {
    u32::try_from(len_bytes).map_err(|_| TextBufferError::TooLong {
        len_bytes,
        max_bytes: u32::MAX,
    })
}

#[cfg(test)]
mod tests {
    use crate::ui::text::{TextBuffer, TextOffset, TextRange, TextRevision};

    #[test]
    fn new_buffer_has_initial_revision_and_no_chunks() {
        let buffer = TextBuffer::new();
        assert_eq!(buffer.revision(), TextRevision::INITIAL);
        assert_eq!(buffer.end(), TextOffset::ZERO);
        assert_eq!(
            buffer.selection(),
            crate::ui::text::TextSelection::collapsed(
                TextOffset::ZERO,
                crate::ui::text::TextAffinity::Downstream
            )
        );
        assert_eq!(buffer.composition(), None);
        assert!(buffer.is_empty());
        assert_eq!(buffer.chunks().count(), 0);
    }

    #[test]
    fn exposes_bounded_utf8_chunks_without_flattening_api() {
        let buffer = TextBuffer::from_text("north—south").unwrap();
        let range = TextRange::new(TextOffset(8), TextOffset(13)).unwrap();
        let chunks = buffer.chunks_in(range).unwrap().collect::<Vec<_>>();

        assert_eq!(buffer.len_bytes(), 13);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start, TextOffset(8));
        assert_eq!(chunks[0].text, "south");
    }
}
