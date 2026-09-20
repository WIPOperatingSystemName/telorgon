use bytemuck::{Pod, Zeroable};

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuSpatial {
    pub local_to_view_0: [f32; 4],
    pub local_to_view_1: [f32; 4],
}
