use bytemuck::{Pod, Zeroable};

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuGlyphInstance {
    pub rect: [f32; 4],
    pub uv_texels: [f32; 4],
    pub color_spatial_clip_page: [u32; 4],
    pub opacity: f32,
    pub flags: u32,
    pub reserved: [u32; 2],
}
