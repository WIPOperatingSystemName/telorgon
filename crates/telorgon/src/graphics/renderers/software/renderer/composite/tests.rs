use super::*;
use crate::graphics::render::{ImageResource, frame_border};

#[test]
fn scaled_placements_sample_the_full_source_and_preserve_output_clip_and_damage() {
    let renderer = SoftwareRenderer;
    let size = SizeI { width: 8, height: 6 };
    let green = ColorRgba8::rgba(0, 255, 0, 255);
    let mut source = frame_border::image_scene(SizeF { width: 4.0, height: 2.0 });
    let row = [[0, 0, 255, 255], [0, 0, 255, 255], [255, 0, 0, 255], [255, 0, 0, 255]];
    source.set_image_resource(ImageResource {
        image: frame_border::IMAGE,
        content_version: 1,
        extent: SizeI { width: 4, height: 2 },
        color_encoding: ImageColorEncoding::Srgb,
        alpha_mode: ImageAlphaMode::Premultiplied,
        pixel_format: ImagePixelFormat::Rgba8,
        pixels: row.into_iter().flatten().cycle().take(32).collect::<Vec<_>>().into(),
    }).unwrap();
    let mut scene = renderer.create_scene().unwrap();
    renderer.apply_scene_delta(&mut scene, &source.take_delta().unwrap()).unwrap();
    let mut surface = SoftwareSurface::default();
    let pixel = |surface: &SoftwareSurface, x: usize, y: usize| {
        surface.pixels_rgba8()[(y * 8 + x) * 4..][..4].to_vec()
    };
    // Negative placement origins, a clip and damage each constrain independent output edges.
    renderer.render_composite(&mut surface, &[], size, None, green).unwrap();
    renderer.render_composite(&mut surface, &[SoftwareCompositeLayer {
        scene: &scene,
        target: RectI { x: -2, y: 1, width: 8, height: 4 },
        clip: Some(RectI { x: 1, y: 2, width: 5, height: 3 }),
        rounded_clips: [None; 2],
    }], size, Some(RectI { x: 0, y: 0, width: 8, height: 4 }), green).unwrap();
    assert_eq!(pixel(&surface, 5, 2), [255, 0, 0, 255]);
    for (x, y) in [(0, 2), (5, 1), (6, 2), (5, 4)] {
        assert_eq!(pixel(&surface, x, y), [0, 255, 0, 255]);
    }
    // Reuse the same retained scene at a smaller size without accumulating scale or clipping
    // away the right half of its source image.
    renderer.render_composite(&mut surface, &[SoftwareCompositeLayer {
        scene: &scene,
        target: RectI { x: 1, y: 1, width: 2, height: 1 },
        clip: None,
        rounded_clips: [None; 2],
    }], size, None, green).unwrap();
    assert_eq!(pixel(&surface, 1, 1), [0, 0, 255, 255]);
    assert_eq!(pixel(&surface, 2, 1), [255, 0, 0, 255]);
    assert_eq!(scene.extent, SizeF { width: 4.0, height: 2.0 });
}
