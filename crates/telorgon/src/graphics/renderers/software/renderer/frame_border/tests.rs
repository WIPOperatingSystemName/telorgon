use super::*;
use crate::graphics::render::{BatchKey, ClipId, PipelineKind, RenderScene, RoundedClip};
use crate::graphics::scene::NodeId;
use crate::ui::CornerRadii;

mod partial;
mod performance;

#[test]
fn frame_border_survives_opaque_and_translucent_controls() {
    let extent = SizeI {
        width: 40,
        height: 30,
    };
    for radius in [0.0, 8.0] {
        for width in [0.0, 0.5, 1.5, 2.0, 3.0] {
            for button_alpha in [0, 128, 255] {
                for frame_alpha in [128, 255] {
                    let rect = RectF {
                        x: 0.0,
                        y: 0.0,
                        width: 40.0,
                        height: 30.0,
                    };
                    let border = BoxInstance {
                        node: NodeId::new(0, 1),
                        rect,
                        view_bounds: rect,
                        background: Some(ColorRgba8::rgba(0, 255, 0, frame_alpha)),
                        border: Border::all(width, ColorRgba8::rgba(255, 0, 0, frame_alpha)),
                        outline: Default::default(),
                        corner_radii: CornerRadii::all(radius),
                        shadows: Default::default(),
                        opacity: 1.0,
                        clip: ClipId(0),
                        spatial: SpatialId(0),
                    };
                    let inner = frame_border::inner(&border);
                    let mut scene = RenderScene::default();
                    scene.extent = SizeF {
                        width: 40.0,
                        height: 30.0,
                    };
                    scene.background = ColorRgba8::rgba(0, 0, 0, 0);
                    scene.boxes.upsert(border.node, border.clone());
                    let mut button = border.clone();
                    button.node = NodeId::new(1, 1);
                    button.rect = inner.rect;
                    button.view_bounds = inner.rect;
                    button.border = Border::default();
                    button.corner_radii = CornerRadii::default();
                    button.background = Some(ColorRgba8::rgba(0, 0, 255, button_alpha));
                    button.clip = ClipId(1);
                    scene.boxes.upsert(button.node, button);
                    scene.clips.upsert(
                        NodeId::new(2, 1),
                        RenderClip {
                            id: ClipId(1),
                            rect: inner.rect,
                            corner_radii: inner.radii,
                        },
                    );
                    scene.set_draw_order(
                        (0..2)
                            .map(|index| DrawItem {
                                kind: PrimitiveKind::Box,
                                index,
                                batch: BatchKey {
                                    pipeline: PipelineKind::AnalyticBox,
                                    resource: 0,
                                    clip: ClipId(index),
                                    blend: BlendMode::Alpha,
                                    target: 0,
                                },
                            })
                            .collect(),
                    );
                    let mut delta = scene.take_delta().unwrap();
                    frame_border::prepare_interior(&mut delta, &border);
                    let mut source = SoftwareRenderer.create_scene().unwrap();
                    SoftwareRenderer
                        .apply_scene_delta(&mut source, &delta)
                        .unwrap();
                    source.set_frame_border(Some(border.clone()));
                    let mut surface = SoftwareSurface::default();
                    let draw = |surface: &mut SoftwareSurface| {
                        SoftwareRenderer
                            .render_composite(
                                surface,
                                &[SoftwareCompositeLayer {
                                    scene: &source,
                                    target: RectI {
                                        x: 0,
                                        y: 0,
                                        width: 40,
                                        height: 30,
                                    },
                                    clip: None,
                                    rounded_clips: [None; 2],
                                }],
                                extent,
                                None,
                                ColorRgba8::rgba(0, 0, 0, 0),
                            )
                            .unwrap()
                    };
                    draw(&mut surface);
                    let first = surface.pixels_rgba8().to_vec();
                    draw(&mut surface);
                    assert_eq!(first, surface.pixels_rgba8(), "cached frame changed");
                    for y in 0..extent.height {
                        for x in 0..extent.width {
                            let p = crate::foundation::PointF {
                                x: x as f32 + 0.5,
                                y: y as f32 + 0.5,
                            };
                            let outer = RoundedClip::new(rect, border.corner_radii).coverage(p);
                            let i = inner.coverage(p).min(outer);
                            let paint =
                                frame_border::InteriorPaint::new(&border).coverage(p).min(i);
                            let a =
                                button_alpha as f32 / 255.0 * if i > 0.0 { paint / i } else { 0.0 };
                            let f = frame_alpha as f32 / 255.0;
                            let expected = [(outer - i) * f, i * f * (1.0 - a), i * a];
                            let at = ((y * extent.width + x) * 4) as usize;
                            let pixel = &first[at..at + 4];
                            for (actual, expected) in pixel[..3].iter().zip(expected) {
                                assert!(
                                    (srgb_decode_byte(*actual) - expected).abs() < 0.02,
                                    "radius={radius} width={width} button={button_alpha} frame={frame_alpha} pixel=({x},{y}) rgba={pixel:?} expected={expected}"
                                );
                            }
                            let expected_alpha = (outer - i) * f + i * (a + f * (1.0 - a));
                            assert!(
                                (pixel[3] as f32 / 255.0 - expected_alpha).abs() < 0.02,
                                "alpha: radius={radius} width={width} pixel=({x},{y}) rgba={pixel:?} expected={expected_alpha}"
                            );
                        }
                    }
                    // Hover/press changes must invalidate the cached interior, while retaining
                    // the exact same border contribution at every shared edge pixel.
                    let mut changed = scene.boxes.get(NodeId::new(1, 1)).unwrap().clone();
                    changed.background = Some(ColorRgba8::rgba(0, 0, 255, 255 - button_alpha));
                    scene.boxes.upsert(changed.node, changed);
                    let mut delta = scene.take_delta().unwrap();
                    frame_border::prepare_interior(&mut delta, &border);
                    SoftwareRenderer
                        .apply_scene_delta(&mut source, &delta)
                        .unwrap();
                    SoftwareRenderer
                        .render_composite(
                            &mut surface,
                            &[SoftwareCompositeLayer {
                                scene: &source,
                                target: RectI {
                                    x: 0,
                                    y: 0,
                                    width: 40,
                                    height: 30,
                                },
                                clip: None,
                                rounded_clips: [None; 2],
                            }],
                            extent,
                            None,
                            ColorRgba8::rgba(0, 0, 0, 0),
                        )
                        .unwrap();
                    for (index, (before, after)) in first
                        .chunks_exact(4)
                        .zip(surface.pixels_rgba8().chunks_exact(4))
                        .enumerate()
                    {
                        assert_eq!(
                            before[0], after[0],
                            "a control update changed the red border"
                        );
                        let p = crate::foundation::PointF {
                            x: (index % extent.width as usize) as f32 + 0.5,
                            y: (index / extent.width as usize) as f32 + 0.5,
                        };
                        let outer = RoundedClip::new(rect, border.corner_radii).coverage(p);
                        if outer - inner.coverage(p).min(outer) > 0.001 {
                            assert_eq!(before, after, "hover recolored a border pixel at {p:?}");
                        }
                    }
                    assert_ne!(
                        first,
                        surface.pixels_rgba8(),
                        "control update used a stale cached frame"
                    );
                }
            }
        }
    }
}
