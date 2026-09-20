use std::fmt;
use std::ops::Range;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextRevision(pub u64);

impl TextRevision {
    pub const INITIAL: Self = Self(0);
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextOffset(pub u32);

impl TextOffset {
    pub const ZERO: Self = Self(0);

    pub const fn from_bytes(bytes: u32) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> u32 {
        self.0
    }

    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    pub fn validate(self, text: &str) -> Result<usize, TextRangeError> {
        validate_offset(text, self)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextRange {
    pub start: TextOffset,
    pub end: TextOffset,
}

impl TextRange {
    pub fn new(start: TextOffset, end: TextOffset) -> Result<Self, TextRangeError> {
        if start > end {
            return Err(TextRangeError::Reversed { start, end });
        }
        Ok(Self { start, end })
    }

    pub const fn collapsed(offset: TextOffset) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    pub const fn is_empty(self) -> bool {
        self.start.0 == self.end.0
    }

    pub const fn len_bytes(self) -> Option<u32> {
        self.end.0.checked_sub(self.start.0)
    }

    pub fn validate(self, text: &str) -> Result<Range<usize>, TextRangeError> {
        if self.start > self.end {
            return Err(TextRangeError::Reversed {
                start: self.start,
                end: self.end,
            });
        }
        let start = validate_offset(text, self.start)?;
        let end = validate_offset(text, self.end)?;
        Ok(start..end)
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextAffinity {
    Upstream,
    #[default]
    Downstream,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextSelection {
    pub anchor: TextOffset,
    pub active: TextOffset,
    pub affinity: TextAffinity,
}

impl TextSelection {
    pub const fn collapsed(offset: TextOffset, affinity: TextAffinity) -> Self {
        Self {
            anchor: offset,
            active: offset,
            affinity,
        }
    }

    pub const fn is_collapsed(self) -> bool {
        self.anchor.0 == self.active.0
    }

    pub const fn is_forward(self) -> bool {
        self.anchor.0 <= self.active.0
    }

    pub const fn range(self) -> TextRange {
        if self.anchor.0 <= self.active.0 {
            TextRange {
                start: self.anchor,
                end: self.active,
            }
        } else {
            TextRange {
                start: self.active,
                end: self.anchor,
            }
        }
    }

    pub fn validate(self, text: &str) -> Result<Self, TextRangeError> {
        validate_offset(text, self.anchor)?;
        validate_offset(text, self.active)?;
        Ok(self)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextRangeError {
    OutOfBounds {
        offset: TextOffset,
        len_bytes: usize,
    },
    NotCharBoundary {
        offset: TextOffset,
    },
    Reversed {
        start: TextOffset,
        end: TextOffset,
    },
}

impl fmt::Display for TextRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds { offset, len_bytes } => write!(
                f,
                "text offset {} is out of bounds for {len_bytes} UTF-8 bytes",
                offset.0
            ),
            Self::NotCharBoundary { offset } => {
                write!(
                    f,
                    "text offset {} is not a UTF-8 character boundary",
                    offset.0
                )
            }
            Self::Reversed { start, end } => write!(
                f,
                "text range is reversed: start {} is after end {}",
                start.0, end.0
            ),
        }
    }
}

impl std::error::Error for TextRangeError {}

fn validate_offset(text: &str, offset: TextOffset) -> Result<usize, TextRangeError> {
    let offset_usize = offset.as_usize();
    if offset_usize > text.len() {
        return Err(TextRangeError::OutOfBounds {
            offset,
            len_bytes: text.len(),
        });
    }
    if !text.is_char_boundary(offset_usize) {
        return Err(TextRangeError::NotCharBoundary { offset });
    }
    Ok(offset_usize)
}

#[cfg(test)]
mod tests {
    use super::{TextAffinity, TextOffset, TextRange, TextRangeError, TextSelection};

    #[test]
    fn validates_empty_end_and_multibyte_scalar_boundaries() {
        let text = "aé中";
        assert_eq!(TextOffset::ZERO.validate(text), Ok(0));
        assert_eq!(TextOffset(1).validate(text), Ok(1));
        assert_eq!(TextOffset(3).validate(text), Ok(3));
        assert_eq!(TextOffset(6).validate(text), Ok(6));
        assert_eq!(
            TextOffset(2).validate(text),
            Err(TextRangeError::NotCharBoundary {
                offset: TextOffset(2)
            })
        );
        assert_eq!(
            TextOffset(7).validate(text),
            Err(TextRangeError::OutOfBounds {
                offset: TextOffset(7),
                len_bytes: 6
            })
        );
    }

    #[test]
    fn validates_range_order_before_slicing() {
        assert_eq!(
            TextRange::new(TextOffset(3), TextOffset(1)),
            Err(TextRangeError::Reversed {
                start: TextOffset(3),
                end: TextOffset(1)
            })
        );
        assert_eq!(
            TextRange::new(TextOffset(1), TextOffset(3))
                .unwrap()
                .validate("aé"),
            Ok(1..3)
        );
    }

    #[test]
    fn selection_preserves_direction_and_affinity() {
        let selection = TextSelection {
            anchor: TextOffset(6),
            active: TextOffset(1),
            affinity: TextAffinity::Upstream,
        };
        assert!(!selection.is_forward());
        assert!(!selection.is_collapsed());
        assert_eq!(
            selection.range(),
            TextRange {
                start: TextOffset(1),
                end: TextOffset(6)
            }
        );
        assert_eq!(selection.validate("aé中"), Ok(selection));
    }

    #[test]
    fn scalar_validation_does_not_claim_grapheme_navigation() {
        let combining = "e\u{301}";
        assert_eq!(TextOffset(1).validate(combining), Ok(1));

        let emoji_zwj = "👩‍💻";
        let after_first_scalar = "👩".len() as u32;
        assert_eq!(
            TextOffset(after_first_scalar).validate(emoji_zwj),
            Ok(after_first_scalar as usize)
        );
    }
}
