use super::*;

#[test]
fn reused_source_scene_accepts_each_publication_and_preserves_stale_delta_guard() {
    use crate::graphics::render::{ImageColorEncoding, ImageResourceDelta, ImageResourceUpdate};
    let mut retained = VulkanScene::default();
    // CPU resource metadata stands in for the external binding; no Vulkan device or FD is
    // needed to exercise the real backend epoch gate, validation and retained scene updates.
    let physical = SizeI {
        width: 8,
        height: 8,
    };
    let mut last = None;
    for version in [1, 2, 7, 24] {
        let extent = SizeI {
            width: 8,
            height: if version == 1 { 8 } else { 4 },
        };
        let mut delta = dma_buf_source_delta(
            physical,
            extent,
            Affine2D::IDENTITY,
            version,
            ImageAlphaMode::Opaque,
        );
        delta
            .image_resources
            .push(ImageResourceDelta::Write(ImageResourceUpdate {
                image: dma_buf_image_id(),
                content_version: version,
                extent: physical,
                rect: full_rect(physical),
                row_bytes: 8 * 4,
                color_encoding: ImageColorEncoding::Srgb,
                alpha_mode: ImageAlphaMode::Opaque,
                pixel_format: ImagePixelFormat::Rgba8,
                pixels: vec![255; 8 * 8 * 4].into(),
            }));
        let stats = retained.apply_delta_checked(&delta).unwrap();
        assert_eq!(
            stats.epoch, version,
            "publication {version} must not be skipped"
        );
        assert_eq!(retained.images[0].content_version, version);
        assert_eq!(retained.images[0].view_bounds.height, extent.height as f32);
        last = Some(delta);
    }
    let mut stale = last.unwrap();
    stale.epoch = 2;
    stale.image_len = 0;
    stale.images.clear();
    let stats = retained.apply_delta_checked(&stale).unwrap();
    assert_eq!(stats.epoch, 24);
    assert_eq!(retained.images[0].content_version, 24);
}

#[test]
fn geometry_changes_invalidate_output_damage_even_at_the_same_extent() {
    let geometry = damage_geometry(1.0);
    let damage = Some(RectI {
        x: 1,
        y: 2,
        width: 3,
        height: 4,
    });
    assert_eq!(
        compatible_materialization_damage(Some(geometry), Some(1), geometry, 2, damage),
        damage
    );
    let mut transformed = geometry;
    transformed.transform = Affine2D::translation(1.0, 0.0);
    assert_eq!(
        compatible_materialization_damage(Some(geometry), Some(1), transformed, 2, damage),
        None
    );
    assert_eq!(
        compatible_materialization_damage(Some(geometry), Some(1), geometry, 3, damage),
        None
    );
    assert_eq!(
        compatible_materialization_damage(None, Some(1), geometry, 2, damage),
        None
    );
}

fn damage_geometry(scale: f32) -> MaterializationGeometry {
    MaterializationGeometry {
        physical: SizeI {
            width: 100,
            height: 100,
        },
        extent: SizeI {
            width: 100,
            height: 100,
        },
        raster: SizeI {
            width: (100.0 * scale) as i32,
            height: (100.0 * scale) as i32,
        },
        transform: Affine2D::IDENTITY,
        scale,
        alpha: ImageAlphaMode::Premultiplied,
    }
}

#[test]
fn materialization_damage_keeps_spaces_and_filter_coverage() {
    let mut geometry = damage_geometry(3.0);
    geometry.transform = Affine2D {
        m11: 0.5,
        m22: 0.5,
        ..Affine2D::IDENTITY
    };
    let rect = RectI {
        x: 20,
        y: 10,
        width: 10,
        height: 10,
    };
    assert_eq!(
        materialization_damage(&[rect], &[], geometry),
        Some(RectI {
            x: 57,
            y: 27,
            width: 36,
            height: 36
        })
    );
    assert_eq!(
        materialization_damage(&[], &[rect], geometry),
        Some(RectI {
            x: 28,
            y: 13,
            width: 19,
            height: 19
        })
    );
    let union = materialization_damage(&[rect], &[rect], geometry).unwrap();
    assert_eq!(
        union,
        RectI {
            x: 28,
            y: 13,
            width: 65,
            height: 50
        }
    );
}

#[test]
fn damage_rotation_clips_and_unknown_history_forces_full() {
    let mut geometry = damage_geometry(1.0);
    geometry.transform = Affine2D {
        m11: 0.0,
        m12: 1.0,
        m21: -1.0,
        m22: 0.0,
        tx: 100.0,
        ty: 0.0,
    };
    let damage = materialization_damage(
        &[],
        &[RectI {
            x: 0,
            y: 0,
            width: 10,
            height: 20,
        }],
        geometry,
    );
    assert_eq!(
        damage,
        Some(RectI {
            x: 79,
            y: 0,
            width: 21,
            height: 11
        })
    );
    assert_eq!(materialization_damage(&[], &[], geometry), None);
    assert_eq!(
        materialization_damage(&[full_rect(geometry.raster)], &[], geometry),
        None
    );
    assert_eq!(merge_damage(damage, None), None);
    assert_eq!(merge_damage(None, damage), None);
}

#[test]
fn missing_snapshot_revisions_invalidate_partial_damage() {
    assert!(!damage_history_contiguous(None, 10));
    assert!(damage_history_contiguous(Some(10), 11));
    assert!(damage_history_contiguous(Some(11), 11));
    assert!(!damage_history_contiguous(Some(10), 12));
    assert!(!damage_history_contiguous(Some(12), 10));
    assert!(!damage_history_contiguous(Some(u64::MAX), 0));
}

#[test]
fn skipped_publications_accumulate_damage_until_materialized() {
    let a = RectI {
        x: 5,
        y: 5,
        width: 10,
        height: 10,
    };
    let b = RectI {
        x: 40,
        y: 30,
        width: 5,
        height: 5,
    };
    let c = RectI {
        x: 10,
        y: 20,
        width: 2,
        height: 2,
    };
    let merged = merge_damage(merge_damage(Some(a), Some(b)), Some(c));
    assert_eq!(
        merged,
        Some(RectI {
            x: 5,
            y: 5,
            width: 40,
            height: 30
        })
    );
    let mut changed = damage_geometry(1.0);
    changed.alpha = ImageAlphaMode::Opaque;
    assert_ne!(
        Some(changed),
        Some(damage_geometry(1.0)),
        "alpha changes invalidate retained content history"
    );
}

fn assert_rect_close(actual: RectF, expected: RectF) {
    for (actual, expected) in [
        (actual.x, expected.x),
        (actual.y, expected.y),
        (actual.width, expected.width),
        (actual.height, expected.height),
    ] {
        assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
    }
}

#[test]
fn every_wayland_buffer_transform_maps_into_its_logical_extent() {
    let physical = SizeI {
        width: 120,
        height: 80,
    };
    for transform in [
        BufferTransform::Normal,
        BufferTransform::Rotate90,
        BufferTransform::Rotate180,
        BufferTransform::Rotate270,
        BufferTransform::Flipped,
        BufferTransform::Flipped90,
        BufferTransform::Flipped180,
        BufferTransform::Flipped270,
    ] {
        let (extent, mapping) =
            dma_buf_surface_mapping(physical, 2, transform, None, false).unwrap();
        let expected = if matches!(
            transform,
            BufferTransform::Rotate90
                | BufferTransform::Rotate270
                | BufferTransform::Flipped90
                | BufferTransform::Flipped270
        ) {
            SizeI {
                width: 40,
                height: 60,
            }
        } else {
            SizeI {
                width: 60,
                height: 40,
            }
        };
        assert_eq!(extent, expected);
        assert_rect_close(
            mapping.transform_rect(RectF {
                x: 0.0,
                y: 0.0,
                width: physical.width as f32,
                height: physical.height as f32,
            }),
            RectF {
                x: 0.0,
                y: 0.0,
                width: expected.width as f32,
                height: expected.height as f32,
            },
        );
    }
}

#[test]
fn dma_buf_viewport_crop_maps_exactly_to_its_destination() {
    let (extent, mapping) = dma_buf_surface_mapping(
        SizeI {
            width: 200,
            height: 100,
        },
        1,
        BufferTransform::Normal,
        Some(ViewportState {
            source: Some(ViewportSource {
                x: 50.0,
                y: 20.0,
                width: 100.0,
                height: 50.0,
            }),
            destination: Some(SizeI {
                width: 400,
                height: 200,
            }),
        }),
        false,
    )
    .unwrap();

    assert_eq!(
        extent,
        SizeI {
            width: 400,
            height: 200
        }
    );
    assert_rect_close(
        mapping.transform_rect(RectF {
            x: 50.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        }),
        RectF {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 200.0,
        },
    );
}

#[test]
fn dma_buf_y_invert_is_normalized_before_surface_transform() {
    let physical = SizeI {
        width: 120,
        height: 80,
    };
    let (_, normal) =
        dma_buf_surface_mapping(physical, 2, BufferTransform::Normal, None, false).unwrap();
    let (_, inverted) =
        dma_buf_surface_mapping(physical, 2, BufferTransform::Normal, None, true).unwrap();
    let point = crate::foundation::PointF { x: 10.0, y: 6.0 };

    assert_eq!(
        normal.transform_point(point),
        crate::foundation::PointF { x: 5.0, y: 3.0 }
    );
    assert_eq!(
        inverted.transform_point(point),
        crate::foundation::PointF { x: 5.0, y: 37.0 }
    );
}
