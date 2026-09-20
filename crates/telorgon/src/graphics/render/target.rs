use crate::foundation::{RectI, SizeI};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RenderTargetInfo {
    pub extent: SizeI,
    pub region: RectI,
    pub sample_count: u8,
    pub color_space: ColorSpace,
    pub alpha_mode: AlphaMode,
}

impl RenderTargetInfo {
    pub fn full(extent: SizeI) -> Self {
        Self {
            extent,
            region: RectI {
                x: 0,
                y: 0,
                width: extent.width,
                height: extent.height,
            },
            sample_count: 1,
            color_space: ColorSpace::Srgb,
            alpha_mode: AlphaMode::Premultiplied,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    Linear,
    Srgb,
    Extended,
    BackendDefined,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AlphaMode {
    Opaque,
    Premultiplied,
}
