use bytemuck::{Pod, Zeroable};

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuMaterialInstance {
    pub rect: [f32; 4],
    pub params_spatial_clip: [u32; 4],
    pub opacity: f32,
    pub material_variant: u32,
    pub flags: u32,
    pub reserved: u32,
    pub resource_range_reserved: [u32; 4],
}
