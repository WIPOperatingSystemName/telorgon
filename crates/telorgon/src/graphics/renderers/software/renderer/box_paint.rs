use super::*;

pub(super) fn draw_box(
    raster: &mut RasterTarget<'_>,
    instance: &BoxInstance,
    spatial: Option<&RenderSpatialNode>,
    clip: Option<&RenderClip>,
    region: RectF,
) {
    draw_box_impl(raster, instance, spatial, clip, region, true);
}

fn draw_box_impl(
    raster: &mut RasterTarget<'_>,
    instance: &BoxInstance,
    spatial: Option<&RenderSpatialNode>,
    clip: Option<&RenderClip>,
    region: RectF,
    fast_interior: bool,
) {
    #[cfg(target_os = "uefi")]
    if uefi_fill::try_draw_box(raster, instance, spatial, clip, region) {
        return;
    }
    let transform = spatial.map_or(crate::foundation::Affine2D::IDENTITY, |value| value.transform);
    let Some(inverse) = transform.inverse() else {
        return;
    };
    let scale = (
        transform.m11.hypot(transform.m12),
        transform.m21.hypot(transform.m22),
    );
    let scale_min = scale.0.min(scale.1).max(f32::EPSILON);
    let outline_extent = (instance.outline.offset + instance.outline.width).max(0.0);
    let mut visual_bounds = transform.transform_rect(outset_rect(instance.rect, outline_extent));
    for shadow in instance.shadows.as_slice() {
        let reach = shadow.spread + shadow.blur * 2.0;
        let shadow_rect = transform.transform_rect(RectF {
            x: instance.rect.x + shadow.offset.x - reach,
            y: instance.rect.y + shadow.offset.y - reach,
            width: instance.rect.width + reach * 2.0,
            height: instance.rect.height + reach * 2.0,
        });
        visual_bounds = union_rect(visual_bounds, shadow_rect);
    }
    let bounds = intersect(
        intersect(visual_bounds, clip.map_or(region, |clip| clip.rect)),
        region,
    );
    let Some(bounds) = clip_to_target(bounds, raster.width, raster.height) else {
        return;
    };
    let radii = [
        instance.corner_radii.top_left,
        instance.corner_radii.top_right,
        instance.corner_radii.bottom_right,
        instance.corner_radii.bottom_left,
    ];
    let border_widths = [
        instance.border.top.width,
        instance.border.right.width,
        instance.border.bottom.width,
        instance.border.left.width,
    ];
    // An inverted placement masks its fully covered interior, including shadows. Skip
    // that whole row span; retain a conservative AA band for the ordinary coverage path.
    let masked_interior = fast_interior.then(|| raster.rounded_clips.iter().flatten()
        .find(|clip| clip.inverted).map(|clip| {
            let inset = clip.radii.top_left.max(clip.radii.top_right)
                .max(clip.radii.bottom_right).max(clip.radii.bottom_left) + 2.0;
            inset_asymmetric(RectF {
                x: clip.rect.x - raster.origin.x as f32,
                y: clip.rect.y - raster.origin.y as f32,
                ..clip.rect
            }, [inset; 4])
        })).flatten();
    let shadow_only = raster.blend_mode == BlendMode::Alpha
        && instance.background.is_none()
        && border_widths.iter().all(|width| *width <= 0.0)
        && instance.outline.width <= 0.0;
    // Only the edge bands need rounded fill/border geometry. The middle has one
    // constant source color, whose exact source-over results can be cached by destination byte.
    let interior = inset_asymmetric(instance.rect, border_widths);
    let inset = inset_radii(radii, border_widths).into_iter().fold(0.0_f32, f32::max) + 1.0;
    let mut flat = inset_asymmetric(interior, [inset; 4]);
    let mut fast_interior = fast_interior && transform == crate::foundation::Affine2D::IDENTITY
        && instance.shadows.as_slice().is_empty() && instance.outline.width <= 0.0
        && matches!(raster.blend_mode, BlendMode::Alpha | BlendMode::Opaque)
        && instance.opacity.is_finite();
    if fast_interior {
        let safe_rect = |rect: RectF, radii: crate::ui::CornerRadii, extra: f32| {
            let inset = radii.top_left.max(radii.top_right).max(radii.bottom_right)
                .max(radii.bottom_left) + extra;
            inset_asymmetric(rect, [inset; 4])
        };
        if let Some(clip) = clip {
            flat = intersect(flat, safe_rect(clip.rect, clip.corner_radii, 1.0));
        }
        for clip in raster.rounded_clips.iter().flatten() {
            if clip.inverted { fast_interior = false; break; }
            let rect = RectF {
                x: clip.rect.x - raster.origin.x as f32,
                y: clip.rect.y - raster.origin.y as f32,
                ..clip.rect
            };
            flat = intersect(flat, safe_rect(rect, clip.radii, 1.0));
        }
        if let Some(paint) = raster.coverage_normalization {
            // Include the extra protected AA band used for descendant interior paint.
            fast_interior &= !paint.contour.inverted;
            flat = intersect(flat, safe_rect(paint.contour.rect, paint.contour.radii, 2.0));
        }
        fast_interior &= flat.width * flat.height >= 4096.0;
    }
    let color = instance.background.unwrap_or_default();
    let alpha = f32::from(color.a) / 255.0 * instance.opacity.clamp(0.0, 1.0);
    let overwrite = raster.blend_mode == BlendMode::Opaque && raster.rounded_clips.iter().all(Option::is_none);
    let table = (fast_interior && (alpha > 0.0 || overwrite)).then(|| {
        let inverse = 1.0 - alpha;
        let rgb = [color.r, color.g, color.b].map(|v| srgb_decode_byte(v) * alpha);
        std::array::from_fn::<_, 4, _>(|channel| std::array::from_fn::<_, 256, _>(|byte| {
            if channel == 3 {
                ((alpha + if overwrite { 0.0 } else { byte as f32 / 255.0 * inverse })
                    .clamp(0.0, 1.0) * 255.0).round() as u8
            } else {
                encode_target_channel(rgb[channel] + if overwrite { 0.0 } else {
                    decode_target_channel(byte as u8, raster.color_space) * inverse
                }, raster.color_space)
            }
        }))
    });
    let uniform: Option<[u8; 4]> = table.as_ref().filter(|_| alpha == 1.0 || overwrite)
        .map(|table| std::array::from_fn(|channel| table[channel][0]));
    for y in bounds.y.floor() as i32..bounds.bottom().ceil() as i32 {
        let end = bounds.right().ceil() as i32;
        let mut next_x = bounds.x.floor() as i32;
        while next_x < end {
            let x = next_x;
            next_x += 1;
            let point_x = x as f32 + 0.5;
            let point_y = y as f32 + 0.5;
            if let Some(mask) = masked_interior.filter(|rect| rect.contains(
                crate::foundation::PointF { x: point_x, y: point_y }))
            {
                next_x = ((mask.right() - 0.5).ceil() as i32).min(end).max(next_x);
                continue;
            }
            if fast_interior && flat.contains(crate::foundation::PointF { x: point_x, y: point_y })
            {
                next_x = ((flat.right() - 0.5).ceil() as i32).min(end).max(next_x);
                if let Some(table) = &table {
                    let start = x.saturating_add(raster.origin.x).max(0) as usize;
                    let end = next_x.saturating_add(raster.origin.x).max(0) as usize;
                    let end = end.min(raster.width);
                    let dy = y.saturating_add(raster.origin.y);
                    if start < end && dy >= 0 && (dy as usize) < raster.height {
                        let row = dy as usize * raster.width;
                        let pixels = &mut raster.pixels[(row + start) * 4..(row + end) * 4];
                        if let Some(color) = uniform {
                            for pixel in pixels.chunks_exact_mut(4) { pixel.copy_from_slice(&color); }
                        } else {
                            for pixel in pixels.chunks_exact_mut(4) {
                                for channel in 0..4 {
                                    pixel[channel] = table[channel][pixel[channel] as usize];
                                }
                            }
                        }
                    }
                }
                continue;
            }
            let mut clip_amount = clip_coverage(point_x, point_y, clip);
            if raster.coverage_normalization.is_some_and(|paint| paint.root != instance.node) {
                clip_amount = clip_amount.min(raster.interior_paint_coverage(x, y));
            }
            if clip_amount <= 0.0 {
                continue;
            }
            let shadow_placement = if instance.shadows.as_slice().is_empty() { 0.0 } else {
                raster.placement_coverage_with_shadow_overlap(x, y, true)
            };
            // Inverted placement clips mask the window interior out of a shadow-only layer.
            // Skip it before evaluating each shadow and the unpainted fill/border geometry.
            if shadow_only && shadow_placement <= 0.0 { continue; }
            let local = inverse.transform_point(crate::foundation::PointF {
                x: point_x,
                y: point_y,
            });
            for shadow in instance.shadows.as_slice().iter().rev().filter(|_| shadow_placement > 0.0) {
                let coverage =
                    shadow_coverage(local.x, local.y, instance.rect, radii, *shadow, scale_min);
                if coverage > 0.0 {
                    let alpha = f32::from(shadow.color.a) / 255.0
                        * (coverage * instance.opacity * clip_amount).clamp(0.0, 1.0)
                        * shadow_placement;
                    let rgb = [shadow.color.r, shadow.color.g, shadow.color.b]
                        .map(|channel| srgb_decode_byte(channel) * alpha);
                    raster.blend_covered_linear_premultiplied(x, y, rgb, alpha);
                }
            }

            if shadow_only { continue; }
            let outline_width = instance.outline.width.max(0.0);
            if outline_width > 0.0 {
                let offset = instance.outline.offset;
                let outer = rounded_coverage(
                    local.x,
                    local.y,
                    outset_rect(instance.rect, offset + outline_width),
                    add_radii(radii, offset + outline_width),
                    scale_min,
                );
                let inner = rounded_coverage(
                    local.x,
                    local.y,
                    outset_rect(instance.rect, offset),
                    add_radii(radii, offset),
                    scale_min,
                );
                let coverage = (outer - inner).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    raster.blend_srgba(
                        x,
                        y,
                        instance.outline.color,
                        coverage * instance.opacity * clip_amount,
                    );
                }
            }

            // Intersect geometric coverage before applying material alpha. Reapplying a
            // matching rounded frame clip to an antialiased box would square its coverage.
            let body_clip = clip_amount.min(raster.placement_coverage(x, y));
            let outer =
                rounded_coverage(local.x, local.y, instance.rect, radii, scale_min).min(body_clip);
            if outer <= 0.0 {
                continue;
            }
            let inner_rect = inset_asymmetric(instance.rect, border_widths);
            let inner_radii = inset_radii(radii, border_widths);
            let inner =
                rounded_coverage(local.x, local.y, inner_rect, inner_radii, scale_min).min(outer);
            let outer = raster.normalize_coverage(outer, x, y);
            let inner = raster.normalize_coverage(inner, x, y).min(outer);
            // Fill and border partition one shape's coverage. Sum their premultiplied
            // contributions before source-over; blending them separately opens an alpha seam.
            let ring = (outer - inner).clamp(0.0, 1.0);
            let border_color = border_color_at(
                local.x - instance.rect.x,
                local.y - instance.rect.y,
                instance.rect.width,
                instance.rect.height,
                border_widths,
                instance.border,
            );
            let mut rgb = [0.0; 3];
            let mut alpha = 0.0;
            for (color, coverage) in [
                (instance.background.unwrap_or_default(), inner),
                (border_color, ring),
            ] {
                let amount =
                    f32::from(color.a) / 255.0 * coverage * instance.opacity.clamp(0.0, 1.0);
                alpha += amount;
                for (channel, value) in rgb.iter_mut().zip([color.r, color.g, color.b]) {
                    *channel += srgb_decode_byte(value) * amount;
                }
            }
            raster.blend_covered_linear_premultiplied(x, y, rgb, alpha);
        }
    }
}

fn shadow_coverage(
    x: f32,
    y: f32,
    rect: RectF,
    radii: [f32; 4],
    shadow: Shadow,
    scale: f32,
) -> f32 {
    let spread = shadow.spread;
    let shifted = outset_rect(
        RectF {
            x: rect.x + shadow.offset.x,
            y: rect.y + shadow.offset.y,
            ..rect
        },
        spread,
    );
    let distance = rounded_signed_distance(x, y, shifted, add_radii(radii, spread));
    let blur = shadow.blur.max(0.0);
    if blur <= f32::EPSILON {
        (0.5 - distance * scale).clamp(0.0, 1.0)
    } else {
        (0.5 - distance / (blur * 2.0 + 1.0 / scale)).clamp(0.0, 1.0)
    }
}

fn border_color_at(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    widths: [f32; 4],
    border: Border,
) -> ColorRgba8 {
    let ratio = |distance: f32, width: f32| {
        if width > 0.0 {
            distance / width
        } else {
            f32::INFINITY
        }
    };
    let candidates = [
        (ratio(y, widths[0]), border.top.color),
        (ratio(width - x, widths[1]), border.right.color),
        (ratio(height - y, widths[2]), border.bottom.color),
        (ratio(x, widths[3]), border.left.color),
    ];
    candidates
        .into_iter()
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map(|item| item.1)
        .unwrap_or_default()
}

pub(super) fn rounded_coverage(x: f32, y: f32, rect: RectF, radii: [f32; 4], scale: f32) -> f32 {
    if radii.iter().all(|r| *r <= 0.0) {
        let half = 0.5 / scale.max(1e-4);
        let horizontal = ((x + half).min(rect.right()) - (x - half).max(rect.x)).max(0.0) * scale;
        let vertical = ((y + half).min(rect.bottom()) - (y - half).max(rect.y)).max(0.0) * scale;
        return horizontal.clamp(0.0, 1.0) * vertical.clamp(0.0, 1.0);
    }
    (0.5 - rounded_signed_distance(x, y, rect, radii) * scale).clamp(0.0, 1.0)
}

fn rounded_signed_distance(x: f32, y: f32, rect: RectF, radii: [f32; 4]) -> f32 {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return f32::INFINITY;
    }
    let local_x = x - rect.x;
    let local_y = y - rect.y;
    let radius = if local_x < rect.width * 0.5 {
        if local_y < rect.height * 0.5 {
            radii[0]
        } else {
            radii[3]
        }
    } else if local_y < rect.height * 0.5 {
        radii[1]
    } else {
        radii[2]
    }
    .clamp(0.0, rect.width.min(rect.height) * 0.5);
    let center_x = rect.width * 0.5;
    let center_y = rect.height * 0.5;
    let qx = (local_x - center_x).abs() - (center_x - radius);
    let qy = (local_y - center_y).abs() - (center_y - radius);
    let outside = if qx > 0.0 && qy > 0.0 { qx.hypot(qy) }
        else { qx.max(0.0) + qy.max(0.0) };
    outside + qx.max(qy).min(0.0) - radius
}

fn outset_rect(rect: RectF, amount: f32) -> RectF {
    RectF {
        x: rect.x - amount,
        y: rect.y - amount,
        width: (rect.width + amount * 2.0).max(0.0),
        height: (rect.height + amount * 2.0).max(0.0),
    }
}

fn inset_asymmetric(rect: RectF, widths: [f32; 4]) -> RectF {
    RectF {
        x: rect.x + widths[3],
        y: rect.y + widths[0],
        width: (rect.width - widths[3] - widths[1]).max(0.0),
        height: (rect.height - widths[0] - widths[2]).max(0.0),
    }
}

fn add_radii(mut radii: [f32; 4], amount: f32) -> [f32; 4] {
    for radius in &mut radii {
        *radius = (*radius + amount).max(0.0);
    }
    radii
}

fn inset_radii(radii: [f32; 4], widths: [f32; 4]) -> [f32; 4] {
    [
        (radii[0] - widths[0].max(widths[3])).max(0.0),
        (radii[1] - widths[0].max(widths[1])).max(0.0),
        (radii[2] - widths[2].max(widths[1])).max(0.0),
        (radii[3] - widths[2].max(widths[3])).max(0.0),
    ]
}

fn union_rect(first: RectF, second: RectF) -> RectF {
    let left = first.x.min(second.x);
    let top = first.y.min(second.y);
    let right = first.right().max(second.right());
    let bottom = first.bottom().max(second.bottom());
    RectF {
        x: left,
        y: top,
        width: (right - left).max(0.0),
        height: (bottom - top).max(0.0),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_shadows_and_empty_border_interiors_match_reference() {
        use crate::graphics::render::RoundedClip;
        use crate::ui::{CornerRadii, ShadowList};
        use crate::ui::layout::{ClipId, SpatialId};
        for shadow in [false, true] {
            for origin in [crate::foundation::PointI { x: -7, y: -3 }, crate::foundation::PointI { x: 2, y: 1 }] {
                let rect = RectF { x: 8.25, y: 9.5, width: 92.0, height: 86.0 };
                let radii = CornerRadii { top_left: 4.0, top_right: 7.0, bottom_right: 2.0, bottom_left: 5.0 };
                let instance = BoxInstance {
                    node: crate::ui::UiNodeId::new(0, 1), rect, view_bounds: rect,
                    background: None,
                    border: if shadow { Default::default() } else {
                        crate::ui::Border::all(1.5, ColorRgba8::rgba(60, 70, 90, 160))
                    },
                    outline: Default::default(), corner_radii: radii,
                    shadows: if shadow { ShadowList::one(Shadow {
                        offset: crate::foundation::PointF { x: 2.0, y: 3.0 }, blur: 5.0, spread: 1.0,
                        color: ColorRgba8::rgba(12, 18, 24, 140),
                    }) } else { Default::default() },
                    opacity: 0.7, clip: ClipId(0), spatial: SpatialId(0),
                };
                let clips = [Some(RoundedClip {
                    rect: RectF { x: rect.x + origin.x as f32, y: rect.y + origin.y as f32, ..rect },
                    radii, inverted: shadow,
                }), None];
                let mut fast: Vec<u8> = (0..112 * 108 * 4).map(|i| (i * 73) as u8).collect();
                let mut reference = fast.clone();
                let before = fast.clone();
                let raster = |pixels| RasterTarget {
                    coverage_normalization: None, pixels, width: 112, height: 108, origin,
                    blend_mode: BlendMode::Alpha, color_space: ColorSpace::Srgb, rounded_clips: clips,
                };
                let damage = RectF { x: 3.5, y: 2.25, width: 104.0, height: 102.0 };
                draw_box_impl(&mut raster(&mut fast), &instance, None, None, damage, true);
                draw_box_impl(&mut raster(&mut reference), &instance, None, None, damage, false);
                assert_ne!(fast, before);
                assert_eq!(fast, reference, "shadow={shadow} origin={origin:?}");
            }
        }
    }

    #[test]
    fn uniform_interior_matches_reference_with_transparency_border_clip_and_damage() {
        use crate::graphics::render::RoundedClip;
        use crate::ui::layout::{ClipId, SpatialId};
        for alpha in [0, 1, 127, 254, 255] {
            for opacity in [0.0, 0.4, 1.0] {
                let rect = RectF { x: 1.25, y: 2.0, width: 92.0, height: 86.0 };
                let instance = BoxInstance {
                    node: crate::ui::UiNodeId::new(0, 1), rect, view_bounds: rect,
                    background: Some(ColorRgba8::rgba(18, 27, 41, alpha)),
                    border: crate::ui::Border::all(1.5, ColorRgba8::rgba(60, 70, 90, 160)),
                    outline: Default::default(), corner_radii: crate::ui::CornerRadii::all(3.0),
                    shadows: Default::default(), opacity, clip: ClipId(0), spatial: SpatialId(0),
                };
                let paint = crate::graphics::render::frame_border::InteriorPaint::new(&instance);
                for mode in [BlendMode::Alpha, BlendMode::Opaque] {
                for rounded in [false, true] {
                for normalization in [None, Some(paint), Some(crate::graphics::render::frame_border::InteriorPaint {
                    root: crate::ui::UiNodeId::new(1, 1), ..paint
                })] {
                let mut fast: Vec<u8> = (0..100 * 96 * 4).map(|i| (i * 73) as u8).collect();
                let mut reference = fast.clone();
                fn raster(pixels: &mut [u8], rect: RectF, normalization: Option<crate::graphics::render::frame_border::InteriorPaint>, mode: BlendMode, rounded: bool) -> RasterTarget<'_> {
                    RasterTarget {
                        coverage_normalization: normalization, pixels, width: 100, height: 96,
                        origin: crate::foundation::PointI { x: 2, y: 1 }, blend_mode: mode,
                        color_space: ColorSpace::Srgb,
                        rounded_clips: [rounded.then(|| RoundedClip::new(rect, crate::ui::CornerRadii::all(4.0))), None],
                    }
                }
                let damage = RectF { x: 3.0, y: 1.0, width: 88.0, height: 86.0 };
                draw_box_impl(&mut raster(&mut fast, rect, normalization, mode, rounded), &instance, None, None, damage, true);
                draw_box_impl(&mut raster(&mut reference, rect, normalization, mode, rounded), &instance, None, None, damage, false);
                assert_eq!(fast, reference, "alpha={alpha} opacity={opacity} mode={mode:?} rounded={rounded}");
                }
                }
                }
            }
        }
    }
}
