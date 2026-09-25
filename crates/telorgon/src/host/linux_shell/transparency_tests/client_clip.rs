use super::*;

#[test]
fn server_frame_masks_application_corners() {
    for radius in [4.0, 10.0] {
        let (mut layers, clips, content) = rounded_frame(radius, 2.0);
        let mut image = ShellLayer::image(
            ShellLayerKey::Surface(9),
            ShellSceneKey::Surface(9),
            1,
            ShellImageUpdate::Full(
                [0_u8, 0, 255, 255]
                    .repeat((content.width * content.height) as usize)
                    .into(),
            ),
            SizeI {
                width: content.width,
                height: content.height,
            },
            content,
            None,
            ImageAlphaMode::Premultiplied,
            ImagePixelFormat::Rgba8,
            true,
        );
        image = image.with_content_clip(content, clips);
        assert_eq!(image.rounded_clips, clips);
        layers.push(image);
        let mut raster = Raster::new();
        raster.draw(layers);
        for x in [content.x, content.right() - 1] {
            assert_ne!(
                raster.pixel(x as usize, (content.bottom() - 1) as usize),
                &[0, 0, 255, 255]
            );
        }
        assert_eq!(
            raster.pixel(
                (content.x + content.width / 2) as usize,
                (content.y + content.height / 2) as usize
            ),
            &[0, 0, 255, 255]
        );
        assert_ne!(
            raster.pixel((content.x - 1) as usize, content.y as usize),
            &[0, 0, 255, 255]
        );
    }
}

#[test]
fn application_alpha_and_rectangular_bounds_are_preserved() {
    let mut image = client(ImageAlphaMode::Premultiplied, [0, 0, 0, 0], true);
    let bounds = RectI {
        x: CONTENT.x + 1,
        width: CONTENT.width - 2,
        ..CONTENT
    };
    image = image.with_content_clip(bounds, [None; 2]);
    assert_eq!(image.clip, Some(bounds));
    let mut raster = Raster::new();
    raster.draw(vec![image]);
    assert_eq!(
        raster.pixel(bounds.x as usize, bounds.y as usize),
        &[0, 255, 0, 255]
    );
}
