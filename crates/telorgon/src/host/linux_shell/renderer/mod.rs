#[cfg(feature = "shell-screencast-linux")]
mod capture_damage;
mod scanout;
mod capture;
pub(super) use capture::{CaptureBuffer, CaptureJob, CaptureView, CaptureSubmitFailure};
pub(super) use scanout::prepare;
mod software;
mod vulkan;

use crate::host::application::AppResult;
use crate::foundation::RectI;

use super::scene::ShellFrame;
use software::SoftwareShellRenderer;
use vulkan::VulkanShellRenderer;

pub(super) use vulkan::VulkanCompletion;
pub(super) use vulkan::{DmaBufPublication, DmaBufQueueResult, DmaBufRelease, DmaBufRetirement};
#[cfg(test)]
pub(super) use vulkan::{
    VULKAN_STAGING_HEADROOM_BYTES_PER_SLOT, VULKAN_STAGING_MIN_BYTES_PER_SLOT,
    vulkan_staging_budget_bytes,
};

pub(super) enum ShellRenderResult {
    Vulkan {
        releases: Vec<DmaBufRelease>,
        discarded: Vec<DmaBufRetirement>,
    },
    Software {
        damage: RectI,
    },
}

/// The only point where the Linux desktop chooses a rendering implementation.
///
/// After construction, frames stay entirely inside the selected backend. The enum does not
/// expose either backend's scene or target types to the Wayland/KMS owner loop.
pub(super) enum ShellRenderer {
    Vulkan(VulkanShellRenderer),
    Software(SoftwareShellRenderer),
}

impl ShellRenderer {
    pub(super) fn clear_capture_cursor(&mut self) {
        if let Self::Vulkan(renderer) = self { renderer.clear_capture_cursor(); }
    }

    pub(super) fn update_capture_cursor(&mut self, frame: ShellFrame) -> AppResult<()> {
        match self {
            Self::Vulkan(renderer) => renderer.update_capture_cursor(frame),
            Self::Software(_) => Err(crate::host::application::AppError::new("Vulkan capture reached software renderer")),
        }
    }

    pub(super) fn capture_revision(&self, cursor: crate::shell::capture::CaptureCursorMode) -> u64 {
        match self {
            Self::Vulkan(renderer) => renderer.capture_revision(cursor),
            Self::Software(_) => 0,
        }
    }

    pub(super) fn retry_capture(&self, receipt: crate::graphics::renderers::vulkan::SubmissionReceipt, job: CaptureJob) -> AppResult<()> {
        match self {
            Self::Vulkan(renderer) => renderer.retry_capture(receipt, job),
            Self::Software(_) => Err(crate::host::application::AppError::new("Vulkan capture reached software renderer")),
        }
    }

    #[cfg(feature = "shell-screencast-linux")]
    pub(super) fn preview_output_scene(&self, layout: crate::shell::capture::CaptureLayout) -> Option<super::capture_scene::CaptureScene> {
        match self {
            Self::Vulkan(renderer) => Some(renderer.preview_output_scene(layout)),
            Self::Software(_) => None,
        }
    }

    pub(super) fn allocate_capture(&self, layout: crate::shell::capture::CaptureLayout) -> AppResult<CaptureBuffer> {
        match self {
            Self::Vulkan(renderer) => renderer.allocate_capture(layout),
            Self::Software(_) => Err(crate::host::application::AppError::new("software capture is not implemented")),
        }
    }

    #[cfg(feature = "shell-screencast-linux")]
    pub(super) fn capture_memory_budget(&self) -> Option<std::sync::Arc<crate::media::video::MemoryBudget>> {
        match self { Self::Vulkan(renderer) => Some(renderer.capture_memory_budget()), Self::Software(_) => None }
    }

    #[cfg(feature = "shell-screencast-linux")]
    pub(super) fn screen_gpu_producer(&self, layout: crate::shell::capture::CaptureLayout, fps: u32) -> Option<Box<dyn crate::media::video::VideoGpuProducer>> {
        match self { Self::Vulkan(renderer) => renderer.screen_gpu_producer(layout, fps), Self::Software(_) => None }
    }
    #[cfg(feature = "shell-screencast-linux")]
    pub(super) fn allocate_video_capture(&self, layout: crate::shell::capture::CaptureLayout, format: crate::media::video::VideoFormat, gpu: bool) -> AppResult<CaptureBuffer> {
        match self { Self::Vulkan(renderer) => renderer.allocate_video_capture(layout, format, gpu), Self::Software(_) => Err(crate::host::application::AppError::new("GPU capture requires Vulkan")) }
    }

    #[cfg(feature = "shell-screencast-linux")]
    pub(super) fn capture_damage(&self, after: Option<u64>, layout: crate::shell::capture::CaptureLayout) -> (u64, Vec<crate::media::video::VideoRect>) {
        match self { Self::Vulkan(renderer) => renderer.capture_damage(after, layout), Self::Software(_) => (0, Vec::new()) }
    }

    pub(super) fn submit_capture_recoverable(&mut self, job: CaptureJob) -> Result<(), CaptureSubmitFailure> {
        match self {
            Self::Vulkan(renderer) => renderer.submit_capture_recoverable(job),
            Self::Software(_) => Err(CaptureSubmitFailure {
                error: crate::host::application::AppError::new("software capture is not implemented"),
                rejected: Some(Box::new(job)),
            }),
        }
    }

    pub(super) fn motion_enabled(&self) -> bool {
        match self {
            Self::Vulkan(r) => r.motion_enabled(),
            Self::Software(_) => true,
        }
    }
    pub(super) fn take_motion_failure(&mut self) -> bool {
        match self {
            Self::Vulkan(renderer) => renderer.take_motion_failure(),
            Self::Software(_) => false,
        }
    }
    pub(super) fn is_vulkan(&self) -> bool {
        matches!(self, Self::Vulkan(_))
    }

    pub(super) fn dma_buf_formats(&self) -> Vec<crate::integrations::wayland::compositor::DmaBufFormat> {
        match self {
            Self::Vulkan(renderer) => renderer.dma_buf_formats(),
            Self::Software(_) => Vec::new(),
        }
    }

    pub(super) fn queue_dma_buf(
        &mut self,
        publication: DmaBufPublication,
        display: &crate::integrations::wayland::server::Display,
    ) -> AppResult<DmaBufQueueResult> {
        match self {
            Self::Vulkan(renderer) => renderer.queue_dma_buf(publication, display),
            Self::Software(_) => Err(crate::host::application::AppError::new(
                "DMA-BUF publication reached the software desktop renderer",
            )),
        }
    }

    pub(super) fn cancel_dma_buf_surface(
        &mut self,
        surface: crate::integrations::wayland::compositor::WaylandSurfaceId,
    ) -> Option<DmaBufRetirement> {
        match self {
            Self::Vulkan(renderer) => renderer.cancel_dma_buf_surface(surface),
            Self::Software(_) => None,
        }
    }

    pub(super) fn completion_event_fd(&self) -> Option<i32> {
        match self {
            Self::Vulkan(renderer) => Some(renderer.completion_event_fd()),
            Self::Software(_) => None,
        }
    }

    pub(super) fn take_acquire_wakeup(&mut self) -> bool {
        match self {
            Self::Vulkan(r) => r.take_acquire_wakeup(),
            Self::Software(_) => false,
        }
    }

    pub(super) fn poll_allocations(
        &mut self,
        trace: &mut super::latency_trace::LatencyTrace,
    ) -> AppResult<bool> {
        match self {
            Self::Vulkan(renderer) => renderer.poll_allocations(trace),
            Self::Software(_) => Ok(false),
        }
    }

    pub(super) fn prepare_render(
        &mut self,
        eligible: &std::collections::BTreeSet<crate::integrations::wayland::compositor::WaylandSurfaceId>,
        trace: &mut super::latency_trace::LatencyTrace,
    ) -> AppResult<std::collections::BTreeSet<crate::integrations::wayland::compositor::WaylandSurfaceId>> {
        match self {
            Self::Vulkan(renderer) => renderer.prepare_render(eligible, trace),
            Self::Software(_) => Ok(eligible.clone()),
        }
    }

    pub(super) fn drain_completions(&self) -> Vec<VulkanCompletion> {
        match self {
            Self::Vulkan(renderer) => renderer.drain_completions(),
            Self::Software(_) => Vec::new(),
        }
    }

    pub(super) fn render(
        &mut self,
        target_index: usize,
        frame: ShellFrame,
        trace: &mut super::latency_trace::LatencyTrace,
    ) -> AppResult<ShellRenderResult> {
        match self {
            Self::Vulkan(renderer) => {
                let result = renderer.render(target_index, frame, trace)?;
                Ok(ShellRenderResult::Vulkan {
                    releases: result.releases,
                    discarded: result.discarded,
                })
            }
            Self::Software(renderer) => renderer
                .render(target_index, frame)
                .map(|damage| ShellRenderResult::Software { damage }),
        }
    }

    pub(super) fn software_pixels(&self) -> Option<&[u8]> {
        match self {
            Self::Vulkan(_) => None,
            Self::Software(renderer) => Some(renderer.pixels()),
        }
    }

    pub(super) fn mark_software_copied(&mut self, target_index: usize) {
        if let Self::Software(renderer) = self {
            renderer.mark_copied(target_index);
        }
    }
}
