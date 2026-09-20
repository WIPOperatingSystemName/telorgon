use super::*;
use crate::foundation::{ColorRgba8, PointF, RectI, SizeF, SizeI};
use crate::ui::layout::LayoutEngine;
use crate::ui::layout::{ClipId, SpatialId};
use crate::graphics::render::{
    BatchKey, BlendMode, BoxInstance, GlyphInstance, ImageAlphaMode, ImageColorEncoding,
    ImageId, ImageInstance, ImageResource, MaterialInstance, MaterialKind, MaterialResource,
    PipelineKind, PrimitiveKind, ReadbackFormat, ReadbackRequest, RenderClip, RenderRequest,
    RenderScene, RenderSpatialNode, RenderTargetInfo, SceneCompiler, TargetLoad, TargetStore,
};
use crate::ui::text::AtlasPageUpdate;
use crate::ui::text::RetainedTextSystem;
use crate::ui::{
    Background, Border, BorderSide, BoxStyle, CornerRadii, LayoutStyle, MaterialId,
    MountWriter, MountedUi, Outline, Shadow, ShadowList, SizeRule, UiNodeId as NodeId,
};
use std::sync::Arc;

#[test]
fn dense_glyph_samples_the_entire_atlas_region() {
    let mut pixels = [0_u8; 2 * 4];
    let mut raster = RasterTarget {
        pixels: &mut pixels,
        width: 2,
        height: 1,
        origin: crate::foundation::PointI::default(),
        blend_mode: BlendMode::Alpha,
        color_space: ColorSpace::Srgb,
        rounded_clips: [None; 2],
    };
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: 2.0,
        height: 1.0,
    };
    let glyph = GlyphInstance {
        node: NodeId::new(0, 1),
        rect,
        view_bounds: rect,
        atlas_x: 0,
        atlas_y: 0,
        atlas_size: SizeI {
            width: 4,
            height: 2,
        },
        color: ColorRgba8::rgba(255, 255, 255, 255),
        opacity: 1.0,
        clip: ClipId(0),
        spatial: SpatialId(0),
    };
    // The right half is opaque; sampling only the logical-sized left half would lose it.
    draw_glyph(
        &mut raster,
        &glyph,
        None,
        None,
        rect,
        &[0, 0, 255, 255, 0, 0, 255, 255],
        SizeI {
            width: 4,
            height: 2,
        },
    );
    assert_eq!(pixels[3], 0);
    assert_eq!(pixels[7], 255);
}

#[test]
fn fractional_glyph_bounds_do_not_sample_outside_the_quad() {
    let mut pixels = [0_u8; 3 * 4];
    let mut raster = RasterTarget {
        pixels: &mut pixels,
        width: 3,
        height: 1,
        origin: crate::foundation::PointI::default(),
        blend_mode: BlendMode::Alpha,
        color_space: ColorSpace::Srgb,
        rounded_clips: [None; 2],
    };
    let glyph = GlyphInstance {
        node: NodeId::new(0, 1),
        rect: RectF {
            x: 0.75,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        view_bounds: RectF {
            x: 0.75,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        atlas_x: 0,
        atlas_y: 0,
        atlas_size: SizeI {
            width: 1,
            height: 1,
        },
        color: ColorRgba8::rgba(255, 255, 255, 255),
        opacity: 1.0,
        clip: ClipId(0),
        spatial: SpatialId(0),
    };

    draw_glyph(
        &mut raster,
        &glyph,
        None,
        None,
        RectF {
            x: 0.0,
            y: 0.0,
            width: 3.0,
            height: 1.0,
        },
        &[255],
        SizeI {
            width: 1,
            height: 1,
        },
    );

    assert_eq!(pixels[3], 0);
    assert!(pixels[7] > 0);
}

#[test]
fn image_tint_recolors_the_source_alpha_mask() {
    let mut pixels = [0_u8; 4];
    let mut raster = RasterTarget {
        pixels: &mut pixels,
        width: 1,
        height: 1,
        origin: crate::foundation::PointI::default(),
        blend_mode: BlendMode::Alpha,
        color_space: ColorSpace::Srgb,
        rounded_clips: [None; 2],
    };
    let node = NodeId::new(0, 1);
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    let image = SoftwareImage {
        extent: SizeI {
            width: 1,
            height: 1,
        },
        color_encoding: ImageColorEncoding::Srgb,
        alpha_mode: ImageAlphaMode::Straight,
        pixel_format: ImagePixelFormat::Rgba8,
        pixels: vec![0, 0, 0, 255],
    };
    let instance = ImageInstance {
        node,
        image: ImageId(1),
        tint: Some(ColorRgba8::rgba(255, 255, 255, 255)),
        rect,
        view_bounds: rect,
        content_version: 1,
        opacity: 1.0,
        clip: ClipId(0),
        spatial: SpatialId(0),
    };

    draw_image(&mut raster, &instance, None, None, rect, &image);

    assert_eq!(pixels, [255, 255, 255, 255]);
}

#[test]
fn bgra_images_are_sampled_in_logical_rgb_order() {
    let sampled = sample_image_linear(
        &[3, 2, 255, 128],
        SizeI {
            width: 1,
            height: 1,
        },
        ImageColorEncoding::Srgb,
        ImagePixelFormat::Bgra8,
        0.5,
        0.5,
    );

    assert!(sampled[0] > 0.99);
    assert!(sampled[1] < 0.001);
    assert!(sampled[2] < 0.001);
    assert!((sampled[3] - 128.0 / 255.0).abs() < 0.001);
}

#[test]
fn retained_scene_readback_is_explicit() {
    let mut ui = MountedUi::default();
    {
        let mut builder = MountWriter::<()>::new(&mut ui);
        builder.root(
            BoxStyle {
                width: SizeRule::Fill(1.0),
                height: SizeRule::Fill(1.0),
                decoration: crate::ui::BoxDecoration {
                    background: Background::Color(ColorRgba8::rgba(255, 0, 0, 255)),
                    ..crate::ui::BoxDecoration::default()
                },
                ..BoxStyle::default()
            },
            LayoutStyle::default(),
            |_| {},
        );
    }
    let extent = SizeF {
        width: 4.0,
        height: 4.0,
    };
    let mut layout = LayoutEngine::default();
    let mut text = RetainedTextSystem::new(100).unwrap();
    layout.update(&mut ui, &mut text, extent, 1.0);
    let mut scene = RenderScene::default();
    SceneCompiler::default().compile(
        &mut ui,
        &layout,
        &mut text,
        &mut scene,
        extent,
        ColorRgba8::default(),
    );
    let delta = scene.take_delta().unwrap();
    let renderer = SoftwareRenderer;
    let mut backend_scene = renderer.create_scene().unwrap();
    renderer
        .apply_scene_delta(&mut backend_scene, &delta)
        .unwrap();
    let mut surface = SoftwareSurface::default();
    let target = SoftwareTarget::new(RenderTargetInfo::full(SizeI {
        width: 4,
        height: 4,
    }));
    let clear = backend_scene.background();
    {
        let mut frame = surface.begin_frame();
        renderer
            .render(
                &mut backend_scene,
                &mut frame,
                &target,
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(clear),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .unwrap();
    }
    let image = surface
        .readback(&ReadbackRequest {
            region: RectI {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            },
            format: ReadbackFormat::Rgba8,
        })
        .unwrap();
    assert_eq!(&image.pixels[0..4], &[255, 0, 0, 255]);
}

#[test]
fn one_backend_creates_independent_per_view_scenes() {
    let renderer = SoftwareRenderer;
    let mut first = renderer.create_scene().unwrap();
    let second = renderer.create_scene().unwrap();
    let mut scene = RenderScene::default();
    let delta = scene.take_delta().unwrap();

    renderer.apply_scene_delta(&mut first, &delta).unwrap();

    assert_eq!(first.epoch, 1);
    assert_eq!(second.epoch, 0);
}

#[cfg(target_os = "linux")]
#[test]
fn desktop_composite_draws_independent_scenes_in_placement_order() {
    fn colored_scene(color: ColorRgba8) -> RenderScene {
        let node = NodeId::new(0, 1);
        let bounds = RectF {
            x: 0.0,
            y: 0.0,
            width: 2.0,
            height: 2.0,
        };
        let mut scene = RenderScene::default();
        scene.extent = SizeF {
            width: 2.0,
            height: 2.0,
        };
        scene.boxes.upsert(
            node,
            BoxInstance {
                node,
                rect: bounds,
                view_bounds: bounds,
                background: Some(color),
                border: Border::default(),
                outline: Outline::default(),
                corner_radii: CornerRadii::default(),
                shadows: ShadowList::default(),
                opacity: 1.0,
                clip: ClipId(0),
                spatial: SpatialId(0),
            },
        );
        scene.set_draw_order(vec![DrawItem {
            kind: PrimitiveKind::Box,
            index: 0,
            batch: BatchKey {
                pipeline: PipelineKind::AnalyticBox,
                resource: 0,
                clip: ClipId(0),
                blend: BlendMode::Opaque,
                target: 0,
            },
        }]);
        scene
    }

    let renderer = SoftwareRenderer;
    let mut red = renderer.create_scene().unwrap();
    let mut blue = renderer.create_scene().unwrap();
    renderer
        .apply_scene_delta(
            &mut red,
            &colored_scene(ColorRgba8::rgba(255, 0, 0, 255))
                .take_delta()
                .unwrap(),
        )
        .unwrap();
    renderer
        .apply_scene_delta(
            &mut blue,
            &colored_scene(ColorRgba8::rgba(0, 0, 255, 255))
                .take_delta()
                .unwrap(),
        )
        .unwrap();
    let mut surface = SoftwareSurface::default();
    renderer
        .render_composite(
            &mut surface,
            &[
                SoftwareCompositeLayer {
                    scene: &red,
                    rounded_clips: [None; 2],
                    target: RectI {
                        x: 0,
                        y: 0,
                        width: 2,
                        height: 2,
                    },
                    clip: None,
                },
                SoftwareCompositeLayer {
                    scene: &blue,
                    rounded_clips: [None; 2],
                    target: RectI {
                        x: 1,
                        y: 0,
                        width: 2,
                        height: 2,
                    },
                    clip: None,
                },
            ],
            SizeI {
                width: 4,
                height: 2,
            },
            None,
            ColorRgba8::rgba(0, 0, 0, 255),
        )
        .unwrap();
    assert_eq!(&surface.pixels_rgba8()[0..4], &[255, 0, 0, 255]);
    assert_eq!(&surface.pixels_rgba8()[4..8], &[0, 0, 255, 255]);
    assert_eq!(&surface.pixels_rgba8()[12..16], &[0, 0, 0, 255]);
}

#[test]
fn rounded_clip_preserves_fractional_pixel_coverage_at_all_corners() {
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: 80.0,
        height: 80.0,
    };
    let instance = BoxInstance {
        node: NodeId::new(0, 1),
        rect,
        view_bounds: rect,
        background: Some(ColorRgba8::rgba(0, 0, 255, 255)),
        border: Border::default(),
        outline: Outline::default(),
        corner_radii: CornerRadii::default(),
        shadows: ShadowList::default(),
        opacity: 1.0,
        clip: ClipId(1),
        spatial: SpatialId(0),
    };
    for scale in [1.0, 1.25, 1.5, 2.0] {
        for inset in [2.0, 2.5] {
            let clip = RenderClip {
                id: ClipId(1),
                rect: RectF {
                    x: inset * scale,
                    y: inset * scale,
                    width: (36.0 - 2.0 * inset) * scale,
                    height: (36.0 - 2.0 * inset) * scale,
                },
                corner_radii: CornerRadii::all((14.0 - inset) * scale),
            };
            let mut pixels = vec![0; 80 * 80 * 4];
            draw_box(
                &mut RasterTarget {
                    pixels: &mut pixels,
                    width: 80,
                    height: 80,
                    origin: crate::foundation::PointI::default(),
                    blend_mode: BlendMode::Alpha,
                    color_space: ColorSpace::Srgb,
                    rounded_clips: [None; 2],
                },
                &instance,
                None,
                Some(&clip),
                rect,
            );
            let mut partial = 0;
            for y in 0..80 {
                for x in 0..80 {
                    let px = x as f32 + 0.5;
                    let py = y as f32 + 0.5;
                    if !clip.rect.contains(PointF { x: px, y: py }) {
                        continue;
                    }
                    let radius = (14.0 - inset) * scale;
                    let cx = if px < clip.rect.x + clip.rect.width / 2.0 {
                        clip.rect.x + radius
                    } else {
                        clip.rect.right() - radius
                    };
                    let cy = if py < clip.rect.y + clip.rect.height / 2.0 {
                        clip.rect.y + radius
                    } else {
                        clip.rect.bottom() - radius
                    };
                    // Independently evaluate the circular corners, away from straight edges.
                    if (px < cx) == (cx < clip.rect.x + clip.rect.width / 2.0)
                        && (py < cy) == (cy < clip.rect.y + clip.rect.height / 2.0)
                    {
                        let expected = ((0.5 - ((px - cx).hypot(py - cy) - radius))
                            .clamp(0.0, 1.0)
                            * 255.0)
                            .round() as u8;
                        let alpha = pixels[(y * 80 + x) * 4 + 3];
                        assert!(
                            alpha.abs_diff(expected) <= 1,
                            "clip coverage at {x},{y}, scale={scale}, inset={inset}: {alpha} != {expected}"
                        );
                        partial += usize::from(expected > 0 && expected < 255);
                    }
                }
            }
            assert!(partial > 0, "must exercise partially covered pixels");
        }
    }
}

#[test]
fn adjacent_square_controls_cover_every_corner_pixel() {
    for scale in [1, 2, 3] {
        for origin in [0, 1] {
            let width = 120 * scale + 2;
            let height = 24 * scale + 2;
            let mut pixels = vec![0; width * height * 4];
            let mut raster = RasterTarget {
                pixels: &mut pixels,
                width,
                height,
                origin: crate::foundation::PointI::default(),
                blend_mode: BlendMode::Alpha,
                color_space: ColorSpace::Srgb,
                rounded_clips: [None; 2],
            };
            for index in 0..3 {
                let rect = RectF {
                    x: (origin + index * 40 * scale) as f32,
                    y: origin as f32,
                    width: (40 * scale) as f32,
                    height: (24 * scale) as f32,
                };
                let instance = BoxInstance {
                    node: NodeId::new(index as u32, 1),
                    rect,
                    view_bounds: rect,
                    background: Some(ColorRgba8::rgba(0, 0, 255, 255)),
                    border: Border::default(),
                    outline: Outline::default(),
                    corner_radii: CornerRadii::all(0.0),
                    shadows: ShadowList::default(),
                    opacity: 1.0,
                    clip: ClipId(0),
                    spatial: SpatialId(0),
                };
                draw_box(&mut raster, &instance, None, None, rect);
            }
            for y in origin..origin + 24 * scale {
                for x in origin..origin + 120 * scale {
                    let offset = (y * width + x) * 4;
                    assert_eq!(
                        &pixels[offset..offset + 4],
                        &[0, 0, 255, 255],
                        "square control seam at ({x},{y}), scale={scale}, origin={origin}"
                    );
                }
            }
        }
    }
}

#[test]
fn border_and_fill_share_coverage_without_an_alpha_seam() {
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: 32.0,
        height: 32.0,
    };
    for alpha in [0, 128, 255] {
        for opacity in [0.5, 1.0] {
            let color = ColorRgba8::rgba(120, 80, 40, alpha);
            let instance = BoxInstance {
                node: NodeId::new(0, 1),
                rect,
                view_bounds: rect,
                background: Some(color),
                border: Border::all(2.0, color),
                outline: Outline::default(),
                corner_radii: CornerRadii::all(14.0),
                shadows: ShadowList::default(),
                opacity,
                clip: ClipId(0),
                spatial: SpatialId(0),
            };
            let mut pixels = vec![0; 32 * 32 * 4];
            draw_box(
                &mut RasterTarget {
                    pixels: &mut pixels,
                    width: 32,
                    height: 32,
                    origin: crate::foundation::PointI::default(),
                    blend_mode: BlendMode::Alpha,
                    color_space: ColorSpace::Srgb,
                    rounded_clips: [None; 2],
                },
                &instance,
                None,
                None,
                rect,
            );
            for y in 0..32 {
                for x in 0..32 {
                    let coverage =
                        rounded_coverage(x as f32 + 0.5, y as f32 + 0.5, rect, [14.0; 4], 1.0);
                    let expected = (f32::from(alpha) * opacity * coverage).round() as u8;
                    assert!(
                        pixels[(y * 32 + x) * 4 + 3].abs_diff(expected) <= 1,
                        "coverage seam at {x},{y}, alpha={alpha}, opacity={opacity}"
                    );
                }
            }
        }
    }
}

#[test]
fn rounded_asymmetric_border_outline_two_shadows_and_local_opacity_render() {
    let node = NodeId::new(0, 1);
    let mut source = RenderScene::default();
    source.extent = SizeF {
        width: 40.0,
        height: 40.0,
    };
    source.boxes.upsert(
        node,
        BoxInstance {
            node,
            rect: RectF {
                x: 10.0,
                y: 10.0,
                width: 18.0,
                height: 18.0,
            },
            view_bounds: RectF {
                x: 4.0,
                y: 4.0,
                width: 34.0,
                height: 34.0,
            },
            background: Some(ColorRgba8::rgba(240, 240, 240, 255)),
            border: Border {
                top: BorderSide {
                    width: 1.0,
                    color: ColorRgba8::rgba(255, 0, 0, 255),
                },
                right: BorderSide {
                    width: 2.0,
                    color: ColorRgba8::rgba(0, 255, 0, 255),
                },
                bottom: BorderSide {
                    width: 3.0,
                    color: ColorRgba8::rgba(0, 0, 255, 255),
                },
                left: BorderSide {
                    width: 4.0,
                    color: ColorRgba8::rgba(255, 255, 0, 255),
                },
            },
            outline: Outline {
                width: 1.0,
                offset: 1.0,
                color: ColorRgba8::rgba(0, 255, 255, 255),
            },
            corner_radii: CornerRadii::all(5.0),
            shadows: ShadowList::two(
                Shadow {
                    offset: PointF { x: 2.0, y: 2.0 },
                    blur: 2.0,
                    spread: 0.0,
                    color: ColorRgba8::rgba(255, 0, 255, 160),
                },
                Shadow {
                    offset: PointF { x: -2.0, y: -2.0 },
                    blur: 1.0,
                    spread: 1.0,
                    color: ColorRgba8::rgba(255, 128, 0, 120),
                },
            ),
            opacity: 0.5,
            clip: ClipId(0),
            spatial: SpatialId(0),
        },
    );
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

    let renderer = SoftwareRenderer;
    let mut scene = renderer.create_scene().unwrap();
    renderer
        .apply_scene_delta(&mut scene, &source.take_delta().unwrap())
        .unwrap();
    let mut surface = SoftwareSurface::default();
    let target = SoftwareTarget::new(RenderTargetInfo::full(SizeI {
        width: 40,
        height: 40,
    }));
    {
        let mut frame = surface.begin_frame();
        renderer
            .render(
                &mut scene,
                &mut frame,
                &target,
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .unwrap();
    }
    let pixel = |x: usize, y: usize| {
        let offset = (y * 40 + x) * 4;
        &surface.pixels_rgba8()[offset..offset + 4]
    };
    assert!(pixel(18, 10)[0] > pixel(18, 10)[1]);
    assert!(pixel(27, 18)[1] > pixel(27, 18)[0]);
    assert!(pixel(18, 26)[2] > pixel(18, 26)[0]);
    assert!(pixel(10, 18)[0] > 80 && pixel(10, 18)[1] > 80);
    assert!(pixel(8, 18)[1] > 80 && pixel(8, 18)[2] > 80);
    assert!(pixel(29, 29)[0] > 0 || pixel(29, 29)[2] > 0);
    assert!(
        pixel(18, 18)[0] < 240,
        "box opacity must be primitive-local"
    );
    assert_eq!(pixel(0, 0), &[0, 0, 0, 255]);
}

#[test]
fn mixed_glyph_image_material_clip_spatial_and_opacity_are_rasterized_in_order() {
    let mut source = RenderScene::default();
    source.extent = SizeF {
        width: 8.0,
        height: 4.0,
    };
    let image_node = NodeId::new(0, 1);
    let glyph_node = NodeId::new(1, 1);
    let material_node = NodeId::new(2, 1);
    source.spatial_nodes.upsert(
        image_node,
        RenderSpatialNode {
            id: SpatialId(1),
            transform: crate::foundation::Affine2D::translation(1.0, 0.0),
        },
    );
    source.clips.upsert(
        glyph_node,
        RenderClip {
            id: ClipId(1),
            rect: RectF {
                x: 3.0,
                y: 0.0,
                width: 1.0,
                height: 2.0,
            },
            corner_radii: Default::default(),
        },
    );
    source
        .set_image_resource(ImageResource {
            image: ImageId(7),
            content_version: 1,
            extent: SizeI {
                width: 2,
                height: 1,
            },
            color_encoding: ImageColorEncoding::Srgb,
            alpha_mode: ImageAlphaMode::Straight,
            pixel_format: ImagePixelFormat::Rgba8,
            pixels: Arc::from([255, 0, 0, 255, 0, 255, 0, 128]),
        })
        .unwrap();
    source.images.upsert(
        image_node,
        ImageInstance {
            node: image_node,
            image: ImageId(7),
            tint: None,
            rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 1.0,
            },
            view_bounds: RectF {
                x: 1.0,
                y: 0.0,
                width: 2.0,
                height: 1.0,
            },
            content_version: 1,
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(1),
        },
    );
    source.set_atlas_updates(
        SizeI {
            width: 2,
            height: 1,
        },
        vec![AtlasPageUpdate {
            page: 0,
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            pixels_a8: Arc::from([255, 255]),
        }],
    );
    source.set_glyphs(vec![GlyphInstance {
        node: glyph_node,
        rect: RectF {
            x: 2.0,
            y: 0.0,
            width: 2.0,
            height: 1.0,
        },
        view_bounds: RectF {
            x: 2.0,
            y: 0.0,
            width: 2.0,
            height: 1.0,
        },
        atlas_x: 0,
        atlas_y: 0,
        atlas_size: SizeI {
            width: 2,
            height: 1,
        },
        color: ColorRgba8::rgba(255, 255, 255, 255),
        opacity: 0.5,
        clip: ClipId(1),
        spatial: SpatialId(0),
    }]);
    source.set_material_resource(MaterialResource {
        material: MaterialId(3),
        content_version: 1,
        kind: MaterialKind::LinearGradientHorizontal,
        colors: [
            ColorRgba8::rgba(255, 0, 0, 255),
            ColorRgba8::rgba(0, 0, 255, 255),
        ],
    });
    source.materials.upsert(
        material_node,
        MaterialInstance {
            node: material_node,
            material: MaterialId(3),
            rect: RectF {
                x: 4.0,
                y: 0.0,
                width: 4.0,
                height: 1.0,
            },
            view_bounds: RectF {
                x: 4.0,
                y: 0.0,
                width: 4.0,
                height: 1.0,
            },
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(0),
        },
    );
    source.set_draw_order(vec![
        DrawItem {
            kind: PrimitiveKind::Image,
            index: 0,
            batch: BatchKey {
                pipeline: PipelineKind::Image,
                resource: 7,
                clip: ClipId(0),
                blend: BlendMode::Alpha,
                target: 0,
            },
        },
        DrawItem {
            kind: PrimitiveKind::Glyph,
            index: 0,
            batch: BatchKey {
                pipeline: PipelineKind::Glyph,
                resource: 0,
                clip: ClipId(1),
                blend: BlendMode::Alpha,
                target: 0,
            },
        },
        DrawItem {
            kind: PrimitiveKind::Material,
            index: 0,
            batch: BatchKey {
                pipeline: PipelineKind::Material,
                resource: 3,
                clip: ClipId(0),
                blend: BlendMode::Alpha,
                target: 0,
            },
        },
    ]);

    let renderer = SoftwareRenderer;
    let mut scene = renderer.create_scene().unwrap();
    renderer
        .apply_scene_delta(&mut scene, &source.take_delta().unwrap())
        .unwrap();
    let mut surface = SoftwareSurface::default();
    let target = SoftwareTarget::new(RenderTargetInfo::full(SizeI {
        width: 8,
        height: 4,
    }));
    {
        let mut frame = surface.begin_frame();
        renderer
            .render(
                &mut scene,
                &mut frame,
                &target,
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .unwrap();
    }
    let pixel = |x: usize| &surface.pixels_rgba8()[(x * 4)..(x * 4 + 4)];
    assert_eq!(pixel(0), &[0, 0, 0, 255]);
    assert_eq!(pixel(1), &[255, 0, 0, 255]);
    assert_eq!(pixel(2), &[0, 188, 0, 255]);
    assert_eq!(pixel(3), &[188, 188, 188, 255]);
    assert!(pixel(4)[0] > pixel(4)[2]);
    assert!(pixel(7)[2] > pixel(7)[0]);
}
