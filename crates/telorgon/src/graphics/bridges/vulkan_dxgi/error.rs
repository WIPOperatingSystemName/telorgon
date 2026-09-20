use ash::vk;
use thiserror::Error;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentErrorKind {
    Unsupported,
    SurfaceLost,
    DeviceLost,
    OutOfMemory,
    InvalidState,
    Native,
}

#[derive(Debug, Error)]
#[error("{context}")]
pub struct PresentError {
    kind: PresentErrorKind,
    context: String,
    native: Option<vk::Result>,
}

impl PresentError {
    pub fn new(kind: PresentErrorKind, context: impl Into<String>) -> Self {
        Self {
            kind,
            context: context.into(),
            native: None,
        }
    }

    pub fn from_vk(context: impl Into<String>, result: vk::Result) -> Self {
        let kind = match result {
            vk::Result::ERROR_SURFACE_LOST_KHR => PresentErrorKind::SurfaceLost,
            vk::Result::ERROR_DEVICE_LOST => PresentErrorKind::DeviceLost,
            vk::Result::ERROR_OUT_OF_DEVICE_MEMORY | vk::Result::ERROR_OUT_OF_HOST_MEMORY => {
                PresentErrorKind::OutOfMemory
            }
            _ => PresentErrorKind::Native,
        };
        Self {
            kind,
            context: format!("{}: {result:?}", context.into()),
            native: Some(result),
        }
    }

    pub const fn kind(&self) -> PresentErrorKind {
        self.kind
    }

    pub const fn native_result(&self) -> Option<vk::Result> {
        self.native
    }
}

pub type PresentResult<T> = Result<T, PresentError>;

impl From<crate::graphics::presentation::PresentationError> for PresentError {
    fn from(error: crate::graphics::presentation::PresentationError) -> Self {
        let kind = match error.kind() {
            crate::graphics::presentation::PresentationErrorKind::Unsupported => {
                PresentErrorKind::Unsupported
            }
            crate::graphics::presentation::PresentationErrorKind::SurfaceLost => {
                PresentErrorKind::SurfaceLost
            }
            crate::graphics::presentation::PresentationErrorKind::DeviceLost => PresentErrorKind::DeviceLost,
            crate::graphics::presentation::PresentationErrorKind::OutOfMemory => {
                PresentErrorKind::OutOfMemory
            }
            crate::graphics::presentation::PresentationErrorKind::InvalidState => {
                PresentErrorKind::InvalidState
            }
            crate::graphics::presentation::PresentationErrorKind::Native => PresentErrorKind::Native,
        };
        Self::new(kind, error.to_string())
    }
}
