use crate::foundation::{RectI, SizeI};

use crate::graphics::render::{RenderBackend, RenderResult};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ReadbackRequest {
    pub region: RectI,
    pub format: ReadbackFormat,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ReadbackFormat {
    #[default]
    Rgba8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadbackImage {
    pub extent: SizeI,
    pub row_bytes: usize,
    pub pixels: Vec<u8>,
}

pub trait RenderReadback<B: RenderBackend> {
    type Pending;

    fn record_readback<'frame>(
        &self,
        backend: &B,
        frame: &mut B::FrameContext<'frame>,
        target: &B::Target<'frame>,
        request: &ReadbackRequest,
    ) -> RenderResult<Self::Pending>;
}
