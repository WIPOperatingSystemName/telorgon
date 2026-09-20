use crate::graphics::render::{RenderError, RenderErrorKind};
use ash::vk;

pub(crate) fn vk_error(context: impl Into<String>, result: vk::Result) -> RenderError {
    let kind = match result {
        vk::Result::ERROR_OUT_OF_HOST_MEMORY | vk::Result::ERROR_OUT_OF_DEVICE_MEMORY => {
            RenderErrorKind::OutOfMemory
        }
        vk::Result::ERROR_DEVICE_LOST => RenderErrorKind::DeviceLost,
        vk::Result::ERROR_FEATURE_NOT_PRESENT
        | vk::Result::ERROR_EXTENSION_NOT_PRESENT
        | vk::Result::ERROR_LAYER_NOT_PRESENT
        | vk::Result::ERROR_INCOMPATIBLE_DRIVER => RenderErrorKind::Unsupported,
        _ => RenderErrorKind::Internal,
    };
    RenderError::new(kind, format!("{}: {result:?}", context.into()))
        .with_backend_code(result.as_raw() as i64)
}

pub(crate) fn internal(context: impl Into<String>) -> RenderError {
    RenderError::new(RenderErrorKind::Internal, context)
}

pub(crate) fn invalid_scene(context: impl Into<String>) -> RenderError {
    RenderError::new(RenderErrorKind::InvalidScene, context)
}

pub(crate) fn unsupported(context: impl Into<String>) -> RenderError {
    RenderError::new(RenderErrorKind::Unsupported, context)
}
