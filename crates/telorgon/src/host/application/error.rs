use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppError {
    message: String,
}

impl AppError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

#[cfg(not(target_os = "uefi"))]
impl From<crate::services::session::Error> for AppError {
    fn from(error: crate::services::session::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<crate::runtime::RuntimeError> for AppError {
    fn from(error: crate::runtime::RuntimeError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<crate::graphics::render::RenderError> for AppError {
    fn from(error: crate::graphics::render::RenderError) -> Self {
        Self::new(error.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
