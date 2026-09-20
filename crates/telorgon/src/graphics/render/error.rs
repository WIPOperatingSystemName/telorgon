use std::fmt;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum RenderErrorKind {
    Unsupported,
    InvalidTarget,
    InvalidScene,
    OutOfMemory,
    DeviceLost,
    HostContract,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderError {
    kind: RenderErrorKind,
    context: String,
    backend_code: Option<i64>,
}

impl RenderError {
    pub fn new(kind: RenderErrorKind, context: impl Into<String>) -> Self {
        Self {
            kind,
            context: context.into(),
            backend_code: None,
        }
    }

    pub fn with_backend_code(mut self, code: i64) -> Self {
        self.backend_code = Some(code);
        self
    }

    pub fn kind(&self) -> RenderErrorKind {
        self.kind
    }

    pub fn context(&self) -> &str {
        &self.context
    }

    pub fn backend_code(&self) -> Option<i64> {
        self.backend_code
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.context)
    }
}

impl std::error::Error for RenderError {}

pub type RenderResult<T> = Result<T, RenderError>;
