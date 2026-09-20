use crate::graphics::presentation::{CompletionProof, PresentDisposition};
use crate::graphics::renderers::vulkan::CompletionPoint;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DxgiPresentOutcome {
    pub completion: CompletionPoint,
    pub transport_completion: CompletionProof<u64>,
    pub present_completion: CompletionProof<()>,
    pub disposition: PresentDisposition,
    pub reconfigure_pending: bool,
    pub maintenance_pending: bool,
}
