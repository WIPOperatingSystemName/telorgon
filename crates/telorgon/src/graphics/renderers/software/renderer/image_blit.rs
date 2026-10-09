//! Exact texel copies for unscaled sRGB images, with ordinary blending at translucent edges.
use super::*;

pub(super) fn draw(
    raster: &mut RasterTarget<'_>,
    instance: &ImageInstance,
    clip: Option<&RenderClip>,
    region: RectF,
    image: &SoftwareImage,
) -> bool {
    let rect = instance.rect;
    if instance.opacity != 1.0 || instance.tint.is_some()
        || image.color_encoding != ImageColorEncoding::Srgb
        || raster.color_space != ColorSpace::Srgb
        || raster.coverage_normalization.is_some()
        || !matches!(raster.blend_mode, BlendMode::Alpha | BlendMode::Opaque)
        || rect.width != image.extent.width as f32
        || rect.height != image.extent.height as f32
        || rect.x.fract() != 0.0 || rect.y.fract() != 0.0
    {
        return false;
    }
    let Some(target) = clip_to_target(
        intersect(intersect(rect, clip.map_or(region, |clip| clip.rect)), region),
        raster.width, raster.height,
    ) else { return true; };
    // Establish the area with unit clip coverage once instead of testing every contour
    // for every interior texel. Rounded edges and inverted apertures keep ordinary coverage.
    let mut flat = rect;
    let safe_rect = |rect: RectF, radii: crate::ui::CornerRadii| {
        let inset = radii.top_left.max(radii.top_right).max(radii.bottom_right)
            .max(radii.bottom_left) + 1.0;
        RectF { x: rect.x + inset, y: rect.y + inset,
            width: (rect.width - inset * 2.0).max(0.0), height: (rect.height - inset * 2.0).max(0.0) }
    };
    if let Some(clip) = clip {
        flat = intersect(flat, safe_rect(clip.rect, clip.corner_radii));
    }
    for clip in raster.rounded_clips.iter().flatten() {
        if clip.inverted { flat = RectF::ZERO; break; }
        flat = intersect(flat, safe_rect(RectF {
            x: clip.rect.x - raster.origin.x as f32,
            y: clip.rect.y - raster.origin.y as f32, ..clip.rect
        }, clip.radii));
    }
    // Resolved translucent frame interiors often repeat one texel over a large area.
    // Cache exact source-over results for that texel; other pixels use the ordinary blend.
    let middle = ((image.extent.height as usize / 2) * image.extent.width as usize
        + image.extent.width as usize / 2) * 4;
    let flat_pixel = &image.pixels[middle..middle + 4];
    let table = (raster.blend_mode == BlendMode::Alpha && image.alpha_mode != ImageAlphaMode::Opaque
        && flat_pixel[3] > 0 && flat_pixel[3] < 255 && rect.width * rect.height >= 4096.0).then(|| {
        let alpha = f32::from(flat_pixel[3]) / 255.0;
        let scale = if image.alpha_mode == ImageAlphaMode::Straight { alpha } else { 1.0 };
        let channels = match image.pixel_format {
            ImagePixelFormat::Rgba8 => [flat_pixel[0], flat_pixel[1], flat_pixel[2]],
            ImagePixelFormat::Bgra8 => [flat_pixel[2], flat_pixel[1], flat_pixel[0]],
        };
        let rgb = channels.map(|v| srgb_decode_byte(v) * scale);
        std::array::from_fn::<_, 4, _>(|channel| std::array::from_fn::<_, 256, _>(|byte| {
            if channel == 3 {
                ((alpha + byte as f32 / 255.0 * (1.0 - alpha)).clamp(0.0, 1.0) * 255.0).round() as u8
            } else {
                encode_target_channel(rgb[channel] + decode_target_channel(byte as u8, raster.color_space)
                    * (1.0 - alpha), raster.color_space)
            }
        }))
    });
    for y in target.y.floor() as i32..target.bottom().ceil() as i32 {
        let end = target.right().ceil() as i32;
        let mut next_x = target.x.floor() as i32;
        while next_x < end {
            let x = next_x;
            next_x += 1;
            if flat.contains(crate::foundation::PointF { x: x as f32 + 0.5, y: y as f32 + 0.5 }) {
                let right = ((flat.right() - 0.5).ceil() as i32).min(end).max(next_x);
                draw_flat_span(raster, image, rect, x, right, y, flat_pixel, table.as_ref());
                next_x = right;
                continue;
            }
            let dx = x.saturating_add(raster.origin.x);
            let dy = y.saturating_add(raster.origin.y);
            if dx < 0 || dy < 0 || dx as usize >= raster.width || dy as usize >= raster.height {
                continue;
            }
            let full_coverage = flat.contains(crate::foundation::PointF { x: x as f32 + 0.5, y: y as f32 + 0.5 });
            let clip_amount = if full_coverage { 1.0 } else { clip_coverage(x as f32 + 0.5, y as f32 + 0.5, clip) };
            let coverage = if full_coverage { 1.0 } else { clip_amount * raster.placement_coverage(x, y) };
            if coverage <= 0.0 { continue; }
            let sx = x - rect.x as i32;
            let sy = y - rect.y as i32;
            let source = (sy as usize * image.extent.width as usize + sx as usize) * 4;
            let pixel = &image.pixels[source..source + 4];
            let (r, g, b) = match image.pixel_format {
                ImagePixelFormat::Rgba8 => (pixel[0], pixel[1], pixel[2]),
                ImagePixelFormat::Bgra8 => (pixel[2], pixel[1], pixel[0]),
            };
            let opaque = image.alpha_mode == ImageAlphaMode::Opaque || pixel[3] == 255;
            if opaque && coverage == 1.0 {
                let destination = (dy as usize * raster.width + dx as usize) * 4;
                raster.pixels[destination..destination + 4].copy_from_slice(&[r, g, b, 255]);
            } else if coverage == 1.0 && pixel == flat_pixel && table.is_some() {
                let table = table.as_ref().unwrap();
                let destination = (dy as usize * raster.width + dx as usize) * 4;
                for (channel, byte) in raster.pixels[destination..destination + 4].iter_mut().enumerate() {
                    *byte = table[channel][*byte as usize];
                }
            } else {
                let alpha = if image.alpha_mode == ImageAlphaMode::Opaque {
                    clip_amount
                } else { f32::from(pixel[3]) / 255.0 * clip_amount };
                let scale = if image.alpha_mode == ImageAlphaMode::Straight {
                    alpha
                } else { clip_amount };
                raster.blend_linear_premultiplied(x, y,
                    [srgb_decode_byte(r) * scale, srgb_decode_byte(g) * scale,
                     srgb_decode_byte(b) * scale], alpha);
            }
        }
    }
    true
}

// Unit-coverage rows need no per-pixel coordinates or contour tests. Copy opaque
// client pixels and blend uniform frame interiors in contiguous destination slices.
fn draw_flat_span(
    raster: &mut RasterTarget<'_>, image: &SoftwareImage, rect: RectF,
    left: i32, right: i32, y: i32, flat_pixel: &[u8], table: Option<&[[u8; 256]; 4]>,
) {
    let dx = left.saturating_add(raster.origin.x).max(0) as usize;
    let end = (right.saturating_add(raster.origin.x).max(0) as usize).min(raster.width);
    let dy = y.saturating_add(raster.origin.y);
    if dx >= end || dy < 0 || dy as usize >= raster.height { return; }
    let sx = (left - rect.x as i32) as usize
        + left.saturating_add(raster.origin.x).min(0).unsigned_abs() as usize;
    let sy = (y - rect.y as i32) as usize;
    let offset = (sy * image.extent.width as usize + sx) * 4;
    let source = &image.pixels[offset..offset + (end - dx) * 4];
    let opaque_bytes = source.chunks_exact(4).all(|pixel| pixel[3] == 255);
    let opaque = image.alpha_mode == ImageAlphaMode::Opaque || opaque_bytes;
    let repeated = !opaque && table.is_some() && source.chunks_exact(4).all(|pixel| pixel == flat_pixel);
    if !opaque && !repeated {
        for (index, pixel) in source.chunks_exact(4).enumerate() {
            let rgb = match image.pixel_format {
                ImagePixelFormat::Rgba8 => [pixel[0], pixel[1], pixel[2]],
                ImagePixelFormat::Bgra8 => [pixel[2], pixel[1], pixel[0]],
            };
            let offset = (dy as usize * raster.width + dx + index) * 4;
            if pixel[3] == 255 {
                raster.pixels[offset..offset + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            } else if let Some(table) = table.filter(|_| pixel == flat_pixel) {
                for channel in 0..4 {
                    let byte = &mut raster.pixels[offset + channel];
                    *byte = table[channel][*byte as usize];
                }
            } else {
                let alpha = f32::from(pixel[3]) / 255.0;
                let scale = if image.alpha_mode == ImageAlphaMode::Straight { alpha } else { 1.0 };
                raster.blend_covered_linear_premultiplied(
                    (dx + index) as i32 - raster.origin.x, y,
                    rgb.map(|byte| srgb_decode_byte(byte) * scale), alpha,
                );
            }
        }
        return;
    }
    let offset = (dy as usize * raster.width + dx) * 4;
    let destination = &mut raster.pixels[offset..offset + source.len()];
    if opaque_bytes && image.pixel_format == ImagePixelFormat::Rgba8 {
        destination.copy_from_slice(source);
    } else if opaque {
        match image.pixel_format {
            ImagePixelFormat::Rgba8 => {
                for (dst, src) in destination.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
                    dst.copy_from_slice(&[src[0], src[1], src[2], 255]);
                }
            }
            ImagePixelFormat::Bgra8 => {
                for (dst, src) in destination.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
                    dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
                }
            }
        }
    } else {
        let table = table.unwrap();
        for pixel in destination.chunks_exact_mut(4) {
            for channel in 0..4 { pixel[channel] = table[channel][pixel[channel] as usize]; }
        }
    }
}

pub(super) fn draw_image(
    raster: &mut RasterTarget<'_>,
    instance: &ImageInstance,
    spatial: Option<&RenderSpatialNode>,
    clip: Option<&RenderClip>,
    region: RectF,
    image: &SoftwareImage,
) {
    if spatial.is_none_or(|node| node.transform == crate::foundation::Affine2D::IDENTITY)
        && draw(raster, instance, clip, region, image)
    {
        return;
    }
    draw_image_reference(raster, instance, spatial, clip, region, image);
}

fn draw_image_reference(
    raster: &mut RasterTarget<'_>,
    instance: &ImageInstance,
    spatial: Option<&RenderSpatialNode>,
    clip: Option<&RenderClip>,
    region: RectF,
    image: &SoftwareImage,
) {
    let transform = spatial.map_or(crate::foundation::Affine2D::IDENTITY, |value| value.transform);
    let Some(inverse) = transform.inverse() else {
        return;
    };
    let rect = transform.transform_rect(instance.rect);
    let Some(target) = clip_to_target(
        intersect(
            intersect(rect, clip.map_or(region, |clip| clip.rect)),
            region,
        ),
        raster.width,
        raster.height,
    ) else {
        return;
    };
    for y in target.y.floor() as i32..target.bottom().ceil() as i32 {
        for x in target.x.floor() as i32..target.right().ceil() as i32 {
            let point_x = x as f32 + 0.5;
            let point_y = y as f32 + 0.5;
            let clip_amount = raster.normalize_coverage(clip_coverage(point_x, point_y, clip).min(raster.interior_paint_coverage(x, y)), x, y);
            if clip_amount <= 0.0 {
                continue;
            }
            let local = inverse.transform_point(crate::foundation::PointF {
                x: point_x,
                y: point_y,
            });
            let u = (local.x - instance.rect.x) / instance.rect.width;
            let v = (local.y - instance.rect.y) / instance.rect.height;
            let sampled = sample_image_linear(
                &image.pixels,
                image.extent,
                image.color_encoding,
                image.pixel_format,
                u,
                v,
            );
            let opacity = instance.opacity.clamp(0.0, 1.0) * clip_amount;
            if let Some(tint) = instance.tint {
                let source_alpha = match image.alpha_mode {
                    ImageAlphaMode::Opaque => 1.0,
                    ImageAlphaMode::Straight | ImageAlphaMode::Premultiplied => sampled[3],
                };
                let alpha = source_alpha * (f32::from(tint.a) / 255.0) * opacity;
                raster.blend_linear_premultiplied(
                    x,
                    y,
                    [
                        srgb_decode_byte(tint.r) * alpha,
                        srgb_decode_byte(tint.g) * alpha,
                        srgb_decode_byte(tint.b) * alpha,
                    ],
                    alpha,
                );
                continue;
            }
            let alpha = match image.alpha_mode {
                ImageAlphaMode::Opaque => opacity,
                ImageAlphaMode::Straight | ImageAlphaMode::Premultiplied => sampled[3] * opacity,
            };
            let rgb_scale = match image.alpha_mode {
                ImageAlphaMode::Straight => alpha,
                ImageAlphaMode::Premultiplied => opacity,
                ImageAlphaMode::Opaque => opacity,
            };
            raster.blend_linear_premultiplied(
                x,
                y,
                [
                    sampled[0] * rgb_scale,
                    sampled[1] * rgb_scale,
                    sampled[2] * rgb_scale,
                ],
                alpha,
            );
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::PointI;
    use crate::graphics::render::RoundedClip;
    use crate::ui::layout::{ClipId, SpatialId};

    #[test]
    fn exact_images_match_reference_with_alpha_formats_damage_and_rounded_placement() {
        for format in [ImagePixelFormat::Rgba8, ImagePixelFormat::Bgra8] {
            for alpha in [ImageAlphaMode::Opaque, ImageAlphaMode::Straight, ImageAlphaMode::Premultiplied] {
                let image = SoftwareImage {
                    extent: SizeI { width: 80, height: 60 }, color_encoding: ImageColorEncoding::Srgb,
                    alpha_mode: alpha, pixel_format: format,
                    pixels: (0..4800).flat_map(|i| [(i * 3) as u8, (i * 2) as u8, i as u8, [0, 127, 254, 255][i as usize % 4]]).collect(),
                };
                let rect = RectF { x: 1.0, y: 1.0, width: 80.0, height: 60.0 };
                let instance = ImageInstance {
                    node: crate::ui::UiNodeId::new(0, 1), image: ImageId(1), rect,
                    view_bounds: rect, content_version: 1, opacity: 1.0, tint: None,
                    clip: ClipId(0), spatial: SpatialId(0),
                };
                for rounded in [false, true] {
                    let mut fast = vec![83; 84 * 64 * 4];
                    let mut reference = fast.clone();
                    let clips = if rounded {
                        [Some(RoundedClip {
                            rect: RectF { x: 3.0, y: 2.0, width: 80.0, height: 60.0 },
                            radii: crate::ui::CornerRadii::all(2.0), inverted: false,
                        }), None]
                    } else { [None; 2] };
                    fn raster(pixels: &mut [u8], clips: [Option<RoundedClip>; 2]) -> RasterTarget<'_> {
                        RasterTarget {
                            coverage_normalization: None, pixels, width: 84, height: 64,
                            origin: PointI { x: 2, y: 1 }, blend_mode: BlendMode::Alpha,
                            color_space: ColorSpace::Srgb, rounded_clips: clips,
                        }
                    }
                    let damage = RectF { x: 2.0, y: 1.0, width: 75.0, height: 55.0 };
                    draw_image(&mut raster(&mut fast, clips), &instance, None, None, damage, &image);
                    draw_image_reference(&mut raster(&mut reference, clips), &instance, None, None, damage, &image);
                    assert_eq!(fast, reference, "{format:?} {alpha:?} rounded={rounded}");
                }
            }
        }
    }
}

#[cfg(test)]
mod flat_tests {
    use super::*;
    use crate::ui::layout::{ClipId, SpatialId};

    #[test]
    fn opaque_rows_preserve_formats_padding_origin_and_damage() {
        for alpha in [ImageAlphaMode::Opaque, ImageAlphaMode::Straight, ImageAlphaMode::Premultiplied] {
            for format in [ImagePixelFormat::Rgba8, ImagePixelFormat::Bgra8] {
                for origin in [crate::foundation::PointI { x: -9, y: -3 }, crate::foundation::PointI { x: 7, y: 4 }] {
                    let image = SoftwareImage {
                        extent: SizeI { width: 80, height: 80 }, color_encoding: ImageColorEncoding::Srgb,
                        alpha_mode: alpha, pixel_format: format,
                        pixels: (0..6400).flat_map(|i| [(i * 73) as u8, (i * 37) as u8, i as u8,
                            if alpha == ImageAlphaMode::Opaque { i as u8 } else { 255 }]).collect(),
                    };
                    let rect = RectF { x: 1.0, y: 2.0, width: 80.0, height: 80.0 };
                    let instance = ImageInstance {
                        node: crate::ui::UiNodeId::new(0, 1), image: ImageId(1), rect, view_bounds: rect,
                        content_version: 1, opacity: 1.0, tint: None, clip: ClipId(0), spatial: SpatialId(0),
                    };
                    let clip = RenderClip { id: ClipId(0), rect: RectF { x: 2.25, y: 3.5, width: 77.0, height: 76.0 },
                        corner_radii: crate::ui::CornerRadii::all(3.0) };
                    let mut fast: Vec<u8> = (0..88 * 88 * 4).map(|i| (i * 73) as u8).collect();
                    let mut reference = fast.clone();
                    let raster = |pixels| RasterTarget {
                        coverage_normalization: None, pixels, width: 88, height: 88, origin,
                        blend_mode: BlendMode::Alpha, color_space: ColorSpace::Srgb, rounded_clips: [None; 2],
                    };
                    let damage = RectF { x: 4.25, y: 1.75, width: 72.5, height: 76.25 };
                    draw_image(&mut raster(&mut fast), &instance, None, Some(&clip), damage, &image);
                    draw_image_reference(&mut raster(&mut reference), &instance, None, Some(&clip), damage, &image);
                    assert_eq!(fast, reference, "{alpha:?} {format:?} origin={origin:?}");
                }
            }
        }
    }

    #[test]
    fn repeated_translucent_texels_match_reference_over_varying_destinations() {
        for alpha in [ImageAlphaMode::Straight, ImageAlphaMode::Premultiplied] {
            for format in [ImagePixelFormat::Rgba8, ImagePixelFormat::Bgra8] {
                let image = SoftwareImage {
                    extent: SizeI { width: 80, height: 80 }, color_encoding: ImageColorEncoding::Srgb,
                    alpha_mode: alpha, pixel_format: format,
                    pixels: [18, 27, 41, 150].repeat(80 * 80),
                };
                let rect = RectF { x: 0.0, y: 0.0, width: 80.0, height: 80.0 };
                let instance = ImageInstance {
                    node: crate::ui::UiNodeId::new(0, 1), image: ImageId(1), rect, view_bounds: rect,
                    content_version: 1, opacity: 1.0, tint: None, clip: ClipId(0), spatial: SpatialId(0),
                };
                let mut fast: Vec<u8> = (0..80 * 80 * 4).map(|i| (i * 73) as u8).collect();
                let mut reference = fast.clone();
                fn raster(pixels: &mut [u8]) -> RasterTarget<'_> {
                    RasterTarget {
                        coverage_normalization: None, pixels, width: 80, height: 80,
                        origin: Default::default(), blend_mode: BlendMode::Alpha, color_space: ColorSpace::Srgb,
                        rounded_clips: [Some(crate::graphics::render::RoundedClip::new(
                            RectF { x: 0.0, y: 0.0, width: 80.0, height: 80.0 },
                            crate::ui::CornerRadii::all(3.0))), None],
                    }
                }
                draw_image(&mut raster(&mut fast), &instance, None, None, rect, &image);
                draw_image_reference(&mut raster(&mut reference), &instance, None, None, rect, &image);
                assert_eq!(fast, reference, "{alpha:?} {format:?}");
            }
        }
    }
}
