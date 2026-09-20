use bytemuck::{Pod, Zeroable};

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuShadowInstance {
    pub rect: [f32; 4],
    pub radii: [f32; 4],
    pub offset_blur_spread: [f32; 4],
    pub color_spatial_clip_flags: [u32; 4],
}
