//! Compose weighted immutable images as one premultiplied group, without a scratch image.
use super::*;

impl SoftwareScene {
    /// Only compositor-owned image scenes use this path. Group opacity and placement
    /// coverage apply after the additive mixture, never separately to its endpoints.
    pub(crate) fn set_image_mix_opacity(&mut self, opacity: Option<f32>) {
        self.image_mix_opacity = opacity;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::render::{BatchKey, PipelineKind, RoundedClip};
    use crate::graphics::scene::NodeId;

    #[test]
    fn direct_mix_matches_materialized_premultiplied_group_with_clipping_and_opacity() {
        let extent = SizeI { width: 5, height: 4 };
        let bounds = RectI { x: 0, y: 0, width: 5, height: 4 };
        for amount in [0.0, 0.125, 0.5, 0.875, 1.0] {
            let mut scene = SoftwareScene { extent: SizeF { width: 5.0, height: 4.0 }, ..Default::default() };
            for (i, pixel) in [[90, 15, 70, 170], [25, 85, 100, 210]].into_iter().enumerate() {
                let image = ImageId(i as u32 + 1);
                let rect = RectF { x: if i == 0 { -1.0 } else { 0.0 }, y: 0.0, width: 5.0, height: 4.0 };
                scene.images.push(ImageInstance {
                    node: NodeId::new(i as u32 + 1, 1), image, rect, view_bounds: rect,
                    tint: None, opacity: if i == 0 { 1.0 - amount } else { amount },
                    content_version: 1, clip: crate::ui::layout::ClipId(0), spatial: SpatialId(0),
                });
                scene.image_resources.insert(image, SoftwareImage {
                    extent, color_encoding: ImageColorEncoding::Srgb, alpha_mode: ImageAlphaMode::Premultiplied,
                    pixel_format: ImagePixelFormat::Rgba8, pixels: pixel.repeat(20).into(),
                });
                scene.draw_order.push(DrawItem { kind: PrimitiveKind::Image, index: i as u32,
                    batch: BatchKey { pipeline: PipelineKind::Image, resource: image.0,
                        clip: crate::ui::layout::ClipId(0), blend: BlendMode::Add, target: 0 } });
            }
            let mut snapshot = SoftwareSurface::default();
            SoftwareRenderer.render_composite(&mut snapshot, &[SoftwareCompositeLayer {
                scene: &scene, target: bounds, clip: None, rounded_clips: [None; 2],
            }], extent, None, ColorRgba8::rgba(0, 0, 0, 0)).unwrap();
            for opacity in [0.0, 0.37, 1.0] {
                let mut reference = scene.clone();
                reference.images.truncate(1);
                reference.images[0].rect.x = 0.0;
                reference.images[0].opacity = opacity;
                reference.draw_order.truncate(1);
                reference.draw_order[0].batch.blend = BlendMode::Alpha;
                reference.image_resources.insert(ImageId(1), SoftwareImage {
                    extent, color_encoding: ImageColorEncoding::Srgb, alpha_mode: ImageAlphaMode::Premultiplied,
                    pixel_format: ImagePixelFormat::Rgba8, pixels: snapshot.pixels_rgba8().into(),
                });
                scene.set_image_mix_opacity(Some(opacity));
                let render = |source: &SoftwareScene| {
                    let mut output = SoftwareSurface::default();
                    SoftwareRenderer.render_composite(&mut output, &[SoftwareCompositeLayer {
                        scene: source, target: bounds, clip: Some(RectI { x: 1, y: 0, width: 4, height: 4 }),
                        rounded_clips: [Some(RoundedClip::new(RectF { x: 0.0, y: 0.0, width: 5.0, height: 4.0 },
                            crate::ui::CornerRadii::all(1.5))), None],
                    }], extent, None, ColorRgba8::rgba(12, 45, 34, 255)).unwrap();
                    output
                };
                assert_eq!(render(&scene).pixels_rgba8(), render(&reference).pixels_rgba8(),
                    "amount={amount} opacity={opacity}");
                scene.set_image_mix_opacity(None);
            }
        }
    }
}

pub(super) fn draw(scene: &SoftwareScene, opacity: f32, target: &mut RasterTarget<'_>, region: RectF) {
    let bounds = RectF { x: 0.0, y: 0.0, width: scene.extent.width, height: scene.extent.height };
    let Some(region) = clip_to_target(intersect(bounds, region), target.width, target.height) else { return; };
    target.blend_mode = BlendMode::Alpha;
    let images = scene.images.iter().filter_map(|instance| {
        (instance.opacity > 0.0).then(|| scene.image_resources.get(&instance.image)
            .map(|image| (instance, image))).flatten()
    }).collect::<Vec<_>>();
    for y in region.y.floor() as i32..region.bottom().ceil() as i32 {
        for x in region.x.floor() as i32..region.right().ceil() as i32 {
            if target.placement_coverage(x, y) <= 0.0 { continue; }
            let point = crate::foundation::PointF { x: x as f32 + 0.5, y: y as f32 + 0.5 };
            let mut mixed = [0.0_f32; 4];
            for (instance, image) in &images {
                if !instance.rect.contains(point) { continue; }
                let sample = sample_image_linear(&image.pixels, image.extent, image.color_encoding,
                    image.pixel_format, (point.x - instance.rect.x) / instance.rect.width,
                    (point.y - instance.rect.y) / instance.rect.height);
                for channel in 0..4 {
                    // Match the RGBA8 additive capture, including its per-endpoint rounding.
                    let value = (mixed[channel] + sample[channel] * instance.opacity.clamp(0.0, 1.0)).clamp(0.0, 1.0);
                    mixed[channel] = if channel == 3 {
                        (value * 255.0).round() / 255.0
                    } else {
                        srgb_decode_byte(encode_target_channel(value, ColorSpace::Srgb))
                    };
                }
            }
            target.blend_linear_premultiplied(x, y,
                [mixed[0] * opacity, mixed[1] * opacity, mixed[2] * opacity], mixed[3] * opacity);
        }
    }
}
