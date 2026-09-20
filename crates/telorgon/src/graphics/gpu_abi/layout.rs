use core::mem::{align_of, offset_of, size_of};

use crate::graphics::gpu_abi::{
    GpuBoxInstance, GpuClip, GpuGlyphInstance, GpuImageInstance, GpuMaterialInstance,
    GpuShadowInstance, GpuSpatial, GpuView,
};

const _: () = {
    assert!(align_of::<GpuView>() == 16);
    assert!(size_of::<GpuView>() == 192);
    assert!(offset_of!(GpuView, clip_from_view_0) == 0);
    assert!(offset_of!(GpuView, clip_from_view_1) == 16);
    assert!(offset_of!(GpuView, clip_from_view_2) == 32);
    assert!(offset_of!(GpuView, clip_from_view_3) == 48);
    assert!(offset_of!(GpuView, view_size_scale) == 64);
    assert!(offset_of!(GpuView, target_size_origin) == 80);
    assert!(offset_of!(GpuView, render_size_inverse) == 96);
    assert!(offset_of!(GpuView, epoch_flags) == 112);
    assert!(offset_of!(GpuView, placement_clip_rects) == 128);
    assert!(offset_of!(GpuView, placement_clip_radii) == 160);

    assert!(align_of::<GpuSpatial>() == 16);
    assert!(size_of::<GpuSpatial>() == 32);
    assert!(offset_of!(GpuSpatial, local_to_view_0) == 0);
    assert!(offset_of!(GpuSpatial, local_to_view_1) == 16);

    assert!(align_of::<GpuClip>() == 16);
    assert!(size_of::<GpuClip>() == 128);
    assert!(offset_of!(GpuClip, view_bounds) == 0);
    assert!(offset_of!(GpuClip, local_rect) == 16);
    assert!(offset_of!(GpuClip, local_from_view_0) == 32);
    assert!(offset_of!(GpuClip, local_from_view_1) == 48);
    assert!(offset_of!(GpuClip, radii) == 64);
    assert!(offset_of!(GpuClip, mask_uv_from_view_0) == 80);
    assert!(offset_of!(GpuClip, mask_uv_from_view_1) == 96);
    assert!(offset_of!(GpuClip, mode_mask_flags) == 112);

    assert!(align_of::<GpuBoxInstance>() == 16);
    assert!(size_of::<GpuBoxInstance>() == 160);
    assert!(offset_of!(GpuBoxInstance, rect) == 0);
    assert!(offset_of!(GpuBoxInstance, radii) == 16);
    assert!(offset_of!(GpuBoxInstance, border_widths) == 32);
    assert!(offset_of!(GpuBoxInstance, fill_border_t_r_b) == 48);
    assert!(offset_of!(GpuBoxInstance, border_l_spatial_clip_flags) == 64);
    assert!(offset_of!(GpuBoxInstance, opacity) == 80);
    assert!(offset_of!(GpuBoxInstance, reserved) == 84);
    assert!(offset_of!(GpuBoxInstance, outline) == 96);
    assert!(offset_of!(GpuBoxInstance, shadow_0) == 112);
    assert!(offset_of!(GpuBoxInstance, shadow_1) == 128);
    assert!(offset_of!(GpuBoxInstance, outline_shadow_colors) == 144);

    assert!(align_of::<GpuShadowInstance>() == 16);
    assert!(size_of::<GpuShadowInstance>() == 64);
    assert!(offset_of!(GpuShadowInstance, rect) == 0);
    assert!(offset_of!(GpuShadowInstance, radii) == 16);
    assert!(offset_of!(GpuShadowInstance, offset_blur_spread) == 32);
    assert!(offset_of!(GpuShadowInstance, color_spatial_clip_flags) == 48);

    assert!(align_of::<GpuGlyphInstance>() == 16);
    assert!(size_of::<GpuGlyphInstance>() == 64);
    assert!(offset_of!(GpuGlyphInstance, rect) == 0);
    assert!(offset_of!(GpuGlyphInstance, uv_texels) == 16);
    assert!(offset_of!(GpuGlyphInstance, color_spatial_clip_page) == 32);
    assert!(offset_of!(GpuGlyphInstance, opacity) == 48);
    assert!(offset_of!(GpuGlyphInstance, flags) == 52);
    assert!(offset_of!(GpuGlyphInstance, reserved) == 56);

    assert!(align_of::<GpuImageInstance>() == 16);
    assert!(size_of::<GpuImageInstance>() == 64);
    assert!(offset_of!(GpuImageInstance, rect) == 0);
    assert!(offset_of!(GpuImageInstance, uv_normalized) == 16);
    assert!(offset_of!(GpuImageInstance, tint_spatial_clip_texture) == 32);
    assert!(offset_of!(GpuImageInstance, opacity) == 48);
    assert!(offset_of!(GpuImageInstance, sampler_key) == 52);
    assert!(offset_of!(GpuImageInstance, flags) == 56);
    assert!(offset_of!(GpuImageInstance, reserved) == 60);

    assert!(align_of::<GpuMaterialInstance>() == 16);
    assert!(size_of::<GpuMaterialInstance>() == 64);
    assert!(offset_of!(GpuMaterialInstance, rect) == 0);
    assert!(offset_of!(GpuMaterialInstance, params_spatial_clip) == 16);
    assert!(offset_of!(GpuMaterialInstance, opacity) == 32);
    assert!(offset_of!(GpuMaterialInstance, material_variant) == 36);
    assert!(offset_of!(GpuMaterialInstance, flags) == 40);
    assert!(offset_of!(GpuMaterialInstance, reserved) == 44);
    assert!(offset_of!(GpuMaterialInstance, resource_range_reserved) == 48);
};

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::{Pod, Zeroable};

    fn assert_pod<T: Pod + Zeroable>() {}

    #[test]
    fn every_gpu_record_is_pod_and_zeroable() {
        assert_pod::<GpuView>();
        assert_pod::<GpuSpatial>();
        assert_pod::<GpuClip>();
        assert_pod::<GpuBoxInstance>();
        assert_pod::<GpuShadowInstance>();
        assert_pod::<GpuGlyphInstance>();
        assert_pod::<GpuImageInstance>();
        assert_pod::<GpuMaterialInstance>();
    }
}
