use super::*;
use crate::foundation::RectI;

#[test]
fn regional_image_updates_preserve_other_pixels_and_older_deltas() {
    let image = ImageId(7);
    let original: Arc<[u8]> =
        Arc::from([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    let mut scene = RenderScene::default();
    scene.extent = SizeF {
        width: 2.0,
        height: 2.0,
    };
    scene
        .set_image_resource(ImageResource {
            image,
            content_version: 1,
            extent: SizeI {
                width: 2,
                height: 2,
            },
            color_encoding: ImageColorEncoding::Srgb,
            alpha_mode: ImageAlphaMode::Straight,
            pixel_format: ImagePixelFormat::Rgba8,
            pixels: Arc::clone(&original),
        })
        .unwrap();
    let first = scene.take_delta().unwrap();

    scene
        .update_image_resource_region(ImageResourceUpdate {
            image,
            content_version: 2,
            extent: SizeI {
                width: 2,
                height: 2,
            },
            rect: RectI {
                x: 1,
                y: 0,
                width: 1,
                height: 1,
            },
            row_bytes: 4,
            color_encoding: ImageColorEncoding::Srgb,
            alpha_mode: ImageAlphaMode::Straight,
            pixel_format: ImagePixelFormat::Rgba8,
            pixels: Arc::from([21, 22, 23, 24]),
        })
        .unwrap();

    let ImageResourceDelta::Write(first_write) = &first.image_resources[0] else {
        panic!("initial image delta must be a write");
    };
    assert_eq!(first_write.pixels.as_ref(), original.as_ref());

    let snapshot = scene.snapshot_delta(SizeI::default(), Vec::new());
    let ImageResourceDelta::Write(snapshot_write) = &snapshot.image_resources[0] else {
        panic!("snapshot image delta must be a write");
    };
    assert_eq!(
        snapshot_write.pixels.as_ref(),
        &[1, 2, 3, 4, 21, 22, 23, 24, 9, 10, 11, 12, 13, 14, 15, 16]
    );
}
