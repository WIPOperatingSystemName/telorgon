use bytemuck::{Pod, Zeroable};

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuImageInstance {
    pub rect: [f32; 4],
    pub uv_normalized: [f32; 4],
    pub tint_spatial_clip_texture: [u32; 4],
    pub opacity: f32,
    pub sampler_key: u32,
    pub flags: u32,
    pub reserved: u32,
}
