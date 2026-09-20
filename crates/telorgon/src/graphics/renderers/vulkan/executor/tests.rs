use super::*;
#[test]
fn growth_requires_every_new_slot_to_be_initialized() {
    let patches = [RangePatch {
        start: 2,
        values: Arc::from([1_u32]),
    }];
    assert!(validate_growth("test", 1, 3, &patches).is_err());
    let patches = [RangePatch {
        start: 1,
        values: Arc::from([1_u32, 2]),
    }];
    assert!(validate_growth("test", 1, 3, &patches).is_ok());
}

#[test]
fn preview_scissor_maps_logical_coordinates_into_a_different_target_extent() {
    let mapping = ViewMapping::new(
        SizeF {
            width: 800.0,
            height: 600.0,
        },
        RectI {
            x: 100,
            y: 50,
            width: 400,
            height: 900,
        },
    );
    let scissor = mapping.logical_rect_to_scissor(RectF {
        x: 200.0,
        y: 150.0,
        width: 400.0,
        height: 300.0,
    });
    assert_eq!(scissor.offset, vk::Offset2D { x: 200, y: 275 });
    assert_eq!(
        scissor.extent,
        vk::Extent2D {
            width: 200,
            height: 450
        }
    );
}

#[test]
fn preview_scissor_clamps_to_the_hosted_target_region() {
    let mapping = ViewMapping::new(
        SizeF {
            width: 100.0,
            height: 100.0,
        },
        RectI {
            x: 300,
            y: 200,
            width: 200,
            height: 100,
        },
    );
    let scissor = mapping.logical_rect_to_scissor(RectF {
        x: -10.0,
        y: 25.0,
        width: 120.0,
        height: 100.0,
    });
    assert_eq!(scissor.offset, vk::Offset2D { x: 300, y: 225 });
    assert_eq!(
        scissor.extent,
        vk::Extent2D {
            width: 200,
            height: 75
        }
    );
}

#[test]
fn damage_clips_the_full_viewport_without_remapping_the_scene() {
    let target_region = RectI {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };
    let damage = RectI {
        x: 320,
        y: 180,
        width: 800,
        height: 600,
    };
    let mapping = ViewMapping::new(
        SizeF {
            width: 1920.0,
            height: 1080.0,
        },
        target_region,
    );

    // The scene still maps one-to-one across the complete output. Damage only restricts
    // which pixels are touched; it must never become a window-sized viewport.
    assert_eq!(
        mapping.logical_rect_to_scissor(mapping.logical_bounds()),
        rect2d(target_region)
    );
    assert_eq!(
        intersect_scissor(rect2d(target_region), damage),
        rect2d(damage)
    );
    assert_eq!(
        mapping.logical_rect_to_scissor(RectF {
            x: 320.0,
            y: 180.0,
            width: 800.0,
            height: 600.0,
        }),
        rect2d(damage)
    );
}
