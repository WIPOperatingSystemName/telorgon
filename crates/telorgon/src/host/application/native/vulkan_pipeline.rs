use crate::foundation::SizeI;
#[cfg(target_os = "windows")]
use crate::graphics::bridges::vulkan_dxgi as dxgi_bridge;
#[cfg(target_os = "windows")]
use crate::graphics::bridges::vulkan_dxgi::{
    AcquiredVulkanDxgiFrame, VulkanDxgiAcquireOutcome, VulkanDxgiBridge,
};
use crate::graphics::presentation::wsi::{
    AcquireOutcome, AcquiredVulkanFrame, PresentCompletion, PresentOutcome, PresentResult,
    PresenterReconfigurePolicy, PresenterRecovery, VulkanWinitPresenter, VulkanWinitSurface,
};
#[cfg(target_os = "windows")]
use crate::graphics::presentation::wsi::{PresentError, PresentErrorKind};
use crate::graphics::renderers::vulkan::{
    VulkanDevice, VulkanRecordedFrame, VulkanRecordingFrame, VulkanTarget,
};

pub(super) enum VulkanPresentationPipeline {
    #[cfg(target_os = "windows")]
    Dxgi(Box<VulkanDxgiBridge>),
    Wsi(Box<VulkanWinitPresenter>),
}

pub(super) enum PipelineAcquireOutcome<'presenter> {
    Ready(PipelineAcquiredFrame<'presenter>),
    Suspended,
    NotReady,
    NeedsReconfigure,
}

pub(super) enum PipelineAcquiredFrame<'presenter> {
    #[cfg(target_os = "windows")]
    Dxgi(AcquiredVulkanDxgiFrame<'presenter>),
    Wsi(AcquiredVulkanFrame<'presenter>),
}

impl VulkanPresentationPipeline {
    pub fn recovery(&self) -> PresenterRecovery {
        match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(presenter) => presenter.recovery(),
            Self::Wsi(presenter) => presenter.recovery(),
        }
    }

    pub fn set_reconfigure_policy(&mut self, policy: PresenterReconfigurePolicy) {
        match self {
            Self::Wsi(presenter) => presenter.set_reconfigure_policy(policy),
            #[cfg(target_os = "windows")]
            Self::Dxgi(_) => {}
        }
    }

    pub fn poll_present_completion(
        &mut self,
        completion: PresentCompletion,
    ) -> PresentResult<bool> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(presenter) => bridge_result(presenter.poll_present_completion(completion)),
            Self::Wsi(presenter) => presenter.poll_present_completion(completion),
        }
    }

    pub fn enforce_retirement_limit(
        &mut self,
        device: &VulkanDevice,
        maximum: usize,
    ) -> PresentResult<()> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(presenter) => {
                bridge_result(presenter.enforce_retirement_limit(device, maximum))
            }
            Self::Wsi(presenter) => presenter.enforce_retirement_limit(device, maximum),
        }
    }

    pub fn resize(&mut self, extent: SizeI) -> bool {
        match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(presenter) => presenter.resize(extent),
            Self::Wsi(presenter) => presenter.resize(extent),
        }
    }

    pub fn suspend(&mut self) -> PresentResult<()> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(presenter) => bridge_result(presenter.suspend()),
            Self::Wsi(presenter) => presenter.suspend(),
        }
    }

    pub fn resume(&mut self, device: &VulkanDevice, extent: SizeI) -> PresentResult<()> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(presenter) => bridge_result(presenter.resume(device, extent)),
            Self::Wsi(presenter) => presenter.resume(device, extent),
        }
    }

    pub fn replace_surface(
        &mut self,
        device: &VulkanDevice,
        surface: VulkanWinitSurface,
        extent: SizeI,
    ) -> PresentResult<()> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(presenter) => {
                drop(surface);
                bridge_result(presenter.resume(device, extent))
            }
            Self::Wsi(presenter) => presenter.replace_surface(device, surface, extent),
        }
    }

    pub fn acquire<'a>(
        &'a mut self,
        device: &VulkanDevice,
        frame: &VulkanRecordingFrame<'_>,
    ) -> PresentResult<PipelineAcquireOutcome<'a>> {
        Ok(match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(presenter) => match bridge_result(presenter.acquire(device, frame))? {
                VulkanDxgiAcquireOutcome::Ready(frame) => {
                    PipelineAcquireOutcome::Ready(PipelineAcquiredFrame::Dxgi(frame))
                }
                VulkanDxgiAcquireOutcome::Suspended => PipelineAcquireOutcome::Suspended,
                VulkanDxgiAcquireOutcome::NotReady => PipelineAcquireOutcome::NotReady,
                VulkanDxgiAcquireOutcome::NeedsReconfigure => {
                    PipelineAcquireOutcome::NeedsReconfigure
                }
            },
            Self::Wsi(presenter) => match presenter.acquire(device, frame)? {
                AcquireOutcome::Ready(frame) => {
                    PipelineAcquireOutcome::Ready(PipelineAcquiredFrame::Wsi(frame))
                }
                AcquireOutcome::Suspended => PipelineAcquireOutcome::Suspended,
                AcquireOutcome::NotReady => PipelineAcquireOutcome::NotReady,
                AcquireOutcome::NeedsReconfigure => PipelineAcquireOutcome::NeedsReconfigure,
            },
        })
    }

    pub fn shutdown(&mut self, device: &VulkanDevice) -> PresentResult<()> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(presenter) => bridge_result(presenter.shutdown(device)),
            Self::Wsi(presenter) => presenter.shutdown(device),
        }
    }
}

impl PipelineAcquiredFrame<'_> {
    pub fn target(&self) -> VulkanTarget<'_> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(frame) => frame.target(),
            Self::Wsi(frame) => frame.target(),
        }
    }

    pub fn submit_and_present(
        self,
        device: &VulkanDevice,
        frame: VulkanRecordedFrame,
    ) -> PresentResult<PresentOutcome> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(acquired) => {
                let outcome = bridge_result(acquired.submit_and_present(device, frame))?;
                Ok(PresentOutcome {
                    completion: outcome.completion,
                    presentation_completion: None,
                    disposition: outcome.disposition,
                    reconfigure_pending: outcome.reconfigure_pending,
                    maintenance_pending: outcome.maintenance_pending,
                })
            }
            Self::Wsi(acquired) => acquired.submit_and_present(device, frame),
        }
    }

    pub fn discard(self, device: &VulkanDevice) -> PresentResult<()> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Dxgi(acquired) => bridge_result(acquired.discard(device)),
            Self::Wsi(acquired) => acquired.discard(device),
        }
    }
}

#[cfg(target_os = "windows")]
fn bridge_result<T>(result: dxgi_bridge::PresentResult<T>) -> PresentResult<T> {
    result.map_err(|error| {
        let kind = match error.kind() {
            dxgi_bridge::PresentErrorKind::Unsupported => PresentErrorKind::Unsupported,
            dxgi_bridge::PresentErrorKind::SurfaceLost => PresentErrorKind::SurfaceLost,
            dxgi_bridge::PresentErrorKind::DeviceLost => PresentErrorKind::DeviceLost,
            dxgi_bridge::PresentErrorKind::OutOfMemory => PresentErrorKind::OutOfMemory,
            dxgi_bridge::PresentErrorKind::InvalidState => PresentErrorKind::InvalidState,
            dxgi_bridge::PresentErrorKind::Native => PresentErrorKind::Native,
        };
        PresentError::new(kind, error.to_string())
    })
}
