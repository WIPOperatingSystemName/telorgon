mod scanout;
pub(super) use scanout::prepare;
mod software;
mod vulkan;

use crate::application_host::AppResult;
use crate::core::RectI;

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
    pub(super) fn motion_enabled(&self) -> bool {
        match self {
            Self::Vulkan(r) => r.motion_enabled(),
            Self::Software(_) => true,
        }
    }
    pub(super) fn is_vulkan(&self) -> bool {
        matches!(self, Self::Vulkan(_))
    }

    pub(super) fn dma_buf_formats(&self) -> Vec<crate::compositor_wayland::DmaBufFormat> {
        match self {
            Self::Vulkan(renderer) => renderer.dma_buf_formats(),
            Self::Software(_) => Vec::new(),
        }
    }

    pub(super) fn queue_dma_buf(
        &mut self,
        publication: DmaBufPublication,
        display: &crate::wayland_server::Display,
    ) -> AppResult<DmaBufQueueResult> {
        match self {
            Self::Vulkan(renderer) => renderer.queue_dma_buf(publication, display),
            Self::Software(_) => Err(crate::application_host::AppError::new(
                "DMA-BUF publication reached the software desktop renderer",
            )),
        }
    }

    pub(super) fn cancel_dma_buf_surface(
        &mut self,
        surface: crate::compositor_wayland::WaylandSurfaceId,
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
        eligible: &std::collections::BTreeSet<crate::compositor_wayland::WaylandSurfaceId>,
        trace: &mut super::latency_trace::LatencyTrace,
    ) -> AppResult<std::collections::BTreeSet<crate::compositor_wayland::WaylandSurfaceId>> {
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
