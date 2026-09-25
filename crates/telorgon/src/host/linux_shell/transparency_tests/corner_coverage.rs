use super::*;

#[test]
fn exterior_shadow_does_not_leave_a_bright_corner_fringe() {
    for background in [
        ColorRgba8::rgba(0, 0, 0, 255),
        ColorRgba8::rgba(255, 255, 255, 255),
    ] {
        let (layers, _, _) = rounded_frame(8.0, 1.5);
        let mut border = layers
            .into_iter()
            .find_map(|layer| match layer.content {
                ShellLayerContent::Decoration { instance, .. }
                    if layer.key == ShellLayerKey::ContentBorder(9) =>
                {
                    Some(instance)
                }
                _ => None,
            })
            .unwrap();
        border.background = Some(ColorRgba8::rgba(30, 30, 30, 255));
        border.shadows = crate::ui::ShadowList::one(crate::ui::Shadow {
            offset: crate::foundation::PointF { x: 0.0, y: 4.0 },
            blur: 12.0,
            spread: 2.0,
            color: ColorRgba8::rgba(0, 0, 0, 210),
        });
        let position = PointI { x: 2, y: 1 };
        let draw = |clipped: bool| {
            let mut shadow = ShellLayer::frame_shadow(9, border.clone(), position).unwrap();
            if !clipped {
                shadow.rounded_clips = [None; 2];
            }
            let mut raster = Raster::new();
            raster.draw(vec![
                ShellLayer::solid(
                    ShellLayerKey::Background,
                    ShellSceneKey::Background,
                    background,
                    RectI {
                        x: 0,
                        y: 0,
                        width: 32,
                        height: 24,
                    },
                ),
                shadow,
                ShellLayer::content_border(
                    9,
                    border.clone(),
                    SizeI {
                        width: 28,
                        height: 22,
                    },
                    position,
                    RectI {
                        x: 2,
                        y: 1,
                        width: 28,
                        height: 22,
                    },
                    border.background,
                ),
            ]);
            raster
        };
        let actual = draw(true);
        let reference = draw(false);
        for y in 0..24 {
            for x in 0..32 {
                assert!(
                    actual
                        .pixel(x, y)
                        .iter()
                        .zip(reference.pixel(x, y))
                        .all(|(a, b)| a.abs_diff(*b) <= 2),
                    "shadow fringe at {x},{y} over {background:?}: {:?} != {:?}",
                    actual.pixel(x, y),
                    reference.pixel(x, y)
                );
            }
        }
    }
}

#[test]
fn split_frame_corners_match_unbroken_opaque_frame() {
    for background in [
        ColorRgba8::rgba(0, 0, 0, 255),
        ColorRgba8::rgba(255, 255, 255, 255),
    ] {
        for width in [1.0_f32, 1.5, 2.0, 2.5, 3.0] {
            let (layers, _, content) =
                rounded_frame_with_aperture(8.0, width, width.round() as i32, 0.0, 255);
            let border = layers
                .iter()
                .find_map(|layer| match &layer.content {
                    ShellLayerContent::Decoration { instance, .. }
                        if layer.key == ShellLayerKey::ContentBorder(9) =>
                    {
                        Some(instance.clone())
                    }
                    _ => None,
                })
                .unwrap();
            let mut border = border;
            border.background = Some(ColorRgba8::rgba(60, 60, 60, 255));
            let layers = layers
                .into_iter()
                .map(|layer| {
                    if layer.key == ShellLayerKey::ContentBorder(9) {
                        ShellLayer::content_border(
                            9,
                            border.clone(),
                            layer.source_extent,
                            PointI { x: 2, y: 1 },
                            content,
                            border.background,
                        )
                    } else {
                        layer.with_frame_outline(&border, PointI { x: 2, y: 1 })
                    }
                })
                .collect();
            let mut split = Raster::new();
            let backdrop = || {
                ShellLayer::solid(
                    ShellLayerKey::Background,
                    ShellSceneKey::Background,
                    background,
                    RectI {
                        x: 0,
                        y: 0,
                        width: 32,
                        height: 24,
                    },
                )
            };
            let mut layers: Vec<_> = layers;
            layers.insert(0, backdrop());
            split.draw(layers);
            let mut reference = Raster::new();
            reference.draw(vec![
                backdrop(),
                ShellLayer::content_border(
                    9,
                    border.clone(),
                    SizeI {
                        width: 28,
                        height: 22,
                    },
                    PointI { x: 2, y: 1 },
                    RectI {
                        x: 2,
                        y: 1,
                        width: 28,
                        height: 22,
                    },
                    border.background,
                ),
            ]);
            for y in 0..24 {
                for x in 0..32 {
                    let actual = split.pixel(x, y);
                    let expected = reference.pixel(x, y);
                    assert!(
                        actual
                            .iter()
                            .zip(expected)
                            .all(|(a, b)| a.abs_diff(*b) <= 2),
                        "{x},{y} width={width}: {actual:?} != {expected:?}"
                    );
                }
            }
        }
    }
}
