use bytemuck::{Pod, Zeroable};

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuView {
    pub clip_from_view_0: [f32; 4],
    pub clip_from_view_1: [f32; 4],
    pub clip_from_view_2: [f32; 4],
    pub clip_from_view_3: [f32; 4],
    pub view_size_scale: [f32; 4],
    pub target_size_origin: [f32; 4],
    pub render_size_inverse: [f32; 4],
    /// Epoch low/high, color mode, flags (bit 0 opaque; bits 1/2 invert clip slots 0/1).
    pub epoch_flags: [u32; 4],
    /// Output-space composite clips. A negative width disables a slot; zero clips everything.
    pub placement_clip_rects: [[f32; 4]; 2],
    pub placement_clip_radii: [[f32; 4]; 2],
}
