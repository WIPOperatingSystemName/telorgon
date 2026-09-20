//! Renderer-owned reusable capture storage and completion messages.

use super::super::capture::FrameTicket;
use crate::compositor_render::capture::VulkanCaptureSlot;
use crate::shell::capture::CaptureCursorMode;

pub(in crate::application_host::shell_wayland) struct CaptureBuffer {
    pub layout: crate::shell::capture::CaptureLayout,
    pub slot: VulkanCaptureSlot,
    pub pixels: Vec<u8>,
}

pub(in crate::application_host::shell_wayland) enum CaptureView {
    Output,
    #[cfg(feature = "shell-screencast-linux")]
    Window(super::super::capture_window::WindowCapture),
}

pub(in crate::application_host::shell_wayland) struct CaptureJob {
    pub view: CaptureView,
    pub direct: Option<(
        crate::compositor_wayland::DirectCaptureJob,
        u64,
        crate::compositor_wayland::OutputTransform,
    )>,
    pub ticket: FrameTicket,
    pub cursor: CaptureCursorMode,
    pub buffer: CaptureBuffer,
}

/// Content revisions only coalesce work; they confer no authorization or resource ownership.
#[derive(Default)]
pub(super) struct CaptureRevisions {
    output: u64,
    combined: u64,
}
impl CaptureRevisions {
    pub fn output_changed(&mut self) {
        self.cursor_changed();
        self.output = self.combined;
    }
    pub fn cursor_changed(&mut self) {
        self.combined = self.combined.wrapping_add(1).max(1);
    }
    pub fn get(&self, mode: CaptureCursorMode) -> u64 {
        match mode {
            CaptureCursorMode::Hidden => self.output,
            CaptureCursorMode::Embedded => self.combined,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_motion_changes_embedded_streams_without_invalidating_hidden_streams() {
        let mut revisions = CaptureRevisions::default();
        revisions.output_changed();
        let output = revisions.get(CaptureCursorMode::Hidden);
        revisions.cursor_changed();
        assert_eq!(revisions.get(CaptureCursorMode::Hidden), output);
        assert_ne!(revisions.get(CaptureCursorMode::Embedded), output);
        let embedded = revisions.get(CaptureCursorMode::Embedded);
        revisions.output_changed();
        assert_ne!(revisions.get(CaptureCursorMode::Embedded), embedded);
        assert_eq!(
            revisions.get(CaptureCursorMode::Hidden),
            revisions.get(CaptureCursorMode::Embedded)
        );
    }
}

/// A returned job proves recording never reached queue submission. Missing job
/// means submission/worker failure requires the existing renderer recovery path.
pub(in crate::application_host::shell_wayland) struct CaptureSubmitFailure {
    pub error: crate::application_host::AppError,
    pub rejected: Option<Box<CaptureJob>>,
}
