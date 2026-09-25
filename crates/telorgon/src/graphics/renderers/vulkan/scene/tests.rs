use super::*;
use crate::foundation::RectF;
use crate::ui::layout::{ClipId, SpatialId};
use crate::graphics::render::{BlendMode, PrimitiveKind};
fn draw(kind: PrimitiveKind, resource: u32) -> DrawItem {
    DrawItem {
        kind,
        index: resource,
        batch: BatchKey {
            pipeline: match kind {
                PrimitiveKind::Box => PipelineKind::AnalyticBox,
                PrimitiveKind::Glyph => PipelineKind::Glyph,
                PrimitiveKind::Image => PipelineKind::Image,
                PrimitiveKind::Material => PipelineKind::Material,
            },
            resource,
            clip: ClipId(0),
            blend: BlendMode::Alpha,
            target: 0,
        },
    }
}
#[test]
fn snapshot_sampling_crops_capacity_and_clamps_upscaled_edges() {
    let mut scene = VulkanScene::default();
    let instance = ImageInstance {
        node: crate::ui::UiNodeId::new(1, 1),
        image: ImageId(7),
        tint: None,
        rect: RectF {
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 180.0,
        },
        view_bounds: RectF::default(),
        content_version: 0,
        opacity: 0.5,
        clip: ClipId(0),
        spatial: SpatialId(0),
    };
    scene.images.push(instance);
    scene.gpu_images.push(convert_image(
        &instance,
        Some(ImageAlphaMode::Premultiplied),
    ));
    scene.crop_materialized_image(
        ImageId(7),
        SizeI {
            width: 101,
            height: 63,
        },
        SizeI {
            width: 128,
            height: 64,
        },
    );
    let uv = scene.gpu_images[0].uv_normalized;
    assert_eq!(scene.gpu_images[0].rect, [0.0, 0.0, 300.0, 180.0]);
    assert_eq!(uv, [0.0, 0.0, 101.0 / 128.0, 63.0 / 64.0]);
    // Mirror sampler coordinates at the first/last output pixel. The last sample
    // must clamp to the final active texel, never interpolate cleared padding.
    for output in [51, 101, 300] {
        for x in [0, output - 1] {
            let u = ((x as f32 + 0.5) / output as f32 * uv[2])
                .clamp(0.5 / 128.0, uv[2] - 0.5 / 128.0);
            let actual = u * 128.0 - 0.5;
            let expected = ((x as f32 + 0.5) / output as f32 * 101.0 - 0.5).clamp(0.0, 100.0);
            assert!((actual - expected).abs() < 0.00002);
        }
    }
    assert!(!scene.image_dirty.ranges.is_empty());
}

#[test]
fn glass_texture_slots_distinguish_sharp_sources_and_plain_images() {
    let mut scene = VulkanScene::default();
    for (id, sharp) in [(1, 8), (2, 9), (3, 8)] {
        scene.material_resources.insert(
            id,
            MaterialResource {
                material: crate::ui::MaterialId(id),
                content_version: 1,
                kind: MaterialKind::LiquidGlass(crate::graphics::render::LiquidGlassMaterial {
                    backdrop: ImageId(7),
                    sharp_backdrop: ImageId(sharp),
                    radii: [0.0; 4],
                    inverse_output_size: [0.01; 2],
                    inverse_bevel: 1.0,
                    blend_softness: 0.0,
                    refraction: 1.0,
                    dispersion: 0.0,
                    fresnel: 0.0,
                    tint: [0.0, 0.0, 0.0, 1.0],
                }),
                colors: [ColorRgba8::rgba(0, 0, 0, 0); 2],
            },
        );
    }
    scene.draw_order = vec![
        draw(PrimitiveKind::Image, 7),
        draw(PrimitiveKind::Material, 1),
        draw(PrimitiveKind::Material, 2),
        draw(PrimitiveKind::Material, 3),
    ];
    scene.rebuild_texture_slots();
    assert_eq!(scene.texture_slots.len(), 3);
    let batches = build_batches(&scene.draw_order);
    let slots: Vec<_> = batches
        .iter()
        .map(|b| scene.texture_slot(b).unwrap())
        .collect();
    assert_ne!(slots[0], slots[1]);
    assert_ne!(slots[1], slots[2]);
    assert_eq!(slots[1], slots[3]);
}

#[test]
fn glyph_upload_keeps_physical_atlas_texels_separate_from_logical_quad() {
    let instance = GlyphInstance {
        node: crate::graphics::scene::NodeId::new(0, 1),
        rect: RectF {
            x: 2.0,
            y: 3.0,
            width: 10.0,
            height: 12.0,
        },
        view_bounds: RectF::ZERO,
        atlas_x: 5,
        atlas_y: 7,
        atlas_size: crate::foundation::SizeI {
            width: 20,
            height: 24,
        },
        color: ColorRgba8::rgba(255, 255, 255, 255),
        opacity: 1.0,
        clip: ClipId(0),
        spatial: SpatialId(0),
    };
    let gpu = convert_glyph(&instance);
    assert_eq!(gpu.rect, [2.0, 3.0, 10.0, 12.0]);
    assert_eq!(gpu.uv_texels, [5.0, 7.0, 20.0, 24.0]);
}

#[test]
fn batching_preserves_mixed_order_and_merges_only_adjacent_compatible_items() {
    let batches = build_batches(&[
        draw(PrimitiveKind::Box, 1),
        draw(PrimitiveKind::Box, 1),
        draw(PrimitiveKind::Glyph, 0),
        draw(PrimitiveKind::Box, 1),
    ]);
    assert_eq!(batches.len(), 3);
    assert_eq!(batches[0].instance_count, 2);
    assert_eq!(batches[1].kind, PrimitiveKind::Glyph);
    assert_eq!(batches[2].first_instance, 3);
}
#[test]
fn mismatched_pipeline_or_secondary_target_is_rejected() {
    let mut invalid = draw(PrimitiveKind::Image, 0);
    invalid.batch.pipeline = PipelineKind::Glyph;
    assert!(validate_draw_order(&[invalid]).is_err());
    invalid = draw(PrimitiveKind::Box, 0);
    invalid.batch.target = 1;
    assert!(validate_draw_order(&[invalid]).is_err());
}

#[test]
fn image_tint_is_packed_with_the_alpha_mask_flag() {
    let tint = ColorRgba8::rgba(240, 241, 242, 192);
    let instance = ImageInstance {
        node: crate::ui::UiNodeId::new(0, 1),
        image: ImageId(7),
        tint: Some(tint),
        rect: Default::default(),
        view_bounds: Default::default(),
        content_version: 1,
        opacity: 1.0,
        clip: ClipId(0),
        spatial: SpatialId(0),
    };

    let packed = convert_image(&instance, Some(ImageAlphaMode::Premultiplied));

    assert_eq!(packed.tint_spatial_clip_texture[0], pack(tint));
    assert_eq!(packed.flags, 1 | (1 << 2));
}

#[test]
fn full_image_upload_detection_rejects_regional_chunks() {
    let extent = SizeI {
        width: 8,
        height: 4,
    };
    let full = ImageUploadChunk {
        offset: vk::Offset3D::default(),
        extent: vk::Extent3D {
            width: 8,
            height: 4,
            depth: 1,
        },
        row_bytes: 32,
        bytes: vec![0; 128].into(),
    };
    assert!(image_upload_is_full(&[full], extent, 4));

    let region = ImageUploadChunk {
        offset: vk::Offset3D { x: 2, y: 1, z: 0 },
        extent: vk::Extent3D {
            width: 3,
            height: 2,
            depth: 1,
        },
        row_bytes: 12,
        bytes: vec![0; 24].into(),
    };
    assert!(!image_upload_is_full(&[region], extent, 4));
}
