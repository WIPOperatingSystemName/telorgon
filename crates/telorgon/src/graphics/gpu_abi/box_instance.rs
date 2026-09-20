use bytemuck::{Pod, Zeroable};

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuBoxInstance {
    pub rect: [f32; 4],
    pub radii: [f32; 4],
    pub border_widths: [f32; 4],
    pub fill_border_t_r_b: [u32; 4],
    pub border_l_spatial_clip_flags: [u32; 4],
    pub opacity: f32,
    pub reserved: [u32; 3],
    /// width, offset, reserved, reserved
    pub outline: [f32; 4],
    /// x, y, blur, spread
    pub shadow_0: [f32; 4],
    /// x, y, blur, spread
    pub shadow_1: [f32; 4],
    /// outline color, shadow 0 color, shadow 1 color, shadow count
    pub outline_shadow_colors: [u32; 4],
}
