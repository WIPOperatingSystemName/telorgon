use bytemuck::{Pod, Zeroable};

pub const CLIP_NONE: u32 = 0;
pub const CLIP_SCISSOR: u32 = 1;
pub const CLIP_ANALYTIC_ROUNDED_RECT: u32 = 2;
pub const CLIP_MASK: u32 = 3;

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuClip {
    pub view_bounds: [f32; 4],
    pub local_rect: [f32; 4],
    pub local_from_view_0: [f32; 4],
    pub local_from_view_1: [f32; 4],
    pub radii: [f32; 4],
    pub mask_uv_from_view_0: [f32; 4],
    pub mask_uv_from_view_1: [f32; 4],
    pub mode_mask_flags: [u32; 4],
}
