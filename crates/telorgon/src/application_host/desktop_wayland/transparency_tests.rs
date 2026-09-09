//! CPU framebuffer tests; no window, device, event loop, or presenting surface is opened.

use std::collections::BTreeMap;

#[cfg(not(target_os = "linux"))]
use super::desktop_wayland_scene_tests::*;
#[cfg(target_os = "linux")]
use super::scene::*;
use crate::core::{ColorRgba8, PointI, RectI, SizeF, SizeI};
use crate::render::{ImageAlphaMode, ImagePixelFormat, RenderBackend, RenderScene};
use crate::renderer_software::{
    SoftwareCompositeLayer, SoftwareRenderer, SoftwareScene, SoftwareSurface,
};

const EXTENT: SizeI = SizeI {
    width: 32,
    height: 24,
};
const CONTENT: RectI = RectI {
    x: 6,
    y: 8,
    width: 20,
    height: 12,
};

struct Raster {
    composition: DesktopComposition,
    scenes: BTreeMap<DesktopSceneKey, SoftwareScene>,
    surface: SoftwareSurface,
}

impl Raster {
    fn new() -> Self {
        Self {
            composition: DesktopComposition::new(EXTENT),
            scenes: BTreeMap::new(),
            surface: SoftwareSurface::default(),
        }
    }

    fn draw(&mut self, layers: Vec<DesktopLayer>) -> DesktopFrame {
        let frame = self.composition.synchronize(EXTENT, layers).unwrap();
        for update in &frame.updates {
            let scene = self
                .scenes
                .entry(update.key)
                .or_insert_with(|| SoftwareRenderer.create_scene().unwrap());
            for delta in &update.deltas {
                SoftwareRenderer.apply_scene_delta(scene, delta).unwrap();
            }
        }
        self.scenes.retain(|key, _| frame.live_scenes.contains(key));
        let layers = frame
            .placements
            .iter()
            .map(|placement| SoftwareCompositeLayer {
                scene: &self.scenes[&placement.scene],
                target: placement.target,
                clip: placement.clip,
                rounded_clips: placement.rounded_clips,
            })
            .collect::<Vec<_>>();
        SoftwareRenderer
            .render_composite(
                &mut self.surface,
                &layers,
                EXTENT,
                frame.damage,
                ColorRgba8::rgba(0, 255, 0, 255),
            )
            .unwrap();
        frame
    }

    fn pixel(&self, x: usize, y: usize) -> &[u8] {
        let index = (y * EXTENT.width as usize + x) * 4;
        &self.surface.pixels_rgba8()[index..index + 4]
    }
}

fn frame_layers(color: ColorRgba8) -> Vec<DesktopLayer> {
    let mut scene = RenderScene::default();
    scene.extent = SizeF {
        width: 28.0,
        height: 22.0,
    };
    scene.background = color;
    DesktopLayer::retained_frame(
        9,
        vec![scene.take_delta().unwrap()],
        SizeI {
            width: 28,
            height: 22,
        },
        PointI { x: 2, y: 1 },
        true,
        Some(CONTENT),
    )
}

#[test]
fn rectangular_frame_backing_cannot_leak_outside_round_corners() {
    let rect = crate::core::RectF {
        x: 0.0,
        y: 0.0,
        width: 28.0,
        height: 22.0,
    };
    let mut border = crate::render::BoxInstance {
        node: crate::scene::NodeId::new(0, 1),
        rect,
        view_bounds: rect,
        background: None,
        border: Default::default(),
        outline: Default::default(),
        corner_radii: crate::ui::CornerRadii::all(8.0),
        shadows: Default::default(),
        opacity: 1.0,
        clip: crate::render::ClipId(0),
        spatial: crate::render::SpatialId(0),
    };
    let mut raster = Raster::new();
    // Repeat rounded -> square -> rounded to exercise repaint of the old corners too.
    for radius in [8.0, 0.0, 8.0] {
        border.corner_radii = crate::ui::CornerRadii::all(radius);
        raster.draw(
            frame_layers(ColorRgba8::rgba(40, 60, 100, 255))
                .into_iter()
                .map(|layer| layer.with_frame_outline(&border, PointI { x: 2, y: 1 }))
                .collect(),
        );
        for (x, y) in [(2, 1), (29, 1), (2, 22), (29, 22)] {
            assert_eq!(
                raster.pixel(x, y),
                if radius > 0.0 {
                    &[0, 255, 0, 255]
                } else {
                    &[40, 60, 100, 255]
                }
            );
        }
        assert_eq!(raster.pixel(16, 3), &[40, 60, 100, 255]);
    }
}

fn client(alpha_mode: ImageAlphaMode, pixel: [u8; 4], visible: bool) -> DesktopLayer {
    DesktopLayer::image(
        DesktopLayerKey::Surface(9),
        DesktopSceneKey::Surface(9),
        1,
        DesktopImageUpdate::Full(
            pixel
                .repeat((CONTENT.width * CONTENT.height) as usize)
                .into(),
        ),
        SizeI {
            width: CONTENT.width,
            height: CONTENT.height,
        },
        CONTENT,
        Some(CONTENT),
        alpha_mode,
        ImagePixelFormat::Rgba8,
        visible,
    )
}

#[test]
fn client_alpha_sees_lower_layers_without_a_frame_backing() {
    for (mode, pixel, expected) in [
        (
            ImageAlphaMode::Premultiplied,
            [0, 0, 0, 0],
            [0, 255, 0, 255],
        ),
        (
            ImageAlphaMode::Premultiplied,
            [0, 0, 0, 128],
            [0, 187, 0, 255],
        ),
        (
            ImageAlphaMode::Straight,
            [0, 0, 255, 128],
            [0, 187, 188, 255],
        ),
        (ImageAlphaMode::Opaque, [0, 0, 255, 0], [0, 0, 255, 255]),
    ] {
        let mut raster = Raster::new();
        let mut layers = frame_layers(ColorRgba8::rgba(255, 0, 0, 255));
        layers.push(DesktopLayer::solid(
            DesktopLayerKey::ContentBackground(9),
            DesktopSceneKey::ContentBackground(9),
            ColorRgba8::rgba(0, 0, 0, 0),
            CONTENT,
        ));
        layers.push(client(mode, pixel, true));
        raster.draw(layers);
        assert_eq!(raster.pixel(12, 12), expected, "{mode:?}");
        assert_eq!(
            raster.pixel(12, 3),
            [255, 0, 0, 255],
            "title bar is unchanged"
        );
    }
}

#[test]
fn transparent_preview_reveals_desktop_not_retained_client_or_normal_backing() {
    for alpha in [0, 128, 255] {
        let mut raster = Raster::new();
        let mut layers = frame_layers(ColorRgba8::rgba(255, 0, 0, 255));
        layers.push(DesktopLayer::solid(
            DesktopLayerKey::ContentBackground(9),
            DesktopSceneKey::ContentBackground(9),
            ColorRgba8::rgba(0, 0, 0, 255),
            CONTENT,
        ));
        layers.push(client(ImageAlphaMode::Opaque, [0, 0, 255, 255], true));
        raster.draw(layers);
        assert_eq!(raster.pixel(12, 12), [0, 0, 255, 255]);

        let mut layers = DesktopLayer::retained_frame(
            9,
            Vec::new(),
            SizeI {
                width: 28,
                height: 22,
            },
            PointI { x: 2, y: 1 },
            true,
            Some(CONTENT),
        );
        layers.push(DesktopLayer::solid(
            DesktopLayerKey::ResizeVeil(9),
            DesktopSceneKey::ResizeVeil(9),
            ColorRgba8::rgba(255, 0, 0, alpha),
            CONTENT,
        ));
        let mut hidden = client(ImageAlphaMode::Opaque, [0, 0, 255, 255], false);
        if let DesktopLayerContent::Image { update, .. } = &mut hidden.content {
            *update = DesktopImageUpdate::Unchanged;
        }
        layers.push(hidden);
        let frame = raster.draw(layers);
        // Source-over is evaluated in linear light, then encoded back to sRGB bytes.
        let expected = match alpha {
            0 => [0, 255, 0, 255],
            128 => [188, 187, 0, 255],
            255 => [255, 0, 0, 255],
            _ => unreachable!(),
        };
        assert_eq!(raster.pixel(12, 12), expected);
        assert_eq!(raster.pixel(12, 3), [255, 0, 0, 255]);
        assert!(frame.surface_revisions.is_empty());
        assert!(
            frame
                .updates
                .iter()
                .flat_map(|update| &update.deltas)
                .all(|delta| delta.image_resources.is_empty())
        );
    }
}

#[test]
fn translucent_frame_pieces_blend_exactly_once() {
    let mut raster = Raster::new();
    let frame = raster.draw(frame_layers(ColorRgba8::rgba(255, 0, 0, 128)));
    assert_eq!(
        frame.updates.len(),
        1,
        "one scene shared across four scissors"
    );
    for y in 0..EXTENT.height as usize {
        for x in 0..EXTENT.width as usize {
            let outside = !(2..30).contains(&x) || !(1..23).contains(&y);
            let content = (6..26).contains(&x) && (8..20).contains(&y);
            let expected = if outside || content {
                [0, 255, 0, 255]
            } else {
                [188, 187, 0, 255]
            };
            assert_eq!(raster.pixel(x, y), expected, "pixel {x},{y}");
        }
    }
}

#[test]
fn rounded_backing_and_resize_geometry_render_at_native_extent() {
    let mut raster = Raster::new();
    let backing = |rect, radius, alpha| {
        DesktopLayer::rounded_solid(
            DesktopLayerKey::ContentBackground(9),
            DesktopSceneKey::ContentBackground(9),
            ColorRgba8::rgba(255, 0, 0, alpha),
            rect,
            radius,
        )
    };
    raster.draw(vec![backing(CONTENT, 5.0, 255)]);
    assert_eq!(raster.pixel(6, 8), [0, 255, 0, 255]);
    assert_eq!(raster.pixel(12, 12), [255, 0, 0, 255]);
    raster.draw(vec![backing(CONTENT, 0.0, 128)]);
    assert_eq!(raster.pixel(6, 8), [188, 187, 0, 255]);
    raster.draw(vec![backing(CONTENT, 5.0, 128)]);
    assert_eq!(raster.pixel(6, 8), [0, 255, 0, 255]);
    assert_eq!(raster.pixel(12, 12), [188, 187, 0, 255]);
    let smaller = RectI {
        width: 12,
        height: 8,
        ..CONTENT
    };
    raster.draw(vec![backing(smaller, 0.0, 255)]);
    assert_eq!(raster.pixel(12, 12), [255, 0, 0, 255]);
    assert_eq!(
        raster.pixel(24, 18),
        [0, 255, 0, 255],
        "old coverage is repainted"
    );
}

fn rounded_frame(
    radius: f32,
    width: f32,
) -> (
    Vec<DesktopLayer>,
    [Option<crate::render::RoundedClip>; 2],
    RectI,
) {
    rounded_frame_with_aperture(radius, width, width as i32, 0.0, 255)
}

fn rounded_frame_with_aperture(
    radius: f32,
    width: f32,
    margin: i32,
    content_radius: f32,
    frame_alpha: u8,
) -> (
    Vec<DesktopLayer>,
    [Option<crate::render::RoundedClip>; 2],
    RectI,
) {
    use crate::render::{
        BatchKey, BlendMode, Border, BoxInstance, ClipId, DrawItem, PipelineKind, PrimitiveKind,
        SpatialId,
    };
    let extent = SizeI {
        width: 28,
        height: 22,
    };
    let position = PointI { x: 2, y: 1 };
    let content = RectI {
        x: 2 + margin,
        y: 8,
        width: 28 - margin * 2,
        height: 15 - margin,
    };
    let rect = crate::core::RectF {
        x: 0.0,
        y: 0.0,
        width: 28.0,
        height: 22.0,
    };
    let border = BoxInstance {
        node: crate::scene::NodeId::new(0, 1),
        rect,
        view_bounds: rect,
        background: Some(ColorRgba8::rgba(60, 60, 60, frame_alpha)),
        border: Border::all(width, ColorRgba8::rgba(255, 0, 0, 255)),
        outline: Default::default(),
        corner_radii: crate::ui::CornerRadii::all(radius),
        shadows: Default::default(),
        opacity: 1.0,
        clip: ClipId(0),
        spatial: SpatialId(0),
    };
    let mut source = RenderScene::default();
    source.extent = SizeF {
        width: 28.0,
        height: 22.0,
    };
    source.background = ColorRgba8::rgba(0, 0, 0, 0);
    source.boxes.upsert(border.node, border.clone());
    source.set_draw_order(vec![DrawItem {
        kind: PrimitiveKind::Box,
        index: 0,
        batch: BatchKey {
            pipeline: PipelineKind::AnalyticBox,
            resource: 0,
            clip: ClipId(0),
            blend: BlendMode::Alpha,
            target: 0,
        },
    }]);
    let mut layers = DesktopLayer::retained_frame(
        9,
        vec![source.take_delta().unwrap()],
        extent,
        position,
        true,
        Some(content),
    );
    let clips = frame_content_clips(&border, position, content, content_radius);
    layers.extend(DesktopLayer::content_corners(
        9,
        border.clone(),
        extent,
        position,
        content,
        clips,
    ));
    layers.push(DesktopLayer::content_border(
        9, border, extent, position, content,
    ));
    (layers, clips, content)
}

#[test]
fn border_only_aperture_has_no_extra_clip_or_frame_fill_backing() {
    for border_width in [0.0, 1.0, 4.0, 6.0] {
        let (layers, clips, _) = rounded_frame(8.0, border_width);
        assert!(clips[1].is_none());
        assert!(
            layers
                .iter()
                .all(|layer| layer.key != DesktopLayerKey::ContentCorners(9))
        );
        assert_eq!(
            clips[0].unwrap().radii.bottom_left,
            (8.0 - border_width).max(0.0)
        );
    }
}

#[test]
fn title_bar_height_does_not_relocate_or_shrink_the_window_contours() {
    let (layers, clips, content) = rounded_frame_with_aperture(8.0, 1.0, 4, 5.0, 255);
    let border = layers
        .iter()
        .find(|layer| layer.key == DesktopLayerKey::ContentBorder(9))
        .unwrap();
    let DesktopLayerContent::Decoration { instance, .. } = &border.content else {
        unreachable!()
    };
    let short_content = RectI {
        y: content.y + 8,
        height: content.height - 8,
        ..content
    };
    assert_eq!(
        frame_content_clips(instance, PointI { x: 2, y: 1 }, short_content, 5.0),
        clips
    );
    assert_eq!(clips[1].unwrap().rect.y, 2.0);
    assert_eq!(clips[1].unwrap().radii.bottom_left, 5.0);
}

#[test]
fn wide_frame_margins_fill_bottom_corners_without_rounding_the_app_top() {
    let (mut layers, clips, content) = rounded_frame_with_aperture(8.0, 1.0, 4, 5.0, 255);
    layers.push(
        DesktopLayer::image(
            DesktopLayerKey::Surface(9),
            DesktopSceneKey::Surface(9),
            1,
            DesktopImageUpdate::Full(
                [0, 0, 255, 0]
                    .repeat((content.width * content.height) as usize)
                    .into(),
            ),
            SizeI {
                width: content.width,
                height: content.height,
            },
            content,
            None,
            ImageAlphaMode::Opaque,
            ImagePixelFormat::Rgba8,
            true,
        )
        .with_content_clip(content, clips),
    );
    let mut raster = Raster::new();
    raster.draw(layers);
    for x in [6, 25] {
        assert_eq!(
            raster.pixel(x, 8),
            [0, 0, 255, 255],
            "app top seam must stay square"
        );
        assert_eq!(
            raster.pixel(x, 18),
            [60, 60, 60, 255],
            "bottom aperture wedge belongs to frame fill"
        );
    }
    for x in [2, 29] {
        assert_eq!(
            raster.pixel(x, 1),
            [0, 255, 0, 255],
            "outer window top must be rounded"
        );
        assert_eq!(
            raster.pixel(x, 22),
            [0, 255, 0, 255],
            "outer window bottom must be rounded"
        );
    }
    assert_eq!(
        raster.pixel(16, 3),
        [60, 60, 60, 255],
        "title bar remains filled"
    );
}

#[test]
fn aperture_corner_fill_preserves_preview_and_client_transparency() {
    for preview in [false, true] {
        for alpha in [0, 128, 255] {
            let (mut layers, clips, content) = rounded_frame_with_aperture(8.0, 1.0, 4, 5.0, 255);
            let layer = if preview {
                DesktopLayer::solid(
                    DesktopLayerKey::ResizeVeil(9),
                    DesktopSceneKey::ResizeVeil(9),
                    ColorRgba8::rgba(0, 0, 255, alpha),
                    content,
                )
            } else {
                DesktopLayer::image(
                    DesktopLayerKey::Surface(9),
                    DesktopSceneKey::Surface(9),
                    1,
                    DesktopImageUpdate::Full(
                        [0, 0, 255, alpha]
                            .repeat((content.width * content.height) as usize)
                            .into(),
                    ),
                    SizeI {
                        width: content.width,
                        height: content.height,
                    },
                    content,
                    None,
                    ImageAlphaMode::Straight,
                    ImagePixelFormat::Rgba8,
                    true,
                )
            };
            layers.push(layer.with_content_clip(content, clips));
            let mut raster = Raster::new();
            raster.draw(layers);
            for x in [6, 25] {
                assert_eq!(raster.pixel(x, 18), [60, 60, 60, 255]);
            }
            let expected = match alpha {
                0 => [0, 255, 0, 255],
                128 => [0, 187, 188, 255],
                _ => [0, 0, 255, 255],
            };
            assert_eq!(
                raster.pixel(16, 14),
                expected,
                "frame must not back the content interior"
            );
        }
    }
}

#[test]
fn translucent_frame_corner_matches_the_uncut_frame_fill() {
    let (layers, _, _) = rounded_frame_with_aperture(8.0, 1.0, 4, 5.0, 128);
    let mut raster = Raster::new();
    raster.draw(layers);
    // Both are fully covered fill pixels. A corner blended twice would be darker/less green.
    assert_eq!(raster.pixel(6, 18), raster.pixel(16, 3));
    assert_eq!(raster.pixel(25, 18), raster.pixel(16, 3));
    assert_eq!(raster.pixel(16, 14), [0, 255, 0, 255]);
}

#[test]
fn rounded_border_keeps_its_inner_rim_and_clips_opaque_client_corners() {
    for (radius, width) in [(8.0, 2.0), (8.0, 0.0), (0.0, 2.0), (200.0, 2.0), (3.0, 6.0)] {
        let (mut layers, clips, content) = rounded_frame(radius, width);
        layers.push(
            DesktopLayer::image(
                DesktopLayerKey::Surface(9),
                DesktopSceneKey::Surface(9),
                1,
                DesktopImageUpdate::Full(
                    [0, 0, 255, 0]
                        .repeat((content.width * content.height) as usize)
                        .into(),
                ),
                SizeI {
                    width: content.width,
                    height: content.height,
                },
                content,
                None,
                ImageAlphaMode::Opaque,
                ImagePixelFormat::Rgba8,
                true,
            )
            .with_content_clip(content, clips),
        );
        let mut raster = Raster::new();
        raster.draw(layers);
        for y in content.y..content.bottom() {
            for x in content.x..content.right() {
                let point = crate::core::PointF {
                    x: x as f32 + 0.5,
                    y: y as f32 + 0.5,
                };
                let coverage = clips
                    .iter()
                    .flatten()
                    .fold(1.0_f32, |a, clip| a.min(clip.coverage(point)));
                let pixel = raster.pixel(x as usize, y as usize);
                if coverage == 0.0 {
                    assert_eq!(
                        pixel[2], 0,
                        "client escaped: r={radius} b={width} at {x},{y}"
                    );
                }
                if coverage == 1.0 {
                    assert_eq!(pixel, [0, 0, 255, 255]);
                }
            }
        }
        if radius == 8.0 && width == 2.0 {
            assert_eq!(
                raster.pixel(4, 19),
                [255, 0, 0, 255],
                "inner curved rim was cut away"
            );
            assert_eq!(
                raster.pixel(27, 19),
                [255, 0, 0, 255],
                "right curved rim was cut away"
            );
            assert_eq!(
                raster.pixel(2, 22),
                [0, 255, 0, 255],
                "outside corner is not clipped"
            );
            assert_eq!(raster.pixel(29, 22), [0, 255, 0, 255]);
        }
    }
}

#[test]
fn rounded_preview_preserves_transparency_and_the_curved_rim() {
    for alpha in [0, 128, 255] {
        let (mut layers, clips, content) = rounded_frame(8.0, 2.0);
        layers.push(
            DesktopLayer::solid(
                DesktopLayerKey::ResizeVeil(9),
                DesktopSceneKey::ResizeVeil(9),
                ColorRgba8::rgba(0, 0, 255, alpha),
                content,
            )
            .with_content_clip(content, clips),
        );
        let mut raster = Raster::new();
        raster.draw(layers);
        assert_eq!(raster.pixel(4, 19), [255, 0, 0, 255]);
        assert_eq!(raster.pixel(2, 22), [0, 255, 0, 255]);
        let expected = match alpha {
            0 => [0, 255, 0, 255],
            128 => [0, 187, 188, 255],
            _ => [0, 0, 255, 255],
        };
        assert_eq!(raster.pixel(16, 16), expected);
    }
}

#[test]
fn rounded_clip_changes_repaint_without_touching_client_pixels() {
    use crate::render::RoundedClip;
    let extent = SizeI {
        width: CONTENT.width,
        height: CONTENT.height,
    };
    let clip = |radius| {
        Some(RoundedClip::new(
            crate::core::RectF {
                x: CONTENT.x as f32,
                y: CONTENT.y as f32,
                width: CONTENT.width as f32,
                height: CONTENT.height as f32,
            },
            crate::ui::CornerRadii::all(radius),
        ))
    };
    let layer = |update, radius| {
        DesktopLayer::image(
            DesktopLayerKey::Surface(9),
            DesktopSceneKey::Surface(9),
            1,
            update,
            extent,
            CONTENT,
            None,
            ImageAlphaMode::Opaque,
            ImagePixelFormat::Rgba8,
            true,
        )
        .with_content_clip(CONTENT, [clip(radius), None])
    };
    let mut raster = Raster::new();
    raster.draw(vec![layer(
        DesktopImageUpdate::Full([0, 0, 255, 255].repeat(240).into()),
        0.0,
    )]);
    assert_eq!(raster.pixel(6, 8), [0, 0, 255, 255]);
    let changed = raster.draw(vec![layer(DesktopImageUpdate::Unchanged, 5.0)]);
    assert!(changed.updates.is_empty());
    assert_eq!(raster.pixel(6, 8), [0, 255, 0, 255]);
    let changed = raster.draw(vec![layer(DesktopImageUpdate::Unchanged, 0.0)]);
    assert!(changed.updates.is_empty());
    assert_eq!(raster.pixel(6, 8), [0, 0, 255, 255]);
    let mut inverse = layer(DesktopImageUpdate::Unchanged, 5.0);
    inverse.rounded_clips[0] = inverse.rounded_clips[0].map(RoundedClip::inverse);
    raster.draw(vec![inverse]);
    assert_eq!(raster.pixel(6, 8), [0, 0, 255, 255]);
    let changed = raster.draw(vec![layer(DesktopImageUpdate::Unchanged, 5.0)]);
    assert!(
        changed.updates.is_empty(),
        "inverse-only clip changes must not reupload images"
    );
    assert_eq!(raster.pixel(6, 8), [0, 255, 0, 255]);
}

#[test]
fn replaced_frame_nodes_update_the_same_border_scene_slot() {
    let (layers, _, _) = rounded_frame(8.0, 2.0);
    let border = layers
        .into_iter()
        .find(|l| l.key == DesktopLayerKey::ContentBorder(9))
        .unwrap();
    let DesktopLayerContent::Decoration { mut instance, .. } = border.content else {
        unreachable!()
    };
    let mut raster = Raster::new();
    raster.draw(vec![DesktopLayer::content_border(
        9,
        instance.clone(),
        border.source_extent,
        PointI { x: 2, y: 1 },
        border.clip.unwrap(),
    )]);
    assert_eq!(raster.pixel(4, 19), [255, 0, 0, 255]);
    instance.node = crate::scene::NodeId::new(99, 5);
    instance.border = crate::render::Border::all(2.0, ColorRgba8::rgba(0, 0, 255, 255));
    let frame = raster.draw(vec![DesktopLayer::content_border(
        9,
        instance,
        border.source_extent,
        PointI { x: 2, y: 1 },
        border.clip.unwrap(),
    )]);
    assert_eq!(raster.pixel(4, 19), [0, 0, 255, 255]);
    assert_eq!(
        frame.updates[0].deltas[0].box_len, 2,
        "one border plus scene clear slot"
    );
}
