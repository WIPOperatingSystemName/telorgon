use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextError {
    message: String,
}

impl TextError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) fn is_atlas_full(&self) -> bool {
        self.message == "glyph atlas is full"
    }
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TextError {}

pub type TextResult<T> = Result<T, TextError>;
