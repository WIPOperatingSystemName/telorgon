//! Exact portable records shared by Telorgon's render planner and GPU shaders.

mod box_instance;
mod clip;
mod color;
mod glyph_instance;
mod image_instance;
mod layout;
mod material_instance;
mod shadow_instance;
mod spatial;
mod view;

pub use box_instance::GpuBoxInstance;
pub use clip::{CLIP_ANALYTIC_ROUNDED_RECT, CLIP_MASK, CLIP_NONE, CLIP_SCISSOR, GpuClip};
pub use color::{pack_srgba8, unpack_srgba8};
pub use glyph_instance::GpuGlyphInstance;
pub use image_instance::GpuImageInstance;
pub use material_instance::GpuMaterialInstance;
pub use shadow_instance::GpuShadowInstance;
pub use spatial::GpuSpatial;
pub use view::GpuView;

pub const GPU_ABI_MAJOR: u32 = 4;
pub const GPU_ABI_MINOR: u32 = 4;
pub const NO_GPU_SLOT: u32 = u32::MAX;
